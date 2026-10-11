# 跨端记忆删除同步（T23 / G3）

2026-10-11 交付。有限记忆（T13/T14）最后遗留项：断线和更正不使状态回退——事件游标、去重、版本、更正/删除同步与补发（[G3 的 20 例](../quality/limited-memory-acceptance.md)）。设计依据 [device-continuity.md](../architecture/device-continuity.md)。范围 = 本机应用记忆（≤30 条、body ≤200 字）；**不含**个人记忆服务、模型使用许可 policy、聊天正文、手机端新建（本地域是唯一创建入口，服务端镜像不含创建路由之外的来源）。

分三个 PR 交付（先引擎后入口）：C1 服务端（分支 `feat/memory-sync-server`）→ C2 桌面（`feat/memory-sync-desktop`）→ C3 手机与验收。

## 服务端（C1）

协调库 schema v6 → **v7**（`companion-storage/src/accounts/memory_sync.rs`）：

- **镜像与墓碑**：`account_memories` 按账号存全部行；墓碑 = body/event_date 置 NULL + deleted_at 非空（表 CHECK 强制）；kind CHECK 与本机三值一致（含 task_fact——服务端接受形状，但本机永远不会产生该类行）。**压缩只删事件、永不删行**——被删的 id 永远不能复活。
- **事件流**：`memory_events` 全局 AUTOINCREMENT seq；`memory_devices` 按会话记确认游标；`memory_accounts` 记代际与 watermark。**游标推进只认客户端下一轮申报**（at-least-once）：pull 递出事件不动游标，丢失响应只会导致重拉，压缩不会删掉没人确认过的事件。谎报超前游标（超出服务端发行过的最大 seq）降级为快照。
- **写路径**：桌面 push 与手机更正/删除共用同一规则集；`memory_requests` 幂等日志按 requestId 重放（负载指纹绑定，同 id 异负载按冲突拒；每账号裁剪至恰 256 条）。版本不匹配 → `memory_revision_conflict`；写墓碑 → `memory_resurrection`；活跃满 30 → `memory_capacity`（与本机 MAX_MEMORIES 同源）。同一删除响应丢失后重放幂等（checked_add 防溢出）。
- **压缩与窗口**（同事务）：先清扫死设备行（撤销会话，或 native issuer token 过期/闲置 30 分钟——与配对链 live() 同判活口径；**被清会话在认证意义上已死**：撤销不可刷新、闲置 token 被 verify 拒绝，回归必是新会话 → 首拉走快照，不可能从被压缩的尾巴续传）；再删所有存活设备已申报的事件；超 512 条 → 代际 +1、watermark 前移、清空。**无存活设备时事件保留**（写入者自己靠回执，但其他端仍需要事件；512 是硬上限）。落后设备（代际不符或游标低于 watermark）自动降级为快照。
- **鉴权口径**：账号登录即可（与 /v1/tasks 同级），不要求配对角色——配对链授权"手机指挥电脑执行副作用"，记忆路由管理账号自有镜像数据（device-continuity"记忆管理分别授权"，v1 该轴 = 账号认证）。
- **路由**：`GET /v1/memories`（手机列表，仅活跃）、`GET /v1/memories/sync?cursor&generation`（桌面拉取）、`POST /v1/memories`（桌面 push）、`POST /v1/memories/{id}/update|delete`（手机写）。请求体 deny_unknown_fields（手写平铺结构——tagged enum 无法拒未知字段）；delete 携带 kind/body/eventDate 按 400 拒（模板复用是客户端 bug，不静默吞）。
- 共享 wire 类型（快照项/事件/响应/回执）放 `companion-core::memory`——桌面反序列化的就是协调器序列化的那一份。

## 桌面（C2）

chat-history.db schema v4 → **v5**（`companion-storage/src/memory_sync_outbox.rs`）——outbox 必须与 memories 同库同事务：

- **入队即折叠**：memory_create/update/delete/delete_all 在各自事务内入队，每条记忆至多一行 pending（create+update→create、update+update→一行、update+delete→delete、create+delete→净零）。**折叠必换 requestId**——在途旧 push 的迟到回执既删不掉折叠行也污染不了 mirror。
- **base 在 push 时按当前 owner 的 mirror 解析**：行不含 owner——update 遇到"该 owner 的服务端没见过此条"降级为 create（账号切换=合并语义）；delete 无 mirror = 净零本地清行。
- **事件应用真值表**（本机与服务端 revision 永不比较，mirror 是唯一汇合点）：新 id 物化为 user_manual 行；活跃行走域更新（含精准清；选择预算超限在同事务内自动取消该选择）；删除走与本机删除共享的 `tombstone_and_prune`；**无 pending 的墓碑被服务端活跃版复活**（聊天不恢复——诚实边界）；task_fact 只记 mirror 永不物化；**pending 行永远优先于远端条目**（只推 mirror）。upsert 的幂等门是 `mirror >= revision`（历史不倒退）；**delete 永不按 mirror 跳过**（快照会预记墓碑 revision 而本地无行可删）。
- **第三条独立 5s 循环**（`apps/desktop/src-tauri/src/remote_memory.rs`）：每 tick 先 PULL 后 PUSH（重连先应用删除再恢复上传）；锁纪律——chat 锁只套同步存储操作绝不跨 await，account 锁可跨 HTTP（remote_documents 先例），两锁不同持。**pull-first 顺带把 pending 行 rebase 到最新 base（按 revision 的后写胜出）**；409 只覆盖 push 先于 pull 的竞态窗口——此时**放弃=删行+清该条 mirror+游标归零**，下一 tick 快照三向 diff 收敛；永不改 base 重推。`memory-changed` 增加 serde-default 的 `remote` 字段（旧 UI 兼容）。

## 手机（C3）

- BFF 白名单加 `GET /api/memories` 与 UUID 路径的 `update|delete` POST；三个 409 码进 knownErrors；push/sync 等 desktop 专路 404（有网关测试）。
- contracts 严格 decode（uuid/三值 kind/活跃 1..200 字/墓碑无内容/日期形状——历法语义留在服务端）。
- `MemoryPanel`：查看+更正+删除，仿 DocumentPanel 纪律（generation 防旧响应、10s 可见性轮询、acting 防并发、不可达降级、后台刷新见到 revision 移动自动关编辑框）；确认措辞"从所有设备移除/同步到所有设备；电脑上引用它的对话会被清理"，**无"开始新对话"**（手机看不到电脑聊天）；冲突映射文案+自动刷新。

## G3 的 20 例执行记录

载体分工：Rust 集成（真 axum 路由 + 真 AccountClient HTTP + 真 HistoryStore，`apps/desktop/src-tauri/src/account/tests.rs`）、协调器 HTTP 测试（`apps/coordinator/src/tests.rs`）、storage 单测（`memory_sync.rs` / `memory_sync_outbox.rs`）、native 真机（`apps/desktop/tests/native/memory-sync.cjs`，走真实 UI）。

| 组 | 案例 | 载体与断言 |
|---|---|---|
| A 收敛基础 | A1 手机删→桌面墓碑+收敛 | 集成 end_to_end：phone delete → desktop 墓碑；列表清空；laptop 快照同收敛 |
| | A2 桌面断线删→重启补发→服务端墓碑 | 集成 offline_delete：f.stop()/restart()，镜像止于墓碑 revision |
| | A3 第二桌面快照收敛 | 集成 end_to_end laptop 段（第二 HistoryStore；同 session 只共享服务端游标，两库各自收敛） |
| | A4 手机更正→桌面域更新路径 | 集成 end_to_end（"prefer coffee"）；精准清语义在 storage 单测 remote_events + 既有 q6 域测试 |
| B 更正冲突 | B1 并发更正→409→放弃→快照收敛 | 集成 conflict：stale base 409、drop 清 mirror+游标、下一 tick 三向 diff 落"phone wins" |
| | B2 连改两次折叠+首推冲突放弃 | storage mutators（折叠换 id）+ 集成 conflict（不 rebase） |
| | B3 手机 update 命中墓碑→resurrection | 协调器 HTTP：memory_resurrection 409 零变更 |
| | B4 手机 delete 落后 revision→conflict | 协调器 HTTP：409 + 自动刷新路径（BFF 映射文案） |
| | B2b 冲突放弃后复活收敛 | storage remote_events：墓碑+upsert → 复活原语（聊天不恢复） |
| C 离线补发 | C1 折叠按序补发+requestId 幂等 | storage journal 重放（同 id 同回执）+ 折叠矩阵；集成 offline |
| | C2 先 pull 后 push 的删除优先 | storage remote_events pending-defer + resurrection drop；循环 pull-first |
| | C3 一批事件按序应用 | storage 分页测试（严格 seq 递增、满页断言）+ remote_events |
| | C4 同批重放两遍零副作用 | mirror>=revision 幂等门（echo 断言 changed=false） |
| D 边界防线 | D1 512 溢出→代际→快照 | storage window_overflow（generation≥2、事件清空、旧 cursor 快照） |
| | D2 未确认设备阻塞、确认后压缩 | storage 双设备测试（MIN 可区分；申报制游标） |
| | D3 撤销/过期会话解除阻塞 | storage departed_sessions（sign_out + 闲置 token） |
| | D4 owner 切换合并 | storage snapshot_diff（本地独有条目→补推 create）+ resolve_push 降级（update→create） |
| E 崩溃竞争 | E1 本机事务中途崩→同生同灭 | enqueue 与变更同事务（q6 crash 注入点现含入队）+ native case 1（重启后行仍在） |
| | E2 push 达服务端响应丢失→重放幂等 | 服务端 journal 重放（同 requestId 同回执；集成 offline 镜像断言） |
| | E3 批应用中崩→重拉幂等 | 一批一事务：全落地或全回滚（游标只在批末推进；SQLite 提交语义，同 q6 先例） |
| | E4 远端删除到达时流在途 | apply 后 invalidate_memory 取消活跃流 + 前缀重载；迟到回包由既有 epoch 门拒绝（DEL12/q6 既有覆盖） |

**两轮 PR 前对抗审查**（2026-10-11，Workflow 四维×3 票）：C1 抓到 13 条确认（含 HIGH：pull 在递出时推游标——丢失响应永久跳过事件，已改客户端申报制；MIN/MAX 压缩不可区分、delete 形状宽松等，全部当场修复并有测试钉住）。C2/C3 抓到 8 条确认，全部修复或文档化：HIGH——远端物化无本地容量预检（并集 >30 时 apply 整体失败、循环永久卡死；现为满载跳过+计数+记 mirror，游标照常推进）；在途 create push 与 create+delete 折叠的竞态（净零折叠让行消失、迟到 ack 无从记服务端事实、服务端遗留幽灵条目并在下次 pull 复活——现折叠保留 delete 行、ack 无条件记录服务端已接受的 revision）；pull 循环无界（服务端恒报 pending 可独占一个 tick；现有页数上限+游标不前进即退出）；每 tick 在账号锁内打 /v1/me（现缓存 owner，401 失效）；MemoryPanel 双框同开与网络失败丢草稿（现互斥关框、网络错保留编辑框）；跨账号删除不在原账号形成墓碑（合并语义固有，完整解需 per-owner 本地状态——诚实边界记录）。

## 诚实边界

1. **冲突放弃不可逆**：败者路径的本地清理（清聊天）不恢复；放弃仅由面板刷新体现（remote 标记的 memory-changed），无专门弹窗。
2. **收敛是轮询级**（桌面 5s/手机 10s，无推送）；"离线端暂停记忆注入"强门未做，以 invalidate+epoch 拒绝在途回复兜底。
3. **鉴权是账号级**：任意有效账号会话可管理记忆（无配对门槛/独立 scope）——后续可收紧。
4. **同步迭代持 AccountState 锁跨 HTTP**（remote_documents 先例），最坏 ~2×12s 超时阻塞账号命令；chat 锁绝不跨 await。
5. **远端更正致预算超限**→同事务自动取消该条选择（面板可见变化，非弹窗）。
6. **outbox 是数据目录级**（行不含 owner）：未上报变更随账号切换流入新账号（合并语义，D4 有测）；**在账号 B 下删除账号 A 时期建的条目不会在 A 的服务端形成墓碑**（delete 按 B 的 mirror 解析为净零）——重新登录 A 后该条目会经快照复活，需再删一次；完整解需 per-owner 本地状态，超出 v1 范围。单人多设备为受测形态。
7. 512 事件窗口与 30 条上限为常量（后者与 MAX_MEMORIES 同源）；512 触发代际重置后全端快照重拉。
8. **复活不恢复聊天**：被删措辞清掉的轮次保持清空。
8b. **本地容量满时远端新条目延迟物化**：本地 30 条满时收到的远端新条目被跳过计数并记 mirror（游标照常推进，循环不卡死）；本地腾出空间后需等下一次快照（窗口代际重置或冲突重置）才补齐。两端各建满 30 才会触发，正常单用户场景不遇。
9. 手机无创建；policy/聊天正文/个人记忆服务不在范围；正文明文存协调库（与邮箱/文档摘录同先例）。
10. native 真机 3 例覆盖本地半边（入队/折叠/墓碑不变式走真实 UI）；HTTP 收敛半边由 Rust 集成对真路由覆盖——真机全链（手机 UI→桌面自动收敛）依赖 `desktop_mobile_same_companion_integration` 的扩展，随下次手机联调执行。

## 验证

- `cargo fmt --check` / `clippy --workspace --all-targets --locked` 干净；`cargo test --workspace --locked` 全绿（storage 98、coordinator 13、core 15、desktop 68）。
- `npm run check` + `test:browser`（55 过 1 跳）+ BFF node:test（9 过）+ contracts vitest（66 过）。
- native 真机（2026-10-11，验收标识 `dev.qiban.companion.acceptance.m2-msync-20261011`，[memory-sync.cjs](../../apps/desktop/tests/native/memory-sync.cjs)）：真实 UI 三例——创建入队且重启后仍在（同事务+持久）、更正折叠为一行并换 requestId、删除全部后未推送的 create 全部净零且墓碑不变式保持。证据在 gitignored `.cache/`；HTTP 收敛半边由上述 Rust 集成对真路由覆盖。native 既有脚本的 schema 断言已随 v5 同步（memory-panel/memory-context/voice-chat/single-instance/q6 harness）。
