# 个人记忆桌面只读视图：交付记录

> 2026 年 9 月 26 日。栖伴桌面任务面板新增"个人记忆"只读区块，接入[个人记忆服务](personal-memory-service.md)的回环 HTTP API——检索/筛选/查看取代链；服务不在线时明确提示，不显示示例数据。本视图不写入、不注入聊天、不自动启动服务。运行说明见[个人记忆服务指南](../development/personal-memory.md)。

## 实现与归属

- **Rust 客户端与命令**（`apps/desktop/src-tauri/src/personal_memory.rs`）：`PersonalMemoryClient` 按仓库 chat/voice 约定实现（每次调用新建 reqwest Client、禁重定向、有界读取 4 MiB / 错误体 16 KiB、永不转发服务响应体），三个无状态 async 命令 `personal_memory_overview / personal_memory_recall / personal_memory_detail`（lib.rs / build.rs / capabilities/main.json 三处注册，仅主窗口）。**离线是 overview 的状态而非错误**（`{online, stats?, serviceUrl}`）；失败码结构化（服务码透传 + `service_offline` / `incompatible`），检索词经 `RequestBuilder::query` 百分号编码（`&`、`#`、中文不截断）。入参校验未过网即拒（类型七类、关键词 ≤200、项目 ≤64、limit 钳 1..50、id ≥ 1）。
- **超时为记录在案偏离**：连接 2s、healthz 3s、读 5s（仓库外网约定 10s；本机回环需要快速离线判定）。
- **TS 契约**（`packages/contracts/src/personal-memory.ts`）：手写严格解码器，分层校验——schema CHECK 不变量严格镜像（seq/importance 界、origin slug、`supersededBy ≠ id`、宽松 GLOB 时间戳逐位），schema 沉默处仅宽松 sanity（project ≤512、tags ≤32×128），避免把 Python 旧库迁移行误判为协议不兼容；stats 校验单快照不变量（`active+superseded ≤ total`、byType 之和 = active）；`personalMemoryErrorMessage` 映射稳定错误码。PROTOCOL_VERSION 不变（纯新增域）。
- **面板**（`apps/desktop/src/features/personal-memory/`）：检索（关键词/类型/项目，固定 20 条）+ 结果卡（类型徽标、重要度、已被取代/已过期状态 chip——与服务同款字符串比较）+ 可展开取代链（最旧在前、当前项 `aria-current`、`→ 被 #id 取代`）。离线块整体替换结果区（地址 + `npm run memory:serve` 启动提示 + "不会显示示例数据" + 重新连接）；检索途中服务掉线（`service_offline`）直接切离线态。浏览器预览仅显示预览注记、零 IPC（CSP 本就禁止前端直连，网络一律走 Rust 命令）。侧栏新增第四个锚点。

## 验证（本机，2026-09-26）

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式/静态 | `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 / 0 警告 |
| Rust | `cargo test --workspace --locked` | 全过；含本模块 5 个真实服务集成测试（内存中拉起真 memory-service router 于临时端口） |
| 前端 | `npm run check`（tsc + vitest + build） | 全过；contracts 新增 28 例解码测试 |
| 浏览器 | `npm run test:browser` | 31 过 1 跳过；新增 3 例（离线态/在线检索+链/预览注记零调用）；既有 8 个 spec 的 invoke mock 补齐 overview 离线回包 |
| 文档 | `npm run verify:docs` | 通过 |
| 真实端到端 | 副本库 + `npm run memory:serve` + 原生桌面 | 见下方走查记录 |

集成测试覆盖：在线/离线 overview、特殊字符检索词编码、项目过滤含全局、类型过滤、limit 钳制、取代链与 not_found 透传、非法入参不触网、超限响应拒绝。

## 真实走查记录（本机工程验证）

复制真实 `memory.db` 至临时目录，`QIBAN_MEMORY_DB` 指副本启动服务；原生桌面打开任务面板"个人记忆"：服务未启动时显示离线块与启动指引、无任何占位数据；启动后"重新连接"转为在线统计（共 N 条 · 活跃 N · 已取代 N，与副本一致）；用真实关键词检索命中；展开条目查看完整取代链；停止服务再刷新回落离线。全程未出现防火墙弹窗（桌面仅发起回环出站连接）。此项为实现者本机走查，不替代独立验收。

## 产品与准出边界

- 只读：不写入、不注入聊天上下文（注入需 M3 式发送边界与 epoch 协调，属后续增量）、不自动启动或驻留管理服务进程。
- 服务地址为常量 `http://127.0.0.1:4322`（服务侧可用 `QIBAN_MEMORY_ADDR` 改端口，桌面侧暂无设置界面——端口不一致时需等设置项，属记录在案限制）。
- 桌面窗口关闭不停止服务；两进程经 WAL 并发读写已在服务侧验证。
- 非Windows矩阵、干净设备、独立体验未执行；本记录为本机工程验证，不宣称外部 CI 或产品门通过。
