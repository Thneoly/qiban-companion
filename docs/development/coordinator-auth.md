# 自有账号服务：本机联调

栖伴自行管理账号、邮箱验证码和会话，不需要托管身份平台。手机首发采用Web；当前交付协调HTTP服务，尚无手机登录页面或设备配对。实现证据见[交付记录](../status/coordinator-auth-api.md)。

## 一键启动（Windows，推荐）

在仓库根目录运行：

```powershell
npm run coordinator
```

也可以直接双击 [apps/coordinator/start.cmd](../../apps/coordinator/start.cmd)。两种入口使用同一份配置。需要已安装Rust/MSVC与Windows C++构建工具；首次构建可能下载依赖。单独启动此后端不需要先执行npm ci。

首次按提示填写SMTP主机、用户名、密码/应用授权码、发件邮箱和受邀邮箱。普通SMTP的TLS默认starttls/587，可选tls/465；识别到Cloudflare主机时默认tls/465，并校验用户名为api_token。本机端口默认4318。SMTP资料需要来自你自己的邮箱服务商，脚本不自动申请账号。下次启动无需再填写。

脚本自动建立独立账号库、生成并保留认证密钥、检查端口、增量构建并前台启动。看到服务监听提示后可访问 `http://127.0.0.1:4318/healthz` 检查进程，按Ctrl+C停止；它是API后端，当前没有手机页面。启动不会发邮件。

```powershell
npm run coordinator:configure  # 修改邮件/受邀邮箱/端口，保存后退出
npm run coordinator:check      # 无交互检查配置、密钥、Cargo和端口
```

`check`不编译，也不验证SMTP连接/送达。端口占用会提示处理，不终止已有进程；正常启动会再次检查端口。配置损坏时明确报错，不自动清空数据。

配置保存在被Git忽略的 `.cache/coordinator/`，目录仅授权当前Windows用户和SYSTEM访问。SMTP密码通过Windows DPAPI加密写入 `settings.clixml`，只能由同一Windows用户/设备解密；启动时以进程环境变量把明文密码传给启动器及其子进程（构建在注入前完成，命令行参数不出现密码，但运行中的启动器进程环境可被同账户管理员读取）。`auth-secret`是受目录权限保护的认证密钥文件，`accounts.db`是账号库，两者不是加密数据库。重新配置邮件不会更换密钥或删除账号库。不要清理此目录；换设备后需重新配置SMTP并单独迁移账号库和认证密钥。

此前手动配置使用的 `.cache/coordinator-accounts.db` 和 `.cache/qiban-auth-secret` 不会自动导入。若已有真实数据，继续使用下方手动入口，或在停止服务并备份后迁移；不要把桌面数据库移入账号库。

启动入口只对当前PowerShell子进程设置脚本执行策略，不改系统策略。实现见[启动脚本](../../apps/coordinator/scripts/start.ps1)，验证记录见[协调服务交付记录](../status/coordinator-auth-api.md)。

## Cloudflare发信排障

Cloudflare Email Sending的SMTP主机为 `smtp.mx.cloudflare.net`，用户名固定为 `api_token`，密码使用具备 `Email Sending: Edit` 权限的API Token；发件域名须已加入Email Sending。它只支持隐式TLS/465，启动向导中必须选择 `tls`，不支持 `starttls`/587。[官方SMTP说明](https://developers.cloudflare.com/email-service/api/send-emails/smtp/)

向导会为该主机选择正确默认值，并在保存/读取配置时拒绝错误TLS模式或用户名；手动环境变量路径由服务在启动时执行同样的校验。配置修改后必须停止旧服务，再执行 `npm run coordinator`；正在运行的进程不会自动重载文件。

没有发送记录时先区分三个阶段：

- 未进入SMTP：邮箱不在受邀列表时，接口仍返回challengeId，但不会发信；还需检查接口是否返回429/503。
- 连接失败：DNS、网络代理、465连通性或TLS握手问题可能发生在SMTP认证之前。进程healthz正常、Token active都不代表邮件连接成功。
- SMTP已接收：再核对Cloudflare投递/抑制日志以及收件箱、垃圾箱。

如果本机域名被网络代理解析为虚拟IP，且TLS握手提前断开，需核查代理的SMTP/465出站规则或换一条可用网络路径；不能仅凭虚拟IP断言代理就是根因。不要关闭证书验证来绕过错误。SMTP的TLS校验使用Windows系统证书存储（SChannel）：若本机网络做TLS拦截，需把拦截方根证书装入本机"受信任的根证书颁发机构"，系统不认的根会在握手阶段被拒绝。

## 运行边界

服务使用Rust/Axum和独立SQLite账号库，只监听127.0.0.1。邮件通过支持TLS的SMTP服务发送，认证状态由栖伴保管。生产HTTPS、反向代理、账号/IP滥用防护、监测、备份和PostgreSQL业务库属于T43，当前不可直接公开试用。

首版为受邀邮箱登录，最多配置100个邮箱。邮箱按ASCII、去首尾空白和全小写归一化；不同大小写视为同一账号。暂无国际化邮箱、邮箱更换/账号合并、密码、短信、社交登录、MFA、通行密钥或公开注册。

验证码8位，10分钟有效，最多5次错误尝试；新验证码使同邮箱旧验证码失效。每邮箱60秒冷却、每小时最多5次，实例全局每小时最多30次；计数持久化，失败发送也占次数。未受邀邮箱返回相同形状的随机challengeId且不发信，不提供账号是否存在的字段；这不是恒定耗时或完整反枚举保证。

SMTP成功仅代表中继服务器确认接收，不保证进入收件箱。发送失败/超时不激活验证码；邮件已发出但激活提交失败时不能据此登录。用户稍后重新获取，不自动重发。验证码HMAC绑定challengeId、邮箱和验证码；数据库只保存HMAC摘要，密钥独立保管。

会话使用256位不透明令牌，由服务端HMAC密钥与客户端登录nonce派生，仅验证响应返回明文，数据库只保存SHA-256摘要；验证响应在网络上丢失时，客户端用同一challengeId、验证码与nonce重试可恢复同一会话，换nonce不能复用已消耗的验证码，会话撤销后恢复同样被拒。最长24小时、闲置30分钟失效；当前不提供刷新令牌，失效后重新验证邮箱。读取会话会更新活动时间，绝对到期不延长。单会话退出与全部退出立即写入服务端状态；全部退出同时作废该账号未用验证码。账号、会话和待办在同一SQLite事务域核验，失效身份不能继续读写。

## 手动本机配置（可选）

从仓库根目录使用独立数据库，不指向桌面的companion.db/chat-history.db/executions.db。新账号库为schema v2；本增量会将既有协调账号库v1事务迁移到v2并保留账号、伙伴及任务。

生成一次独立的验证码HMAC密钥（不要覆盖已存在文件、不要打印或提交密钥）：

```powershell
$authSecretPath = Join-Path $PWD '.cache/qiban-auth-secret'
$authSecretBytes = New-Object byte[] 32
$authRng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
$authRng.GetBytes($authSecretBytes)
$authRng.Dispose()
$authSecretStream = [System.IO.File]::Open($authSecretPath, [System.IO.FileMode]::CreateNew)
$authSecretWriter = New-Object System.IO.StreamWriter($authSecretStream)
$authSecretWriter.Write([Convert]::ToBase64String($authSecretBytes))
$authSecretWriter.Dispose()
[Array]::Clear($authSecretBytes, 0, $authSecretBytes.Length)
```

.cache必须存在。密钥文件及SMTP凭据只授权运行账号读取；部署时通过受控秘密配置提供，不存前端或Git。轮换HMAC密钥会使旧验证码无法通过，不会自动撤销已登录会话；会话撤销走服务端退出流程。

```powershell
$env:QIBAN_AUTH_SECRET_FILE = Join-Path $PWD '.cache/qiban-auth-secret'
$env:QIBAN_ALLOWED_EMAILS = 'tester-one@example.com,tester-two@example.com'
$env:QIBAN_SMTP_HOST = 'smtp.example.com'
$env:QIBAN_SMTP_USERNAME = 'YOUR-SMTP-USER'
$env:QIBAN_SMTP_PASSWORD = [System.Net.NetworkCredential]::new('', (Read-Host 'SMTP password' -AsSecureString)).Password
$env:QIBAN_SMTP_FROM = 'Qiban <no-reply@example.com>'
$env:QIBAN_SMTP_TLS = 'starttls'
$env:QIBAN_COORDINATOR_DB = Join-Path $PWD '.cache/coordinator-accounts.db'
$env:QIBAN_COORDINATOR_PORT = '4318'
$env:CARGO_HOME = Join-Path $PWD '.cache/cargo'
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target'
cargo run -p companion-coordinator --locked
```

SMTP TLS只允许starttls（587端口，必须升级TLS）或tls（465端口）；没有忽略证书验证、明文发送或打印验证码的运行配置。真实发信仅在显式调用request-code且邮箱在受邀列表时发生；启动不会发送邮件。未准备SMTP/测试收件人时先运行自动化测试，不使用真实地址冒烟。

## HTTP接口

HTTP /v1与桌面IPC v2分别版本化，POST使用application/json。登录以外请求携带Authorization: Bearer令牌；不接受URL令牌、Cookie或客户端accountId/user_id。所有响应no-store，不返回上游SMTP错误正文，不打印请求/验证码/令牌。

| 方法与路径 | 输入 | 返回/行为 |
|---|---|---|
| GET /healthz | 无 | 服务进程存活，不保证SMTP可用 |
| POST /v1/auth/request-code | email | challengeId；若可发送则先由SMTP确认接收并激活 |
| POST /v1/auth/verify-code | challengeId、code、nonce | accessToken、sessionId、expiresAt、idleTimeoutSeconds；nonce为客户端生成的32字节base64url（43字符），令牌由服务端密钥与nonce派生，响应丢失后用同一组参数重试可取回同一会话 |
| GET /v1/me | Bearer token | accountId、companionId |
| GET /v1/tasks | Bearer token | 当前账号待办 |
| POST /v1/tasks | requestId（UUID）、title | 创建/去重；仅记录，不自动执行 |
| GET /v1/tasks/{id} | Bearer token | 本账号任务，越权与不存在均404 |
| POST /v1/tasks/{id}/cancel | revision | queued取消，版本不符409 |
| POST /v1/logout | allSessions（布尔） | revoked、allSessions；同步完成服务端撤销 |

错误码区分401身份/验证码无效、429频率限制、503存储/发信不可用；request-code期间的并发状态变化（如全部退出作废了待激活验证码）按503上报，不伪装成验证码错误。除healthz外的接口限制16个并发请求、8 KiB JSON正文，邮件整体等待最多10秒；healthz不在并发限制内，邮件堆积不会掩盖进程存活信号。尚无按来源IP限制、可持久邮件队列或分布式速率限制，不把本地限制当成公网运营防护。

令牌不得进入URL、截图、代码提交或共享终端记录。手机Web下一增量需实现同源BFF/HttpOnly会话或明确的端侧凭据方案与CSRF防护；目前没有登录页面，也没有把长期令牌放localStorage的实现。

## 可重复验证

```powershell
cargo test -p companion-coordinator -p companion-storage --locked
```

协调服务测试使用仅在测试编译中存在的内存收件箱，生成真实随机验证码/令牌并走真实存储及HTTP路由；另有实际TCP请求。存储测试覆盖到期、错误次数、全局/邮箱限额、撤销、代码作废、事务失败回滚、v1迁移和重开。没有绕过验证码的产品测试模式。

真实准出还需SMTP投递与邮件到达、两真实邮箱/设备、HTTPS浏览器、Cookie/凭据保护、刷新/重开体验、邮件滥用、备份与删除恢复、独立安全复核。源码构建和内存收件箱通过均不等于真实发信或手机接续通过。

设计依据：[OWASP会话管理](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)、[一次性令牌与尝试限制](https://cheatsheetseries.owasp.org/cheatsheets/Forgot_Password_Cheat_Sheet.html)、[lettre邮件库](https://docs.rs/lettre/latest/lettre/)。
