# 通用模型设置与 Live2D 实验

日期：2026-09-15。入口仍是桌面角色；设置和待办管理放在辅助面板。实际验收见[最新会话记录](../status/session-chat.md)及[渲染基线记录](../status/model-settings-live2d.md)。

## 配置模型

1. 从仓库根目录运行 `npm run desktop`，或运行已构建的 `target/release/companion-desktop.exe`。
2. 点击角色，在气泡中点击“模型设置”。
3. 填写 **API 基地址**与**模型编码**，选择是否需要 API Key，然后点击“保存模型设置”。基地址不包含 `/chat/completions`，应用会自动追加该路径。
4. 需要密钥时，点击“设置 / 更换 API Key”，在 Windows 原生窗口的**密码栏**填写 Key，用户名保持 `API Key`。无需把密钥发送给开发助手。
5. 回到角色，重新打开气泡，点击“聊一聊”并发送内容。每次发送均使用保存的配置；正在生成的请求使用发起时的配置快照。

本版支持 **OpenAI-compatible Chat Completions 的流式文本协议**，可填写其他云服务或本机兼容服务。它不等于支持所有厂商的全部 API；Responses、Anthropic 原生协议、额外自定义请求头、工具调用和图像输入尚未适配。远端地址必须使用 HTTPS；本机 `localhost`、`127.0.0.1`、`[::1]` 可以使用 HTTP，例如 `http://127.0.0.1:11434/v1`。不需要鉴权的本机服务可取消勾选 API Key。

“智谱通用 API 预设”只是快捷填充：

| 字段 | 预设 |
|---|---|
| API 基地址 | `https://open.bigmodel.cn/api/paas/v4` |
| 模型编码 | `glm-5.3` |
| API Key | 用户自己的通用 API Key |

`glm-5.3`是初始预设。现已在用户账号实测该型号及用户随后选择的`glm-5.3-flash`；新启动读取已保存的选择，不覆盖为预设。其他账号仍应以控制台实际可调用型号为准，不自动降级。智谱[通用 HTTP 接口](https://docs.bigmodel.cn/cn/guide/develop/http/introduction)与[Coding Plan 工具接入](https://docs.bigmodel.cn/cn/coding-plan/tool/others)属于不同使用范围；本应用按通用 API 接入。兼容服务是否接受当前参数、响应时间和费用，须用该服务实际验证。

## 阅读与状态

点击“展开阅读”在角色窗口内查看连续问答，“收回气泡”恢复紧凑形态。新回复默认跟随到底部；向上翻阅时停止自动滚动，点击“回到最新”恢复。状态区显示等待、回复中、完成、停止和未完成。部分失败不会加入前文，可编辑后重新发送；若仅记录读取失败，会保留回复完成状态并提示重开读取，不自动重发。实际结果见[阅读交付记录](../status/chat-reading.md)。

## 配置与对话的边界

非密钥设置保存到本机应用数据目录的 `model-settings.db`。密钥按规范化后的完整 API 基地址分别保存到 Windows 凭据管理器，目标名为 `dev.qiban.companion/model/{baseUrl}`。切换地址不会把旧地址的 Key 发给新地址，也不会自动删除旧 Key；切回对应地址可删除。取消勾选 API Key 后该请求不发送 Authorization。此存储保护不等于隔离同一 Windows 用户下的恶意进程。

密钥通过原生输入窗口直接进入 Rust，不进入 React、IPC 返回值、SQLite、仓库或 `VITE_*` 环境变量。前端只得到“是否已配置”。请求不跟随重定向，服务商错误正文不转发到界面或普通日志。

对话当前是本次运行内的临时多轮文本：最多保留6轮、问答合计12000个Unicode字符，超限移除最早完整问答。发送的内容、固定角色提示词及保留前文传给配置的服务；只有成功完整问答进入前文。没有磁盘历史、长期记忆、工具、电脑操作、语音或后台任务。收起气泡、切换到待办、隐藏、安静陪伴或窗口失焦会卸载对话界面并请求停止；已完成问答仍留在Rust内存，重新打开可继续。点击“最近 N 轮”查看、“清空对话”删除本机记录；退出程序或改变API地址/模型编码会清空。清空不代表删除服务商侧已收到的内容。停止先屏蔽旧文本再取消网络请求，已经产生的服务用量仍可能计费。

每次最多输入2000个 Unicode 字符；并发1条；连接超时10秒、总时限90秒；请求输出上限1024 tokens；流量上限4 MiB、文本上限64 KiB。无自动重试，不编造 token 费用；服务未返回用量时显示未知。服务端截断、异常结束、鉴权或额度错误会显示失败，部分回复保留到气泡收起。浏览器预览不保存模型设置、不配置密钥、不调用模型。

## 准备 Live2D 实验资源

默认角色仍是自制 SVG。Live2D 通过气泡中的“Live2D 实验”显式切换；缺少资源或加载失败时回到 SVG 并提示。实验使用官方 **Hiyori 示例角色**，不是栖伴的商业角色。

先阅读 [Live2D SDK 条款](https://www.live2d.com/zh-CHS/sdk/license/)及[示例素材条款](https://www.live2d.com/zh-CHS/learn/sample/)。确认适用于本次内部技术验证后，在根目录执行：

```powershell
npm run prepare:live2d --workspace @companion/desktop
npm run desktop
```

准备脚本下载24份固定版本的运行时、示例及许可证文件，校验清单中的 Git blob SHA-1 或 SHA-256。文件进入被忽略的 `apps/desktop/public/live2d-local/`，不提交第三方模型二进制。已有文件与清单不符时拒绝覆盖。之后加载使用本地资源，不从 CDN 动态拉取可执行代码。

**本地资源存在时，Vite/Tauri 构建会把它们打包。** 当前本机构建含示例，限本次内部验证使用；正式分发前必须确认 SDK 及素材授权或替换为拥有相应权利的角色。干净检出的仓库默认不含这些文件，普通构建正常，实验按钮会提示缺少资源。

实现采用 PixiJS 6.5.10 与固定的 pixi-live2d-display 0.4.0 浏览器发行文件。未安装其带入部署工具的 npm 包。`@pixi/unsafe-eval` 在此用于安装 CSP 兼容替代实现，避免运行时生成 JavaScript；CSP 仅为 Cubism Core 放开 WebAssembly 编译，不允许通用 `unsafe-eval`。

实验只验证加载、待机动画、30fps上限、安静时停止模型更新/绘制、隐藏时释放画布。没有口型、语音中断、情绪状态机或自定义栖栖 Live2D 模型；隐藏文档也暂停绘制。长期资源与多屏测试仍待补齐。

## 验证

```powershell
npm run check
npm run test:browser
# 已准备本机示例后，额外执行实际渲染检查：
$env:QIBAN_TEST_LIVE2D='1'
npm run test:browser
Remove-Item Env:QIBAN_TEST_LIVE2D
cargo test --workspace --locked
# 显式运行使用临时虚构 Key 的 Windows 凭据集成测试：
cargo test -p companion-desktop credential_is_scoped_and_can_be_removed --locked -- --ignored
```

普通 CI 跳过需要本机授权示例的实际渲染测试，但仍检查缺失资源的回退。模拟/本机 HTTP 服务通过不能替代真实模型验收；本账号已完成两种模型的真实调用与停止验收，具体数据见最新记录；新增服务和型号仍需分别实测。

参考项目[awesome-digital-human-live2d](https://github.com/wan-h/awesome-digital-human-live2d)提供了数字人交互拆分的参考；本项目只借鉴边界并采用独立渲染器，没有移植其 Next.js/Python 服务或把它的 MIT 许可等同于 SDK/模型的商业授权。资源来源与固定提交见[下载清单](../../apps/desktop/scripts/live2d-sample-manifest.json)。