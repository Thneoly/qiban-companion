# 通用模型设置与 Live2D 数字人

更新：2026-09-21。入口仍是桌面角色；设置和待办管理放在辅助面板。实际验收见[本机记录保存与恢复](../status/chat-history-persistence.md)及[渲染基线记录](../status/model-settings-live2d.md)。

## 配置模型

1. 从仓库根目录运行 `npm run desktop`，或运行已构建的 `target/release/companion-desktop.exe`。
2. 点击角色，在气泡中点击“模型设置”。
3. 填写 **API 基地址**与**模型编码**，设置**最大输出 tokens**（128～8192整数，默认1024），选择是否需要 API Key，然后点击“保存模型设置”。基地址不包含 `/chat/completions`，应用会自动追加该路径。
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

预算通过 `max_tokens` 发出，其他API协议或参数名仍需单独适配。只调整预算不会清空前文；已发起请求采用原快照，后续请求使用新值。旧设置自动兼容为1024，不自动提高上限。详细测试与真实调用见[预算交付记录](../status/model-output-budget.md)。

## 阅读与状态

点击“展开阅读”在角色窗口内查看连续问答，“收回气泡”恢复紧凑形态。新回复默认跟随到底部；向上翻阅时停止自动滚动，点击“回到最新”恢复。状态区显示等待、回复中、完成、停止和未完成。部分失败不会加入前文，可编辑后重新发送；若仅记录读取失败，会保留回复完成状态并提示重开读取，不自动重发。实际结果见[阅读交付记录](../status/chat-reading.md)。

## 配置与对话的边界

非密钥设置保存到本机应用数据目录的 `model-settings.db`。密钥按规范化后的完整 API 基地址分别保存到 Windows 凭据管理器，目标名为 `dev.qiban.companion/model/{baseUrl}`。切换地址不会把旧地址的 Key 发给新地址，也不会自动删除旧 Key；切回对应地址可删除。取消勾选 API Key 后该请求不发送 Authorization。此存储保护不等于隔离同一 Windows 用户下的恶意进程。

密钥通过原生输入窗口直接进入 Rust，不进入 React、IPC 返回值、SQLite、仓库或 `VITE_*` 环境变量。前端只得到“是否已配置”。请求不跟随重定向，服务商错误正文不转发到界面或普通日志。

完整文本问答在本机明文保存，所有模型合计最多6轮/12000个Unicode字符，超限淘汰最早整轮。重启可恢复；地址和模型隔离前文，切回可恢复未淘汰记录。发送内容、固定角色提示词及当前模型的保留前文传给配置的服务。收起、待办、隐藏、安静或失焦仍请求停止；半截回复不保存。点击“最近 N 轮”查看，“清空对话”删除本机全部模型记录，失败会提示并保留原记录；不删除服务商已收到的内容。[手动有限记忆](../status/memory-model-context.md)需另行显式选择并授权给当前模型；没有工具或电脑操作；独立语音实验不写入这份历史。停止后已产生的服务用量仍可能计费。

每次最多输入2000个 Unicode 字符；并发1条；连接超时10秒、总时限90秒；请求输出上限可配置128～8192 tokens（默认1024）；流量上限4 MiB、文本上限64 KiB。无自动重试，不编造 token 费用；服务未返回用量时显示未知。服务端截断、异常结束、鉴权或额度错误会显示失败，部分回复保留到气泡收起。浏览器预览不保存模型设置、不配置密钥、不调用模型。

## 选择角色与场景

点击角色 → **角色与场景**。选择小舞台、无背景或完整家园，调节背景不透明度；默认 65% 的小舞台，角色不随背景变淡。选择无背景或 0% 后可继续拖动角色，原背景区域不会阻挡鼠标。

**2D 数字人**使用本地 Live2D 模型。点击“导入模型文件夹”，选择包含单个 `.model3.json`、对应 `.moc3`、PNG 和动作文件的完整目录；导入成功自动切换，并保存供重开恢复。“栖栖”切回原创 SVG，“移除本机模型”删除应用副本，不改源文件。取消选择不修改配置；导入错误或渲染失败会显示原因。

目前支持 Cubism 3/4 model3、一个本机模型、最多 256 文件/80 MB；不支持 ZIP 和远程模型地址。外观和模型保存在 WebView2 的 IndexedDB 中，浏览器预览也保留外观；不会上传模型。详细资源限制、动作约定及原生实测见[数字人与轻量场景记录](../status/avatar-appearance.md)。

运行库在开发/构建前自动准备并校验固定哈希，安装包包含运行库和许可文件，不包含示例角色。首次准备需要网络，应用运行时不从 CDN 拉脚本。商用发行授权与角色授权仍需完成，不把“能运行”当成已经获准发行。

本机内部渲染验证可准备官方 Hiyori 示例：

```powershell
npm run prepare:live2d --workspace @companion/desktop
```

在角色设置中选择 `apps/desktop/public/live2d-local/Hiyori` 文件夹即可体验。示例不是栖伴的商业角色；条款见 [SDK](https://www.live2d.com/en/sdk/license/)和[示例素材](https://www.live2d.com/zh-CHS/learn/sample/)。示例下载清单固定版本并校验哈希，资源目录被 Git 忽略。普通开发构建仍会复制本地 public 内容；要生成不含样例的产物请使用 `npm run build:installer`。

PixiJS 6.5.10 / pixi-live2d-display 0.4.0，渲染上限 30 fps。安静、隐藏文档或系统减少动画时暂停绘制；隐藏或切换形象销毁实例。模型有同名动作/表情时跟随文字状态，否则保留待机及文字提示。没有语音口型、摄像头追踪或真正的 3D 数字人。

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