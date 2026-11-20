# 文档入口

当前 Windows 开发版本包含普通桌宠、快捷操作栏和客户端速览，支持查询 ChatGPT 套餐及 Codex 额度。多客户端速览与详情切换已用第二个合成来源验证，DSH 真源待接入。 [当前验证范围](plan/validation.md#当前审查与交接) · [客户端集成设计](design/integrations.md)

首次阅读：[当前开发基线](plan/baseline.md) → [产品边界](design/product.md) → [后续主线](plan/roadmap.md) → [待决策事项](plan/decisions.md)。

| 读者 / 问题 | 文档 |
| --- | --- |
| 用户：现在能做什么，如何配置 | [使用说明](user/guide.md) |
| 开发者：如何构建、验证和维护文档 | [开发工作流](development/workflow.md) |
| 开发者：当前读取器和配置到底支持什么 | [运行时、配置与 v2 素材契约](development/runtime-v2.md) |
| 验证：跨阶段实测、证据位置和未覆盖场景 | [工程验证记录](plan/validation.md) |
| 设计：功能范围、用户路径与缺省策略 | [产品设计](design/product.md) |
| 设计：模块边界、平台差异与重构顺序 | [架构设计](design/architecture.md) |
| 设计：桌宠快捷操作、客户端面板和托盘分工 | [面板与窗口交互](design/panels.md) |
| 设计：素材替换、版本、动作与制备流程 | [素材协议](design/assets.md) |
| 设计：ChatGPT / DSH 数据查询及后续对话 | [客户端集成](design/integrations.md) |
| 设计：包体、内存、帧延迟如何取舍 | [成本与性能](design/performance.md) |
| 交付：版本、附件、升级和回退 | [发布管理](development/release.md) |

状态约定：**现状**有代码或本轮核对支持；**历史实测**只适用于原报告条件；**提案**尚未实现或未由维护者选定；**已确认**来自本轮维护者答复。提案不等于当前产品承诺。

仓库保留当前契约、设计、决策和验收结论。原始日志和截图按运行保存在本地 `artifacts/`，完整程度取决于设备与留存情况。需要共享的发行证据随对应附件或持久存储保存，并登记位置与哈希。 [本地产物说明](../artifacts/README.md)
