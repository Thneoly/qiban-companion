# 个人记忆服务：运行与切换指南

`apps/memory-service` 是独立的个人记忆服务（Rust，单 exe），同一份 SQLite 库同时服务两类入口：Claude Code（MCP stdio）与回环 HTTP。实现与验证证据见[交付记录](../status/personal-memory-service.md)。

## 子命令

```powershell
cargo run -p memory-service --locked -- mcp     # MCP stdio（Claude Code 拉起，无需手动运行）
cargo run -p memory-service --locked -- serve   # HTTP 守护进程，默认 http://127.0.0.1:4322
```

根目录 npm 入口：`npm run memory:mcp`、`npm run memory:serve`。没有默认子命令：直接运行会打印用法并退出，避免误挂 stdin。

## 配置

| 环境变量 | 默认 | 说明 |
|---|---|---|
| `QIBAN_MEMORY_DB` | `%USERPROFILE%\.personal-memory\memory.db`（无则 `$HOME`） | 数据库路径；目录不存在会创建 |
| `QIBAN_MEMORY_ADDR` | `127.0.0.1:4322` | HTTP 监听地址（端口族：协调服务 4318、手机网页 4320） |

无密钥、无配置文件；`serve` 只绑定回环，不做鉴权（见"演进路径"）。

## 首次打开旧库时会发生什么

Python 旧库（`user_version=0`、12 列 `memories` 表）在第一次被本服务打开时原地迁移：先整库预读校验（类型枚举、悬空取代/矛盾引用不过即拒绝，文件保持原样），再在数据库旁创建一次性备份 `memory.db.v1.bak`，然后单事务重建为 v2（保留全部 id 与内容，追加全局变更序号 `seq` 与写入来源 `origin`）。迁移后 `journal_mode=wal`，支持 MCP 与 HTTP 两进程同时读写。无法识别的库结构、更新的 `user_version` 一律 fail-closed。

## HTTP API（v1）

所有响应带 `Cache-Control: no-store` 与 `X-Content-Type-Options: nosniff`；错误统一为 `{"error":{"code":"..."}}`。请求体上限 128 KiB，并发上限 16（超发 429 `busy`）。

| 方法与路径 | 用途 | 备注 |
|---|---|---|
| GET `/healthz` | 存活检查 | 在限流门外 |
| GET `/v1/memories?query=&project=&type=&limit=` | 检索活跃记忆 | limit 钳制 1..50；project 过滤含全局记忆 |
| POST `/v1/memories` | 新建 | body `{type,title,content,project?,importance?,tags?[]}` → 201 |
| GET `/v1/memories/{id}` | 单条 + 完整取代链 | 404 `not_found` |
| PATCH `/v1/memories/{id}` | 改正文 | body `{content}` |
| POST `/v1/memories/{id}/supersede` | 取代 | body `{title,content}`；已被取代的行拒绝再取代 |
| POST `/v1/memories/{id}/forget` | 软过期 | 幂等 |
| GET `/v1/stats` | 统计 | |
| GET `/v1/personality/summary` | 人格摘要 | 确定性聚合 preference/insight/person 各前 3 条，非生成式 |
| GET `/v1/sync?since=&limit=` | 增量变更 | 返回 seq 大于 since 的行当前状态 + `currentSeq`；limit 默认 200 上限 1000 |

示例：

```powershell
curl.exe http://127.0.0.1:4322/v1/stats
curl.exe "http://127.0.0.1:4322/v1/memories?project=R2R&limit=5"
curl.exe -X POST http://127.0.0.1:4322/v1/memories -H "Content-Type: application/json" -d '{\"type\":\"decision\",\"title\":\"示例\",\"content\":\"内容\"}'
```

## Claude Code 切换步骤

1. 确认没有残留的 python server 进程。
2. `cargo build --release -p memory-service --locked`，把 `target\release\memory-service.exe` 复制到 `C:\Users\<user>\.personal-memory\memory-service.exe`（脱离 target 目录存活）。
3. 修改 `~/.claude.json` 的 `mcpServers.personal-memory`：

```json
{"type": "stdio",
 "command": "C:\\Users\\<user>\\.personal-memory\\memory-service.exe",
 "args": ["mcp"]}
```

4. 新开 Claude Code 会话，首次调用 `memory_recall` 会触发迁移并返回既有记忆；核对 `.personal-memory` 下出现 `memory.db.v1.bak`。
5. 旧 `server.py` 留在原处即可，配置不再引用它。

## 演进路径与预留槽位

本机（现状）→ 局域网多设备 → VPS/边缘 → 互联网流转。API 已按"天生可远程"设计：`/v1/sync?since=` 提供基于全局 `seq` 的增量拉取（软删除/取代都是行状态，无 tombstone 问题）。Phase 2 局域网时需要在 `src/http.rs` 的 gated 路由前叠加认证中间件（代码注释已标注位置），并实现 `POST /v1/sync/push` 与多写者冲突合并（取代链语义：更新者胜、历史保留、矛盾标记不隐式覆盖）。在只有本机可验证之前不建设这些能力。

## 可重复验证

```powershell
cargo test -p memory-service --locked          # 含真实子进程 stdio 集成测试
cargo clippy -p memory-service --all-targets --locked -- -D warnings
```

在真实数据副本上演练迁移：复制 `memory.db` 到临时目录，`$env:QIBAN_MEMORY_DB` 指向副本后运行 `serve`，核对 `/v1/stats` 与 `/v1/sync?since=0` 的行数、副本旁的 `.v1.bak`、`PRAGMA user_version`（应为 2）。
