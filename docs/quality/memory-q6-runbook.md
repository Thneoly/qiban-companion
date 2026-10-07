# Q6 固定验收包与复核操作

2026-09-19。配套[Q6准入准出规则](limited-memory-acceptance.md)及[本轮实际结果](../status/memory-q6-local.md)。本包把发送边界、模型回答质量和独立体验分别记录；只有三者都满足，才能评审M4。

## 50例固定输入

唯一输入基线为[合成样本JSON](../../apps/desktop/tests/fixtures/memory-q6-v1.json)，版本`q6-v1`。每例有独立ID、操作序列、允许发送的条目/版本/日期、禁止旧正文、最终有效条数、用户问题和人工评分说明。用例无真实用户资料。动态UUID和创建/确认时间由应用生成，检查关联、顺序与时间关系，不伪造固定系统时间。

| ID | 内容 | 例数 |
|---|---|---:|
| P01～P10 | 称呼、回答风格、学习节奏、emoji等手动偏好，含重开 | 10 |
| E01～E10 | 有日期/无日期/旧经历/用户陈述，不补充缺失细节 | 10 |
| C01～C10 | 更正、旧epoch/revision、重复提交、类别日期变化、无效更正 | 10 |
| S01～S10 | 默认关闭、新地址/模型、切回、子集、撤回、空选择、数量/字符预算及顺序 | 10 |
| N01～N10 | 猜测、任务宣称、非法输入、恶意资料、自动保存宣称、未选择内容与来源不足 | 10 |

`expected.references`表示允许进入模型请求的资料，**不是模型可以照搬为已验证事实的清单**。例如N07的恶意“已删除文件”和N10的个人自述可以作为用户资料出现，但不能证明工具执行或外部核实；质量评分以各例`review.required`为准。无允许资料的案例单列误用；另记录恶意指令服从与来源提升问题，不能因文本在记忆库里就判答复正确。

## 20例删除与竞争

以下ID每个对应独立执行，不把一次测试重命名成多个通过结果。Rust用例名称统一包含`q6_delNN`；原生用例在[deletion.cjs](../../apps/desktop/tests/native/q6/deletion.cjs)。

| ID | 验证 | 层及边界 |
|---|---|---|
| DEL01 | 单条删除、正文清空、策略关闭、全部聊天清理 | storage |
| DEL02 | 全删两条及选择清理 | storage |
| DEL03 | 已提交删除重试不清之后的新聊天 | storage |
| DEL04 | 旧epoch全删不误删后来新增记忆 | storage |
| DEL05 | 更正增加版本并清所有模型聊天 | storage |
| DEL06 | 停用保留记忆而清聊天 | storage |
| DEL07 | 移除一个选择保留其他选择及全部本机条目 | storage |
| DEL08 | 清聊天保留记忆及许可 | storage |
| DEL09 | 请求已到本地HTTP服务后删除，旧回复失效，新请求无旧值 | 原生宿主＋HTTP屏障 |
| DEL10 | 删除后旧预览发送被拒，服务未收到该请求 | 原生宿主＋HTTP计数 |
| DEL11 | 流式中更正，旧请求取消，新请求只有新值 | 原生宿主＋HTTP屏障 |
| DEL12 | 回复已组装但尚未回写时删除，完成回调拒绝 | 宿主协调单元；不是随机时间竞争 |
| DEL13 | 删除事务变更完成、提交前进程直接退出，重开回滚 | storage子进程；测试二进制专用崩溃注入 |
| DEL14 | 提交后、发窗口通知前直接退出，重开不复活 | 宿主子进程，实际change_memory通知闭包 |
| DEL15 | SQLite只读连接拒绝删除并保留全部旧值 | storage；不代表物理只读磁盘/ACL矩阵 |
| DEL16 | 旧窗口更正/全删/发送被拒，后来新增数据保留 | 原生IPC |
| DEL17 | 选择导出位置期间删除，旧快照不能覆盖目标文件 | 宿主导出函数；不重复冒充原生对话框交互 |
| DEL18 | 已导出文件仍存在，新导出不含删除正文 | 宿主导出函数 |
| DEL19 | 未来版本及损坏库拒绝且原字节不改 | storage；合法v1迁移由既有迁移测试覆盖 |
| DEL20 | 第二进程无法取得同目录锁，独立目录可并存，释放后可重开 | Windows宿主文件锁子进程 |

存储实现见[memory_q6_tests.rs](../../crates/companion-storage/src/memory_q6_tests.rs)。崩溃注入由`cfg(test)`隔离，不进入发布构建，也没有新增故障控制IPC。

## 本地运行与证据

从仓库根运行。需要Node 24、交互式Windows/WebView2、本机Playwright依赖及Rust工具链。使用未存在过的独立标识，脚本会拒绝已有资料目录或已占用的调试端口。

```powershell
node apps/desktop/tests/native/q6/validate.cjs
# 将下面JSON写到.cache/q6-acceptance.conf.json，替换unique-run为本轮独立名称。
```

```json
{"productName":"Qiban Q6 Acceptance","identifier":"dev.qiban.companion.acceptance.q6-unique-run"}
```

```powershell
npm run tauri --workspace @companion/desktop -- build --no-bundle --config (Resolve-Path .cache/q6-acceptance.conf.json).Path
Copy-Item -LiteralPath target/release/companion-desktop.exe -Destination .cache/q6-acceptance.exe
$env:QIBAN_ACCEPTANCE_EXE = (Resolve-Path .cache/q6-acceptance.exe).Path
$env:QIBAN_ACCEPTANCE_ID = 'dev.qiban.companion.acceptance.q6-unique-run'
node apps/desktop/tests/native/q6/runner.cjs
# 使用刚输出的runId目录，执行另外16个Rust删除案例并合并审计。
node apps/desktop/tests/native/q6/audit.cjs .cache/<本轮runId>
npm run build:desktop # 恢复正常标识的日常演示构建
```

每轮在`.cache/<runId>/`保留`facts.json`、`deletion-native.json`、`summary.json`、`cargo-q6.log`、`audit.json`，包括提交基线、工作区是否有修改、二进制/用例/测试驱动摘要、协议/数据库版本、逐例结果与合成payload。失败不会从报告分母中移除；重跑使用新标识和runId，不能覆盖旧证据。普通产品日志不复制这些正文。

`runner.cjs`执行50个发送边界案例和4个原生删除案例；另外16个组件案例由`audit.cjs`真实调用Cargo并核对逐项名称，零匹配不会当成通过。正常本地轮共56次localhost请求，不是56次付费模型请求。模拟服务只返回固定文字，报告始终将模型质量和独立体验标为未执行，M4门为Hold。

## 真实模型与评分

先明确调用上限和目标服务，再使用新的独立验收构建运行[live.cjs](../../apps/desktop/tests/native/q6/live.cjs)。该脚本没有默认额度；只接受5例试验或50例完整复核，每次最多8192输出tokens（产品设置上限；混合推理模型的思维链与正文共用输出预算，2048曾使正文被截断）、每例一次、不自动重试，首个失败即停止后续调用。实际费用以服务商账单为准。密钥继续由Rust从系统凭据读取，不放入脚本、命令参数或浏览器。

```powershell
# 仅在本轮调用额度明确后设置；复用上面的独立构建方法，不能复用已有资料标识。
$env:QIBAN_Q6_LIVE_COUNT = '5'
$env:QIBAN_Q6_BASE_URL = '<已配置凭据的API基地址>'
$env:QIBAN_Q6_MODEL = '<已确认的模型编码>'
node apps/desktop/tests/native/q6/live.cjs
```

5例试验固定为P01/E02/C01/S01/N01。真实模型流程按本地已验证的最终资料重建独立条目、明确启用后，通过实际宿主文字链请求目标模型；不在付费服务上创建不存在的“测试模型B”。因此真实模型轮专门衡量回答质量，范围变化和删除竞争仍由本地轮证明。UUID/确认时间由本轮应用生成。`live-review.json`保存回答、失败、实际使用快照与用量；未执行的案例仍保留。

当前真实脚本仅完成语法检查，尚无真实服务执行证据。由复核者按50例逐项标注来源正确、遗漏、误用及理由：失败/无回答仍在50分母中，至少48例来源正确才满足原95%门槛。5例试验不能通过50例门。模型自称“记得”或HTTP成功都不是合格判据。

可先生成[人工复核表](../../apps/desktop/tests/native/q6/review.cjs)：

```powershell
node apps/desktop/tests/native/q6/review.cjs .cache/q6-review-unique.json
```

## 非实现者体验（约10分钟）

在独立验收资料中进行，避免清理日常聊天。主持人只给任务，不提前解释正确答案；记录首次理解、操作失败和困惑。

1. 保存“请称呼我为鹿鸣”和一条不带日期的合成经历；重开，找到它们。
2. 仅允许当前模型使用称呼。查看发送预览，说明将发送什么；真实发送须在已确认额度内。
3. 更正称呼，先取消确认，再实际提交，观察聊天与记忆各发生什么变化。
4. 导出、删除该条记忆、重开，检查记忆与已导出文件。
5. 不看说明回答：哪些信息保存了？哪些发送给服务？哪些操作清掉了什么？删除不能撤回什么？

记录复核者身份、构建、时间、原话、是否接受清聊天的取舍和待修问题。由实现助手填写“替用户通过”无效；用户尚未复核时保持未执行。三类证据齐备、缺陷处理且关键边界100%通过后，再做M4准出评审。
