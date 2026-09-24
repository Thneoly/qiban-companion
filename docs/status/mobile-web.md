# 手机 Web 首轮交付

日期：2026-09-24。对应 T41/T42 登录与隔离以及 O3 跨端同一伙伴的首个网页增量；不表示 T21 设备配对、T44 动作授权或 G3 已准出。

## 实现范围

- 新增实际 npm 工作区 `apps/mobile-web`：受邀邮箱验证码登录、轻量伙伴形象、账号/伙伴标识、共享待办创建与取消、恢复与退出。
- 电脑浏览器和手机网页复用协调服务的账号、伙伴映射与账号内待办。原生桌面数据库不自动迁移或同步；聊天、记忆、模型配置、Live2D 外观未接入。
- 同源 Node 网关只代理明确列出的账号和待办路由，上游固定回环 Rust API；无任意代理、远程执行、附件或文件读写入口。
- HTTPS 会话使用 `__Host-`、Secure、HttpOnly、SameSite=Strict Cookie；令牌不进入响应 JSON、URL、localStorage 或 sessionStorage。会话有效性与撤销每次由 Rust 后端检查。
- 校验精确 Host、Origin、代理 HTTPS 标记及写请求头；拒绝跨站请求，设置 CSP、禁止嵌入和缓存，限制请求大小、并发与聚合速率。临时邀请制联调的聚合限制不等同生产反滥用体系。
- 网页每 10 秒拉取，恢复网络和重新可见时刷新；请求版本防止退出后迟到数据重新显示。当前页面对验证、新建重试保留 nonce/requestId；刷新会清空未确认提交状态，需要先核对列表。
- `npm run mobile:https` 构建页面，校验下载的官方 cloudflared，输出临时 HTTPS 链接，关闭后结束共享；未触发实际邮件发送。

## 验证记录

- `npm run check`：桌面 TypeScript、34 条既有 Vitest 用例、桌面构建及手机构建/网关测试通过；网关最终为 5 组。
- `npm run test:mobile:integration`：真实 Rust HTTP/SQLite/随机验证码认证 + Edge 桌面与手机尺寸独立上下文通过，约 65 秒。覆盖同账号两次独立登录、相同伙伴/待办、另一账号隔离、Cookie 恢复、创建回执丢失重试去重、离线恢复、跨端取消、退出后的迟到响应、单设备与全局退出。
- `cargo test -p companion-coordinator --locked`：8 条通过，1 条显式选择的浏览器集成测试默认忽略；该浏览器集成已用上一条命令单独执行通过。
- `cargo clippy -p companion-coordinator --all-targets --locked -- -D warnings` 与 `cargo fmt --all -- --check` 通过。
- 文档/目录/计划检查及 8 条独立原型流程通过。
- 首次公网尝试因代理 fake-IP 将 Cloudflare 节点解析到 198.18.x.x、TLS EOF 失败。启动脚本已改为等待隧道连接注册，失败有诊断，不把分配到 URL 视为成功。用户调整网络后重新连接成功。
- 公网 HTTPS 冒烟：Edge 手机尺寸页面返回 200，未登录 `/api/state` 返回 401，`isSecureContext=true`，无脚本异常或横向溢出。截图在被忽略的 `apps/mobile-web/test-results`。未代替用户申请或读取真实验证码。
- GitHub Actions 远端仍为 `disabled_manually`，未触发 CI。

浏览器登录自动化使用独立临时数据库和内存邮件适配器，公网冒烟仅验证登录页和未登录边界，不能代替真实手机、真实 SMTP 和公网登录后的全链路人工验收。

## 尚未准出的内容

真实手机邮箱登录、不同移动网络可达性与独立用户体验需按[联调指南](../development/mobile-web.md)验收。iOS Safari、微信内浏览器和其他手机兼容性尚未验证。固定生产入口、设备配对、授权远程执行、记忆删除跨端收敛不在本增量内。

下一步先完成上述真人双端验收，再接入原生桌面账号视图；远程动作仍需 T44 授权与 T21 配对前置，不将共享待办记录视为已经交办执行。
