# 发送边界感知：常驻边界行与弹窗预计轮数

2026-10-10。M4 准出评审[张力点 3](../quality/m4-exit-review.md) 的销项交付，对应两条真实体验困惑：①发送时用户不知道"消息与勾选记忆会发往模型服务"；②删除/更正时用户不确定影响范围。分支 `feat/send-boundary-visibility`。

## 语义

**常驻发送边界行**（聊天气泡输入框上方，不折叠）：发送将把本条消息、最近 K 轮对话和已选记忆（应用 N 条 · 个人 M 条）发往模型服务 {host} · {model}。计数与发送准入实际核对的预览同源；无勾选时如实省略记忆段；个人记忆启用但服务未连接时标注"本次不携带"；host 截断显示，完整基地址保留在可展开预览里。初次使用指南第 1 条同步说明勾选记忆会随消息交给服务。

**三处确认弹窗接入 `chat_usage_impact`**（[精准清](memory-precise-clear.md)引入的只读咨询命令，此前零调用点）：弹窗打开时预计算将影响的轮数。措辞区分两种语义：

- **MemoryPanel 更正/删除/删除全部**（无豁免，恒清全部使用过的 scope）："预计清除最近 N 轮，之前的对话保留……实际以提交后的回执为准"；N=0 → "目前没有本机对话使用过这条记忆，预计聊天记录保持不变"。
- **两处收回选择**（MemoryPolicyPanel / PersonalMemoryPanel）：咨询不感知"仍在其他模型选用"的豁免，N 是**上界** → "预计**最多**清除最近 N 轮……仍在其他模型选用的对话不动"；N=0 同上。

咨询值与提交回执（`clearedTurns`）之间可能落新轮——文案不用"恰好/全部"，回执为权威值。咨询失败（存储不可用、协议不兼容）fail-open 回落静态规则句，弹窗仍可操作。

## 实现

- 共享 hook `apps/desktop/src/lib/usage-impact.ts`：`useUsageImpact(active, appIds, personalIds)`——`active` 只在弹窗打开时发起 invoke，id 变化（换条目/重开）自动重取；数组以 join 键做稳定依赖，避免每次渲染重取。
- 两处 policy 面板把 confirm 布尔改为**打开时捕获被收回的 id 集合**（`confirmIds`），后台刷新移动快照也不改变弹窗咨询的 id；MemoryPanel 的 `Pending` 同理拆为宿主形状的 `request` 与弹窗专用 `impactIds`，确认按钮只提交前者（宿主枚举 `deny_unknown_fields`，见下节）。PersonalMemoryPanel 补齐与 MemoryPolicyPanel 一致的两个守卫：弹窗打开时锁定勾选与保存按钮，后台刷新（memory-changed / focus）关闭弹窗并恢复已保存选择——确认提交的集合恒等于弹窗预计数描述的集合。
- ACL 与精准清交付时一致：`chat_usage_impact` 只授予主窗（capabilities/main.json），pet 窗不授予（三处弹窗都在主窗）。
- 浏览器 mock 三处（memory / memory-policy / personal-memory spec）补 `chat_usage_impact` handler 并记录请求，spec 断言 `{appIds, personalIds}` 请求形状、重开重取、N=0 分支与 fail-open 回落。

## 诚实边界

- 弹窗 N 为咨询值：预计算与提交之间在途轮可能落库；收回选择处的 N 恒为上界（豁免不可见）。
- 预计轮数来自 v4 使用账本：v4 之前回填近似的漏归因旧轮同样不计入预计数（[产品文档 §5](../product/limited-memory.md)）。
- 常驻边界行只描述**下一次发送**将携带什么、发往哪里；已发给服务商的历史内容不可撤回是弹窗与产品文档的既有口径，边界行不重复也不弱化。
- hook fail-open：咨询失败不阻塞操作、不显示"未知"——回落静态规则句。

## 验证

- `npm run check`（typecheck、Vitest、双端构建）与浏览器 Playwright 55 过 1 跳（外部 Live2D 资产）通过。chat.spec 新增常驻边界行断言（空态、带轮次、带记忆、启用但离线四态）；三记忆 spec 断言弹窗预计轮数、请求形状与两种报告分支。
- **Native CDP 真机验证（2026-10-10，验收构建 `b2-20261010a-8f3c21e4`，`tests/native/impact-acl.cjs`）**：空咨询返回 `{scopes:[],affectedTurnsTotal:0}`；pet 窗 invoke 被 ACL 拒绝（空 id 与带 id 两态）；新建记忆→启用选择→本地夹具真实发送一轮后，咨询按账本归因（`affectedTurnsTotal=1`、scope 为夹具模型）；确认弹窗真机渲染"预计清除最近 1 轮"，发送前为"预计聊天记录保持不变"。证据在 gitignored `.cache/b2-20261010a-*/`。

## 对抗审查（2026-10-10，PR 前）

四维审查（React 正确性 / 语义措辞 / 测试真实性 / 边界与 ACL）+ 逐条对抗验证，8 条发现 7 条确认，全部当场修复：

- **HIGH**：确认按钮曾把整个 `Pending`（含 `impactIds`）spread 进 `memory_mutate` 请求，宿主 `#[serde(deny_unknown_fields)]` 拒收——真机上所有更正/删除/删除全部的**确认操作**会反序列化失败。门禁全盲的原因：浏览器 mock 形状无关、native 脚本只点过"返回，不修改"。修复 = 上述 `request`/`impactIds` 拆分；浏览器 spec 对两个确认请求做精确形状断言，验收脚本改为**真实点下确认按钮**（更正成功、回执报清 1 轮、`chat_history` 归零——修复前此步必然失败）。
- **MEDIUM**：PersonalMemoryPanel 弹窗打开时勾选仍可交互、后台刷新重置选择不关弹窗 → 确认提交的集合可能偏离弹窗预计数描述的集合。修复 = 移植 MemoryPolicyPanel 的两个守卫；spec 断言弹窗打开时 fieldset/保存按钮锁定、memory-changed 关闭弹窗并恢复已保存选择。
- **LOW**：删除全部按钮在单条弹窗打开时可点，切换首帧短暂复用上一条目的预计数。修复 = 补 `!!pending` 守卫。
- **测试盲区**（均经变异复现：改坏文案后全套件仍绿）：chat.spec 离线态断言未跨"本条消息→记忆段"接缝、在线态未钉"未连接"注记缺席、personal-memory spec 未覆盖 N=0 分支。修复 = 跨接缝全串断言、`not.toContainText('未连接')`、N=0 弹窗分支断言。

修复后门禁重跑全绿（check / browser 55 过 1 跳），native 验收在 `b2-20261010b-4d7e93f1` 重新通过（新增 `confirmed update through the real backend` 一例）。

## 遗留

- M4 评审张力点 3 与[精准清记录](memory-precise-clear.md)遗留节的"弹窗静态口径 / 命令零调用点"随本交付销项。
- 跨端删除同步（G3）不受本交付影响，仍待执行。
