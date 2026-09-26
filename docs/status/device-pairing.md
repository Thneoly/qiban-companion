# T44/T21 配对与动作授权首轮交付

日期：2026-09-26。对应 O3 的配对闭环及动作授权内核；[协议与门禁](../architecture/device-authorization.md)、[运行指南](../development/device-pairing.md)。T44/T21 全包、T22 和 G3 未准出。

## 实现

- 电脑生成五分钟一次性码，手机同账号另一会话核对电脑/范围并明确确认；两端显示配对关系，支持按版本撤销。重新生成使旧待确认码失效，消费或撤销后码摘要被清除。
- 服务端生成设备 ID 并绑定当前会话；拒绝自身配对、跨账号、过期码、已消费码及旧 revision。名称不作为可信硬件身份。退出/会话失效后需要重新配对。
- 桌面通过三个主面板 IPC 访问明确路由；手机 BFF 仅白名单开放列表、核对、确认和撤销，保留同源/CSRF/Cookie 边界。配对码不进入持久化前端配置；账号令牌仍不进入前端。
- core 定义文档摘录范围、不可变动作绑定；storage 实现资源版本、准备、手机确认、电脑单次准入与撤销。校验 actionId、资源/授权版本、参数摘要、期限和两端会话；变更资源使旧确认失效。
- 撤销与准入在同一串行化事务中判定：尚未准入可取消，已准入只记录 cancel_requested，等待实际执行器核对。本轮未开放动作 HTTP、文件读写或远程执行。
- 协调库 schema 2 → 3，新增设备、配对、资源与授权表，保留既有账号/伙伴/待办。原生本地数据不迁移。配对记录上限 100/账号，动作记录 500/账号，历史清理后续设计。

## 已执行验证

- `cargo test --workspace --locked`：coordinator 9、core 13、desktop 36、storage 55 条通过；4 条 opt-in 集成默认忽略。新增存储 6 条覆盖一次性/自身/跨账号、期限与会话撤销、动作精确确认与单次准入、资源版本变化、迁移/重开以及撤销竞争；新增 HTTP 用例覆盖严格请求字段与账号隔离。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。
- `npm run test:browser`：36 条通过、1 条外部 Live2D 资产缺失跳过。扩展桌面账号 UI 用例，验证生成码、确认后隐藏码、状态刷新与撤销，账号层为受控 IPC 替身。
- `npm run test:desktop-account`：6 条通过，约 65 秒；实际 Rust 账号模块 + Windows 凭据管理器 + 协调 HTTP/SQLite + 手机尺寸 Edge 独立登录。覆盖同一伙伴/共享待办、核对电脑标识、确认配对、撤销、旧码重放拒绝和原生重读撤销状态。内存测试邮箱，不发送真实邮件。

- `npm run test:mobile:integration`：既有双浏览器登录、Cookie 恢复、账号隔离、待办去重、断线/取消/退出回归通过，约 65 秒。
- `npm run check`：TypeScript、37 条 Vitest、桌面/手机生产构建与 6 组手机网关测试通过。
- `npm run verify:docs`：54 篇 Markdown、349 个本地链接及目录/计划容量检查通过；`npm run verify:prototype` 的 8 条原型流程通过。
- `npm run build:desktop` 成功生成 `target/release/companion-desktop.exe`，未生成安装包。

首次界面检查因新旧列表共用 CSS 类触发测试定位歧义，已改为限定各自区域并重新通过；并非绕过产品行为断言。桌面配对与手机页面截图已检查，保留在被忽略的 `apps/desktop/test-results/desktop-pairing.png`、`mobile-pairing.png`。

## 尚未完成

- 真实 Tauri 窗口 + SMTP + 用户手机的人工联合验收仍待补。本次自动化没有修改用户账号或触发真实发信。
- 当前是与会话绑定的设备关系，尚无长期设备密钥、扫码、设备在线/能力报告或正式远程服务部署。
- 单次动作授权内核尚未连到 T16 本地执行台账；确认页面、资源选取/参数摘要生成、出站派发、动作前核对、在途停止和结果回执属于下一增量。不能把已配对或已准入写成任务已执行。
- CI 按用户要求保持停用；工程自测不是外部 CI、独立 QA 或 G3 通过。
