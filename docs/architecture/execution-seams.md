# 执行缝与任务事实：技术设计

2026-10-11 设计稿（未实现，已经过一轮对抗审查修正）。[产品语义](../product/task-facts.md)｜[验收设计](../quality/task-fact-acceptance.md)。锚定现有代码：`ExecutionStore`（companion-storage/execution.rs）、`prepare_document`（companion-core/execution.rs，document_excerpt 的领域逻辑在此，remote_documents 只做 claim/admit/receipt 编排）、`ActionScope`/`ACTION_SCOPES`（companion-core/authorization.rs）、组装点 chat.rs 的 system+reference_block+personal_block 序列、memories 表的 `task_fact` 预留（M1 起列级 CHECK 含三值、**行级 CHECK 只允许 preference/experience 活跃行**、手动创建拒绝、跨端 v5/v7 已含其镜像与跳过物化规则）。

## 引言：吸收的三条外部不变式

对照 [deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) 架构文档，采纳其三条工程不变式，**不采纳**其插件生态与组合框架（第三方代码挂载宿主与本项目"类型化工具、不接入模型自动决定授权"的模型直接冲突）：

1. **Model-visible means logged**：凡发给模型的内容可从本地日志重建（→ §4 发送审计）。
2. **Seam 三角色**：能力 = Definition + Provider + Consumer；换 Provider 整个执行世界迁移（→ §2）。
3. **工具输出契约**：每个执行能力声明规范输出 schema 与纯投影（→ §3）。

## 0. 承重变更清单（先说清动哪里）

task_fact 不是"零 schema 改动兑现预留"——对抗审查确认了四层承重点，全部列入本设计：

| 层 | 现状 | 变更 |
|---|---|---|
| chat-history `memories` 行级 CHECK | 活跃行只允许 preference/experience（memory-schema.sql） | v6 放宽为三值 |
| `read_active`/`validate_schema` | 遇 task_fact 行直接 `Unavailable`（memory.rs kind match） | kind 感知重构：task_fact 行独立校验（无 event_date 要求、source_kind=`executor`）；**全局 30 校验拆为手动 ≤30 / task_fact ≤50 分列** |
| 同步引擎两处容量检查 | 桌面物化前 `count(*)` 全 kind 比 30（memory_sync_outbox）；服务端 create 容量同样全 kind（accounts/memory_sync.rs） | 两处都按 kind 分列（纯查询逻辑，**无 schema 迁移**——实现时确认协调库不需要 v8） |
| 服务端 peer-update 路由 | 接受 kind=task_fact（手机可更正任务事实正文，违背来源唯一） | 拒绝：task_fact 只可 delete，不可 create/update（同为查询逻辑，无迁移） |

## 1. task_fact 沉淀管线（主增量）

**写入路径**：执行终态 → 宿主 → chat-history 的 memories 行（kind=task_fact，source_kind=`executor`）。新增内部写入原语（不走 `MemoryDraft::validate`，用自己的形状校验），且**显式调用 `memory_sync_outbox::enqueue`**——enqueue 是内联在四个 mutator 里的函数而非触发器，"复用同步管线"指调用同一函数，不是免调用。

**身份与重做**：新增 v6 表 `task_fact_index(task_fingerprint TEXT PRIMARY KEY, memory_id)` 维护任务↔事实映射（memories 表不加列）。**重做 = 新行**：同 fingerprint 的新任务终态 → 旧行置墓碑（走域删除但见下"不清聊天"例外）、新 memory_id 落新行、映射指向新行——不复用旧 id（旧 id 的墓碑在本地与服务端都保留，复用会在两端撞"墓碑不可复活"禁令）。**用户删除后重做**：删除把旧行置墓碑并清映射；重做落全新 id，服务端是新条目不触 resurrection。

**两阶段与对账**：登记表 `execution_fact_pending(task_id)` 放 **execution.db（v2）**——与执行终态事件真正同事务（chat-history 是另一个 SQLite 文件，放那边同事务是伪命题）。宿主在执行事务提交后读登记→写事实→删登记。崩溃窗口由对账闭合：启动与每次 pull 循环后扫终态任务，登记在而无映射的补写（按 fingerprint 幂等，重复对账不产生第二行）。

**事实内容**（≤200 字，纯函数投影，不读原文件）：任务描述截断 + 结果状态 + 日期。

**上限与驱逐**：task_fact 活跃 ≤50；沉淀时超限挤掉最旧（created_at 序）。**驱逐只置墓碑、不触发精准清**：驱逐是系统行为而非用户决定，不应静默清用户聊天（精准清的确认门语义保留给用户动作）；被驱逐事实的旧引用残留在已发历史里，与"历史不回改"一致，记入产品边界。**手动记忆的 30 条与条目永不被触碰**（分列后由校验结构保证）。

**失效协调与跨端**：task_fact 的用户删除/更新走既有记忆域（epoch、精准清、enqueue/ack）；跨端展示与删除复用既有管线。**非零代码但小**：服务端 v8（容量分列+peer-update 拒 kind）、桌面物化容量分列、contracts/面板放行展示。旧桌面跳过物化规则不变（skipped_kind 路径）。

**schema 汇总**：chat-history **v6** = memories 行级 CHECK 放宽 + `task_fact_index` + `chat_dispatch_log` + `reminders`；**v7** = dispatch log 定稿（加 prompt、history_turns 取代 id 区间——6 轮全局逐出会打破区间，诚实的记录是"当时携带了几轮"）；execution.db **v2** = `execution_fact_pending`；协调库**无新版本**（容量分列与 peer 拒更正均为查询逻辑）。

## 2. 执行器缝（ExecutorProvider）

`document_excerpt` 的领域逻辑在 companion-core 的 `prepare_document`（校验/指纹/产物名），编排（claim→admit→execute→receipt）在 remote_documents 循环。缝设计提取**执行世界**为三角色：

```rust
/// Service Definition：执行世界的能力面。本地实现是默认 Provider；
/// 沙箱/远程是将来的一次 Provider 替换，消费者不动。
trait ExecutionProvider {
    fn slugs(&self) -> &'static [ActionScope];               // 该世界能执行的 scope
    fn prepare(&self, request: &PreparedAction) -> Result<ExecutionPlan, ExecutionError>;
    fn execute(&self, plan: &ExecutionPlan, signal: Cancel) -> Result<Artifact, ExecutionError>;
}
```

- **Provider**：`LocalFilesystemProvider`（prepare_document 迁入 + 现有写根约束）。
- **Consumer**：remote_documents 循环与桌面本地交办入口（T17 阶段二）。
- 契约不变量：execute 只返回 `Artifact`（存在性+哈希+路径），不返回内容给模型层；取消传播沿用既有序列。
- 编译期静态注册（与 ACTION_SCOPES 同构），无运行时插件加载。

## 3. 工具输出契约

每个 scope 静态声明（companion-core）：

```rust
struct ScopeContract {
    slug: &'static str,
    /// 成功产物的规范形状（存在性/哈希/结果状态枚举）。
    output_schema: JsonSchema,        // 校验 ExecutionPlan 与 Artifact
    /// 纯投影：task 终态 -> TaskFactDraft。不读文件、不依赖环境。
    fact_projection: fn(&ExecutionTask) -> TaskFactDraft,
}
```

机械保证：**"任务事实只来自执行系统"**（fact 只能由 ScopeContract 投影产生，宿主无其他入口，服务端 peer 路由也拒收）；**"产物存在与内容标准分开验收"**（output_schema 只证存在性与完整性）。模型可见的工具 schema（若未来暴露）按显式白名单序列化。

## 4. 发送审计不变式（model-visible means logged）

**不变式**：每次发给模型服务的请求，其全部模型可见组成可从本地日志重建。落点在 chat.rs 组装序列之后、发送之前。

`chat_dispatch_log`（v6 新表）**独立生命周期**——审计行先于轮次产生（取消/超时/epoch 失配的请求没有轮次行），且聊天日常全局逐出（6 轮/1.2 万字）与精准清都不该销毁审计，否则审计窗口静默退化：

- 保留策略：环形最近 **200 条**（超出删最旧），随"全部删除聊天"整体清空；不挂轮次级联。
- 每行：request_id（UNIQUE——同 id 重发是客户端 bug，拒绝而非静默复制历史）、结果状态（成功/取消/超时/失配丢弃——失败请求也入账，这是审计的意义；流中被"确认并开始新对话"打断记**取消**，显式取消先于迟到回包的失配丢弃）、model/base_url/时间、system 版本号、应用记忆块全文（≤800 字，空段为显式 NULL 标记）、个人块摘要、**当时携带的轮数**与本轮 prompt 全文（id 区间会被 6 轮全局逐出打破——记数量是诚实上限；prompt 让无轮次行的失败请求也可完整重建）、实际条数/字数。
- 重建规则：system 按版本号查常量；记忆块全文与 prompt 在行内；其余按摘要指向。**审计是观察者**，不进任何模型请求。
- 与发送边界行的关系：同源于组装函数，预览=下一次，审计=已发生，一致性列入验收。
- 范围：桌面文字链与文档流确认消息；语音链（voice 轮次）暂不入账，记为边界。

## 5. 提醒（T18a 挂点）

chat-history v6 顺带 `reminders(id, due_at, message, status)`；桌宠侧到点提示走既有通知路径；重启恢复=启动扫描；取消即删。**创建者只能是用户经 UI**——模型不可调用、不产生 task_fact、不经配对授权（纯本机无外部副作用，故无 ActionScope；这条豁免线以"用户手设"为前提钉死）。UTC 存储本地显示。

## 6. 不做的事

- 不做动态插件/脚本执行；不做模型自动授权。
- 不做交谈式提取（不扫聊天生成事实）。
- 本批不加新 scope（T17"读取资料/引用"是阶段二，走既有配对授权面）。
- 审计不覆盖语音链（后续单独评估）。
