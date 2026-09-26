# 项目协作与目录约定

本项目使用npm workspace和Cargo workspace。处理任务前先阅读根README及相关目录代码；实现与验证记录见docs/status，产品目标不等同已实现功能。

## 文件归属

- 根目录只放README、AGENTS、工作区清单/锁文件及跨工作区配置，不新增专题报告、演示HTML或一次性脚本。
- apps/desktop拥有桌面界面、Tauri宿主、应用资源、应用专属构建/测试配置和端到端测试。浏览器预览仍属于该应用。
- apps/coordinator拥有独立协调HTTP服务、账号/邮件适配、服务配置和服务专属测试；不依赖Tauri或桌面UI。
- apps/memory-service拥有个人记忆独立服务（MCP stdio与HTTP双协议、旧库迁移）及其专属测试；自包含不依赖companion crates，不与栖伴应用数据库互写。
- packages放可跨前端复用的TypeScript包；当前contracts提供IPC契约。共享包不能反向依赖apps。
- crates放不依赖桌面UI的Rust模块。companion-core保持纯领域逻辑；companion-storage依赖core；Tauri宿主负责组装，核心层不能反向依赖Tauri。
- docs按product、planning、architecture、quality、research、status、development分类。新增或迁移文档后同步docs/README.md，并使用可迁移的相对链接。
- prototypes放独立研究/交互原型，不进入正式产品构建或依赖链。
- scripts/verification放文档、结构与原型检查；一次性本机辅助文件放被忽略的.cache目录。
- 单元测试跟随所属包/模块；应用端到端测试归应用。只有实际出现跨应用集成测试时才新增根tests目录。

不为未来功能预建空的cloud、mobile、hardware或services目录；在有实际实现与清晰职责时增加工作区。模型密钥不得进入前端代码或VITE环境变量。

## 迁移与验证

目录移动前确认源/目标位于工作区内且不会覆盖现有文件。移动后修正相对链接、npm脚本、测试配置、CI及运行路径；保留工具约定的target、dist、node_modules生成位置并由.gitignore排除。

默认从根目录运行npm run check、npm run verify:docs和npm run verify:prototype。桌面入口/测试路径变化时运行npm run test:browser；Rust代码或工作区变化时运行对应cargo检查。只报告实际执行结果，不把生成配置写成已通过外部CI或商业门。

Git使用main与短期功能分支；提交前检查暂存内容。协作细节见docs/development/git-workflow.md；不对main强制推送，不自动创建发布标签。
