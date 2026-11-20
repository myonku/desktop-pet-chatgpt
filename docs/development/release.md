# 发布、版本与兼容管理

状态：已有本机 Windows x64 验证 ZIP、素材校验与装配和快捷方式工具。公开发布流水线及正式发行清单待实施。

## 首次正式发布门槛

M1 以本机开发验收收尾，暂缓附件交付及任何发布。M2 基础查询、M3 候选的开发验证继续按计划推进。

首次正式发布须验收完整的 Windows ChatGPT 客户端集成，包含在聊天窗口绑定已授权的客户端会话、发送并接收回复，以及回到客户端核验同一会话。实际支持的会话类型、接口版本与增强能力随对话切片验证。 [产品范围](../design/product.md) · [会话与权限](../design/integrations.md#对话的后续边界)

## 版本轴

| 版本 | 当前 | 提案 |
| --- | --- | --- |
| 应用 | Cargo.toml 为 0.1.0 | 发布版本按对外行为与兼容承诺独立递增 |
| 包协议 | 读取器与制备工具采用 v2 PNG，`engine_api = 2` | Windows 首发默认包使用 v2，读取兼容性独立于应用版本 |
| 角色包 | 现有 pack_version 是字符串；旧文档要求正式包与程序同版 | 独立语义版本，内容改变即换版本；不可覆盖已发布同 ID/版本内容 |
| 配置/缓存 | 配置 schema 1；其他存储需各自定义 | 分别标版本，缓存可丢弃，用户数据不能静默覆盖 |
| 客户端适配 | Windows ChatGPT 连接默认启用，速览打开时自动查询套餐与 Codex 额度，聊天窗口共享状态并保留手动刷新 | 随应用发布，明确每种客户端、平台、版本与字段的测试范围。 [M2 实测](../plan/validation.md#m2-windows-默认连接与自动刷新) |

首发附件使用 v2 清单及其中的兼容声明，同步记录实测应用与素材组合。未对外发布的开发设计按 [D19](../plan/decisions.md) 迭代。打 `v<app-version>` 标签前确保 Cargo.lock 同步。Python 工具无需仅因素材变化而跟随应用版本；若独立分发再制定工具版本。

## 发行内容

Windows 首发推荐便携 ZIP。每个发行组合需要程序包、可安装的默认素材、依赖/来源清单、SHA-256、已知限制与回退说明。用户最终安装布局仍保持 EXE 旁 `resources/default/manifest.json` 可用；下载拆成两个 ZIP 时给出确定的安装步骤，不能只发一个无法首启的裸 EXE。

本机 Windows 开发附件采用已安装的 Windows App Runtime 2（最低 2.4，架构与 EXE 一致）。发行清单须注明版本、架构以及随包携带或预装的方式。使用实际发行组合在干净环境复验桌宠、托盘和面板的首次打开，并把随包运行时计入下载与安装预算。[面板技术方案](../design/panels.md)

默认素材和全量/高清扩展分开列体积及适用条件。经确认可再分发的资源才纳入公开包；这也包括已编译进 EXE 的图标、字体等，不因在源码树里就默认许可齐备。原始私信许可材料只保留受控索引，公开清单写来源和许可结论。

推荐 `dev`（本地实验）、`preview`（有限支持和迁移可能）、`stable`（声明范围内完整验收）三类渠道；开发期首个可发布版本可先为 preview。Linux 先选运行库/发行版基线和桌面环境，再决定 tar、AppImage 或系统包；文件能执行不等于桌宠交互完整。

## 发布清单最少字段

```text
app_version, source_commit, source_clean, target, features
toolchain, lockfile_hashes, binary_hashes
pack_id, pack_version, schema_version, engine_api, manifest_hash, payload_hashes
download_bytes, installed_bytes, dependencies, license_inventory
client_support_matrix, desktop_support_matrix
validation_report_location, validation_report_hash, known_limits
upgrade_from, config_schema, rollback_instructions
```

该清单是待实现的装配输出，不是现有命令已经生成的文件。预览实验可有源码补丁快照；公开正式附件建议从干净已审定提交构建，绝不把 dirty 工作树的产物冒称某标签。

## 候选流程

1. 锁定版本、目标、内容、兼容和预算；检查工作树/锁文件/来源清单。
2. 在独立 `artifacts/dist/<version>/<target>/build` 以默认 features 构建；不得带 development 或参考 FFmpeg 路径。
3. 装配程序、默认素材与依赖；对素材源和装配快照都做分发校验，保留精确 manifest 字节。
4. 在新目录解压实际附件，以普通用户/只读安装目录验证启动、设置、恢复、换包、离线、客户端查询和桌宠对话。
5. 执行声明范围的设备和性能矩阵；验证退出、升级与回退；核对包体与入包动作，不用旧候选报告替代。
6. 生成哈希/来源/支持表与用户发布说明，再发布；不可覆盖同名版本附件。

现有素材装配命令见[开发工作流](workflow.md)。它验证已声明载荷和预算，但不验证作者许可、程序版本兼容、实际画质或常驻性能，不能独自决定发布资格。

## 升级、回退与数据

先下载/解包到新目录并校验，再在程序退出后切换；保留上一程序、匹配包和配置快照。应用与素材可独立升级，但切换前必须查兼容表。首次启动新版配置迁移须可恢复，写超前版本失败时保留原文件；旧程序回退读取旧快照，不要求其理解新 schema。

首发手动升级；自动更新后续需另设计签名、可信发布源、完整下载校验和故障恢复。普通 SHA-256 只能检查文件与给定清单一致，不能独立证明清单来源可信。

维护窗口与旧数据导入支持范围按发布说明列出。上游 v0.5.1 的显式用户数据导入按已核验的来源提供，后续是否保留该外部导入入口单独决策。
