# 跨端记忆删除同步（T23 / G3）

2026-10-11 起。交付[有限记忆遗留](../../docs/quality/limited-memory-acceptance.md)中唯一未执行的跨端项：断线和更正不使状态回退——事件游标、去重、版本、更正/删除同步与补发（G3 的 20 例）。设计依据 [device-continuity.md](../architecture/device-continuity.md)；范围 = 本机应用记忆（≤30 条、body ≤200 字，kind 含 task_fact 预留），**不含**个人记忆服务、模型使用许可 policy、聊天正文。

分三个 PR 交付（先引擎后入口）：C1 服务端 → C2 桌面 → C3 手机 + 20 例验收。本文随各 PR 增补。

## 服务端（C1，分支 `feat/memory-sync-server`）

协调库 schema v6 → **v7**（`companion-storage/src/accounts/memory_sync.rs`）：

- **镜像与墓碑**：`account_memories` 按账号存全部记忆行，墓碑 = body/event_date 置 NULL + deleted_at 非空（表 CHECK 强制，与本机 chat-history 的墓碑不变式同构）；kind CHECK 与本机三值一致（preference/experience/task_fact）。**压缩只删事件、永不删行**——被删的 id 永远不能复活。
- **事件流**：`memory_events` 全局 AUTOINCREMENT seq（每账号单调随之免费成立），upsert 带全字段、delete 只带 revision。`memory_devices` 按会话记确认游标（device_id = session_id）；`memory_accounts` 记代际与 watermark。
- **写路径**：桌面 push 与手机更正/删除共用同一规则集——`memory_requests` 幂等日志按 requestId 重放（绑定负载指纹，同 id 异负载按冲突拒绝；每账号裁剪至 256 条）；版本不匹配 → `memory_revision_conflict`；对墓碑写入 → `memory_resurrection`；活跃上限 30 → `memory_capacity`。同一删除在响应丢失后重放保持幂等（base+1 == 墓碑 revision）。
- **压缩与保留窗口**（同事务，每次接受的写后）：先清扫死设备行（会话已撤销，或 native issuer 的 token 已过期/闲置 30 分钟——与配对链 live() 同一判活口径；sign_out 也在同事务删行），再删除所有存活设备已应用的事件；事件数超 512 → 代际 +1、watermark 前移、清空事件。**无存活设备时事件保留**（写入者自己靠回执，但其他端仍需要事件；512 窗口是硬上限）。落后设备 pull（代际不符或游标低于 watermark）自动降级为全量快照 + 新代际。
- **鉴权口径**：账号登录即可（与 /v1/tasks 同级），不要求配对角色。配对链授权的是"手机指挥电脑执行副作用"；记忆路由管理的是账号自有镜像数据——device-continuity 明言"记忆管理分别授权"，v1 该轴 = 账号认证。
- **路由**：`GET /v1/memories`（手机列表，仅活跃）、`GET /v1/memories/sync?cursor&generation`（桌面拉取）、`POST /v1/memories`（桌面 outbox push）、`POST /v1/memories/{id}/update|delete`（手机写）。请求体 deny_unknown_fields（手写平铺结构而非 tagged enum——后者无法拒绝未知字段）。
- core 层新增 `MemorySyncError`（稳定机器码）与 `validate_synced_body`（同步只做形状校验、接受 task_fact——手动创建拒绝它，但桌面可能已有该类行，对端编辑不应因形状被拒）。

## 验证（C1）

- `cargo test --workspace --locked` 全绿：storage 89 例（新增 9 例覆盖冲突矩阵/墓碑复活拒绝/容量/幂等日志与裁剪/压缩门与追平/512 溢出代际滚动/死会话解除阻塞/跨账号隔离）、coordinator 13 例（新增 HTTP 集成：双真实登录走完整回环、机器码断言、形状严格性、跨账号隔离）。
- `cargo fmt --check` / `cargo clippy --workspace --all-targets --locked` 干净。

## 桌面（C2，待续）

## 手机与验收（C3，待续）

## 诚实边界

随 C2/C3 完成后在此汇总（冲突放弃不 rebase、轮询级收敛延迟、同步持锁最坏时长、outbox 数据目录级语义等，见各 PR 记录）。
