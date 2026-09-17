# T13/T14 M1：单实例、领域契约与数据库迁移

2026-09-18，分支`feat/memory-foundation`，接续[有限记忆设计](limited-memory-design.md)。M1底层增量已实现并完成本机验证；**记忆管理面板、导出、模型许可和注入尚未开放**，T13/T14及G2没有整体准出。

## 已实现行为

| 部分 | 本次变化 |
|---|---|
| Windows单实例 | 首个Tauri插件处理重复启动并恢复已有角色；打开任何数据库前持有资料目录的独占文件句柄，补启动竞争和插件通信失败的缺口。锁失败不打开数据库，显示启动提示；不同应用标识使用独立目录。锁文件保留，正常退出或进程终止均由系统释放句柄 |
| 领域规则 | 手动偏好/经历、固定用户来源、可空本地日历日期、30条有效容量、每条1～200个Unicode字符；拒绝非法控制符、伪造来源字段和手动任务事实 |
| 数据契约 | Rust条目序列化与TypeScript严格解码共享同一JSON夹具；无效来源、任务事实、删除项、不精确整数及重复ID不会当作正常数据。现有IPC协议仍为1，尚未注册记忆命令 |
| 数据库迁移 | `chat-history.db`从v1事务升级到v2，保留既有聊天内容、ID、顺序和模型范围；空库也走相同迁移。记忆及策略表初始为空、epoch为0；未来版本和损坏/不兼容库拒绝覆盖，失败回滚DDL与版本号 |
| 内部存储原语 | 创建、读取、更正、删除、全删及epoch；创建不清聊天，更正/删除/全删同事务清空全部模型聊天。删除清除正文、来源、日期和时间内容列，仅留ID、类型、revision和deleted_at |
| 并发与失败 | 检查expectedEpoch和expectedRevision；计数限制为JavaScript安全整数上限9007199254740991，溢出拒绝。已提交单删可重试且不再次清新聊天；过期全删拒绝，避免误删后来新建条目 |
| 选择约束 | 纯领域选择校验：按明确顺序、最多5条/800字、拒绝重复或不存在ID。更正检查全部引用策略的预算；删除移除引用并更新策略版本，空策略关闭。策略写入API待M3 |

单实例使用[Tauri官方插件](https://v2.tauri.app/plugin/single-instance/)，锁定版本见Cargo.lock。额外文件锁由Windows `OpenOptionsExt::share_mode(0)`实现，必须一直持有到进程退出，不能通过删除锁文件“解锁”。非Windows暂拒绝开启资料库，不声称已实现跨平台排他。

存储方法目前只供Rust内部调用，尚未接到UI。调用方必须在M2开放更正/删除前完成影响确认、提交后的聊天缓存清空和在途失效；直接把这些方法注册为IPC会破坏当前内存缓存的一致性。M3再完成模型许可、上下文预览和协议v2发送准入。既有“清空聊天”已在事务内递增epoch并保留记忆，但当前聊天发送还未使用这个epoch。

## 实际验证

| 检查 | 本机结果 |
|---|---|
| `npm run check` | 类型检查、13项Vitest、前端生产构建通过 |
| `QIBAN_TEST_LIVE2D=1`下`npm run test:browser` | 21项通过，包含既有文字、指南及Live2D回归 |
| `cargo test --workspace --locked` | 42项通过，1项凭据仓库测试按既有设置忽略；本轮不读取真实密钥 |
| `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `npm run verify:docs`、`npm run verify:prototype` | 33篇Markdown、176个本地链接、计划容量核对及8条原型流程通过 |
| Windows验收构建与生产构建 | 无安装包release构建通过；链接器有信息性输出，未阻止构建 |
| 独立原生验收 | v1合成聊天在真实窗口恢复；同时发起4个重复进程全部退出并唤回隐藏角色；强制结束后重开成功；聊天ID与内容保留、记忆和许可仍为空 |

Rust测试另覆盖：跨进程资料目录互斥及独立目录可并存；闰年/emoji/边界容量；来源字段注入拒绝；编辑/删除/全删中的晚期SQL失败整体回滚；只读写失败；条目/策略/epoch溢出；删除重开不复活；未来版本文件字节不变；迁移中途故障不留下新表或新版本。

另以普通生产构建与验收构建同时运行，确认两个标识可并存、各自的重复启动均退出；保留生产角色运行。

上述原生测试使用`dev.qiban.companion.acceptance.m1-20260918a`，合成资料保留在独立验收目录，没有调用模型。脚本见[原生单实例验收](../../apps/desktop/tests/native/single-instance.cjs)及[窗口可见性检查](../../apps/desktop/tests/native/window-visible.ps1)。完整50例事实/更正、20例请求/删除竞争、独立人工体验、干净系统、跨用户/RDP及跨端矩阵仍未执行，不用这些工程测试替代[Q6准出](../quality/limited-memory-acceptance.md)。

## 重跑原生验收

需要Node 24、已安装项目依赖，以及可打开原生窗口的Windows会话。每次使用新标识；脚本发现已有目录会拒绝执行，避免覆盖任何既有资料。不要对生产标识运行。

```powershell
# 从仓库根目录运行；每次用新的末尾标识。
$testId = 'dev.qiban.companion.acceptance.m1-' + [guid]::NewGuid().ToString('N')
$testConfig = Join-Path $PWD '.cache/memory-acceptance.conf.json'
[System.IO.Directory]::CreateDirectory((Join-Path $PWD '.cache')) | Out-Null
[System.IO.File]::WriteAllText($testConfig, (@{productName='Qiban M1 Acceptance';identifier=$testId} | ConvertTo-Json))
npm run tauri --workspace @companion/desktop -- build --no-bundle --config $testConfig
# 构建输出位置沿用根README约定；构建前退出该路径的应用。
Copy-Item -LiteralPath target/release/companion-desktop.exe -Destination .cache/memory-acceptance.exe
$env:QIBAN_ACCEPTANCE_EXE = (Resolve-Path .cache/memory-acceptance.exe).Path
$env:QIBAN_ACCEPTANCE_ID = $testId
$env:QIBAN_CDP_PORT = '9441' # 须未占用；仅在验收进程开启
node apps/desktop/tests/native/single-instance.cjs
npm run build:desktop # 恢复普通生产标识的构建产物
```

下一步是M2：记忆面板、操作影响确认与JSON导出，先在模型使用关闭状态下完成真实保存、重开和删除体验。M1的内部原语是准入基础，不表示M2可以省略宿主协调。
