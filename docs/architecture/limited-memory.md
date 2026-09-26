# 有限记忆技术设计：T13/T14本地增量

设计基线v0.1，2026-09-17；2026-09-18已完成[M1底层增量](../status/memory-foundation.md)，2026-09-19已完成[M2面板与导出](../status/memory-panel.md)，同日完成[M3模型使用](../status/memory-model-context.md)，M4完整验收仍待执行。产品行为以[有限记忆设计](../product/limited-memory.md)为准，测试以[验收设计](../quality/limited-memory-acceptance.md)为准。

## 1. 决策与现有代码差距

M1已在`HistoryStore`增加同库记忆存储原语、删除标记及版本2迁移，并增加Windows单实例保护和Rust/TypeScript数据契约。M2已接入记忆管理IPC、原生导出、提交后的缓存失效与在途取消，流式和完成提交比较请求ID/会话版本/epoch。M3已开放策略写入、许可预览及受控注入，`ChatRequest`升级协议2并要求expectedScope/expectedContextEpoch。模型切换递增epoch，即使切走再切回也拒绝旧预览。下文作为完整设计，实际实现与测试边界以M1～M3状态记录为准。

首版不用向量库、图数据库、摘要模型或背景抽取。30条容量下按用户明确选择注入，选择超过5条或800字直接拒绝设置，不暗中截断，也不假装按语义相关性排序。记忆附加处理不产生独立推理请求；正文和元数据仍增加模型输入，实际tokens与供应商费用须测试，字符数不是tokens或人民币。

| 目录 | 拟实现职责 |
|---|---|
| crates/companion-core | Memory类型、类别、容量/日期校验、版本冲突、状态转换及选择规则；无文件/网络/Tauri依赖 |
| crates/companion-storage | HistoryStore所在库的迁移、条目/策略读写、删除/清理聊天的同库事务、快照导出；无UI依赖 |
| apps/desktop/src-tauri | 命令组装、原生文件对话框、协调锁/取消、模型配置核对、请求上下文和窗口事件 |
| packages/contracts | DTO、错误码、运行时解码，不能反向引用应用 |
| apps/desktop/src/features | 记忆面板、编辑与影响确认、气泡预览和真实使用回执；不直连SQLite/模型 |

## 2. 数据与迁移

在现有`chat-history.db`上设计版本1→2迁移，让记忆改变和聊天清理能够同事务完成；不另建无法原子清理的memory.db。不改变任务、模型设置或desktop-settings.db（后者已是独立版本2）。迁移保留所有现有问答，默认无记忆、无使用许可；未来版本拒绝覆盖，失败回滚。新增列/表用明确SQL，需拿真实版本1夹具测试升级。

| 对象 | 必需字段与约束 |
|---|---|
| memory_meta | 单行schema逻辑元数据、context_epoch（单调整数）；与revision共同限制在JavaScript安全整数范围0～9007199254740991，溢出拒绝写入，不回绕 |
| memories | id（Rust生成UUID）、kind（preference/experience/task_fact）、body、source_kind、source_label、event_date可空、created_at、confirmed_at、updated_at、revision、deleted_at可空 |
| memory_policy | 规范化base_url＋model联合唯一，enabled默认false、revision；不保存API Key |
| memory_selection | scope引用＋memory_id联合唯一，position 0～4；只可选择未删除条目；明确排序 |

新建只允许preference/experience，source_kind由宿主固定为user_manual，source_label固定为“用户在记忆面板填写”。编辑保留创建时间，更新确认/更正时间及revision；不保留旧正文历史。task_fact的结构和来源定义在本设计中保留，但IPC禁止用户或模型创建，后续T16才开放受信的内部写入路径。

编辑正文还须核对所有选择该ID的策略，若任一范围因此超过800字，整次编辑拒绝并列出受影响模型，用户先减少选择再编辑；不能保存后静默省略条目。关闭策略时清空其选择，重新开启仍须明确选择。空选择可保存为关闭状态，不能显示“已启用0条”冒充个性化生效。

正文去首尾空白，拒绝空白、NUL、非法控制字符及超过200个Unicode字符。用户填写的经历日期是本地日历日期，不伪造成精确UTC事件时刻；不填时明确“未指定日期”。系统写入时间为UTC，不用时间戳决定并发先后。来源正确表示能追溯到用户陈述，不保证陈述本身客观真实。

删除事务清空body/source_label/event_date等内容列，只保留id、kind、revision及deleted_at等最小删除元数据；删除标记不参与检索、导出或上下文。选择关系同步删除。不软删却继续保留旧正文。删除标记暂不自动压缩，上限作为待测资源项，不能为了30条有效容量擅自复用旧ID；跨端上线前另定义保留/压缩规则。开启既有secure_delete也不构成备份或取证级擦除承诺。

## 3. IPC草案

下表保留目标设计契约。M2实际注册`memory_list`（items/contextEpoch/modelUseEnabled=false）、`memory_mutate`（action=create/update/delete/delete_all，对应下表四个写操作，统一返回contextEpoch/chatCleared/notificationsDelivered）、`memory_export`（status=cancelled或saved/count），并新增pet只读`chat_context_epoch`。M3已注册`memory_policy_set`与main/pet只读`chat_context_preview`，`chat_generate`按协议2要求预览的scope/epoch。M3的`memory_list`只返回items/contextEpoch，模型许可与使用预算由preview提供；policy写入亦要求expectedScope，但只能用于比较宿主当前范围。Rust生成ID、时间、来源及版本，前端只能提交用户可编辑内容和期望版本；前端不可提交任意system消息或外部来源证明。

| 命令 | 输入重点 | 回执/失败边界 |
|---|---|---|
| memory_list | 无路径参数 | 条目、当前配置范围的策略、contextEpoch；删除项不返回正文 |
| memory_create | kind、body、eventDate、expectedEpoch | 已提交卡片及新epoch；并发旧快照拒绝，不覆盖或隐式合并 |
| memory_update | id、expectedRevision、expectedEpoch、编辑值、restartConversation=true | 同事务改记忆/清聊天/增epoch；缺少影响确认拒绝 |
| memory_delete | id、expectedRevision、expectedEpoch、restartConversation=true | 首次删除检查版本；同一删除已提交的重试返回deleted，不重建、不重复清新聊天 |
| memory_delete_all | expectedEpoch、restartConversation=true | 删除有效内容、关闭所有策略、清聊天、增epoch；旧epoch重试先重读，不再次删除后来新增数据 |
| memory_policy_set | expectedRevision、expectedEpoch、enabled、selectedIds、必要的restartConversation | 范围从宿主当前模型推导；新范围启用必须显式操作，不能传任意scope来绕开当前展示 |
| memory_export | 无路径/SQL/凭据参数 | 宿主原生保存对话框后选定位置写UTF-8 JSON；success/cancelled/error明确区分 |
| chat_context_preview | 当前模型来自宿主 | scope、epoch、选中id/revision及正文预览；disabled时列表为空 |
| chat_generate扩展 | 原有字段＋expectedScope＋expectedContextEpoch | 发送前校验快照；不接收前端传来的记忆正文，实际用哪些由Rust决定 |

统一错误包括invalid_input、capacity_exceeded、selection_too_large、conflict、context_changed、storage_unavailable、confirmation_required、export_failed；前端对未知值报协议不兼容，不把错误当空库。新增上下文必需字段属于不兼容变更，实施前把RuntimeInfo/共享PROTOCOL_VERSION升为2并更新现有模拟夹具，不能沿用v1又静默发送旧快照。

main拥有管理与导出命令；pet拥有只读预览和打开记忆面板动作，沿用聊天命令。新权限逐条授予，不让pet直接写记忆或让main发任意模型请求。

## 4. 更正、删除与请求竞争

协调对象统一负责聊天状态和记忆事务；锁顺序固定为模型设置→上下文协调锁→数据库。网络期间不持有锁。UI的影响确认不是并发锁，提交还须比较revision/epoch。

1. 用户确认更正/删除/停用等操作，宿主在协调锁内验证当前epoch与版本。
2. 单个事务修改条目/策略、清空全部chat_turns并递增epoch；事务失败不发布成功、不清内存，旧数据仍是有效状态。
3. 提交后、释放协调锁前，清空Conversation缓存、作废旧请求ID和版本，发送取消信号；后续增量和完成回调都比较epoch。确认取消只是停止本地接收，不声称撤回供应商请求。
4. 通知pet/main刷新；IPC成功回执含新epoch和刷新状态。通知失败显示“本机已提交，另一窗口待刷新”，不能笼统报删除失败诱导重复操作，也不声称所有已显示文字瞬间消失；窗口重新获得焦点或发送前必须重新核对。未刷新期间禁止使用旧预览发送。
5. 更正/删除后才获得发送准入的请求不得含旧正文；较早已准入的请求视为在途，即使底层尚在发送，也不能承诺供应商没收到。它的迟到结果不得落盘或重新显示为当前回答。

新建、启用、增加选择也递增epoch并使旧快照/旧在途结果失效，但不清聊天；移除选择、关闭、编辑、删除执行完整清理。既有chat_clear也走协调路径递增epoch、清聊天而保留记忆，UI更新说明。不是只在删除按钮处加一次状态重置。

必须补应用单实例约束后才能声明这些竞争保证：第二进程退出并引导用户使用已运行实例，不让两个独立内存缓存同时写同一资料库。验收标识可与生产标识并存，各自互斥；未实现该前置时不得开放记忆写入。M1已实现Windows插件唤回与资料目录文件锁，并验证重复启动、异常退出后重开及独立资料目录互斥；跨操作系统和Windows多用户/RDP矩阵仍未实测。

## 5. 注入、可观察性与导出

每次发送在协调锁内读取一致的策略/有效条目/聊天快照，按保存的position选取，校验总量；新模型没有policy时为关闭。构造上下文为角色规范＋标明“用户确认的参考资料”的结构化记忆块＋该模型最近问答＋本轮输入。记忆正文是资料，不提升为system/developer指令，也不授予工具、路径或系统能力；恶意“忽略规则”字样仍需负向用例。模型能否服从偏好要实测，不能靠提示词作安全保证。

请求证据记录requestId、scope、epoch、memory id/revision、数量、附加字符数和结果状态，不在诊断日志复制正文。UI的“本轮使用”指实际提交给模型的快照，并非声称模型引用了每条。删除/更正后旧证据不应保留正文，当前气泡和内存流片段需清除。

导出仅有效卡片，包含formatVersion、导出时间、类型、正文、用户来源、经历日期及创建/确认/更正时间；排除密钥、聊天全文、删除正文、使用许可与内部请求日志。不提供导入，避免旧导出绕过删除标记。导出是额外用户文件，不能自动随删除撤回。

导出快照与记忆修改同样比较epoch：文件对话框不持锁；真正写入前重新读取快照，写入阶段与修改串行。先写临时文件再完成替换，取消不产生内容文件，覆盖由原生保存流程确认，失败清理自身临时文件；是否使用插件/原生API由实施核对兼容性，禁止前端任意路径写入。可移动备份、云同步、导入重放尚未实现，不能把本地删除标记算成T23完成。

## 6. 实施拆分与门禁

| 子项 | 依赖与准入 | 可验收准出 |
|---|---|---|
| M1／T13 单实例、领域与迁移 | 本文规则、v1库夹具、冲突/失败样本明确 | 单实例排他；版本2迁移保留聊天；容量、来源、版本、删除标记与回滚测试通过 |
| M2／T14 记忆面板与导出 | M1接口和错误契约可用 | 新增/查看/改/删/全删/导出，不伪造成功；明示聊天清理范围；暂不启用模型注入 |
| M3／T13 发送与失效协调 | M1/M2及协议v2、取消链可用 | 分模型许可、预览校验、事务清理、窗口刷新与迟到结果过滤；请求夹具证明未跨范围发送 |
| M4／T13/T14 本地验收 | M3完成、固定样本与复核者就绪 | Q6本地集、原生重启及独立人工体验证据；故障未关闭不标完成 |

以上是原T13/T14内的拟拆分，不额外承诺人日或声称原2～4人日足够；单实例、导出、协调成本须实施前重估并用实际容量排期。任务事实待T16，跨端授权/删除同步待T21/T23/T42。没有这些能力时仅标记“手动本地记忆子项”，不关闭完整T13/T14或G2。

## 7. 个人记忆注入（协议 v3，2026-09-26）

第二个记忆来源——独立[记忆服务](../status/personal-memory-service.md)中的个人记忆——按本设计的准入语义注入聊天；实现与证据见[注入交付记录](../status/personal-memory-chat-injection.md)。关键语义决定：

- **双家族、双预算**：应用记忆（本文件 §2-5）与个人记忆（服务所有，整数 id + 单调 `seq` 版本）是两个家族，各自 5 条 / 800 字，各自独立的 user-role 参考块（`user_confirmed_reference` 与 `personal_memory_reference`），系统指令永不包含任何记忆正文。
- **epoch 复用**：个人策略写入推进同一个全局 `context_epoch`——所有既有协调（气泡 memory-changed、发送 admission、迟到结果过滤）零新增机制即覆盖个人选择变化。本地不镜像服务行、不建 FK、不做级联：服务侧 forget/supersede 无本地事件，漂移由发送准入兜住。
- **(id, seq) + active 双重准入**：`validUntil` 到期**不 bump seq**（时间性失效没有版本变化），因此发送时把新鲜取数按活跃谓词过滤后与期望做**有序 (id, seq) 相等**；`not_found` 同为漂移。预览在线而发送时无法核对 → **拒绝**（preview==sent）；知情降级只发生在预览时就离线（`expectedPersonal:null`，回执如实记 offline）。发送端另有 ≤800 防御复检，防服务侧 update 增长内容后仅靠解码器误报。
- **锁纪律不变，三明治翻转**：全部本地准入在锁内完成后，解锁取数（持有 active 槽=互斥扩展到取数全程；取数包进 cancelled select），失败在触碰模型网络前返回——"网络期间不持锁"依旧成立。取数并发且连接失败整批短路，焦点轮询不被死端口拖慢。
- **有界陈旧（记录在案）**：批量 detail 取数非单 SQLite 快照，理论上可跨一次 supersede 交错读；窗口与既有"准入→模型发送"同类且更小。语音链路继续 `memories:&[]` 不注入。
