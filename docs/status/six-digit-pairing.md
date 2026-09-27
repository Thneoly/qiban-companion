# 六位数字配对码

2026-09-27。针对长配对码不便在手机输入的问题，本轮采用六位数字方案。启动与操作见[配对指南](../development/device-pairing.md)，协议见[授权设计](../architecture/device-authorization.md)。

## 已实现

- 电脑生成并突出显示六位数字；保留前导零。手机请求数字键盘，支持手输及带空格粘贴，先核对设备再明确确认。
- 五分钟有效、同账号另一登录会话、一次性消费与撤销继续生效。配对只开放文档摘录范围，每个文档动作仍需确认。
- 核对与确认共用账号级五次失败预算，首次失败起固定五分钟窗口。换码、成功核对、换会话和重启均不清除当前窗口计数；达到限制返回独立错误，不影响邮箱登录或其他账号。
- 协调数据库 schema 4 → 5：增加持久化失败计数，作废旧 pending 配对码；保留 active 关系及已有文档授权。旧客户端和服务需要一起更新，不能降级数据库。
- 无新增外部依赖。本轮没有二维码或相机入口。

## 本地验证

已执行并通过：

- `npm run check`：68 条 Vitest、8 组手机网关测试，以及桌面/手机前端构建。
- `npm run test:browser`：41 条通过；1 条因缺少外部 Live2D 角色资产跳过。含六位码及前导零展示断言。
- `cargo fmt --all -- --check`、`cargo test --workspace --locked --quiet`：169 条通过、6 条 opt-in 默认忽略；`cargo clippy --workspace --all-targets --locked --quiet -- -D warnings` 通过。
- `npm run test:desktop-account`：6 条通过（测试运行约 83 秒），使用实际 Rust 账号模块、Windows 凭据、协调服务与手机浏览器页面，覆盖带空格六位码输入、核对/确认/撤销、已消费码拒绝，以及文档取消、准入响应丢失不写入、产物回执恢复。
- `npm run build:desktop`：成功生成 `target/release/companion-desktop.exe`（未打包安装器）。
- 新增/更新测试覆盖随机数拒绝采样、前导零、旧码拒绝、持久化与并发失败预算、独立 HTTP 错误、schema 4 迁移后已配对文档动作可用。
- `npm run verify:docs`、`npm run verify:prototype`、`git diff --check` 通过。

测试使用独立数据和内存邮件，不调用真实 SMTP 或大模型。

## 使用与待验收

正常停止旧协调服务、手机网关及托盘中的桌面，再在根目录的三个终端分别运行 `npm run coordinator`、`npm run mobile:https` 和 `npm run desktop`。手机访问 HTTPS 脚本本次输出的地址并刷新页面，电脑重新生成六位码后配对。若继续使用 release 可执行文件，需先运行 `npm run build:desktop` 更新它。

真实手机键盘、真实 Tauri 窗口与手机的人工联合体验仍待用户验收；自动化手机视口测试不等于真机验收。CI 保持停用，不据此宣布 T44/T21 全包或商业门通过。
