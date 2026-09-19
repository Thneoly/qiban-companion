# T13/T14 M3：模型许可、预览与受控记忆上下文

2026-09-19，分支`feat/memory-model-context`，接续[M2记忆面板](memory-panel.md)。M3本机工程增量已实现；完整Q6质量集、真实模型表现和独立体验留到M4，不据此关闭T13/T14或G2。

## 用户现在能做什么

1. 点击桌宠 → “我们的记忆”，保存偏好或经历。
2. 在“让交流用上这些记忆”查看当前API地址和模型，显式启用、勾选并保存。每个地址＋模型独立，默认关闭；最多5条、正文合计800个Unicode字符，按勾选顺序使用。保存设置本身不发模型请求。
3. 回到“聊一聊”，展开“下次发送的记忆”核对正文、来源、经历日期与确认时间。界面区分正文预算和包含元数据的附加字符数，不把字符数当tokens或价格。
4. 请求收到服务端响应后显示“本轮已提交N条记忆”，可查看本轮条目、版本与接收范围。它表示实际请求快照，不证明模型引用了每条；该记录只在当前窗口保留，不新建正文诊断日志。

切换地址或模型不继承许可；切回旧范围可恢复已保存选择。新建条目不会自动加入任何选择。关闭、移除选择、更正或删除前确认清空全部模型的本机聊天并停止在途回复；取消确认不写库。仅增加选择不清聊天，但会作废旧预览和在途请求。

## 实现与边界

| 层 | 实现 |
|---|---|
| core | MemoryScope、MemoryPolicy、MemoryPolicyChange与确认错误；保持纯领域数据，无Tauri依赖 |
| storage | 策略读取/保存、范围/版本/epoch比较、选择预算；许可收回与聊天清理同事务，写失败回滚；更正前返回受影响模型以便调整预算 |
| 宿主 | `chat_context_preview`为main/pet只读；`memory_policy_set`仅main。范围从当前设置推导，expectedScope只是比较值，不能借此授权另一个模型 |
| 发送 | 同一锁内读取许可、有效条目、聊天与epoch；请求必须携带expectedScope/expectedContextEpoch，过期则网络前拒绝；前端不能提交记忆正文 |
| 上下文 | 固定角色规范＋user角色的结构化参考资料＋当前模型聊天＋本轮输入；正文不插进system指令，不增加工具或文件权限 |
| 回执 | 绑定requestId的流事件和完成结果包含scope、epoch、记忆id/revision、正文/附加字符数，不在回执复制正文；收到HTTP响应才报告提交，连接失败不猜测服务端是否收到 |
| 变更 | 沿用M2的缓存清理、取消、迟到结果过滤及窗口补核；模型切换递增epoch，即使A→B→A也不能复用旧预览 |

IPC协议提升到2，并更新共享解码和既有浏览器/原生夹具；v1聊天请求直接拒绝。`memory_list`改为仅items/contextEpoch，模型许可由预览接口提供。聊天库仍是schema v2，使用M1已有策略表，无新增迁移或工作区。

模型设置与聊天分库：切换范围时，在设置锁内先使旧epoch失效，再保存配置；保存失败保留原配置，允许旧预览被保守作废，不能让配置先切换而旧请求继续。单纯修改输出预算不改变使用范围。当前语音实验继续不携带记忆。

记忆是用户陈述而非高优先级指令；恶意文本只作为JSON数据，不能获得不存在的执行权限。但模型是否正确引用和遵循偏好仍须真实模型验证，未用本地夹具代替这项结论。导出文件、服务商留存和用户自行粘贴的内容不受本机删除自动撤回。

## 验证与复现

| 验证 | 实际结果 |
|---|---|
| `npm run check` | 类型检查、16项Vitest、前端构建通过；严格校验预览预算/顺序/许可和v2回执 |
| `npm run test:browser`（本地Live2D样本启用） | 29例通过；包括显式启用、5条/800字边界、撤回确认、错误保留、模型切换、发送预览和使用回执 |
| `cargo test --workspace --locked` | 49通过、1项既有系统凭据测试忽略，doc-tests通过 |
| 格式、静态检查及文档 | fmt、workspace clippy `-D warnings`、文档检查（35份Markdown、200个本地链接与计划容量）和桩DOM原型检查通过 |
| Rust边界 | 数据库触发器制造撤回事务失败，验证许可/epoch/聊天全部回滚；重开保留许可，删除移除选择，跨模型默认关闭，旧scope/epoch拒绝 |
| 原生Windows | 独立资料完成默认关闭、UI启用、请求JSON核对、重启、地址/模型隔离、A→B→A旧预览拒绝、pet写权限拒绝、生成中撤回、更正、删除后重开 |

原生脚本：[memory-context.cjs](../../apps/desktop/tests/native/memory-context.cjs)。每次使用未存在过的`dev.qiban.companion.acceptance.m3-*`标识，脚本拒绝已有目录；通过专用WebView调试端口驱动自己启动的验收进程。最终源码构建复核标识为`dev.qiban.companion.acceptance.m3-20260919b`。每轮8次localhost合成请求，不读取生产密钥、不调用付费模型；`.cache/<标识>-<UUID>/`保存合成请求、结果和界面截图，不纳入Git。

先将独立配置写到`.cache/memory-context-acceptance.conf.json`，例如：

```json
{"productName":"Qiban M3 Acceptance","identifier":"dev.qiban.companion.acceptance.m3-unique-run"}
```

```powershell
npm run tauri --workspace @companion/desktop -- build --no-bundle --config D:/Game/.cache/memory-context-acceptance.conf.json
Copy-Item -LiteralPath target/release/companion-desktop.exe -Destination .cache/memory-context-acceptance.exe
$env:QIBAN_ACCEPTANCE_EXE = (Resolve-Path .cache/memory-context-acceptance.exe).Path
$env:QIBAN_ACCEPTANCE_ID = 'dev.qiban.companion.acceptance.m3-unique-run'
node apps/desktop/tests/native/memory-context.cjs
npm run build:desktop # 恢复正常标识的演示构建
```

下一步M4按[Q6设计](../quality/limited-memory-acceptance.md)展开固定50例事实/更正与20例删除竞争集，补真实模型表现及非实现者体验。本机工程自测不等于干净目标设备、外部CI或商业阶段门通过。

后续[M4本地固定验收](memory-q6-local.md)已补齐50例发送边界及20例删除竞争集；模型质量和独立体验仍待验证。
