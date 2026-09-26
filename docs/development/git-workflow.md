# Git 协作约定

仓库根目录即 npm 和 Cargo 工作区根目录。代码、锁文件、配置、原始图标与文档纳入 Git；依赖、构建产物、缓存、数据库、凭据和测试结果由 .gitignore 排除。

## 日常修改

main 保存可检查的基线。功能和修复使用短期分支，如 `feat/desktop-pet-window`、`fix/task-cancellation`、`docs/device-protocol`，完成后通过 PR 整合回 main；无需长期维护 develop 分支。

```sh
git switch main
git pull --ff-only
git switch -c feat/desktop-pet-window
# 修改后运行适用验证
git status --short
git diff
git add <明确的文件或目录>
git diff --cached
git commit -m "feat(desktop): add pet window"
git push -u origin feat/desktop-pet-window
```

提交围绕一个可说明的变化，类型可用 feat、fix、refactor、test、docs、chore。PR 写明实际行为与验证结果，不把占位能力写成已上线服务。

## 合并与版本

- 合并前运行与修改相关的检查，入口见[README](../../README.md)。Windows工作流保留构建、Rust、浏览器和文档/原型验证定义；当前已暂停自动执行，按下文执行本机检查。
- CI 失败先修复；强制分支保护以远端实际配置为准，本文不代表已开启保护。
- 不对 main 强制推送，不提交令牌、私钥或本机数据库；密钥只使用 CI secrets 或适当的本地凭据存储。
- 可执行文件和安装包不进入源码历史；以后经发布流程创建 Release 附件，当前不创建发布标签或 Release。
- .gitattributes 统一文本为 LF，二进制图标保留原样。身份配置仅影响本仓库，不改全局 Git 身份。

## CI暂停（2026-09-21）

按项目发起人要求，远端 `Desktop skeleton`（工作流ID `357098028`）已设置为 `disabled_manually`，检查时没有未完成运行，无需取消。当前分支的 `.github/workflows/ci.yml` 仅保留 `workflow_dispatch`，移除 push/pull_request 自动触发。远端停用立即生效；源码触发规则通过PR合并后进入main。

暂停不代表检查通过，也不修改分支保护或历史运行。本机继续执行README中与改动相关的验证。

用户决定恢复时，可在GitHub Actions页面启用，或执行：

```sh
gh workflow enable ci.yml --repo Thneoly/qiban-companion
gh workflow run ci.yml --repo Thneoly/qiban-companion --ref main
```

手动运行要求main已包含 `workflow_dispatch`。恢复自动CI还需通过PR加回push/pull_request事件；仅启用工作流不会恢复本分支已移除的自动触发。
