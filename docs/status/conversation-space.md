# 界面整理与宽屏文字对话

2026-10-18。针对功能纵向堆叠和文字输入空间不足，调整桌面界面的信息层级；保留原有模型、记忆授权及语音链。

## 使用入口

从桌面角色打开「任务面板」，在「日常」点击「打开文字对话」；也可以从角色的「聊一聊」点击「展开阅读」。浏览器预览可演示布局，真实模型与窗口行为需运行桌面版。

| 导航 | 内容 |
|---|---|
| 日常 | 与栖栖对话、本机待办、应用内提醒 |
| 文档 | 本机文档摘录、手机确认与结果核对 |
| 记忆 | 应用记忆、模型使用许可、独立个人记忆服务视图 |
| 设备 | 邮箱登录、共享待办、设备配对与状态 |
| 设置 | 文字模型设置、语音配置与实验 |

面板只显示当前用途，切换导航保留表单草稿；账号、模型或记忆快捷入口会定位到对应导航。角色气泡将角色与场景、模型设置、记忆、使用指南收进「更多」。

## 文字对话行为

- 展开后窗口目标尺寸为 760×720 逻辑像素，受当前屏幕工作区限制；收回恢复 320×440 和原伙伴位置。阅读窗口不置顶，顶部角色名称区域可拖动。
- 文字输入改为多行：Enter 发送，Shift+Enter 换行；中文输入法确认选字不会发送。保留原有 2000 字输入上限。
- 展开阅读时，切换到其他应用不会自动关闭对话；放大、收回气泡复用同一聊天组件，保留草稿、流式回复和已有上下文，不重复请求模型。
- 显式关闭、隐藏或离开聊天仍沿用原生命周期；不承诺未发送草稿跨退出恢复。普通小气泡仍按原规则在失焦时收起，语音进行中的既有例外保留。
- 模型和记录说明折叠收纳；发送边界说明仍常驻，记忆预览和提交回执仍可展开。记录删除、记忆许可、语音录音与打断语义不变。

## 实现与验收

桌面代码位于 [App](../../apps/desktop/src/App.tsx)、[角色](../../apps/desktop/src/features/pet/Pet.tsx)、[聊天](../../apps/desktop/src/features/chat/ChatBubble.tsx) 与 [原生窗口](../../apps/desktop/src-tauri/src/pet.rs)。原生仍由 pet 窗口负责聊天 IPC，未创建第二份会话；阅读位置不会覆盖伙伴持久化位置。

新增 [导航回归](../../apps/desktop/tests/browser/workspace.spec.ts)、扩展 [聊天阅读回归](../../apps/desktop/tests/browser/chat-reading.spec.ts)，并增加 [隔离原生验收](../../apps/desktop/tests/native/conversation-space.cjs)。原生脚本复用 Q6 Harness，要求新建的 acceptance 标识和专用构建，拒绝现有配置；仅调用本机模拟模型，不使用真实密钥和用户聊天数据。运行方式及结果见下文。

本机原生验收已通过：导航与草稿保留、760×720 展开、真实窗口失焦、生成中缩放只发送一次、320×440 收回、位置保持、重开恢复历史。证据在被忽略的 `.cache/dev.qiban.companion.acceptance.q6-ui-20261018a-93676db6-0845-499b-bcdd-6ee8603f6020/`。

本次实际执行：

| 检查 | 结果 |
|---|---|
| `npm run check` | 93 条 Vitest、9 组手机网关测试、双端构建通过 |
| `npm run test:browser` | 最终整套 60 过 / 1 条外部 Live2D 角色资产缺失跳过；已修正旧导航用例 |
| `cargo test -p companion-desktop --lib --locked --quiet` | 69 过 / 5 条 opt-in 忽略 |
| 桌面 Clippy（all-targets，warnings deny）、Cargo fmt check | 通过 |
| `npm run verify:docs`、`npm run verify:prototype` | 通过；原型为 DOM stub 检查 |
| 隔离原生 `conversation-space.cjs` | 本机窗口、导航、流式连续性、位置和重开验证通过 |
| `npm run build:desktop` | 按正式标识生成 `target/release/companion-desktop.exe`；未生成安装包或发布标签 |

构建仍有既有的事件模块动态/静态导入提示，浏览器记忆夹具有重复 key 提示；不影响上述通过结果。本次未运行真实云模型或真实语音听感验收。CI 保持暂停。

仍待人工验证：多显示器与混合 DPI、低分辨率系统工作区、连续长时间文字/语音混用的舒适度。自动回归不等于这些体验门已通过。

## 本地体验

在仓库根目录运行 `npm run desktop`；或 `npm run build:desktop` 后退出旧进程，再运行 `target/release/companion-desktop.exe`。本次不更新已安装快捷方式指向的旧安装包，不创建发布标签。

原生验收脚本使用 `QIBAN_ACCEPTANCE_EXE` 指向隔离构建、`QIBAN_ACCEPTANCE_ID` 指向与构建一致且从未运行过的 `dev.qiban.companion.acceptance.q6-...` 标识、`QIBAN_CDP_PORT` 指定空闲本机端口，再运行 `node apps/desktop/tests/native/conversation-space.cjs`。不得把生产版 exe 或正式应用标识填入验收变量。
