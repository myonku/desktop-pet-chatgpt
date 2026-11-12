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

包内文件采用以 `/` 分隔的相对路径，每段仅允许 ASCII 字母、数字、`._-`，且不能是空段、`.` 或 `..`。解析后的文件须在包根内。页目录由清单决定。PNG 为默认兼容档，请求 BC7 时按设备与页面支持情况回退到可用的 RGBA8 或 PNG。包无效且无可用包时使用内置占位，运行中的动作回退见 [G-03](G-03-architecture.md#2-核心契约rust-实现草案语义)。320×320 代表页参考 4–5 帧横排，预算见 [G-06](G-06-validation.md#3-性能实验与预算)。

v1 读取器拒绝未知字段。P2-01 先记录首批产品动作 ID、必备待机绑定、可选动作回退、画布坐标单位及变换顺序，再用同一套规则制作合成包和代表包。路线参数由动作目录或领域配置提供，包内的 `movement_us` 定义该动作允许产生位移的时间窗。需要变更清单字段时，先明确现有 v1 包的兼容策略。

运行时页限制比整包静态校验更严格。当前单播放页上限 6 MiB，BC7 同时计入压缩块和 CPU 命中用 RGBA。PNG 输入和解码器工作区各有 3 MiB 限额。清单及单个音频/文本载荷各有 1 MiB 限额。P2 制包和 P4 附件装配均使用 `validate-pack --distribution` 检查声明的全部页档。可选资源未声明时按语义键回退，已声明载荷的有效性仍按包校验规则检查。

## 素材工作区

| 路径 | 用途 |
| --- | --- |
| `assets/source/<素材集>/` | 本地素材原件、来源说明、清单和许可文件 |
| `assets/packs/<pack-id>/<pack-version>/` | 已转换、可校验和待装配的解包目录 |
| `<exe-dir>/resources/default/` | 发布程序约定的默认包根 |
| `<exe-dir>/resources/<其他目录>/` | 可在产品配置中选择的其他本地包 |

素材转换统一以根 `assets/` 为工作区。素材原件、转换包与独立发行 ZIP 由 Git 忽略。转换或重新分页应保留动作 ID、时序、资源映射和来源记录。资源/依赖台账登记 ID、来源、作者或权利主体、获取日期、SHA-256、许可依据、允许用途、转换链和拟入包范围。私人通信证据保存在受控位置，公开文档只留脱敏索引。

通过配置的 `pack_path` 指向解包根。产品默认包按 EXE 所在目录定位，显式路径按下节解析。安装或更新先解到新目录并校验，再切换目录，或在退出后替换默认包。保留上一版本用于回退。

## 配置分层

| 层 | 入口 | 职责 |
| --- | --- | --- |
| 构建 | `Cargo.toml`、`rust-toolchain.toml` | 工具链、依赖和 features |
| 产品偏好 | `%LOCALAPPDATA%/DesktopPetChatGPT/config.json`，带整数 `schema_version` | 用户可选的启动值与设置偏好，例如界面大小和功能开关 |
| 开发覆盖 | `--dev-config <path>` 显式读取 TOML。仓库根的 `desktop-pet.dev.toml` 由 Git 忽略，随仓库发布 [desktop-pet.example.toml](../desktop-pet.example.toml) | 覆盖本次启动的尺寸与素材包 |

产品启动时以经过校验的用户配置值覆盖内置默认值，默认行为以 [G-02](G-02-feature-spec.md) 为准。发布版设置使用同一套校验。用户确认一项变更后，存储层将新值原子写入 `config.json`，下次启动继续使用已保存的值。预览中的中间值不写盘。仅支持重启生效的设置应在界面标明。应用启动前可直接编辑用户配置文件。

程序内置所有产品设置的默认值。设置中的「恢复默认设置」从同一默认值来源生成配置，原子保存成功后应用可即时生效的设置，其余标明需重启。保存失败时提示未保存，保留原配置文件。坏配置的原件先保留或备份，再由用户明确触发修复或恢复默认。

产品配置中的素材路径可为绝对路径，或相对于用户配置文件目录。未指定时从 EXE 旁的 `resources/default/` 查找。遇到格式、值或超前 schema 错误，保留原文件、提示原因并使用安全缺省值。后续自动保存保留该原文件。配置原子保存、迁移和备份见 [G-03](G-03-architecture.md#5-存储与迁移)。

P2-02 在设置接入前定义最小产品 schema，统一校验缺项、取值范围与路径，区分已保存的用户值和本次生效值。该节点同时确定首次推导尺寸的持久化方式，区分推导尺寸和用户手动保存的尺寸。P2-03 基于该 schema 验证旧配置导入和回退。

首轮实现读取以下两个同名字段，按开发 TOML → 用户 JSON → 内置默认值逐字段取值。开发覆盖与用户文件分开保存。素材包路径统一从配置读取。

| 字段 | 首轮规则 |
| --- | --- |
| `initial_scale_percent` | 50–200 的整数，内置默认 70。当前乘以 P1 的 256×256 DIP 基准，100 可复现 P1 尺寸。显示器物理尺寸探测待 P2-01 实测接入 |
| `pack_path` | 含 `manifest.json` 的本地目录。相对路径以各自配置文件所在目录为准。未设置时查找 EXE 旁的 `resources/default/`，缺包时使用占位 |

用户 JSON 示例为 `{"schema_version": 1, "initial_scale_percent": 70, "pack_path": "packs/dawn"}`。开发 TOML 可省略版本，复制样板后用 `cargo run --locked -- --dev-config desktop-pet.dev.toml` 启动。格式、跟踪与参照参数仍由 `DESKTOP_PET_FORMAT`、`DESKTOP_PET_TRACE`、`DESKTOP_PET_TRACE_INPUT` 和参照 feature 下的 `DESKTOP_PET_REFERENCE_*` 控制。采样脚本为每次运行生成独立 TOML，并显式使用 100% 以保留 P1 对照尺寸。运行时保存与一键恢复随 P2-02 设置界面实施。

## 源码、程序与素材附件

- 源码归档来自标记的 Git 树，包含常驻规范和阶段文档。
- 程序 ZIP 按明确清单装配。收录默认 feature 的 EXE、已审定的运行依赖、许可通知、版本与校验清单和用户说明。P4 核实 CRT、字体、干净机及回退；历史文档和阶段验证产物留在开发侧。
- 素材 ZIP 是独立附件，ZIP 根直接放 `manifest.json` 和清单引用的文件。经确认可分发的包才附到 release，并在发布说明中列出程序版本、适用的素材包版本、附件 SHA-256、来源与分发依据。安装时把 ZIP 内容解到指定包根。

在本地装配时使用新输出文件名，命令会输出整个 ZIP 的 SHA-256。工具校验输入及装配快照，包括整包载荷和实际播放的页预算，成功后才写入目标附件。`validate-pack <目录>` 检查清单和整包载荷，`--distribution` 额外试读全部已声明的页档。

```powershell
python scripts/release/assemble-asset-pack.py --validator "$build/validate-pack.exe" --pack $pack --output artifacts/dist/local/pet-assets-dawn-0.1.0.zip
```

用合成包或已获相应权限的素材验证装配。现有素材的公开发布资格按 [许可说明](../README.md#许可) 和 P4 清单审定。
