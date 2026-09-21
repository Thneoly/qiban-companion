# 工程骨架与扩展约定

## 当前实现边界

```text
React页面 → CompanionClient → native invoke → Tauri commands
                                            → TaskStore(SQLite)
                                            → companion-core(领域校验)
         → browser preview（仅在非Tauri环境，独立内存）
```

Tauri命令提供运行信息、任务列表/创建/取消，以及角色窗口动作和交互区域上报。build.rs为命令生成权限，main与pet能力文件分别逐项授权：pet可读取运行信息、创建待办和操作角色，不可读取/取消任务；main保留任务管理入口。两个窗口仅使用本地页面。没有shell、任意文件系统、远程页面或HTTP插件权限。渲染层只处理用户显示与交互；输入在Rust再次校验，前端校验不能替代宿主校验。

任务创建使用UUID，毫秒时间戳及revision。SQLite schema v1由事务创建；发现更高数据库版本时拒绝启动，不做破坏性降级。取消事务读取并更新同一任务；队列取消可重复调用，revision仅第一次增加。运行中/结果未知任务不能调用队列取消宣称副作用结束。

普通待办仍为**本地待办仓库**。另有[受限文档执行台账](../status/local-document-execution.md)，在独立executions.db持久保存task/attempt/action及状态事件，显式确认后创建本机草稿并核验恢复。它没有多人资源隔离或云端授权，不能直接用于远程动作。

## 桌面呈现与窗口生命周期

默认 pet 窗口为320×440逻辑像素、透明、无系统装饰、置顶、不显示任务栏入口。主屏右下角定位完成、托盘安装成功后，由前端就绪命令显示角色。main 窗口预加载但默认隐藏，通过气泡或托盘显式打开并聚焦。启动和托盘恢复不主动调用 set_focus；前台应用焦点行为仍需扩充实机覆盖。

前端共享 AvatarArtwork，独立 Pet 组件负责角色、气泡及快捷待办；SurfaceRouter 区分角色入口与 ?view=panel。业务存储仍走 CompanionClient，不进入窗口管理模块。浏览器路由在同一页面内切换，临时待办可保留到刷新。

透明背景不等于鼠标穿透。前端上报角色、把手和气泡的可见矩形，Rust每40ms将系统光标的物理坐标按窗口位置与缩放换算为逻辑坐标，在矩形外设置原生忽略鼠标事件。按住左键时保留当前命中模式，支持原生拖动。此实现有轮询延迟，按矩形而非SVG像素命中；角色轮廓内部的透明小块仍可能接收点击。安静陪伴让整个窗口穿透并禁止聚焦，托盘恢复互动。

每约2秒检查窗口是否处于可用显示器工作区，必要时夹回边界；托盘提供回到主屏的入口。已有负坐标和超出边界的纯逻辑测试，尚未覆盖真实混合DPI与拔插显示器；位置持久化到宿主专用desktop-settings.db：移动后400ms防抖，后台每250ms检查保存，正常退出补存；启动加载后按当前工作区夹回可见范围。偏好存储失败保留原文件并回到默认位置，记录诊断。两个窗口收到关闭事件时隐藏，完全退出通过托盘完成。隐藏期间跳过窗口命中查询；常驻两个WebView的CPU、内存与功耗尚未量化。

窗口与托盘由 apps/desktop/src-tauri 负责，core/storage不依赖Tauri。实现依据[Tauri窗口定制](https://v2.tauri.app/learn/window-customization/)与[系统托盘](https://v2.tauri.app/learn/system-tray/)；实测与限制见[交付记录](../status/desktop-pet.md)。

## 模型配置与渲染增量

模型HTTP适配、配置SQLite与Windows凭据封装归桌面宿主，不反向进入companion-core/storage。main独占模型设置读写、原生密钥输入与删除命令；pet独占配置状态、生成和停止命令。密钥按完整API基地址保存，只由Rust读取；非密钥配置保存在model-settings.db。IPC不返回原始Key。

当前协议为可配置基地址与模型编码的Chat Completions SSE，智谱仅为默认预设。前端Channel逐请求接收增量并按requestId过滤迟到消息；宿主仅允许一个生成，锁内登记取消信号，退出时释放。停止先屏蔽旧回调，再通知Rust丢弃网络future；请求使用配置快照且不自动重试。收起气泡会停止当前生成，已完成问答保留在Rust进程内；不声称远端已撤销计费。输入、输出、片段、总响应、连接和整体耗时均有限制。

Conversation领域逻辑位于companion-core，HistoryStore位于companion-storage，宿主组装独立chat-history.db。所有模型合计最多6轮/12000个Unicode字符，仅事务保存完整问答；按规范化地址和模型载入上下文，切换不跨范围发送，重启可恢复。chat_history/chat_clear仅授权pet；清空删除所有范围的记录，生成中须先停止。会话版本使切换和删除后的迟到结果无法写回；historySaved区分生成完成与保存成功。前端不能注入system角色或任意历史；没有长期记忆。版本迁移、失败及删除边界见[本机记录验收](../status/chat-history-persistence.md)。

Live2DRenderer与待办/模型协议独立，按需加载本地Core、固定浏览器运行时与模型。默认SVG，异常回退；安静和文档隐藏停止绘制，角色隐藏卸载释放画布。30fps上限不代表资源门禁通过。CSP只允许本地脚本与所需WebAssembly编译；未开放远程页面、任意浏览器HTTP或工具执行。完整边界与资源授权见[配置说明](../development/model-settings-live2d.md)，实际检查见[本轮状态](../status/model-settings-live2d.md)。

阅读模式仍在pet窗口内，由前端切换气泡布局；原生窗口大小与权限不变，已有ResizeObserver上报变化后的命中区域。ConversationReader只显示记录及当前片段，用户上翻时停止跟随增量。ChatBubble把等待、流式中、成功、停止及失败分别显示；记录刷新失败不会改写已经成功的模型结果。详见[阅读增量](../status/chat-reading.md)。

ModelConfig新增带默认值的max_output_tokens，旧JSON缺字段仍按1024读取，保存前校验128～8192整数；ModelStore验证失败不落盘。ChatConfig只暴露数值，传输在请求开始时取快照并写入max_tokens。预算不参与会话身份，不会因调整预算清空记录；长度错误包含本次上限而非当前可能已变化的设置值。见[预算记录](../status/model-output-budget.md)。

## 首次使用偏好

T07指南嵌入pet气泡，首次点击才展示，不自动打开面板或调用模型。desktop-settings.db从1事务迁移到2，新增onboarding完成标志，保留pet_position；guide_status/guide_complete只授权pet。完成成功才隐藏指南，失败可重试或本次跳过；手动入口始终保留。独立验收配置仅覆盖应用标识，不新增路径注入IPC，详见[首次指南](../status/first-use-guide.md)。

## 有限记忆（本机M1～M3已实现）

T13/T14已支持手动本机记忆、JSON导出，以及按API地址和模型确认的使用许可。chat-history.db版本2保存记忆/许可与聊天，更正/删除/收回使用与清理聊天同事务；宿主协调单实例、版本冲突、取消和迟到结果过滤。发送采用协议2的scope/epoch预览校验，正文只放入user角色参考资料。详见[技术设计](limited-memory.md)与[M3实测边界](../status/memory-model-context.md)，完整Q6和独立验收未完成。

## 协议和变化

T04语音实验新增main独占的voice_probe/voice_cancel/voice_key_set，宿主voice模块持有独立单请求取消状态。前端显式录音或选择文件，点击运行后上传PCM16 WAV；Rust在固定配置快照上调用独立语音地址的ASR/TTS及既有文字地址的文本流解析，返回有界WAV与阶段用量。语音不读写聊天历史，也不执行工具；停止/失焦释放本地媒体并过滤旧结果。文字与语音凭据按各自地址隔离读取，语音密钥只经原生窗口输入；不增加前端外网权限；CSP允许本地blob媒体播放。参数、限制及真实429结果见[语音验证](../status/voice-chain-spike.md)。本机语音并发与文本并发独立，尚不是T45全局用量控制。

Rust的序列化对象与TypeScript共享包共同维护IPC v2，前端对未知状态、字段和协议版本拒绝解码。新增字段/状态需同时更新解码与边界测试；不兼容变化提升protocolVersion。当前不是自动生成类型，后续对象增多时再评估代码生成。

SQLite暂将领域对象保存为JSON及索引键，适合此小型骨架；需要检索/审计/执行时应新增规范化表和迁移，避免在JSON中拼接完整执行系统。存储接口只收领域参数，不收前端SQL和任意路径。手动记忆JSON导出与记忆/聊天本机删除已实现；账户、跨端和全产品数据导出/删除尚未实现，勿将开发版当成可公开试用版。

## 下一增量

1. T02/T03：透明宠物窗口、气泡收起和托盘基础已实现；下一步补齐混合DPI/多屏拔插、托盘实点回归、资源测量；位置持久化已加入，见[位置恢复实测](../status/pet-position-memory.md)。
2. T05/T16：执行事件及attempt/action模型，区分停止说话、取消请求、已取消和结果未知。
3. T41/T42/T43：独立账号与云适配、资源隔离、最小运行基础；服务端保管模型凭据。
4. 通用文本模型适配与本机配置已加入；本账号glm-5.3及glm-5.3-flash已有真实回复/停止证据，费用金额与稳定性仍待测量，再接PTT语音及角色状态。

手机前端可复用contracts，但必须经账号及设备授权，不能直接复制桌面invoke适配。角色呈现通过Avatar组件边界替换为经许可Live2D/VRM资产。Wasm仅在具体渲染/算法库需要时使用；WASI、摄像头和硬件保持后续独立增量。

本次骨架搭建是用户直接授权的实现工作，不表示产品计划门禁已通过；未启动任何外部招募、部署、交易或设备控制。
