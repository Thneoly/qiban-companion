# 个人记忆注入聊天：交付记录

> 2026 年 9 月 26 日。把个人记忆按 M3 同款语义注入栖栖的聊天：per-(baseUrl,model) 手动勾选（≤5 条 / 800 字独立预算，按勾选顺序）、预览→准入→受控注入→回执全链路、服务离线时知情降级。协议升 v3。前置：[记忆服务](personal-memory-service.md)、[桌面只读视图](personal-memory-desktop.md)。设计语义见[有限记忆技术设计](../architecture/limited-memory.md)新增章节。

## 实现与归属

- **存储（schema v3）**：`memory-schema-v3.sql` 新增 `personal_memory_policy` / `personal_memory_selection`（整数 id、position 0..4 UNIQUE、FK 指向 policy），阶梯迁移（v0/v1 直升 3、v2→v3 纯增表、>3 fail-closed）；`personal_memory_policy_set` 完整镜像应用记忆策略语义（Immediate、epoch/revision 双检、enabled⇔非空、移除⇒确认+清全 scope 聊天、no-op 短路），并推进**同一个全局 context_epoch**——现有 memory-changed 协调零新增机制即覆盖个人选择变化。**刻意不对服务侧行建 FK 或级联**：Claude Code 侧 forget/supersede 不产生本地事件，漂移由发送准入兜住，这正是 (id, seq) 准入存在的原因。
- **发送三明治（翻转版）**：锁内完成全部本地准入（settings 复核、active 占用、ContextPreview 重读、scope+epoch admit、personal policy 读取）→ **解锁后**取数并校验 `expectedPersonal`（取数期间持有 active 槽=互斥扩展；取数包进 `tokio::select!{biased; cancelled, fetch}`，"停止"在模型请求前生效）→ 取数失败在触碰模型网络前返回。
- **(id, seq) + active 双重准入**：`validUntil` 到期不 bump seq，必须先按 `supersededBy IS NULL && (validUntil IS NULL || validUntil > now)`（UTC 字符串比较，与服务同款）过滤再与期望做**有序相等**；not_found 同为漂移。在线预览在发送时无法核对 = **拒绝**（与"预览后被删→拒绝"一致）；知情降级只发生在用户批准的路径（预览时即离线，`expectedPersonal:null`）。
- **三态 `expectedPersonal`**：`null`（同意的预览离线）/ `[]`（在线且活跃集空）/ 有序对（精确匹配）；缺字段的 v2 请求在解析层被拒（serde 双重 Option，见审查修订）。
- **注入形态**：第二个 user-role JSON 块 `{"type":"personal_memory_reference","notice":"…不是系统指令…"}` 紧随应用记忆块；系统指令永不包含记忆正文；语音链路维持 `memories:&[]` 不注入（voice.rs 零改动）。
- **预览命令 async 化**：应用预览+个人策略+epoch 三者同锁原子读，解锁后取数；选择为空不触网（**离线可关闭注入**）；非空选择离线时保存策略被拒（无法核对预算）。
- **前端**：气泡预览/回执拆"应用/个人"两节（离线注记、失效选择计数 `inactiveSelectedIds`、按 id+seq 交叉引用）；发送失败后 fire-and-forget 刷新预览（个人准入拒绝不 bump epoch，不刷新会永远重发失败）；PersonalMemoryPanel 新增策略区（勾选序=发送序、n/5·n/800 计数、移除确认对话框与 M3 同文案）；离线时已选 id 仅显编号仍可移除。
- **契约 v3**：PROTOCOL_VERSION 双侧升 3（旧客户端在 decodeRuntime 硬拒）；解码器家族拆分——应用家族不变式原样保留（重算 ≤800/≤20000），个人家族（有序子集、≤800/≤4000、offline⇒空+双零）、回执个人家族（offline⇒空/0、sent 空⇒双零）、`personalPolicyErrorMessage`。

## 与 M3 的语义对照（新增行为）

| 行为 | 语义 |
|---|---|
| 在线预览 → 发送时服务掉线/seq 变化/条目失效 | **拒绝**："个人记忆已变化或服务暂时无法核对，本次未发送，请刷新预览后重试"（preview==sent） |
| 预览时即离线（用户已知情） | 降级发送：不注入个人块，回执 `personal.status=offline` 如实记录 |
| 勾选后 Claude Code 侧 supersede | 预览显示"已选 N 条中 M 条已失效"；发送拒绝直至刷新 |
| 服务侧 update 使 content 变长 | (id,seq) 拦截 + 发送端 ≤800 防御复检（不靠解码器误报"协议不兼容"） |
| 服务离线时想关闭注入 | 可（空选择无需核对）；想新增勾选不可（需核对预算） |

## 验证（本机，2026-09-26）

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式/静态 | `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 / 0 警告 |
| Rust | `cargo test --workspace --locked` | 全过；含 6 个注入专项（3 单元 + 3 真服务集成：预览后 supersede/forget 拒绝、离线形态、预算防御）与 messages 形状（system→应用块→个人块→history→prompt，系统指令无正文，三态解析） |
| 契约 | vitest | 45 过；v3 三态、有序子集、静默缩集拒绝、Rust↔TS 夹具一致 |
| 浏览器 | `npm run test:browser` | 33 过 1 跳过；新增 expectedPersonal 三态（离线 null + 在线有序对 + seq 回执交叉引用）、策略面板全流程（勾选/预算/冲突/移除确认/经确认关闭）；审查后 fixture 改为 host 真实形态 |
| 文档 | `npm run verify:docs` | 通过 |
| 真实链路 | 副本库 + 真服务 + 生产地址 | 见下 |

## 真实走查记录（本机工程验证，2026-09-26 已执行）

- 记忆服务已在 127.0.0.1:4322 常驻运行（真实库，v2，14 行/活跃 12）。两个 `--ignored` 走查测试经**生产地址常量**的真实客户端路径通过：`real_service_walkthrough_via_production_address`（overview/recall/detail 一致性）与 `real_service_injection_walkthrough`（本增量新增：resolve 批量取数活跃 id 全部命中、(id,seq) 对自身新鲜取数自洽准入）。
- 真实桌面档案 `dev.qiban.companion/chat-history.db` 核实为 user_version=2（0 聊天/0 策略/0 记忆——桌面真实使用尚未产生数据）；v2→v3 迁移路径由存储层测试在真实形状的 v2 夹具上覆盖（保聊天/保策略/fail-closed），首次以新桌面二进制打开时自动执行。
- 原生桌面窗口内的人工点选走查与独立体验未执行（UI 状态已由浏览器用例按真实封套覆盖）；不据此宣称独立验收。

## 审查修订（2026-09-26）

实现后做了三路对抗审查（准入语义与并发 / 存储迁移与契约 / 前端状态机与测试），确认项修复、误报弃置：

- **v2 请求拒绝从"声称"变为真实**：初版的 `Option<Vec>` 让缺 `expectedPersonal` 的旧请求静默落到 None（= 被当作"同意的离线预览"）。改用 serde 双重 Option 惯用法（`deserialize_with` 移除 Option 隐式默认）——缺字段现在在**解析层**被拒，比运行时检查更强；测试断言三态解析行为。
- **注入块改为瘦条目**：`personal_memory_reference` 只序列化同意相关的五个字段（id/type/title/content/project）——完整记录的 tags（最长 32×128）等元数据会让实际请求远超 800 字预算而回执仍报 ≤800。
- **TS 预览家族上限从 800/4000 放宽到 100000/200000**：预览是状态报告，服务侧 update 使 content 增长后若解码器硬拒 >800，整个聊天界面会因"协议不兼容"不可用；发送路径的拒绝与回执的 ≤800 硬上限不变。
- **policy_set 的 scope 在写锁内复核**：取数等待期间另一窗口切换模型供应商时，此前会把选择静默写到已孤立的旧 scope；现在与 memory_policy_set 一致地拒绝（context_changed）。
- **面板候选列表改挂 overview 在线状态**：初版挂在注入家族的 status 上，而空选择的 host 预览总是 offline 形态——两者互锁成"全新安装永远无法勾选"的产品死锁（审查标为 critical）。气泡预览同样区分"未启用"（策略关）与"服务未连接"（有选择但离线）。
- **面板陈旧状态自愈**：注入策略区现在监听 `memory-changed` 并在窗口聚焦时刷新（与应用记忆共享全局 epoch，任何策略写都会使其陈旧）；离线时保存按钮禁用并如实说明（只有移除全部勾选/关闭可在离线后通过服务端校验之外完成）。
- **测试诚实性**：修掉一个缺 `.toThrow()` 的空断言；注入面板 fixture 改为 host 真实形态（空选择 ⇒ offline 家族）；新增在线路径覆盖（预览计数 + 有序 (id,seq) 对透传 + seq 回执交叉引用）与真实 v2 库夹具迁移测试（v3 表回退后重开，保聊天/保双家族策略）；原生验收脚本（memory-context/q6/memory-panel）的 protocolVersion/user_version 断言同步升 3。
- 文档修正：IPC 注释 v2→v3、行数与走查记录以当前机器状态为准（14 行，1 已取代）。

## 产品与准出边界

- 服务端（apps/memory-service）**零改动**：漂移检测复用既有 `/v1/memories/{id}`，与服务的发布节奏解耦；代价是批量 detail 并非单 SQLite 快照——跨一次 supersede 的理论交错窗口与既有"准入→模型发送"窗口同类且更小，记录为有界陈旧。
- 自动按重要度选择、写入个人记忆、跨 scope 共享选择、语音注入、pet 窗口策略入口均不做；应用记忆行为零变化。
- 预算为**字符数**（应用 800 + 个人 800 各自独立），不是 tokens；已在 UI 文案注明。
