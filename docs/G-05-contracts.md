# 工程、配置与交付约定

## 文档维护

- 常驻文档放在 `docs/` 根层，按 `G-01` 起连续编号。新增从下一号分配，普通修订保留编号。整编时统一更新现行文件、索引和交叉引用，历史引用保留其快照语境。
- 活跃阶段使用 `docs/stages/<阶段>/`，入口列出节点、依赖和进展。报告登记节点、日期、构建提交、工具链与 features、设备、输入哈希、命令、预期与实测、产物位置和未验证项。
- 验收后将阶段目录移入 `docs/archive/<阶段>/`，更新阶段入口和归档索引。阶段编号和文件名随阶段保留。历史命令、状态和路径可保留，用于追溯当时的结果。
- 影响全局的决定在结项前写入对应常驻文档。改动行为或配置时同步规格、架构或工程契约及验收矩阵。当前构建所需证据按现行工具重新采集。

## 工程与产物

- `src/main.rs` 是桌宠入口，`src/lib.rs` 为桌宠和 `validate-pack` 提供共用实现。模块职责见 [G-03](G-03-architecture.md)。
- `tests/` 通过公开接口测试包加载和资源查找。需要私有状态的白盒测试置于对应模块的独立测试文件。内置合成 PNG 仍随程序构建，合成包由 `scripts/validation/` 生成。

| 路径 | 用途 |
| --- | --- |
| `target/` | Cargo 默认构建输出，本地和 CI 使用相同约定 |
| `artifacts/validation/<阶段>/fixtures/` | 生成的验证输入 |
| `artifacts/validation/<阶段>/tools/` | 本地验证依赖 |
| `artifacts/validation/<阶段>/runs/<运行名>/` | 单次验证的日志、采样和报告，每次使用唯一运行名 |
| `artifacts/dist/<版本>/` | 待验收的发布附件与校验值 |

需要隔离 Cargo 构建时，显式使用 `--target-dir target/<用途>`。验证脚本在仓库根运行。以下合成包复测需要 Python 3.11 或更新版本。采样命令显式指定被测 EXE。

```powershell
cargo build --release --locked
python -m pip install --target artifacts/validation/p1/tools ispc_texcomp==1.0.1 texture2ddecoder==1.0.6 psutil==7.1.3
$env:PYTHONPATH = (Resolve-Path artifacts/validation/p1/tools).Path
python scripts/validation/p0/generate-p0-fixtures.py
python scripts/validation/p1/generate-p1b-fixtures.py
$build = 'target/release'
$pack = 'artifacts/validation/p1/fixtures/dawn'
& "$build/validate-pack.exe" --distribution $pack
python scripts/validation/p1/measure-p1b-playback.py --exe "$build/desktop-pet-chatgpt.exe" --pack $pack --output artifacts/validation/p1/runs/dawn-smoke-01 --formats png --rounds 1 --warmup 2 --seconds 10
```

开发用 FFmpeg 对照通过 `cargo build --release --locked --features p1b-reference --target-dir target/reference` 单独编译，命令显式传入独立安装的 FFmpeg 和素材输入。普通构建与程序 ZIP 使用默认 features。Windows CI 由维护者手动触发，执行锁定构建、格式、Clippy 及默认和全 feature 测试。

## 素材包 v1

包根是解包后直接包含 `manifest.json` 的目录。清单字段由 [Rust 类型](../src/media/manifest.rs) 定义，检查规则由 [读取端](../src/media/pack.rs) 执行。契约发布时随源码提供。

| 清单部分 | v1 约定 |
| --- | --- |
| 包元数据 | `schema_version = 1`，独立的 `pack_id` 与 `pack_version`，`color_space = "sRGB"`、`alpha = "straight"`，源文件 SHA-256 与转换工具记录 |
| 页 | `id` 唯一，`size` 为宽高。必备静态 RGBA8 PNG，可选原始 RGBA8 和 BC7，三档使用同一帧表。每个载荷声明路径和小写 SHA-256 |
| 动作与帧 | 稳定动作 ID，逻辑尺寸、循环、锚点、偏移、缩放、方向列表、可选移动时窗和帧列表。帧使用页 ID、`[x,y,w,h]` 和正整数微秒时长。移动时窗使用同一时间基准 |
| 附属资源 | 通过稳定 `key` 和可选 `client_id` 选择。图像引用页，声音为规范 PCM16 WAVE，文本为 UTF-8。客户端项优先，再回退通用项 |

包内文件采用以 `/` 分隔的相对路径，每段仅允许 ASCII 字母、数字、`._-`，且不能是空段、`.` 或 `..`。解析后的文件须在包根内。页目录由清单决定。PNG 为默认兼容档，请求 BC7 时按设备与页面支持情况回退到可用的 RGBA8 或 PNG。解码或校验失败时进入占位模式。320×320 代表页参考 4–5 帧横排，预算见 [G-06](G-06-validation.md#3-性能实验与预算)。

## 素材工作区

| 路径 | 用途 |
| --- | --- |
| `assets/source/<素材集>/` | 本地素材原件、来源说明、清单和许可文件 |
| `assets/packs/<pack-id>/<pack-version>/` | 已转换、可校验和待装配的解包目录 |
| `<exe-dir>/resources/default/` | 发布程序约定的默认包根 |
| `<exe-dir>/resources/<其他目录>/` | 可在产品配置中选择的其他本地包 |

素材转换统一以根 `assets/` 为工作区。素材原件、转换包与独立发行 ZIP 由 Git 忽略。转换或重新分页应保留动作 ID、时序、资源映射和来源记录。资源/依赖台账登记 ID、来源、作者或权利主体、获取日期、SHA-256、许可依据、允许用途、转换链和拟入包范围。私人通信证据保存在受控位置，公开文档只留脱敏索引。

当前程序通过 `DESKTOP_PET_PACK` 指向解包根。产品默认包按 EXE 所在目录定位，显式路径按下节解析。安装或更新先解到新目录并校验，再切换目录，或在退出后替换默认包。保留上一版本用于回退。

## 配置分层

| 层 | 入口 | 职责 |
| --- | --- | --- |
| 构建 | `Cargo.toml`、`rust-toolchain.toml` | 工具链、依赖和 features |
| 产品偏好 | `%LOCALAPPDATA%/DesktopPetChatGPT/config.json`，带整数 `schema_version` | 用户可选的启动值与设置偏好，例如界面大小和功能开关 |
| 开发覆盖 | Git 忽略的根目录 `desktop-pet.dev.toml`，由显式开发入口读取 | 测试包、帧时长覆写、跟踪和参照参数；一次性环境参数可进一步覆盖 |

产品启动时以经过校验的用户配置值覆盖产品缺省值，缺省值及回退行为以 [G-02](G-02-feature-spec.md) 为准。可由配置指定的偏好集中解析，功能模块使用解析结果。设置界面使用同一套校验，支持运行时应用的设置立即生效，确认保存后成为下次启动值。仅支持重启生效的设置应在界面标明。应用启动前可直接编辑用户配置文件。

产品配置中的素材路径可为绝对路径，或相对于用户配置文件目录。未指定时从 EXE 旁的 `resources/default/` 查找。遇到格式、值或超前 schema 错误，保留原文件、提示原因并使用安全缺省值。后续自动保存保留该原文件。配置原子保存、迁移和备份见 [G-03](G-03-architecture.md#5-存储与迁移)。

开发 TOML 路径相对于仓库根解析。显式开发会话按一次性环境参数、开发 TOML、产品偏好及其缺省值的顺序取值，仅允许开发入口覆盖约定的范围。开发覆盖不写入用户配置、素材清单或发布默认值。帧时长覆写等调试参数须记入实验记录，正式验收使用素材原时序。

当前原型通过 `DESKTOP_PET_PACK`、`DESKTOP_PET_FORMAT`、`DESKTOP_PET_TRACE`、`DESKTOP_PET_TRACE_INPUT` 和参照 feature 下的 `DESKTOP_PET_REFERENCE_*` 启动。上表中的用户配置与开发 TOML 是后续实现契约，具体字段随功能落地时定义。

## 源码、程序与素材附件

- 源码归档来自标记的 Git 树，包含常驻规范和阶段文档。
- 程序 ZIP 按明确清单装配。收录默认 feature 的 EXE、已审定的运行依赖、许可通知、版本与校验清单和用户说明。P4 核实 CRT、字体、干净机及回退；历史文档和阶段验证产物留在开发侧。
- 素材 ZIP 是独立附件，ZIP 根直接放 `manifest.json` 和清单引用的文件。经确认可分发的包才附到 release，并在发布说明中列出程序版本、适用的素材包版本、附件 SHA-256、来源与分发依据。安装时把 ZIP 内容解到指定包根。

在本地装配时使用新输出文件名，命令会输出整个 ZIP 的 SHA-256。工具校验输入及装配快照，包括整包载荷和实际播放的页预算，成功后才写入目标附件。`validate-pack <目录>` 检查清单和整包载荷，`--distribution` 额外试读全部已声明的页档。

```powershell
python scripts/release/assemble-asset-pack.py --validator "$build/validate-pack.exe" --pack $pack --output artifacts/dist/local/pet-assets-dawn-0.1.0.zip
```

用合成包或已获相应权限的素材验证装配。现有素材的公开发布资格按 [许可说明](../README.md#许可) 和 P4 清单审定。
