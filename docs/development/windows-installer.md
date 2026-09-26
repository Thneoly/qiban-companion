# Windows 安装包与数据保留

2026-09-19。当前为内部试用的Windows x64 NSIS安装包，未签名、未创建Release，安装包通过不等于产品准出。实际结果见[安装验收记录](../status/windows-installer.md)。

## 构建与使用

从仓库根运行：

```powershell
npm run build:installer
Get-FileHash -LiteralPath 'target/release/bundle/nsis/栖伴_0.1.0_x64-setup.exe' -Algorithm SHA256
```

双击生成的安装程序，按向导安装；从开始菜单“栖伴”或安装结束的运行选项启动。角色出现在桌面右下角，点击打开首次指南。日常设置、聊天、记忆仍使用`dev.qiban.companion`资料目录，与源码直接运行版本共享；安装前通过托盘退出正在运行的栖伴，避免两个版本同时占用资料。

安装采用当前用户模式，默认路径为`%LOCALAPPDATA%\栖伴`；提供简体中文与英语。已装WebView2时复用现有运行时，缺少时下载安装引导程序，需要联网；此缺失运行时分支尚未在干净设备实测。未签名版本可能显示Windows发行者提示，当前仅用于内部验收，不宣称商用发行准备完成。

实现采用[Tauri官方Windows安装配置](https://v2.tauri.app/distribute/windows-installer/)，配置覆盖层为[tauri.installer.conf.json](../../apps/desktop/src-tauri/tauri.installer.conf.json)。`npm run build:desktop`继续只构建可执行文件；安装包使用独立的`installer`前端模式，关闭本机`public`研究素材任意复制，包含原创SVG、构建代码及固定清单的Live2D运行库/许可声明，支持用户本地导入模型；不包含Hiyori或用户模型。见[数字人增量](../status/avatar-appearance.md)。

构建后的[素材检查](../../apps/desktop/scripts/verify-installer-assets.cjs)拒绝白名单外文件、软链接、示例目录及旧实验入口，重新核验运行库哈希；未来新增正式静态素材需要更新明确的允许范围。[CI](../../.github/workflows/ci.yml)新增安装构建和7天保留的未签名工作流附件，未创建发布标签或自动更新服务；配置存在不等于外部CI已通过。

## 卸载与重装

| 内容 | 默认卸载 | 在卸载向导勾选删除应用数据 |
|---|---|---|
| 程序、快捷方式、卸载注册项 | 移除 | 移除 |
| `%LOCALAPPDATA%\dev.qiban.companion`中的待办、聊天、记忆、配置和位置 | 保留，重装后恢复 | 删除 |
| 同标识的Roaming应用数据 | 保留 | 删除 |
| Windows凭据管理器中的模型/语音密钥 | 保留 | 仍保留；卸载器不管理系统密钥 |
| 用户已导出的JSON、服务商记录 | 不撤回 | 不撤回 |

需要删除文字模型密钥时，应在卸载前通过模型设置“删除密钥”处理；不同API地址的凭据分别管理。卸载勾选删除资料不代表撤销服务商密钥或删除全部系统凭据。默认卸载与同版本重装已实测，勾选删除资料的交互尚未执行。

安装配置禁止降低应用版本；本轮只有`0.1.0`，同版本覆盖重装不冒充跨版本升级。未来升级须同时验证数据库迁移、失败恢复和旧安装包拒绝，不能通过降级程序回退已经迁移的数据。

## 可重复的隔离安装验收

使用一个从未用过的名称，写入`.cache/installer-acceptance.conf.json`：

```json
{"productName":"Qiban Installer Acceptance unique-run","identifier":"dev.qiban.companion.acceptance.installer-unique-run"}
```

```powershell
npm run build:installer -- --config (Resolve-Path .cache/installer-acceptance.conf.json).Path
node apps/desktop/tests/native/installer-smoke.cjs .cache/installer-acceptance.conf.json 'target/release/bundle/nsis/Qiban Installer Acceptance unique-run_0.1.0_x64-setup.exe'
```

先退出所有栖伴进程。脚本只接受验收前缀，拒绝已有资料/注册项及占用的CDP端口；安装位置限定在仓库`.cache`下的新目录。测试使用Node 24、Windows/WebView2和Playwright；当前NSIS测试路径不支持空格。脚本会实际安装、启动、同版本重装、默认卸载、再次安装和最终卸载；合成资料、截图和逐项结果留在忽略目录，失败时保留现场，不能拿生产标识替代验收标识。脚本使用NSIS同步等待参数，测试目录可能保留卸载器自身文件；核对的是应用主程序与卸载注册项移除，不宣称测试目录被彻底清空。

结果包括安装包/二进制SHA256、提交基线及工作区修改标记。只使用一次无Key的localhost模拟请求，不读取生产聊天、不调用付费模型。进程重开采用受控终止，不能代替人工托盘退出体验。验证结束运行`npm run build:desktop`恢复日常开发演示构建。

独立测试Windows账号/干净设备依[首次使用验收单](../quality/first-use-acceptance.md)执行；缺少WebView2、非管理员账号、交互式卸载勾选、不同版本升级、多屏/休眠、签名信誉和独立用户体验均需另补证据。
