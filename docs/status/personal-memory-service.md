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
| recall 负数 limit | 语义为无限返回 | 钳制到 1 | 防整库倾倒 |
| 重复取代已被取代的行 | 允许（产生链分叉） | 拒绝 | 保持取代链线性 |
| 对通知消息回错误帧 | 偶发 | 永不响应 | 符合 JSON-RPC/MCP 规范 |
| 输出编码 | UTF-8 wrapper 修补 | 原生 UTF-8 | 移除 Windows GBK 适配层 |

## 验证（本机，2026-09-26）

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | 通过 |
| 静态 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 警告 |
| 单元+集成 | `cargo test -p memory-service --locked` | 29 库测试 + 2 子进程集成测试全过 |
| 全工作区 | `cargo test --workspace --locked` | 全过（含既有 desktop/storage/core） |
| 文档 | `npm run verify:docs` | 通过 |
| 真实数据演练 | 副本迁移 + HTTP + MCP 走查 | 见下方增量记录 |

测试覆盖点：六工具往返与 Python 字段名逐一对齐；迁移保留 id/内容、首个新 id = 旧最大+1、备份只建一次、重开不重复迁移；malformed 旧库 fail-closed 且文件字节不变；同进程双连接交替写 seq 无重号；MCP 子进程存活期间外部连接提交的数据立即可见（跨进程 WAL）；HTTP CRUD/坏输入 400/缺行 404/sync 分页/429 饱和。

## 真实数据切换记录（2026-09-26）

- 演练：复制真实 `memory.db`（9 行，活跃 8）到临时副本，`QIBAN_MEMORY_DB` 指向副本分别跑 `serve` 与 `mcp`：`/v1/stats` total=9/active=8，`/v1/memories?limit=50` 9 行，`/v1/sync?since=0` 9 行 currentSeq=9，副本旁生成 `.v1.bak`，`user_version=2`、`journal_mode=wal`。
- 正式切换：手工再留一份 `memory.db.manual-backup`；`cargo build --release`；拷贝 exe 至 `~/.personal-memory/memory-service.exe`；更新 `~/.claude.json` 的 `mcpServers.personal-memory` 指向新 exe（args `["mcp"]`）；新会话验证 `memory_recall` 命中。旧 `server.py` 保留在原处不再被引用。

## 产品与准出边界

- 本轮只交付服务本体；栖伴桌面/手机接入 HTTP API 属下一增量，未实现。
- HTTP 仅回环、无鉴权；Phase 2（局域网）再叠加认证中间件（`src/http.rs` 中已标注插入位置）。
- `POST /sync/push` 未实现：当前没有第二台设备可验证多写者冲突合并，按"不建设无法验证的能力"纪律延后；`seq` 基础已就绪。
- 检索仍是 SQL `LIKE`（`%`/`_` 不转义，与 Python 一致）；FTS5/trigram 语义检索是记录在案的升级路径。
- `contradicts` 列保留但无工具使用（R2R 矛盾标记的占位）。
- 非Windows路径矩阵、干净设备验收未执行；本记录为本机工程验证，不宣称外部 CI 或产品门通过。
