# 文档入口

本项目仍在开发期。当前能运行的是 Windows 普通桌宠；Linux 和真实客户端集成尚未实现。2026-11-14 起按产品能力重新组织设计，不再沿用 P0–P5 阶段驱动开发。

首次阅读：[现状审查](plan/baseline.md) → [产品边界](design/product.md) → [后续主线](plan/roadmap.md) → [待决策事项](plan/decisions.md)。

| 读者 / 问题 | 文档 |
| --- | --- |
| 用户：现在能做什么，如何配置 | [使用说明](user/guide.md) |
| 开发者：如何构建、验证和维护文档 | [开发工作流](development/workflow.md) |
| 开发者：当前读取器和配置到底支持什么 | [现行 v1 契约](development/runtime-v1.md) |
| 设计：功能范围、用户路径与缺省策略 | [产品设计](design/product.md) |
| 设计：模块边界、平台差异与重构顺序 | [架构设计](design/architecture.md) |
| 设计：桌宠快捷操作、客户端面板和托盘分工 | [面板与窗口交互](design/panels.md) |
| 设计：素材替换、版本、动作与制备流程 | [素材协议](design/assets.md) |
| 设计：ChatGPT / DSH 数据查询及后续对话 | [客户端集成](design/integrations.md) |
| 设计：包体、内存、帧延迟如何取舍 | [成本与性能](design/performance.md) |
| 交付：版本、附件、升级和回退 | [发布管理](development/release.md) |

状态约定：**现状**有代码或本轮核对支持；**历史实测**只适用于原报告条件；**提案**尚未实现或未由维护者选定；**已确认**来自本轮维护者答复。提案不等于当前产品承诺。

仓库保留当前契约、设计、精简决策和可执行验收方式。原始日志、截图、每轮试验日记放在被忽略的 `artifacts/`，发布证据另随对应 Release 保存；本地忽略目录不能充当团队唯一证据库。旧文档撤出当前树前已逐文件备份，位置与追溯规则见[现状审查](plan/baseline.md)。
