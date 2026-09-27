# 设备心跳与在线状态：交付记录

> 2026 年 9 月 27 日。跨端体验路线第 2 项：手机能看到电脑的在线/离线/最后联系时间与当前支持的操作；服务不可达时明确提示状态未知。对应 T44/T21 的"设备能力/在线状态"子项（工程级）。设计语义见[设备授权架构](../architecture/device-authorization.md)新增"设备心跳与在线状态"节。前置：[六位码配对](six-digit-pairing.md)。

## 实现与归属

- **存储（accounts schema v6）**：`paired_devices` 增 `last_heartbeat`（租约戳）与 `capabilities`（canonical slug JSON ≤8）两列（纯 ALTER 阶梯迁移，v0 新库与 v5 旧库同路径）；`AccountStore::heartbeat` 在 `access()` 事务内只更新**本会话绑定**的设备行（UNIQUE(account_id, session_id)），不建行、不碰配对状态/动作授权/资源行/任何有效期；`pair()` 读取时推导 `desktopOnline/desktopLastHeartbeatAt/desktopCapabilities`（在线 = live() ∧ 心跳 ≤15s；未来时间戳防御；坏 capabilities 降级空数组不拖垮列表）。
- **协调器**：`POST /v1/devices/heartbeat`（gated 中间件继承），原始列表 ≤8、每项匹配 `ActionScope::slug()`，未配对返回 `{"updated":false}`；**单一事实源** `ACTION_SCOPES`/`slug()` 常量（桌面发送、协调器校验、手机标签三处引用同一常量，无手写 slug）。
- **桌面发送端**：5 秒循环**登录即心跳**（与 pending 无关——空闲桌面也保持可见）；best-effort：错误忽略（401 由 request() 删 vault 令牌自停，404 旧协调器落 unavailable）；能力列表由 `ACTION_SCOPES` 生成。有意行为变化：运行中的桌面不再 30 分钟闲置过期（心跳经 auth.verify 刷新 last_seen）。
- **手机 UI**：配对行两行展示——首行配对状态（已配对/等待确认/…权威不变），次行仅在有 presence 数据且 pending/active 时显示"电脑在线 · 最后联系 X · 可执行：文档摘录（每次动作仍需确认）"或"电脑离线 · 最后联系 X/从未"；**服务不可达横幅**（0/503 → "无法连接服务，电脑在线状态未知"，保留最后一轮成功数据，恢复后清除；401 仍走既有过期路径）；补 visibilitychange 即时刷新。
- **契约**：`Pairing` 增三字段——`desktopOnline: boolean | null`（**null = 旧协调器 = 状态未知**，非离线）、`desktopLastHeartbeatAt`、`desktopCapabilities`（**开集** slug 正则，未来 scope 原样显示不解码失败）；缺字段降级 null/null/[]，存在但非法仍拒。纯增量，无协议版本 bump（Rust `#[serde(default)]` + TS 解码器对未知字段本就丢弃，均有测试证据）。

## 验证（本机，2026-09-27）

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式/静态 | `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 / 0 警告 |
| Rust | `cargo test --workspace --locked` | 全过；含 6 个存储租约测试（迁移/只更新本会话行/租约超时/跨账号隔离/撤销拒绝/新登录不复活）+ 协调器 HTTP 测试 + 桌面心跳测试 + core 混跑 serde 测试 |
| 契约 | vitest | 48 过（presence 存在/缺失降级/非法拒绝/超 8 项/开集未来 slug） |
| 浏览器 | `npm run test:browser` | 通过（既有桌面 spec 兼容，presence 缺省不影响） |
| 手机集成 | `npm run test:mobile:integration` | 真协调器+真网关+真手机浏览器：在线（含能力与"仍需确认"文案）→ 15.5s 停跳→离线（含最后联系）→ 恢复→在线 → setOffline→状态未知横幅+数据保留 |

## 准出对照

| 准出 | 证据 |
|---|---|
| ① 退出/断网后约定超时内更新 | 租约 15s + 轮询 10s = 最坏 25s（文档化契约）；continuity 停跳 15.5s 后手机显示离线；被杀/断电由同一租约机制覆盖 |
| ② 重连恢复、旧会话不能上报 | continuity 恢复心跳→在线；存储测试：sign_out 后心跳被拒、新登录新 session 心跳 updated=false、旧配对 expired+offline 永不复活；心跳绑定 UNIQUE(account_id,session_id) 且在 access() 重验内 |
| ③ 已配对/可执行分列 | 手机两行 UI + contracts 解码 + `capabilityLabels`；presence 线独立于配对状态线 |
| ④ 在线不跳过授权 | 改动面不触 documents/authorization 任何决策路径（心跳只写两列）；confirm→admit 全套既有测试回归通过；UI 文案持续声明"每次动作仍需确认" |
| 附加：状态未知 | 手机 0/503 横幅 + setOffline 浏览器断言；与"电脑离线"严格区分 |

## 产品与准出边界

- 无 `/offline` 主动下线路由（优雅退出也走 15s 租约；若 25s 体感太慢，后续增量加 2s 超时的 best-effort 退出通知）。无 WebSocket。
- 配对 ≤30 分钟绝对有效期约束在线展示窗口（`paired_devices.expires_at` 创建时冻结，现状语义不变）；长期设备身份/配对续期在 T21 积压。
- opt-in 全链路（真桌面进程被杀→手机 15~25s 内离线）与真实手机人工验收未执行，如实记录；C7 扩展留作后续。
- 混跑限制：旧桌面（不发心跳）恒显示离线；旧手机对新字段降级为状态未知。均已文档化并有测试。
