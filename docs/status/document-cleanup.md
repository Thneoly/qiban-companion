# 结果副本与记录清理：交付记录

> 2026 年 9 月 29 日。跨端体验路线第 4 项：手机查看已授权文档结果（已核验副本）、清理已结束任务记录且范围与影响明确。前置：[任务进度与恢复入口](task-progress-recovery.md)、[设备心跳与在线状态](device-presence.md)。语义见[设备授权架构](../architecture/device-authorization.md)文档适配协议。

## 实现与归属

- **结果副本（零服务端改动）**：产物文件字节 == DTO 预览字节（桌面写 `task.preview` 并读回校验、服务端 artifact_hash=digest(preview)），手机本就持有完整结果内容。「保存副本到手机」先在**手机端**用 `crypto.subtle` SHA-256 对比 artifactHash，不符即拒绝保存——副本声明因此是"手机已按回报哈希核验"，比"服务端已核验"更强。Blob 下载命名为 `<源名去扩展>-<actionId 前 8>.md`（blob URL 延迟 60s 释放，立即释放会中断尚未开始的下载——e2e 抓出的真 bug）。
- **删除（storage）**：`delete_document` 在一个 BEGIN IMMEDIATE 事务内删除**三表**（shared_documents + action_authorizations + authorized_resources，资源行带 NOT EXISTS 保险）——孤儿资源行会让下次分享的裸 INSERT 撞主键 503。可删集合 = completed|failed|cancelled（`invalidated_state` 映射到自身的不可变态）；unknown 与在途 409 拒绝（unknown 的桌面自动核对回执必须保持可见）；任一参与者可删，观察者/缺行 403（document() 前置，与 cancel 同先例）；返回删前快照。配额是实时 COUNT，删除即释放（存储测试覆盖 20 条满→删→再分享）。
- **删除（协调器/网关）**：`POST /v1/documents/{id}/delete`（Empty body 镜像 cancel；错误映射现成：Conflict→409 pairing_conflict、Denied→403 pairing_denied，文档无 404 语义——丢失响应后重试得 403，手机映射为"该记录已不存在（可能已在其他设备删除），已刷新列表"专属文案）；网关白名单 confirm|cancel|delete。
- **桌面收敛（安全关键）**：删除前已验证 sync 只迭代服务端列表，消失的记录不会触发任何重分享/重准入/重执行。本增量补两件事：① `sync()` 主循环后的**反向清扫**——台账相位 ∈ claiming|admitted（只有服务端曾列出过该记录才可能到达的"证明相位"）且本轮列表缺席 ⇒ 取消本地等待任务、台账标 reported、绝不重分享/回执/碰文件；`sharing` 相位缺席 ≡ POST 未达服务器，保持可重试，不清扫。② `share()` 对 reported 相位**明确拒绝**（"该摘录已执行完毕，本地不会再次执行；请重新选择文件生成新任务"）——删除后同内容重分享会在服务端全新插入，而桌面本地任务已终态无法再执行，不拒绝就是手机端静默 5 分钟过期。
- **手机 UI**：列表分「进行中的任务」/「已结束的任务」两段（`deletableDocumentStates` 是分段、时钟 ticker 与删除可见性的单一事实源）；已结束段一段话讲清范围与影响（副本核验/删除不碰电脑文件/不可撤销/结果未知需电脑核对后清理）；删除按钮带同风格确认；保存路径文案修正为真实的 `remote-documents/<账号目录>/document-drafts/`。
- **契约**：导出 `deletableDocumentStates` + `isDeletableDocumentState`；无 DTO 变化、无 schema 迁移、无新状态串。

## 验证（本机，2026-09-29）

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式/静态 | `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 / 0 警告 |
| Rust | `cargo test --workspace --locked` | 全过；新增存储删除测试（终态门/三表原子/重分享再插入/配额释放）、协调器删除段（双角色/409/二次 403/列表反映）、remote_jobs 相位过滤单测 |
| 桌面原生 | `npm run test:desktop-account`（opt-in，~144s，含一次真实 61s OTP 冷却） | 清理场景全绿：原始 HTTP 准入+台账钉在 admitted+终态回执+桌面自删 → sync 反向清扫收敛（台账 reported、本地任务 Cancelled、文件未动）+ reported 重分享拒绝 |
| 契约+前端 | `npm run check` | vitest 全过（含可删集合成员/unknown 不可删）+ 双端构建 + 网关 8 组（delete 入 ops 循环） |
| 手机集成 | `npm run test:mobile:integration`（160s） | 真链路：已结束段断言；副本下载事件（文件名 `进度-xxxxxxxx.md`、字节==预览、"核验一致"提示）；丢失响应删除（服务端已处理、手机见网络错误提示、自动刷新显示记录已消失）；他处已删竞态（手机删除得 403→"已不存在"文案→行消失）；其余六态/心跳断言全保留 |

## 准出对照

| 用户要求 | 证据 |
|---|---|
| 手机查看已授权文档结果 | 已结束段 + completed 核对保存结果（#3）+ 保存副本（手机端哈希核验）+ e2e 下载字节断言 |
| 清理记录 | 终态记录删除（双角色/三表原子/配额释放——存储+HTTP 测试）；桌面本地收敛（清扫原生测试） |
| 范围和影响明确 | 删除确认四要素一句（消失且不可撤销/电脑不再执行补报/草稿文件与已开始写入不受影响）+ 已结束段提示 + unknown 不可删说明 + 本记录边界节 |

## 产品与准出边界

- unknown 与在途记录不可删；所有活动会话都已失效的行（dead-session）无人能删，20 条配额仍可能被这类行占满——如实记录。
- 清扫不覆盖 `sharing` 孤儿（准入前被手机取消+删除且桌面从未 sync 过：本地任务停留 WaitingConfirmation、占 1/100 本地执行位，可手动重分享恢复）——与"POST 从未到达"不可区分，是保持重试性的代价。
- 桌面本地 executions.db 历史（100 条/账号目录）与已落盘文件**不随删除清理**；桌面端无删除 UI（有 API 能力，路线按手机侧为先）。
- 手机端删除后同内容重分享需重新生成预览（reported 拒绝）；无批量清理、无自动归档。删除承诺针对**原记录的重放**；桌面用户仍可对同一文件主动发起新分享——那是一个需手机重新确认的全新任务（sharing 相位的旧条目重试同样会全新插入），不是原记录复活。
- cancel_requested/unknown 记录在桌面会话永久失效（重新登录后新会话对旧配对是观察者、回执被拒）后既不可删也不可变，每条占 1 个配额位；极端积累可占满 20 条配额。如实记录为后续待办。
- 副本保存仅在仿真浏览器验证过下载事件与字节一致；真实手机人工验收归路线第 1 项。
