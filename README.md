# 栖伴 · Companion

React + TypeScript + Tauri 2 + Rust 的桌面 AI 伙伴工程骨架。暂用“栖伴／栖栖”作为开发名称。产品、设计、计划与验证资料统一进入[文档中心](docs/README.md)。

当前可运行：角色互动样机、待办创建/列表/取消、桌面 SQLite 持久化、浏览器内存预览、类型化 IPC 与错误反馈。**任务只进入待办，不会执行。** 已加入可配置的 Chat Completions 临时多轮对话与 Live2D 实验入口；已使用本机配置验证 glm-5.3 和 glm-5.3-flash 的真实回复及停止。未接入语音、长期记忆、账号、手机控制、自动更新或收费。默认入口已改为透明悬浮角色：点击打开气泡，拖动底部把手移动，任务面板按需打开。支持托盘恢复、隐藏和安静陪伴；SVG 角色是自制原型资产。

最新增量见[对话阅读与状态反馈](docs/status/chat-reading.md)，模型与会话验收见[临时多轮会话](docs/status/session-chat.md)，配置及渲染基线见[通用模型设置与 Live2D](docs/status/model-settings-live2d.md)。此前实现与实测边界见[位置记忆与下一增量](docs/status/pet-position-memory.md)及[桌面角色交付记录](docs/status/desktop-pet.md)，初始骨架见[历史记录](docs/status/implementation-status.md)。已通过本机原生窗口与交互冒烟检查；这仍是可演示样机。

## 启动

需要 Node.js 22.12+（本机使用24）、当前 stable Rust、Windows C++构建工具及WebView2。前置条件参考[Tauri Windows环境](https://v2.tauri.app/start/prerequisites/)。推荐Windows 11。

```powershell
cd D:\Game
npm ci
npm run dev         # 浏览器预览：http://127.0.0.1:1420
```

另选桌面模式（先结束占用1420端口的预览进程）：

```powershell
npm run desktop     # 自动启动Vite并编译/打开Tauri客户端
```

### 体验桌面角色

构建后可直接运行：

```powershell
.\target\release\companion-desktop.exe
```

- 首次启动角色出现在主屏右下角；移动后重启恢复上次位置，并按当前屏幕边界修正。任务面板默认隐藏。
- 点击角色展开气泡；输入内容后点“记下来”保存待办。
- 按住角色下方“拖动这里”移动，停止后约一秒内自动保存位置；Esc 或切换到其他窗口收起气泡。
- 气泡里的“任务面板”打开待办管理；关闭面板仍保留角色。
- “模型设置”可填写 API 基地址、模型编码、是否使用 Key；保存后在原生密码窗口设置密钥，再回到“聊一聊”。智谱仅为可修改预设，支持其他兼容 Chat Completions 服务。
- 对话保留本次运行最近6轮/合计1.2万字，点击“最近 N 轮”查看、“清空对话”清除。收起会停止生成，但保留完整问答；退出重启或更换地址/模型会清空。
- 点击“展开阅读”集中查看问答；向上翻阅时暂停自动跟随，“回到最新”恢复。生成状态区区分等待、回复中、完成、停止和失败。
- “Live2D 实验”需要先准备本机示例资源；默认仍用 SVG，缺少资源会回退。详见[配置与资源准备](docs/development/model-settings-live2d.md)。
- “安静陪伴”保留角色并让鼠标完全穿透；“隐藏”收起角色。点击系统托盘图标恢复互动，图标可能位于任务栏的隐藏图标区。
- 右键托盘可找回角色到主屏、打开面板或退出应用。

`npm run dev` 只在浏览器模拟角色交互，不能提供原生透明窗口、系统托盘或跨应用鼠标穿透。浏览器辅助面板路径为 `/?view=panel`。

浏览器预览只使用内存，刷新清空；桌面模式通过Rust保存到系统本地应用数据目录 `dev.qiban.companion/companion.db`。数据库含任务标题，当前未加密，使用测试资料即可。前端不能提供数据库路径或执行任意SQL。原生IPC失败会显示错误，不会悄悄切换到假数据。

## 工程结构

```text
apps/desktop/                  桌面应用及应用专属配置
  src/features/                角色、待办等界面功能
  src/lib/                     原生IPC与预览适配
  src-tauri/                   Tauri宿主、窗口权限、Rust命令层
  assets/brand/                应用图标源文件
  tests/browser/               桌面Web界面的端到端检查
  playwright.config.ts         应用专属测试配置
packages/contracts/            TypeScript共享协议及边界测试
crates/companion-core/          不依赖UI的Rust领域核心
crates/companion-storage/       SQLite存储与迁移
docs/
  product/                     用户、市场与商业分析
  planning/                    进度、待办与阶段门
  architecture/                技术设计与工程边界
  quality/                     设计评审、测量与验收
  research/                    假设研究与验证方法
  status/                      实际交付和验证记录
prototypes/companion-validation/ 早期研究原型，不进入产品构建
scripts/verification/          文档及原型验证脚本
.github/workflows/             Windows CI定义
```

Rust工作区与npm工作区分开，保留 `Cargo.lock` 和 `package-lock.json`。模型密钥不应放入 `VITE_*` 环境变量，它们会进入前端产物。密钥在 Windows 原生密码窗口输入并保存到系统凭据管理器；API 地址、模型编码等非密钥配置单独保存。

根目录仅放工程入口、工作区配置和锁文件；Vitest配置负责跨包单元测试，Playwright配置归桌面应用。代码归属与新增目录规则见[AGENTS.md](AGENTS.md)。`node_modules/`、`target/`、`.cache/`及各应用内的`dist/`、`test-results/`都是被忽略的本地生成目录。

## 验证与构建

```powershell
npm run check             # TS、Vitest、前端生产构建
npm run test:browser      # 使用本机Edge，无需下载另一套浏览器
npm run verify:docs       # 文档链接、目录约定及计划人日核对
npm run verify:prototype  # 独立验证早期研究原型
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
npm run build:desktop     # 生成Windows可执行文件，不生成安装包
```

Windows CI配置见仓库的Actions；本次本机结果与未验证项单独记录，不据此宣称外部CI或产品门禁通过。标准Rust目标目录下可执行文件为 `target/release/companion-desktop.exe`；自定义 `CARGO_TARGET_DIR` 时以对应目录为准。默认关闭安装包、签名和自动发布；源码构建不代表通过产品计划G3/G4/G5。

若所在自动化环境只允许写项目目录，可在当前PowerShell会话使用项目缓存（不修改全局配置）：

```powershell
$env:CARGO_HOME = Join-Path $PWD '.cache/cargo'
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target'
```

npm缓存已限定在根目录 `.cache/npm`。Rust依赖需可访问crates.io或具备完整可信缓存；出现TLS错误应修复环境或使用已校验缓存，不关闭证书验证。`--offline` 仅在所需索引及包已缓存时适用。

## 后续接入边界

先在独立领域/适配层增加持久任务事件、attempt/action ID和真实执行器，再开放工具；运行中取消需要请求取消及结果核对，不可直接标为已取消。账号、资源归属、动作授权版本及撤销是远程接续的前置。完整方案见[架构说明](docs/architecture/overview.md)与[开发计划](docs/planning/agile-stage-gate-plan.md)。

构建配置依据[Tauri Vite集成](https://v2.tauri.app/start/frontend/vite/)，窗口原生调用使用[Tauri能力控制](https://v2.tauri.app/security/capabilities/)。业务层没有引入自有Wasm/WASI、微服务或模型供应商SDK；可选Live2D的Cubism Core使用WebAssembly。
