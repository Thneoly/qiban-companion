# T11：语音轮次进入桌宠聊天链

2026-10-07。分支 `feat/voice-chat-turn`，接续[真实语音全链记录](voice-real-chain.md)。**桌宠「聊一聊」支持语音轮次：按住麦克风说话→停止→转写自动作为正式消息发送→回复流式分句、逐句合成顺序朗读，可随时打断；打断后迟到片段不播放、本轮不入历史。** 这是 T11 的工程交付；T12 口型与人工听感复核是下一增量（PR-B），不在此宣称。

## 三项既定决策（用户拍板）

1. **语音是正式聊天链的皮肤**：转写文本走现有 `chat_generate`，历史、应用/个人记忆注入、epoch 守卫、取消、恢复全部继承，`chat.rs` 零改动；历史只存文本，永不存音频。
2. **录音停止→转写→自动发送**：字幕行显示「我听到：…」后立即作为消息发出；发送被拒（文字回复进行中等守卫）时字幕保留，可手动改写重发。
3. **「du dudu」提示音**：首句保留（当作「栖栖要开口」的通知）；后续句由 Rust 侧按能量包络裁剪，裁剪不可靠时下一轮回退为整段单次合成（`first=true`）。

## 命令面（voice_companion.rs）

| 命令 | 行为 | 关键守卫 |
|---|---|---|
| `voice_transcribe` | multipart 上传整段 WAV，返回转写文本 | 配置自读（不接受前端传模型）、`expected_base_url` 不符拒、上传前 `wav_seconds(30)`、无密钥给指引文案 |
| `voice_speak` | 合成单句，**raw IPC 返回 WAV 字节**（仓库首例 `tauri::ipc::Response`），`trimmed` 标志经 Channel 先于 resolve 发出 | `seq≤64`、文本 trim 后 1..=500 字、输出过 `wav_seconds(60)` 失败落盘诊断、在途总数>12 拒新 |
| `voice_turn_cancel` | 取消该轮全部在途合成 | RAII 守卫按 (turn_id, request_id) 精确移除；执行体 `select!{biased}` 先看取消再看结果 |

三处注册铁律已核对：`build.rs` commands、`capabilities/pet.json`（`voice_transcribe`/`voice_speak`/`voice_turn_cancel`/`voice_settings_get`）、`lib.rs` invoke_handler + `manage(VoiceTurnState)`。`voice_settings_get` 扩展 `hasVoiceKey`（凭据管理器实查，桌宠据此 fail-closed 禁用麦克风并给指引）。语音实验室（VoiceLab）行为与 JSON 返回不动。

## 提示音裁剪与回退

- `trim_leading_tone`：20ms 帧 RMS 包络；帧分类静音/恒定段/语音段；仅当 1.7s 内开头三段恒定段的帧数落在实测区间（智谱 glm-tts 2026-10-07 测得 6/19/7 帧）且相邻隔≥3 帧静音时，裁到第三段结束界（前留 3 帧）；裁后重过 `wav_seconds`。任何不匹配原样返回 `trimmed=false`。
- 回退判据（仅证据驱动）：一轮后续句 ≥2 且未裁占比 ≥50% → 下一轮 `singleShot` 整段一次合成（>500 字按码点粗切）；后续句全部裁剪成功（≥1 句、0 未裁）则恢复分句；单句轮（裁剪未被尝试）不改变上一轮决定——避免「提示音消失又反复回归」的振荡。两态（裁剪/回退）都如实经 Channel 上报，不假装裁成功。
- 首句恒不裁：提示音保留为「栖栖要开口」通知，这是产品语义不是缺陷。

## 播放队列（useVoiceTurn）

- 双预取：当前句合成完成后预取下一句（合成藏在上句播放背后）；播放中途新句到达时 `enqueue` 立即预取队首未取句（流式主场景的真正延迟隐藏）。
- 每轮 `turn_id` + `seq` + 前端 epoch 三重守卫：epoch 不匹配的迟到合成直接丢弃，不播放。
- 打断 = `chat_cancel` + `voice_turn_cancel` + 停 Audio + epoch 递增；录音与朗读互斥（说话中按麦克风先打断再录）。
- 设备释放双时机：`MediaRecorder.onstop` 与 blur/打断路径都 `track.stop()`；blob URL 在每次播放 settle 后 `revokeObjectURL`。
- 分句 `sentences.ts` 纯函数：终止符 `。！？；!?;…\n`；ASCII `.` 仅在后跟空白时终止（`3.14`/`v2.10` 永不切、`Hello. This` 正常切，句尾 `.` 等后续 delta 补空白或最终 flush）；≥1 个内容字符才成句（纯标点句丢弃）；>80 字软切（就近 `，、,：:`），无则硬切；码点计数防代理对截断。

## 验证证据

- Rust：`voice_companion.rs` 5 组测试（注册表按轮取消+在途上限、transcribe 用已存配置+错误映射、speak 后续句裁剪+trimmed 进度、turn_cancel 停在途+注册表清空、非法载荷不发任何请求）+ `voice.rs` SpeechClient 提取后既有断言逐字保留全绿；`cargo fmt`/`clippy -D warnings`/`cargo test` 通过。
- 前端：`npm run check`（含 contracts 解码边界、`sentences.test.ts`）通过；`test:browser` 新增 `voice-chat.spec.ts` 9 例——未配置 fail-closed、录音一次上传+blur 取消+10s 上限、转写自动发送带记忆快照与 epoch、顺序播放+预取（blob URL 映射回句子序号钉死播放顺序，并用 ArrayBuffer 返回分支覆盖真机形态）、句间打断（park 时序）不残留并发播放循环、打断丢迟到+双 cancel+可开新轮、首句保留+裁剪标志+回退、单句失败不毒化下轮、字幕先于回复流。
- 对抗审查（四维：打断/迟到/释放语义、裁剪启发式与校验、raw IPC 与注册铁律、测试真实性）后的修复：interrupt() 唤醒 park 中的播放循环（否则旧循环迟到退出清掉 driving 标志，新一轮出现并发播放与 blob 泄漏，major；已加 park 时序回归，且用「只摘除 interrupt 侧唤醒」的探针验证过可回归——用例变红于 250ms 窗口内出现第二路播放）；聊天回调绑定轮次身份（旧轮迟到 settle 不再把新轮标记完成）；`finishRecording` 加录制态守卫；英文句点+空白分句；singleShot 回退改证据驱动；`voice_turn_cancel` 补 turn_id 校验；零请求负测改用本地 listener 证明零连接；native 脚本 fixture 断言改为主流程内重抛、轮询超时显式报错、补 chat_cancel 与 progress 形状断言、采集图改为手势后惰性建立。
- 真机：`tests/native/voice-chat.cjs` 已入库并于 2026-10-09 **真机运行通过**（8 用例全绿，标识 `m2-20261009d`，证据落 `.cache/`：wire.json/requests.json/两截图）。env 门禁：`QIBAN_ACCEPTANCE_EXE` + 独立验收标识、全新 profile、`instance.lock`+db 版本校验、CDP、本机 HTTP fixture 断言无凭据、假 gUM 不碰真实硬件。覆盖：pet 窗口 mic 可用性首断言（权限状态非 denied）、raw IPC 真机为 ArrayBuffer 且字节精确、全链自动发送（fixture 侧可见转写 multipart 21KB/句序合成）、打断、第二轮完整（队列预取在 wire 上直接可见 `-s0`+`-s1`）、轨道全 ended、无 blob 泄漏。**观察层改版**（三次调试的教训）：真机 WebView2 上 `window.__TAURI_INTERNALS__`/`window.ipc` 均为不可写注入，页面侧 invoke tap 赋值**静默失败**（browser mock 测不出）；改为 CDP 侧 `page.on('request')` 观察 `http://ipc.localhost/<command>` fetch（命令在 URL 路径、参数 JSON 在 postData）。打断证据相应升级为传输级：wire 上 `chat_cancel` 恰一条且 requestId 与本轮 `chat_generate` 精确配对、`voice_turn_cancel` 序列恰为探针+本轮；Rust drop 在途 future → fixture 服务端两条 held 连接（chat 流式 + tts 未响应头）均观测到 close——比页面侧「已停止」拒绝字符串更强（该拒绝语义由 Rust 单测钉住）。

## 边界（沿用计划风险清单）

1. ~~麦克风权限跨窗口继承未真机验证（最高风险）~~ **已验证**：native 首断言过——pet 窗口 `getUserMedia` 存在、权限状态 `'prompt'` 非 denied；真实授权弹窗与真实硬件录音仍留人工清单（脚本用假 gUM 不碰硬件）。
2. 裁剪是启发式非保证，两侧都有失配面：服务商特征变化（模型/音色/版本更新后三段布局漂移）→ 保留提示音并触发整段回退；内容侧——开头连续三段平稳延音（如风格化的「啊—— 嗯—— 哦——」）会被整体当提示音裁掉，上界 1.7s 真语音（静音计入最多 ~2.5s）。失配靠 trimmed 上报与回退两态如实可见，人工听感清单复核。
3. 取消语义有毫秒级窗口：`voice_speak` 在读取配置（凭据管理器）之后才注册进取消表，恰好在其间到达的 `voice_turn_cancel` 通知不到它——该次调用会完整跑完（≤60s、计费）后结果被前端丢弃。有界（每次竞争最多漏一个请求）；正确修复需 turn 级共享 watch（避免 subscribe-after-send 盲区），留待需要时做。
4. 费用：每轮 1 ASR + N TTS + 1 chat；本地丢弃不退款（「在途调用可能计费」如实呈现）；不发明语音单价，`audio_cost` 恒 null。上传方向 wav 走 JSON number[]（10s 上限约 1.3MB、30s 硬顶约 4MB，Tauri 无默认上限、实测几十 ms 级）；下行已是 raw Response——上行对称改 raw payload 属优化项未排期。
5. 首响预期 ~1.5-2.5s（首句短文本）；`stream:true` 不押注，未实现。
6. 一套 `voice_config` 两个入口：实验室可改可存，桌宠只读；`expected_base_url` 守卫作废跨配置在途轮次。多服务商档案不在本轮。
7. raw `ipc::Response` 为仓库首例：browser mock + 真机直连探针双层验证；异常时回退 JSON number[] 的封装点保留在 `toWav`。
8. 语音回复长度沿用 chat 链 `max_output_tokens`，长回复=多句 TTS，靠打断兜底。
9. ~~朗读中的轮次在 pet 窗口失焦时会随气泡收起而打断~~ 已按用户反馈（2026-10-09 听感复核）改为**说话中/转写中失焦不收气泡、朗读继续说完**（ChatBubble 向 Pet 上报 `onVoiceActive`，blur 豁免；卸载时复位防气泡卡死）。Esc、× 按钮、托盘安静/隐藏仍立即停（显式意图）；录音中失焦仍取消录音（半截录音无用，气泡随之收起）。
10. 回声场景仅 `echoCancellation` 请求项 + 人工清单待核；噪声/多说话人/多服务商矩阵仍无证据（T04 遗留边界不变）。

## 人工听感清单（2026-10-09 首轮复核反馈）

- **首句「du dudu」提示音听不到了**——代码链路审计排除产品侧回归（`first=true` 首句不裁剪有单测钉住、播放从头播完整 blob），2026-10-07 同链路实测存在；主嫌智谱服务端行为变化。**切分待办**：在任务面板「语音实验」直接合成一句——有声则问题在桌宠播放侧（回查），无声则确认服务端变化（产品再决定是否前端补提示音）。
- **失焦停止朗读体验不好**——已修复（见边界 9），说话中失焦继续说完。
- **嘴型对齐观感不佳**——用户决定暂缓（见 voice-mouth.md）。
- 仍待复核：裁剪成功率与回退模式听感（含延音开头是否被误裁）/ 句间停顿（预取 vs 实测每句 1.9-3.8s 合成）/ 外放回声时序残留 / 说话中收起气泡·安静模式·托盘退出的设备释放 / 账单 N 轮核对（沿用 V5 方法）。

接续：PR-B（T12 口型）已合并（PR #48/#49）；native 脚本真机已绿（含口型增项的人工听感仍待复核）；backlog 中 T11 从「真机验收待补」改「脚本化真机验收已过，人工听感待核」。
