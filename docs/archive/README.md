# 阶段归档

这里保留结项时的记录及当时的路径、命令和判断。原始本地产物可能已经清理。继续实施以[常驻文档](../../README.md#文档索引)为准，需要当前证据时按[现行工程约定](../G-05-contracts.md#工程与产物)重新验证。

| 阶段 | 归档                                                                                                                                                                                        |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P0   | [工程基线](p0/R-01-p0-baseline.md)与[动作盘点快照](p0/p0-legacy-actions.json)                                                                                                                 |
| P1a  | [透明首帧](p1/R-02-p1a-first-frame.md)、[窗口输入](p1/R-03-p1a-window-input.md)、[UI 与恢复](p1/R-04-p1a-ui-recovery.md)                                                                       |
| P1b  | [离线资源包](p1/R-05-p1b-offline-packs.md)、[有界播放](p1/R-06-p1b-bounded-playback.md)、[格式初测](p1/R-07-p1b-format-comparison.md)、[代表素材验收](p1/R-08-p1b-representative-validation.md) |
| P2   | [普通桌宠结项](p2/README.md)、[离线运行证据](p2/P2-04-offline-validation.md) |

移出工作树的技术评估和早期存档可从 Git 快照查阅。

```powershell
git show 17e47dc:docs/G-07-media-rendering-decision-2026-11-07.md
git ls-tree -r --name-only 17e47dc -- docs/archive
```
