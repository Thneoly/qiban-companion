# 精准清聊天：按使用账本的前缀截断

2026-10-09。M4 准出评审[张力点 1](../quality/m4-exit-review.md) 的销项交付：更正/删除记忆或收回选择时，不再清空全部模型的聊天，改为按每轮发送时落账的使用记录做**逐 scope 前缀截断**。分支 `feat/memory-precise-clear`（合并后回填 PR 号）。

## 语义

对每个被移除的记忆条目（应用记忆按 id，个人记忆按服务行 id、无视 seq），在各 (基地址, 模型) 对话里找到**第一个使用过它**的轮次，删除该轮及其后所有轮；之前的轮次保留。论证：turn1 注入记忆 X 后，turn2 起的模型输入包含 turn1 的对话文本，故旧措辞的影响只能从前缀截断处消除；更早的轮次从未见过该条目，保留是安全的。从未使用过该条目的模型对话完全不动。

**收回选择（停用/移除勾选）有一处豁免**：仍在其他模型选用的对话不动——那些模型此后每轮仍注入该条目全文，删其历史消除不了任何暴露、只造成不可逆丢失；账本行保留，等真正删除/更正条目时再补齐截断（更正恒清全部使用过的 scope：旧措辞本身就是要消除的污染）。

删除全部记忆与单条删除同构（对全部有效条目并集做截断）；N=0 时仍要求确认（仍会停止在途回复并作废预览），但不删任何轮。

## 实现

- **Schema v4**：新表 `chat_turn_usage`（FK→`chat_turns` ON DELETE CASCADE；对 memories **故意无 FK**——条目墓碑化后账本仍须能看到 id）。每轮每类至多 5 行，唯一索引防重复。迁移把既有轮次按"迁移时刻各 scope 启用选择"回填（个人记忆行 seq=NULL，发送时按 id 匹配）。版本门从 `>3` 提到 `>4`。
- **写入时机**：轮次生成成功、追加历史**同一事务**内落账（数据来自发送回执：应用记忆 id+revision、个人记忆 id+seq）。失败/取消/超时的轮次不落库，无悬挂行。
- **五处替换**：`memory_update`/`memory_delete`/`memory_delete_all`/`memory_policy_set`（移除集合 = 旧选择 − 新选择）/`personal_memory_policy_set`（按 personal_id）全部从无 WHERE 的 `DELETE FROM chat_turns` 改为逐 scope `DELETE ... WHERE base=? AND model=? AND id>=cutoff`，各自 Immediate 事务内、epoch 递增后、commit 前。
- **回执**：`MemoryCommit.cleared_turns`（u64，0=未动任何聊天）；`chat_cleared` 退化为 `cleared_turns>0`。
- **宿主失效**：`invalidate_memory` 在 0 轮受影响时只取消在途；有轮受影响时**从磁盘重载存活前缀**而非清空内存——清空会造成"内存空、磁盘留前缀、重启复活"的幽灵分裂。重载前的 clear 仍递增版本，在途完成照旧被拒（DEL12 不变）。
- **新命令** `chat_usage_impact`（只读）：按 (appIds, personalIds) 预计算各 scope 将删/将留轮数，供确认弹窗显示。咨询值——提交回执的 `clearedTurns` 是权威值（两读之间可能落新轮）。三处注册齐全（generate_handler / build.rs / capabilities/main.json），pet 窗不授予（弹窗都在主窗）。
- **UI 口径**：三处确认弹窗改述精准语义，按钮名对齐产品文档承诺的**「确认（收回）并开始新对话」**；回执按 `clearedTurns` 如实报告（`>0`："已清除使用过该记忆的最近 N 轮对话，其余保留"；`=0`："已有聊天记录保留"）。

## 诚实边界

- **削弱**：「改/删/收 ⇒ 全部清空」不再成立。账本漏记即漏清：唯一漏记源是 v4 之前的历史回填近似——迁移前中途改过勾选的旧轮可能漏归因（影响 ≤6 轮存量，升级后新轮精确）。容量淘汰/清空对话时账本行随轮次级联删除，属正常（轮次已不存在）。
- **弹窗竞态**：预计算 N 与提交之间在途轮可能落库。N 为咨询值、回执为权威值，文案不用"恰好/全部"。
- **不变**（既有测试未放松）：事务原子性与崩溃回滚、DEL10/12 双闸、幂等删除不清新聊天、6 轮/12000 字限额、secure_delete、单实例、未来版本拒绝写入。

## 验证

- Rust：companion-storage 78 / desktop 64 / core 14 / coordinator 12 全绿；fmt+clippy 干净。Vitest 90、浏览器 Playwright 55 过 1 跳（外部 Live2D 资产，与本项无关）、`npm run check`、`npm run verify:docs` 通过。
- **Mutation 钉子**（按项目纪律摘除后必须红，数字为 2026-10-09 复核实测）：①`prune_chats` 摘 per-scope 过滤（删 WHERE base/model）→ 13 用例红（7 条 q6 + 6 条存储测试）；②`id>=` 改 `id=`（丢后缀语义）→ 3 用例红（含专属后缀测试）；③`append_with_usage` 摘账本写入 → 16 用例红。
- 新增代表性测试：v3/v2 真文件迁移回填（含个人记忆 seq=NULL）、账本校验（重复/超限/坏 id）、淘汰与清空级联、前缀截断保留前缀、personal 跨 seq/scope 按 id 匹配、`usage_impact` 只读不落刀、宿主删除后重载存活前缀且重启复验、`clearedTurns` 契约必填。
- native 验收脚本（`memory-panel.cjs`、`memory-context.cjs`、`voice-chat.cjs`、`q6/harness.cjs`、`single-instance.cjs`）已同步 user_version=4 与按钮名；**真机复核待 PR 合并后重建二进制执行**（见各脚本复现命令）。

## 遗留

- 确认弹窗当前为静态口径（定性描述影响范围）；打开时接入 `chat_usage_impact` 显示具体轮数属 M4 待修项 B（发送边界感知强化），同分支后续 PR。
- 回填盲区已写入[产品文档 §5](../product/limited-memory.md)；跨端删除同步（G3）不受本交付影响，仍待执行。
