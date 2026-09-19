# T13/T14 M2：本机记忆面板与导出

2026-09-19，分支 `feat/memory-panel`，接续 [M1 基础](memory-foundation.md)。手动记忆管理子项已实现并通过本机工程验证；**模型使用仍关闭，M3、M4及完整T13/T14未准出**。

## 可体验行为

点击桌宠 → “我们的记忆”，在辅助面板填写偏好或共同经历，可选经历日期。最多30条，每条最多200个Unicode字符；来源固定为用户填写，保存聊天不会自动生成记忆。

- 保存、查看、重开恢复、更正、单条删除、全部删除均接入真实SQLite。
- 新增停止当前生成，保留已保存聊天；更正或删除须先确认停止回复并清空本机全部模型的聊天记录。
- 卡片展示来源、日期、创建及确认时间。失败保留输入，读取失败不会显示成空库。
- 原生JSON导出区分保存、取消与失败，覆盖已有文件由Windows对话框确认。仅导出有效记忆，不含聊天、密钥、删除正文或模型使用许可。
- 明示本机明文保存、模型使用未启用、导出文件不会随之后删除自动撤回；本版没有导入。

浏览器预览可展示界面，但不保存记忆；真实操作须运行桌面版。现有模型地址、编码及密钥配置不因本功能改变。

## 实现与竞争边界

| 部分 | 实现 |
|---|---|
| 面板 | `apps/desktop/src/features/memory/`；桌宠入口打开并定位面板 |
| 主窗口IPC | `memory_list`、带action的`memory_mutate`、无路径参数的`memory_export`；返回值经共享契约严格解码 |
| 桌宠权限 | 仅新增只读`chat_context_epoch`，没有记忆写权限 |
| 提交协调 | 记忆事务、聊天内存、取消状态共用ChatState锁；检查epoch/revision，提交失败不清缓存、不发布成功 |
| 迟到回复 | 每段流及完成提交校验请求ID、会话版本和数据库epoch；旧请求的退出清理不能取消新请求 |
| 窗口刷新 | 事件携带epoch；通知失败回执区分“已提交、另一窗口待刷新”；焦点恢复与发送前重新核对，旧读取/迟到片段不能覆盖新状态 |
| 导出 | Windows COM原生保存对话框，选择期间不持聊天锁；写入前重核epoch，以同目录临时文件完成替换；拒绝应用私有目录与非JSON目标 |

为使M2删除可用，提前完成M3中的本机取消与缓存失效部分。当前`ChatRequest`仍为协议1；M3须升级协议2，补齐expectedScope/expectedContextEpoch、分模型许可、上下文预览和实际使用证据。尚无任何记忆正文注入模型请求。

## 实际验证

| 检查 | 结果 |
|---|---|
| `npm run check` | 类型检查、14个Vitest及前端构建通过 |
| `npm run test:browser`（启用本地Live2D样本） | 26例通过；覆盖确认取消、草稿保留、容量/Unicode、窄窗口、通知失败、删除后迟到片段及发送前补核epoch |
| `cargo test --workspace --locked` | 45通过、1项系统凭据测试按既有规则忽略；doc-tests通过 |
| `cargo fmt --all -- --check`、workspace clippy `-D warnings` | 通过 |
| 文档与研究原型检查 | `npm run verify:docs`（34份Markdown、188个本地链接与计划容量）、`npm run verify:prototype`通过；后者是桩DOM检查 |
| 原生Windows受控验收 | 新建→重开→更正→导出取消/保存/覆盖→生成中删除→重开为空→导出空集通过；桌宠写权限与缺少确认均拒绝 |

原生脚本为 [memory-panel.cjs](../../apps/desktop/tests/native/memory-panel.cjs)，文件对话框驱动为 [export-dialog.ps1](../../apps/desktop/tests/native/export-dialog.ps1)。独立应用标识与合成数据，1次localhost流式请求，未调用付费模型、未读取生产密钥。检查实际请求不含记忆正文，删除后数据库仅有清空内容的删除标记。最终源码构建复核标识为`dev.qiban.companion.acceptance.m2-20260919b`。截图和JSON证据保存在忽略的`.cache/<验收标识>-<UUID>/`，不提交用户数据或构建产物。

复现时为每轮选择一个未使用过的标识；脚本拒绝已有资料目录。先将下列配置写入`.cache/memory-panel-acceptance.conf.json`：

```json
{"productName":"Qiban M2 Acceptance","identifier":"dev.qiban.companion.acceptance.m2-unique-run"}
```

```powershell
npm run tauri --workspace @companion/desktop -- build --no-bundle --config D:/Game/.cache/memory-panel-acceptance.conf.json
# 构建产物此时属于独立验收标识，先复制为专用文件。
Copy-Item -LiteralPath target/release/companion-desktop.exe -Destination .cache/memory-panel-acceptance.exe
$env:QIBAN_ACCEPTANCE_EXE = (Resolve-Path .cache/memory-panel-acceptance.exe).Path
$env:QIBAN_ACCEPTANCE_ID = 'dev.qiban.companion.acceptance.m2-unique-run'
node apps/desktop/tests/native/memory-panel.cjs
npm run build:desktop  # 恢复正常标识的演示构建
```

需交互式Windows会话、Node 24（原生SQLite测试）、WebView2和本机Playwright依赖；自动化只操作该验收进程的窗口。原生导出暂仅实现Windows，其他系统返回明确失败。未将本机自测视为外部CI、干净设备、非实现者体验、Q6完整50例/20例或G2通过；下一增量是M3模型使用许可与可核对的受控注入。
