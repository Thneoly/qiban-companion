# 个人记忆服务Rust化：交付记录

> 2026 年 9 月 26 日。把 `~/.personal-memory/server.py`（Python 282 行，仅 MCP stdio）重写为本仓库的独立 Rust 服务 `apps/memory-service`，同一份 `memory.db` 增加 MCP 与回环 HTTP 双协议，为"本机 → 局域网 → VPS → 互联网流转"的演进打基础。工具语义、数据与 Claude Code 配置兼容性以本文为准；运行手册见[个人记忆服务指南](../development/personal-memory.md)。

## 实现与归属

- 归属：`apps/memory-service` 单 crate（lib+bin，包名 `memory-service`），完全自包含，不依赖 `companion-core`/`companion-storage`——个人记忆（七类、重要度、取代链）与应用内有限记忆（三 Kind、容量上限、分模型许可）是两个领域模型，且该服务需以单一 exe 部署在 `~/.personal-memory` 独立存活。归属约定同步写入 [AGENTS.md](../../AGENTS.md)。
- 存储（`src/store.rs` + `schema-v2.sql`）：`PRAGMA user_version` 门控的 v2 结构，在 v1 全部列上增加全局单调 `seq`（`memory_meta.next_seq` 分配，JS 安全上界 2^53-1，溢出拒绝写入）与 `origin`（写入来源戳，`mcp`/`http`）。时间戳保持 TEXT UTC `YYYY-MM-DD HH:MM:SS`，字符串比较语义与 `datetime('now')` 一致。
- v1→v2 迁移：检测（12 列精确匹配的 Python 旧表）→ 预读校验（类型枚举、悬空 `superseded_by`/`contradicts` 引用、importance 范围；任何一条不过即 fail-closed，文件与目录不留任何新文件）→ 文件级备份 `memory.db.v1.bak`（仅首次创建，崩溃重试不覆盖）→ 单事务内 `defer_foreign_keys` + 重建 + 按 id 保序重插（seq 1..N）→ AUTOINCREMENT 续接旧最大 id。
- 并发：WAL + `busy_timeout(5s)` + `BEGIN IMMEDIATE` 写事务。**这是对仓库桌面存储先例（单进程 `Mutex<Connection>`、非 WAL）的记录在案偏离**：本服务天然两进程并发写（Claude Code 的 MCP 子进程 + HTTP 守护进程），WAL 是唯一无需应用层跨进程锁的方案。
- MCP（`src/mcp.rs`）：同步 stdio JSON-RPC；六个工具名与中文描述逐字沿用 Python 版（`~/.claude/CLAUDE.md` 的既有指令依赖这些名字）；无 id 消息一律不产生输出；坏行静默跳过；每请求 `catch_unwind`，panic 转为错误响应保进程存活；stdout 只输出协议帧。
- HTTP（`src/http.rs`）：axum 0.8 回环监听（默认 `127.0.0.1:4322`），`/healthz` 门外；`/v1/*` 门内有请求体上限、并发信号量（16，超发 429 `busy`）、`no-store`/`nosniff` 响应头；错误封套 `{"error":{"code"}}`。端点：memories CRUD、supersede、forget、stats、`/v1/sync?since=`（变更行 + 当前最大 seq）、`/v1/personality/summary`（确定性聚合，非生成式）。

## 与 Python 版的语义差异（有意为之，逐条记录）

| 行为 | Python 版 | 本版 | 理由 |
|---|---|---|---|
| `memory_update` 未知 id | 静默返回成功 | 报错 | 修复"静默失败"（R2R session 末尾实际发生过声称写入但未落库） |
| recall 负数/0 limit | 0 返回空、负数为无限返回 | 一律钳制到 1 | 防整库倾倒 |
| recall 传入枚举外 `type` | 静默返回空列表 | 报 tool error | fail-fast，与 remember 的枚举校验一致 |
| `importance` 传 0 或越界值 | 库 CHECK 约束报错 | 入参校验报错 | 均为报错；本版不再静默改写为默认值 |
| stats 的 by_type/by_project | 含已过期行（仅排除已取代） | 仅统计活跃行 | 与 active 口径自洽 |
| 重复取代已被取代的行 | 允许（产生链分叉） | 拒绝 | 保持取代链线性 |
| 对通知消息回错误帧 | 偶发 | 永不响应 | 符合 JSON-RPC/MCP 规范 |
| stdin 出现非法 UTF-8 字节 | 替换字符后跳过该行 | 同左（lossy 解码后跳过） | 保持一致，防整会话中断 |
| 输出编码 | UTF-8 wrapper 修补 | 原生 UTF-8 | 移除 Windows GBK 适配层 |

## 验证（本机，2026-09-26）

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | 通过 |
| 静态 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 警告 |
| 单元+集成 | `cargo test -p memory-service --locked` | 33 库测试 + 2 子进程集成测试全过 |
| 全工作区 | `cargo test --workspace --locked` | 全过（含既有 desktop/storage/core） |
| 文档 | `npm run verify:docs` | 通过 |
| 真实数据演练 | 副本迁移 + HTTP + MCP 走查 | 见下方记录 |

测试覆盖点：六工具往返与 Python 字段名逐一对齐；迁移保留 id/内容、首个新 id = 旧最大+1、AUTOINCREMENT 底线恢复（迁移前删除过最高 id 也不复用）、备份只建一次、重开不重复迁移；malformed 旧库（含超长标题、空内容、非标准时间戳、REAL importance、悬空/自引用——均为 Python 可产出状态）fail-closed 且文件字节不变、不留备份；sync 游标为页内最大 seq（分页不跳行）；复合读（stats/personality/chain/get+chain）单读快照；同进程双连接交替写 seq 无重号；MCP 子进程存活期间外部连接提交的数据立即可见（跨进程 WAL）；HTTP CRUD/坏输入 400（含路径参数走统一错误封套）/缺行 404/sync 分页/429 饱和。

### 审查修订（2026-09-26）

实现后对本 crate 做了多维度对抗审查（迁移数据安全/并发/MCP 兼容/HTTP 面/文档声明核对，确认项逐条修复、反驳项弃置），主要修订：v1 预校验补全到整个 v2 CHECK 面（否则 Python 可产出的越界行会在备份创建后于重建时永久卡死启动）；`changes_since` 游标改为同一读事务内的页内最大 seq（原全局最大值会跨快照跳行）；复合读统一走单读快照；MCP stdin 非 UTF-8 字节不再中断会话；锁中毒后可恢复；HTTP 路径参数拒绝走统一封套；请求体上限提高到覆盖 \u 转义最坏情形；serve 拒绝非回环绑定（Phase 2 前无鉴权不该暴露）。

## 真实数据演练记录（2026-09-26，已完成）

复制真实 `memory.db`（9 行，活跃 8）到临时副本，`QIBAN_MEMORY_DB` 指向副本分别跑 `serve` 与 `mcp`：`/v1/stats` total=9/active=8（分组与逐条内容核对无误），`/v1/memories?limit=50` 8 条活跃，`/v1/sync?since=0` 9 行 currentSeq=9，`/v1/memories/1` 含取代链；副本旁生成 `.v1.bak`（user_version=0、9 行原样），主库 `user_version=2`、`journal_mode=wal`；serve 运行期间经 MCP 子进程写入 id=10 成功且 HTTP 侧立即可见（双进程并发写验证）；演练副本与进程已清理。

## 正式切换（2026-09-26 已执行并验证）

- 手工备份 `memory.db.manual-backup`（v1 原样）已创建。
- `cargo build --release` 后 exe 已部署为 `~/.personal-memory/memory-service.exe`；部署副本在真实数据副本上再次验证（stats 9/8、recall 命中）。
- `~/.claude.json` 的 `mcpServers.personal-memory` 已指向新 exe（args `["mcp"]`）。
- **真实库迁移已触发并核实**：经部署 exe 的 `memory_stats` 调用执行，主库 `user_version=2`、`journal_mode=wal`、10 行（含迁移前最后一笔 Python 写入）、`max_seq=10`；自动备份 `memory.db.v1.bak` 为 10 行原始 v1。旧 `server.py` 保留在原处不再被引用。

## 切换后事故记录（2026-09-26）

切换约 4 小时后出现持续 `database is locked`：所有外部写连接（含本服务 mcp 子命令）无法获得 WAL 写锁，读正常。逐进程隔离定位为**旧 Python MCP 服务进程**（切换时仍在运行的会话所属进程）长期持有写锁：迁移后旧服务的每次调用会在 v2 库上执行 `executescript`（创建 v1 命名的旧索引，属写操作）、其 `tool_remember` 因 `seq NOT NULL` 必然失败且错误路径不关闭连接，异常 traceback 的引用循环使泄漏连接滞留。结束旧进程后锁立即释放，写入恢复（补写记忆 id=13），守护重启正常。

**教训（已写入下方边界）**：切换步骤"确认无残留 python server 进程"在执行时因旧进程属于活跃会话而被跳过——切换清单必须包含"终止所有旧服务进程"，仅改配置不够。新服务在事故中的行为符合设计：fail-closed、明确报错、不损坏数据。

## 产品与准出边界

- 本轮只交付服务本体；栖伴桌面/手机接入 HTTP API 属下一增量，未实现。
- HTTP 无鉴权，serve 拒绝非回环绑定（`QIBAN_MEMORY_ADDR` 传非回环地址会直接退出）；Phase 2（局域网）再叠加认证中间件（`src/http.rs` 中已标注插入位置）并放开绑定。
- 打开旧库做迁移期间持有写锁：若另一个进程恰好在此窗口启动，可能在 `busy_timeout(5s)` 后失败退出，重试即恢复；本机 9 行数据迁移为毫秒级，不构成实际问题，大库迁移需预留窗口。
- `POST /sync/push` 未实现：当前没有第二台设备可验证多写者冲突合并，按"不建设无法验证的能力"纪律延后；`seq` 与分页游标基础已就绪。
- 旧 `server.py` 代码与 v2 库不兼容（会创建旧索引、写路径必然失败并泄漏连接）：切换后不得再用任何旧服务进程连本库；切换清单必须包含终止旧进程。
- 检索仍是 SQL `LIKE`（`%`/`_` 不转义，与 Python 一致）；FTS5/trigram 语义检索是记录在案的升级路径。
- `contradicts` 列保留但无工具使用（R2R 矛盾标记的占位）。
- 非Windows路径矩阵、干净设备验收未执行；本记录为本机工程验证，不宣称外部 CI 或产品门通过。
