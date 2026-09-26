# 文档中心

工程启动和命令见[项目README](../README.md)。根目录不再存放专题报告；本文是统一入口。

| 分类 | 文档 | 用途 |
|---|---|---|
| 产品 | [有限记忆设计](product/limited-memory.md) · [可行性报告](product/feasibility-report.md) · [商业决策](product/commercial-decision.md) | 目标客户、市场、产品形态与收益假设 |
| 计划 | [敏捷阶段门计划](planning/agile-stage-gate-plan.md) · [交付待办](planning/delivery-backlog.md) | 目标、产能、依赖、准入准出 |
| 架构 | [有限记忆技术设计](architecture/limited-memory.md) · [工程架构](architecture/overview.md) · [技术可行性](architecture/technical-feasibility.md) | 当前代码边界及目标架构 |
| 质量 | [Q6执行与复核手册](quality/memory-q6-runbook.md) · [有限记忆验收设计](quality/limited-memory-acceptance.md) · [首次使用验收单](quality/first-use-acceptance.md) · [设计评审](quality/design-review.md) · [测量与验收协议](quality/verification-protocol.md) | 设计缺口、处理记录、验证方法 |
| 研究 | [基础假设审视](research/foundational-hypotheses-review.md) · [验证工具包](research/validation-kit.md) · [模拟协议](research/simulation-protocol.md) | 早期探索材料，不等同实际客户证据 |
| 开发 | [Windows安装与数据保留](development/windows-installer.md) · [个人记忆服务指南](development/personal-memory.md) · [Git协作约定](development/git-workflow.md) · [模型与Live2D配置](development/model-settings-live2d.md) | 分支、运行与本机配置 |
| 状态 | [Windows安装交付](status/windows-installer.md) · [个人记忆服务Rust化](status/personal-memory-service.md) · [M4本地Q6验收](status/memory-q6-local.md) · [M3模型记忆使用](status/memory-model-context.md) · [M2记忆面板与导出](status/memory-panel.md) · [M1单实例、契约与迁移](status/memory-foundation.md) · [T13/T14设计进度](status/limited-memory-design.md) · [首次使用指南](status/first-use-guide.md) · [本机对话保存与恢复](status/chat-history-persistence.md) · [T04语音验证](status/voice-chain-spike.md) · [T08角色状态](status/companion-expression.md) · [模型输出预算](status/model-output-budget.md) · [对话阅读与状态](status/chat-reading.md) · [真实模型与临时会话](status/session-chat.md) · [通用模型与Live2D交付](status/model-settings-live2d.md) · [位置记忆与下一增量](status/pet-position-memory.md) · [桌面角色交付](status/desktop-pet.md) · [骨架交付记录](status/implementation-status.md) | 已实现能力、实际测试和未完成范围 |

[早期交互验证原型](../prototypes/companion-validation/index.html)是独立研究材料，不是桌面产品入口。统一执行 `npm run verify:docs` 和 `npm run verify:prototype` 检查文档及原型。

所有文档使用相对链接；引用其他分类时以当前文档为基准。新文档按用途放入对应目录，并更新本索引。计划中的目标能力、候选进度与实际实现记录分开维护。
