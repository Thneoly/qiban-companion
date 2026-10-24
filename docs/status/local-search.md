# 本地检索（只读，模型驱动）· 交付记录

状态：已交付（C1 #80 / C2+C3 #81，2026-10-24）。方案决定与边界见 docs/planning/delivery-backlog.md「本地检索能力·工具调用循环」。

## 能力与边界

- **L1 文件名检索 / L2 内容检索与片段读取**，仅限用户在设置里显式添加的授权目录（默认空=关闭；最多 8 个；拒绝应用数据目录与重叠目录）。
- **L3（执行/修改）明确不做**——与陪伴定位不符，风险不可控。
- 一切只读：栖栖不会修改任何文件。

## 隐私与审计（产品承诺）

- **检索结果会发送给你配置的模型服务**——设置面板与聊天边界行都如实披露（`已授权目录的检索结果也会发送给该服务`），授权变更即时刷新（`local-roots-changed` 事件）。
- **模型可见即被记录**：每次工具执行的审计行（tool_invocation_log，ring 200）存**实际发送的 8KB 摘录原文**——「什么文件内容离开了这台机器」可本地核查。审计行在**发送之前**写入（含取消场景：工具在阻塞线程内完成执行+审计，取消不再产生已读未记的缺口）。
- dispatch 行携带 tool_rounds 与轮间 assistant 文本（intermediate_text，16K 字符截断）——被取消的轮次可完整重构。
- 清空对话=同事务删除聊天与工具日志（与 dispatch log 同边界）；**授权目录是常设授权，只能经设置显式移除**。

## 循环与预算

- 授权目录非空时四个工具随请求下发（OpenAI tools / Anthropic tool_use 双协议原生形状；为空不发 tools 键，线上兼容）。
- 每轮次 ≤4 个工具轮（≤5 个 HTTP 请求）、每轮 ≤3 个调用（超限调用内联 budget 错误应答但永不执行）、总预算 120s；回放按协议渲染（OpenAI assistant.tool_calls+role:tool；Anthropic content blocks+单条 user 的 tool_result，按轮分组）。
- 引擎预算：走查 5 万节点/深 24/2 秒/扫描 8MB；结果负载 8KB 上限且永远合法 JSON；回复跨轮累计超 11000 字符提前终止（本机保存上限 12000）。
- 沙箱：模型给的路径不可信——canonicalize 包含 + reparse point/junction 拒绝 + 文本扩展名与大小门；逃逸尝试=denied 并留审计行。

## 已知边界（如实）

- 无轮询/监视——只在对话轮内检索。
- v1 无目录内容跳过清单：node_modules 等会消耗走查预算（预算封顶即 truncated 如实标注）。
- Anthropic usage 取 message_start 引导值（终值在 message_delta，未解析——已知低估）。
- OneDrive 占位文件在读取时可能被物化下载。
- 工具审计为明文（与现有归档同边界）。
- loopback 端点不走去往代理（本机服务直连；代理常见拒绝中继 loopback）。

## 工程锚点

- 引擎：apps/desktop/src-tauri/src/local_search.rs（纯函数+沙箱+预算）；命令 local_roots_get/add/remove（COM 目录选择器）三处注册。
- 存储：chat-history.db v8（authorized_roots / tool_invocation_log / dispatch 两列）；local_search_store.rs（ring/独立连接审计）。
- 循环：chat.rs run_tool_turn（≤5 请求；每轮重读授权；audit-before-send；轮顶 ensure_current；usage 跨轮求和、任一轮缺失则 None）。
- 系统提示 v2（单一条件句常量，两种状态都为真）；语音实验室保持无工具，伙伴语音经 chat_generate 自动继承。
- 测试：引擎沙箱矩阵/预算/分页单测；双协议 SSE 工具事件与 transcript 渲染单测；全链 loop 测试（fixture_sequence 多步：回放/审计摘录相等/逃逸 denied/轮上限恰 5 请求/无根无 tools 键）；browser：设置面板隐私文案+增删、toolStatus 状态行+边界披露。
