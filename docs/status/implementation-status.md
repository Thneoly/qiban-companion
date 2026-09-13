# 骨架交付记录

本页保留初始骨架的历史状态；透明角色与托盘的后续实现见[2026-09-14桌面角色交付](desktop-pet.md)。

日期：2026-09-13。用户授权：先搭建代码框架。工作目录：`D:\Game`。

## 已实现

- npm工作区与Rust工作区；React/TypeScript/Vite、Tauri 2与Rust分层。
- 桌面相处页面、自制SVG测试角色及打招呼交互；支持窄屏预览与减少动画偏好。
- IPC v1、响应解码、结构化错误、四个原生命令及main窗口能力白名单。
- 待办创建、列表、队列取消；Rust输入校验、SQLite迁移和事务保存。
- 原生SQLite与浏览器临时内存分别标识；原生调用失败不会回退为预览成功。
- 锁文件、Windows CI定义、开发说明与测试入口。

## 本机验证结果

| 验证 | 结果／边界 |
|---|---|
| TypeScript严格检查 | 通过 |
| Vitest协议与预览测试 | 4项通过 |
| Vite生产构建 | 通过 |
| Rust工作区测试 | 5项通过；包含SQLite关闭重开、取消幂等、拒绝更新版本数据库 |
| rustfmt检查 | 通过 |
| Clippy所有目标，警告作为错误 | 通过 |
| Edge浏览器端到端 | 2项通过；创建/取消、角色反馈、刷新清空、390px窄屏无横向溢出 |
| 页面视觉检查 | 已查看1280px宽截图，无主要布局遮挡 |
| Tauri Windows release构建 | 通过；`target/release/companion-desktop.exe`，约10.2MB |
| 原生窗口人工端到端 | 未执行；编译及存储测试不能代替真实WebView/IPC交互验证 |
| 外部CI／签名／安装包／发布 | 未执行；CI仅为已编写配置 |

编译机：Node 24.10.0、Rust 1.98.1、Windows MSVC工具链。Rust网络下载遇到环境TLS凭据错误后，使用项目目录中的已有依赖缓存完成离线编译；未关闭证书校验，也未修改全局工具链配置。依赖的准确版本以两份lock文件为准。

浏览器测试最初的npm子进程退出在此Windows环境中挂起，现改为测试进程内启动和关闭Vite；最终测试命令正常退出0。release链接器输出了创建导入库的提示，不影响构建成功。

## 尚未完成

真实AI对话/语音、透明悬浮宠物/托盘、长期记忆、任务执行器及动作审计、账号/用户隔离、手机配对/撤销、云服务与费用上限、用户导出删除、自动更新和商业计费均尚未实现。

对应计划的T01/T02/T03/T05/T06/T16仅有部分基础产物；没有将完整工作包或G0～G6标为通过。下一步可直接在此骨架上实现窗口样机或持久执行事件，不需要重建项目。

## 目录结构修订｜2026-09-13

产品资料已归档到docs的相应分类；早期HTML归prototypes，验证脚本归scripts/verification。桌面Playwright配置、端到端用例和图标源文件归apps/desktop。根README、文档索引、AGENTS约定、npm命令和CI引用同步更新。

本轮执行并通过：npm run check（类型、4项单元测试、前端构建）、npm run test:browser（2项端到端）、npm run verify:docs（本地链接/目录及计划容量检查）、npm run verify:prototype（8组脚本流程）。Rust源码及Cargo工作区未改动，本轮未重复原生构建；上表Rust与release结果保留为上一轮实际记录。

浏览器测试输出改为各用例在apps/desktop/test-results下的独立目录；旧根test-results归入被忽略的.cache/artifacts/browser-before-restructure，避免与当前结果混淆。
