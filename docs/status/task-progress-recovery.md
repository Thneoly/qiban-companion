# 任务进度与恢复入口：交付记录

> 2026 年 9 月 27 日。跨端体验路线第 3 项：手机能区分六个任务状态（等待确认、等待电脑、执行中、完成、取消请求、结果未知），并提供刷新与核对结果入口。前置：[设备心跳与在线状态](device-presence.md)、[六位码配对](six-digit-pairing.md)。呈现语义见[设备授权架构](../architecture/device-authorization.md)文档适配协议新增"手机端六态呈现"小节。

## 实现与归属

- **核心思路**：六态 = 服务端状态（**零改动**）× 配对 presence 租约（PR #31）的**手机端展示层推导**。无 schema 迁移、无协议 bump、无生产 Rust 改动；新增的唯一"状态"是 `confirmed→等待电脑`、`admitted→执行中` 的呈现拆分，"执行中"始终标注"（推断）"。
- **契约（packages/contracts/src/remote-documents.ts）**：纯函数 `deriveDocumentPhase(doc, pairing|null|undefined, now) -> {key,label,hint}` 与 `relativeTime(at, now)`（自 PairingPanel 私有函数上移，输出一致）；`documentStates` 保持不动（仍是 decode 白名单与桌面前端渲染源）。pairingId 不匹配时按无 presence 处理，绝不用错误行装饰。
- **手机 UI（DocumentPanel）**：文档 5s 轮询不变，新增 pairings 10s 轮询（对齐 PR #31 约定超时契约）+ online/focus/visibilitychange 即时刷新；按 `authorization.pairingId` join。渲染派生标签 + hint 行；awaiting/confirmed 标签带"（剩余 m:ss）"倒计时（1s ticker 仅在这类文档存在时启用），到期显示"（已到有效期）"并声明以服务端刷新为准——**手机永不自行宣布终态**。
- **诚实性修正**：取消按钮从 cancel_requested/unknown 移除（服务端 `invalidated_state` 保留这两个状态，按下是空操作）；这两态改为指引 + "刷新此任务"按钮（waiting_desktop/executing 同样提供）。pairings 读取失败 ⇒ 该轮按无 presence 推导（标签退化为"在线状态未知/以回报为准"），**绝不用陈旧 presence 装饰**；0/503 的文档读取显示"无法连接服务，任务状态未知"横幅并保留最后一轮数据（与 PairingPanel 同款模式）。
- **核对结果入口**：completed 的展开块升级为"核对保存结果"——预览、保存路径、产物哈希与前 12 位、以及"电脑回报完成时，服务端已核对该哈希与这份预览的摘要一致"的声明。依据：`receipt_document` 拒绝 `artifact_hash ≠ digest(preview)` 的回执，completed 只可能在哈希匹配时存在——手机展示的是**服务端已核验的事实**，不声称手机侧核验、不声称可下载（下载/查看是路线第 4 项）。unknown 指引"电脑在线会自动核对/上线后自动核对，无需重新确认"——依据：桌面 sync 循环在 unknown 仍在 pending 集内，每 5s 核对重发回执。
- **协调器测试**：文档端点此前 HTTP 覆盖为零；新增全链路回归（share→confirm→admit→receipt 的角色/单次准入/哈希校验/幂等/终态冲突 409、准入前取消、cancel_requested→unknown→completed 补齐、跨账号列表为空）。控制器会话需要同邮箱第二次登录，测试内等满真实 60s OTP 冷却一次（与 mobile-peer.mjs 同一理由：尊重真实限流，不加测试缝）；5 分钟过期与同账号观察者隔离仍由存储层测试覆盖（HTTP 层无时钟缝/第三会话缝）。

## 验证（本机，2026-09-27）

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式/静态 | `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 / 0 警告 |
| Rust | `cargo test --workspace --locked` | 全过；新增协调器文档生命周期测试（含一次真实 61s OTP 冷却等待） |
| 契约+前端 | `npm run check` | vitest 76 过（含新增六态全矩阵 8 状态 × online/offline/null、倒计时/过期钳制、"（推断）"仅 admitted+online、四标签子串兼容）；网关 8 组不变 |
| 手机集成 | `npm run test:mobile:integration` | 真协调器+真网关+仿真 Edge 浏览器：四份文档走全流程——等待手机确认→确认→"已确认，等待电脑保存"+在线 hint→准入→"执行中（推断）"→回执→"已保存并核验"+核验声明；unknown→自动核对→补齐；确认前取消后按钮消失；准入后停跳 15.5s→"已开始，电脑离线"+恢复指引→恢复心跳→"执行中（推断）"；双横幅（配对"在线状态未知"+文档"任务状态未知"）且数据保留、恢复后清除 |

## 准出对照

| 用户要求 | 证据 |
|---|---|
| 区分等待确认/等待电脑/执行中/完成/取消请求/结果未知 | contracts 全矩阵单测 + 手机渲染 + continuity.mjs 逐态真链路断言；执行中标注"（推断）" |
| 等待电脑 ≠ 电脑离线不可用 | confirmed 在线/离线/未知三档 hint（离线含"最后联系 X"与有效期提醒） |
| 刷新入口 | 面板"刷新文档状态"+每任务"刷新此任务"+online/focus/visibilitychange 即时刷新 |
| 核对结果入口 | completed 核对保存结果（预览+路径+哈希+服务端核验声明）；unknown 恢复指引与自动核对说明 |
| 诚实性 | 不可达≠离线≠结果未知三分；取消按钮不再出现在空操作状态；手机不宣布终态 |

## 产品与准出边界

- 执行中是 presence 推断（会话级租约），非任务级真相；桌面本地 `ExecutionStatus::Running` 仍不上报（服务端"执行中"真状态明确不做）。
- 结果内容下载/查看与记录清理是路线第 4 项；20 条文档/500 动作上限不变。
- 无新网关路由：单文档 GET、准入、回执仍桌面直连；手机只用列表刷新 + 本地 join。
- 两个面板各自 10s 拉 pairings（合计约 +6 请求/分钟，对 600/分钟网关聚合上限无碍）；共享 presence hook 记为后续重构。
- 旧协调器（desktopOnline=null）无 presence 装饰、不声称执行中——contracts 单测覆盖，真链路不可仿真（与 PR #31 同一限制，如实记录）。真实手机人工验收未执行，待路线第 1 项统一进行。
