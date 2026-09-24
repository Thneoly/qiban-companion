# 手机 Web 登录与 HTTPS 联调

电脑浏览器和手机浏览器登录同一个受邀邮箱，显示同一个账号、同一个伙伴 ID 和协调服务中的同一份待办。页面支持验证码登录、重开恢复、新建、取消、刷新、退出本设备和退出所有设备。

此阶段共享的是账号服务待办。原生 Tauri 桌面本地待办、聊天、记忆和外观仍独立；网页中的栖栖是固定轻量形象。登录不代表已经配对电脑，也不授权远程执行。实现与检查见[交付记录](../status/mobile-web.md)。

## 启动

在 `D:\Game` 打开两个 PowerShell 终端。第一次更新代码后运行 `npm ci`，需要 Node.js 22.12+、Rust 和本机已配置的 SMTP。

终端一：

```powershell
npm run coordinator
```

终端二：

```powershell
npm run mobile:https
```

第二条命令构建网页，检查后端健康，然后下载并校验固定版本的官方 cloudflared（约 55 MB，缓存于 `.cache/mobile-web`），启动仅监听 `127.0.0.1:4320` 的网页服务和临时 HTTPS 隧道。无需 Cloudflare 账号、SMTP 之外的邮件配置或手机证书安装。不修改系统 PATH、防火墙或 DNS。

控制台出现 `Open on BOTH computer and phone: https://….trycloudflare.com` 后，电脑和手机都打开这条完整链接。最近一次链接也写在被 Git 忽略的 `.cache/mobile-web/last-url.txt`；文件可能保留已经失效的旧链接，以当前控制台为准。关闭隧道终端后链接失效，重启会换链接并要求重新登录，服务端伙伴和待办仍保留。

首次下载需可访问 GitHub；隧道需访问 Cloudflare，包括出站 7844 端口。若日志显示 `198.18.x.x` / `198.19.x.x` 与 TLS EOF，检查代理 fake-IP/TUN：让 `*.argotunnel.com` 使用真实 DNS，或临时关闭 TUN 后重试。诊断日志在 `.cache/mobile-web/tunnel.log`。不要通过关闭证书验证解决网络错误。地址创建成功不保证手机所在网络能访问，需实际打开确认。

临时链接使用 [Cloudflare Quick Tunnels](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/trycloudflare/)，定位开发联调，不作为正式上线入口。第三方隧道终止公网 TLS，页面和 API 流量经过该服务。只开放本次网页与账号待办接口；SMTP、数据库文件和电脑文件操作不对外开放。

## 首次验收

1. 电脑打开 HTTPS 链接，填写已配置的受邀邮箱，获取 8 位验证码并登录。
2. 新建一条待办，例如“周末整理旅行清单”。展开“核对账号与伙伴标识”。
3. 距离上次请求验证码至少 60 秒后，手机打开同一链接，用同一邮箱申请新的验证码并登录。两个浏览器使用独立会话。
4. 比较账号 ID、伙伴 ID 和待办。手机取消该待办，电脑点击“刷新”或等待最多约 10 秒，确认也显示已取消。
5. 刷新网页或关闭后重开，仍应恢复有效登录及列表。关闭手机网络、电脑新增一条待办，手机恢复网络后应重新获取。
6. 手机退出本设备，电脑应保持登录；电脑“退出所有设备”后，其他页面下一次请求应退出，不再显示旧账号数据。

验证码只进入自己的收件箱，不需要复制账号 ID、伙伴 ID、API Key 或令牌。未受邀邮箱也会得到相同的请求回执，但不会收到邮件；检查邀请名单应使用本机配置向导。验证码申请不自动重试，网络失败时不要连续点击发信；验证码验证重试复用同一个 nonce，新建待办在当前页面重试复用 requestId。

会话最长 24 小时、空闲最多 30 分钟，由后端判定。网页可见时每 10 秒拉取，切回页面和网络恢复时重新核对。后台页不持续轮询。断线期间保留最后显示的数据并提示连接问题，不做离线提交队列。

## 本机预览及自定义端口

不需要手机时，运行 `npm run mobile`，电脑打开 `http://127.0.0.1:4320`。这个回环地址不能在手机使用；它使用独立的本机 Cookie，与 HTTPS 登录不共享会话。

如果协调服务向导中改过端口，在第二个终端指定：

```powershell
$env:QIBAN_COORDINATOR_URL = 'http://127.0.0.1:你的端口'
$env:QIBAN_WEB_PORT = '4320'
npm run mobile:https
```

已有固定 HTTPS 反向代理时，构建后设置 `QIBAN_WEB_ORIGIN=https://你的域名` 并运行 `npm run start --workspace @companion/mobile-web`。代理转发至回环网页端口，保留该域名的 Host，并由可信代理设置 `X-Forwarded-Proto: https`。不要把 4318 直接映射公网。应用不会信任请求头来选择上游或推断允许的来源。正式部署还需要域名、运维、备份、滥用防护及权限审查，当前脚本不承担生产部署。

## 目录与验证

`apps/mobile-web/src` 是 React 页面；`server` 是仅面向该页面的 Node 同源网关，复用现有 Rust 协调服务；`scripts` 是 HTTPS 和测试入口；`tests/browser` 是浏览器集成测试。账号规则仍归 `apps/coordinator`，没有新增一套账号数据库。

```powershell
npm run test:mobile
npm run test:mobile:integration
```

前者检查生产构建和网关边界；后者通过 Rust 测试专用实例运行真实 HTTP、SQLite、验证码认证和两个独立 Edge 浏览器上下文，其中一个模拟 390px 手机。邮件使用测试内存收件箱，不向真实邮箱发信、不读取本机账号数据库。验证码读取路由仅编译进测试程序，生产二进制和网页网关都不提供。测试保留真实发信冷却，约需 65 秒以上，截图在 `apps/mobile-web/test-results`，不纳入 Git。
