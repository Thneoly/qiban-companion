# P1-b：局域网手机接续（内嵌协调服务）技术设计

2026-10-18 设计稿（未实现）。已经过三路对抗审查（安全/架构生命周期/产品完整性）并按发现修订；动工前仍需对实现 PR 做常规对抗审查。配套[P1 方案](../planning/p1-trial.md)。

## 目标与边界

试用版手机接续不依赖外部部署：桌面进程内嵌协调服务，手机与电脑**同一 WiFi** 下浏览器直访 `http://<电脑局域网IP>:<端口>`，六位码配对后使用任务确认、文档协作与跨端记忆的全部既有能力。

**边界（如实）**：
- 仅同 WiFi；IP 变了回面板重看
- 局域网明文 HTTP（无 TLS）。信任模型=家用网络且**同网所有设备皆可信**（WPA2-PSK 不防网内主动者：ARP 欺骗下首次加载 SPA 的 JS 可被注入从而窃取刚发的会话）——公共 WiFi 不得使用（引导文案写明）
- 异地/公网接续不在本设计（未来云接续届时邮箱账号体系回归）

## 现状盘点（经审查修正）

可复用的地基：
- `companion_coordinator::router(AppState)` 已是库函数（main.rs 只是启动壳）；lib 路径无环境变量依赖；与桌面 `tauri::async_runtime` 兼容（desktop 测试已验证同进程 serve 模式）
- `VerifiedIdentity::from_verified_provider` 纯构造；配对六位码预算（5 分钟窗/5 次锁定/持久化/offer 重生成撤旧码）扎实
- 手机前端全部走相对路径 fetch（换 host:port 零改动）；记忆面板账号级鉴权不经配对链
- mobile-web dist 实测 257KB，tauri resources 打包可行

**审查证伪的三个直觉**（设计初稿曾据此低估工作量）：
1. server.mjs 网关不是"开发态白名单代理"——它是浏览器面防御层（Host/Origin 校验、CSRF 姿态、HttpOnly cookie 会话、CSP/XFO/nosniff、限流、`/api/state` 聚合）。coordinator router 是纯 Bearer、无任何上述防御（lib.rs 注释自述）。**网关语义必须移植成 Rust，不是退役**
2. 端口是本地状态的键：会话 vault scope、记忆同步 owner（含 outbox 待上行变更）、文档任务台账目录全部按 `port` 落库。随机端口=outbox 孤儿化+游标作废+台账碎片化（**数据丢失级**）
3. `issuer` 与会话 TTL 硬编码在鉴权路径：token 验证 SQL 只认 `NATIVE_ISSUER`；会话 24h 绝对过期+30min idle 永久判死；不存在"reserve session"API；`live()` 对 issuer 有 SQL 特判。本地账号必须定义自己的铸币与续期路径

## 核心设计

### 1. 服务形态：进程内嵌 + 持久固定端口

桌面 setup 在 `tauri::async_runtime` 上 spawn `axum::serve`（监听 `0.0.0.0:<port>`），Exit 事件置 CancellationToken 触发 graceful shutdown——但**不把优雅关停当正确性依赖**（SQLite 短事务靠 rollback journal 兜底，与现有三个后台循环同等姿态）。

- **端口持久固定**：首次选定空闲端口后写入 account_settings 复用；被占时重选并做 owner/vault 的键迁移（一次Toast 提示）。否决"每次启动随机"：port 键控的数据连续性优先
- **库文件**：app 数据目录下独立 `coordinator.db`（v0→v7 空库迁移已验证）；与 `npm run coordinator` 开发库互不干扰
- **开发态并存**：外部链路原样保留；桌面连接模式"内嵌/外部"进设置页，默认内嵌
- 依赖账：companion-coordinator 从 dev-dep 转正（lettre+native-tls 进生产树，体积记账）；tower-http 新增

### 2. 浏览器面防御内移（原网关语义移植，硬性前置）

内嵌组合在 coordinator router 外再包一层：

- **Host 白名单**：请求 Host 必须命中 `127.0.0.1:port` 或当前局域网 IP:port 字面量——防 DNS rebinding（否则任意远程网页可借局域网内浏览器把域名 rebind 到本服务，"仅局域网猜码"的威胁模型被证伪；手机浏览器不实现 Private Network Access，不能指望它）。静态资源另加 `Cross-Origin-Resource-Policy`
- **`/api` 适配层**（几十行 Rust）：`/api/*` 前缀改写到 `/v1/*`、`GET /api/state` 聚合（me+tasks）、cookie 会话签发/清除（HttpOnly+SameSite=Strict，沿用网关方案——手机端零改动、保住"零存储"安全姿态与既有浏览器测试）、401 清 cookie
- **CSRF/Origin 姿态**：POST 需 Origin 比对 + `X-Qiban-Request` 头 + JSON content-type（照 server.mjs 语义）
- **静态资源安全头**：CSP/X-Frame-Options DENY/nosniff/Referrer-Policy（ServeDir 响应不经 coordinator 的 response_boundary，须自挂）
- **限流**：匿名端点（exchange、healthz）叠加速率限制（网关 600/min 语义内移），防局域网内 DoS 烧预算

### 3. 本地账号：NATIVE_ISSUER + 启动自愈重铸

- 首次启动以 `NATIVE_ISSUER`、`subject=<随机 UUID>` 构造本地账号（issuer 沿用现有值——鉴权 SQL 与 `live()` 特判零改动；内存档的"专属 issuer"方案被审查否决：与 token 通道互斥）
- **铸币**：新增 storage 方法"以稳定 session_id 重铸 token"（UPDATE native_tokens 的 last_seen/expires_at）；**桌面每次启动检测无有效会话即自动重铸**——覆盖首次、24h 过期、手机"退出所有设备"后的自愈；不存在的"长期会话"承诺删除
- 桌面自留会话的 last_seen 由既有双循环 API 调用天然刷新（30min idle 无虞）；24h 绝对上限由重铸覆盖
- `request-code`/`verify-code` 在内嵌组合**不注册**；`CodeMailer` 传 stub + 占位 allowlist（`["local@invalid"]`，满足构造校验；closed-invitation 分支构成第二道兜底）
- pepper：内嵌形态不需要（它只服务 OTP 推导；token 验证不碰 pepper——初稿的"pepper 丢失=会话全灭 kill-switch"声明不实，删除）。会话全灭=既有"退出所有设备"
- 内嵌模式下桌面 signed_out 界面不渲染邮箱表单（永远走不通），替换为本地账号状态+故障自述

### 4. 手机信任：exchange = 单事务完整 accept + 铸会话（仅内嵌组合）

`POST /v1/pairings/exchange`（无会话鉴权；**只注册进内嵌组合**，外部/开发组合不挂——router 组装参数化，防止把"无鉴权六位码换会话"带进公网多账号部署）：

- **单事务完整 accept 语义**：验码（全局 code_hash 反查 pending 配对——单账号内嵌形态下错码预算记账到唯一本地账号，多账号归账难题因此不存在）→ 消费 code_hash → name/状态/desktop-live 检查（照 accept_pairing 清单）→ 登记 controller 设备绑定 → 置 active → 铸手机会话。**不是 preview 复用**（preview 不消费码，同码可重放多次）
- 响应形状对齐 `SessionToken`；body 含客户端 nonce（丢响应可幂等找回，学 verify_code）
- 语义变化如实记录：原"preview→手机核对电脑名/权限→确认"三步并为"输码即入"一步——[设备授权文档](device-authorization.md)同步更新该节
- **撤销配对 = 收回入场券**：`revoke_pairing` 扩展——经 exchange 获得的会话随配对撤销连带终结（否则"手机丢了"的自然操作撤不掉账号级访问，旧形态同样存在但旧门槛是邮箱 OTP+受邀名单，新门槛只有六位码，必须补上）
- 手机 24h 会话到期/闲置 30min 后：回电脑取新码重走 exchange（幂等重绑同一设备行）

### 5. 手机静态资源与前端改动

- mobile-web dist 作为 tauri resource 打包（257KB），`ServeDir` + SPA fallback 在 `/` 出
- 手机端改动收窄为：登录门（main.tsx 邮箱表单段）改为"输码即入"+ 保留"高级：使用账号服务"入口；相关文案（同账号/邮箱表述）随改。**api.ts/会话存储/各面板零改动**（经 `/api` 适配层与 cookie 方案保住）

### 6. 地址发现与引导

- 面板「设备」区显示：`http://<主局域网IP>:<端口>` + 六位码生成（复用 offer）+ "同一 WiFi"提示 + "地址不对？查看全部候选 IP"
- 主 IP 取默认路由出口（UdpSocket connect trick，不实际发包）
- 防火墙引导覆盖三条路径：首次弹窗"允许访问"（专用网络）；**点了取消后的手动放行步骤**；Windows 把家庭 WiFi 归为"公用网络"时的勾选陷阱
- 桌面文案改造清单：devices 导航描述、AccountPanel 的邮箱表单区与"本机账号服务连接"块（手动端口表单与自动固定端口冲突，内嵌构建替换/隐藏）、桌面 PairingPanel"在手机的「连接电脑」输入"指向新入口

## 承重变更表

| 变更 | 位置 | 风险 |
|---|---|---|
| 内嵌服务编排（固定端口、启停、Exit 钩子） | desktop lib.rs + account.rs | 中 |
| 浏览器面防御层（Host 白名单/CSRF/安全头/限流/`/api` 适配+聚合+cookie） | coordinator 新模块（仅内嵌组合挂载） | **高**：新防御面，逐项对照 server.mjs 语义移植 + 专用测试 |
| exchange 端点（单事务 accept+铸币+nonce） | pairing.rs + authorization.rs | **高**：新鉴权面；错码预算/重放/锁定/跨账号隔离负测必配 |
| revoke_pairing 连带撤销 exchange 会话 | authorization.rs | 中：撤销语义变化 |
| 本地账号构造 + 启动自愈重铸 + storage 重铸方法 | account.rs + native_auth.rs | 中 |
| 端口持久固定 + 占用重选迁移 | account.rs + account_settings | 中：数据键迁移 |
| mobile dist 进 resources + ServeDir | 构建脚手架 + tauri.conf.json | 低 |
| 手机登录门"输码即入"+文案 | mobile-web main.tsx 等 | 低 |
| 桌面面板地址展示+文案改造（含 signed_out 替换） | desktop 前端 | 低-中 |
| coordinator dev-dep 转正 + tower-http | Cargo | 低（记账） |

## 验收设计（摘要，执行单另立）

- 集成（真 router 同进程）：本地账号首启构造→exchange 错码预算/锁定/过期码/重放→正确码单事务（配对 active+会话+绑定）→下游（任务/文档/记忆）全链→撤销配对→手机会话实际终结→桌面重启台账/游标/outbox 连续→手机"退出所有设备"后桌面自愈重铸
- native（真机）：面板显示地址→手机浏览器打开→输码→任务确认→E1 记忆场景局域网版
- 防御负测：Host 不匹配拒、rebinding 姿态（伪 Origin/无自定义头 POST 拒）、静态响应带安全头、匿名端点限流
- 边界验收：防火墙三条恢复路径、电脑休眠唤醒 IP 变化、手机闲置 30min 重输码、运行>24h、内嵌组合无 request-code/verify-code/exchange 不在开发组合
- 修正一处初稿口误：手机从不心跳（心跳是桌面循环）；"心跳原样复用"指桌面心跳指向内嵌端口

## 开放点（实现前定）

1. cookie 会话在内嵌 Rust 的签发实现（签名方案 vs 随机 token 表）——倾向随机 token 表（复用 native_tokens，cookie 值即 token）
2. 防火墙手动放行引导的呈现位置（招募说明 vs 面板内故障自述）——倾向面板内自述+招募说明双份
