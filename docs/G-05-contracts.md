# 工程、配置与交付约定

## 文档维护

- 常驻文档放在 `docs/` 根层，按 `G-01` 起连续编号。新增从下一号分配，普通修订保留编号。整编时统一更新现行文件、索引和交叉引用，历史引用保留其快照语境。
- 活跃阶段使用 `docs/stages/<阶段>/`，入口列出节点、依赖和进展。报告登记节点、日期、构建提交、工具链与 features、设备、输入哈希、命令、预期与实测、产物位置和未验证项。
- 验收后将阶段目录移入 `docs/archive/<阶段>/`，更新阶段入口和归档索引。阶段编号和文件名随阶段保留。历史命令、状态和路径可保留，用于追溯当时的结果。
- 影响全局的决定在结项前写入对应常驻文档。改动行为或配置时同步规格、架构或工程契约及验收矩阵。当前构建所需证据按现行工具重新采集。

## 工程与产物

- `src/main.rs` 是桌宠入口，`src/lib.rs` 为桌宠和 `validate-pack` 提供共用实现。模块职责见 [G-03](G-03-architecture.md)。
- `tests/` 通过公开接口测试包加载和资源查找。需要私有状态的白盒测试放在对应模块的独立 `tests.rs`，仅测试构建编译。合成包生成、用例驱动、窗口操作和结果判定放在 `scripts/validation/`。程序提供诊断日志及按 feature 启用的诊断入口，内置占位 PNG 供已启动场景恢复使用。
- `scripts/assets/` 转换和装配本地素材。发布附件由 `scripts/release/` 装配。功能模块、Cargo feature 和诊断名称按用途命名，阶段编号用于验证脚本、产物和阶段报告。

| 路径 | 用途 |
| --- | --- |
| `target/` | 日常开发及 CI 的 Cargo 编译和缓存目录 |
| `artifacts/validation/<阶段>/fixtures/` | 生成的验证输入 |
| `artifacts/validation/<阶段>/tools/` | 本地验证依赖 |
| `artifacts/validation/<阶段>/runs/<运行名>/` | 单次验证的日志、采样和报告，每次使用唯一运行名 |
| `artifacts/dist/<版本>/` | 独立发布构建、装配暂存目录、待验收附件与校验值 |

需要隔离 Cargo 构建时，显式使用 `--target-dir target/<用途>`。验证脚本在仓库根运行。以下合成包复测需要 Python 3.11 或更新版本。采样命令显式指定被测 EXE。

```powershell
cargo build --release --locked --features development --target-dir target/development
python -m pip install --target artifacts/validation/p1/tools ispc_texcomp==1.0.1 texture2ddecoder==1.0.6 psutil==7.1.3
$env:PYTHONPATH = (Resolve-Path artifacts/validation/p1/tools).Path
python scripts/validation/p0/generate-p0-fixtures.py
python scripts/validation/p1/generate-p1b-fixtures.py
$build = 'target/development/release'
$pack = 'artifacts/validation/p1/fixtures/dawn'
& "$build/validate-pack.exe" --distribution $pack
python scripts/validation/p1/measure-p1b-playback.py --exe "$build/desktop-pet-chatgpt.exe" --pack $pack --output artifacts/validation/p1/runs/dawn-smoke-01 --formats png --rounds 1 --warmup 2 --seconds 10
```

| Cargo feature | 用途 |
| --- | --- |
| `development` | 启用开发 TOML、素材档位和诊断日志 |
| `window-diagnostics` | 独立启用窗口定位、尺寸模式、诊断日志和中文输入预览。可以分别测试产品配置和开发配置 |
| `ffmpeg-decoder` | 含 `development`。增加外部 FFmpeg 解码对照，需显式指定 FFmpeg 和源视频。窗口定位如需控制则另外启用 `window-diagnostics` |

普通构建与程序 ZIP 使用默认 features。Windows CI 由维护者手动触发，执行锁定构建、格式、Clippy 及默认和全 feature 测试。

## 素材包 v1

本项目在开发阶段随源码声明并维护素材协议。v1 以当前运行时素材包为基线。协议、转换器和资源可随开发调整，影响读取兼容性的改动更新协议版本及适配说明。后续由本项目独立维护素材组织和装配方式，不持续绑定上游仓库发行附件的目录、命名或清单。上游资源可经转换按当前协议装包。

ZIP 解包后，包根须直接包含 UTF-8 JSON 文件 `manifest.json` 及其引用的所有载荷。以下是采用 v1 的示例布局，子目录和文件名由清单决定。

```text
<pack-root>/
├── manifest.json
├── media/
│   ├── idle-00.png
│   ├── idle-00.rgba8     可选
│   ├── idle-00.bc7       可选
│   └── poster.png        可选
└── extras/               可选
    ├── click.wav
    └── name.txt
```

| 清单部分 | v1 约定 |
| --- | --- |
| 包元数据 | `schema_version` 为整数 1，`pack_id` 与 `pack_version` 为非空字符串，`color_space` 为 `"sRGB"`，`alpha` 为 `"straight"`。必填非空 `source_sha256` 数组和 `tools` 字符串映射，分别记录源文件哈希和转换工具 |
| `pages` | 非空列表。每页包含唯一 `id`、像素宽高 `size` 和必备 `png` 载荷。可选 `rgba8`、`bc7` 载荷与 PNG 共用宽高和帧表。所有载荷对象含 `path` 和原始文件字节的 `sha256` |
| `actions` | 非空列表。每个动作含唯一 `id`、`logical_size`、`looping`、`anchor`、`offset`、`scale`、`directions` 和非空 `frames`，可选 `movement_us`。方向列表支持 `"left"`、`"right"`，无方向要求时可为空。帧用 `page` 引用页 ID，`rect` 为页内像素 `[x,y,w,h]`，`duration_us` 为正整数微秒。移动时窗 `[start,end]` 与帧时长使用同一时间单位 |
| `resources` | 必填列表，可为空。每项含 `key` 与 `kind`，可选 `client_id`。同一 `key` 与 `client_id` 组合唯一。图像 `kind = "image"` 用 `page` 引用页 ID，`"audio"` 和 `"text"` 用 `file` 记录载荷。请求指定客户端资源时优先精确匹配，再查找通用项 |

PNG 为静态、8 位 RGBA，使用直通 alpha。原始 RGBA8 为逐行连续的 RGBA 字节，无文件头。BC7 为无文件头的 4×4 块流，每块 16 字节，页宽高须为 4 的倍数。页和逻辑画布各边为 1–4096。PNG 为默认兼容档，请求 BC7 时按设备与页面支持情况使用 RGBA8 或 PNG。声音为 RIFF WAVE，支持单声道或双声道 PCM16，含标准 16 字节 `fmt ` 块，紧接 `data` 块。文本为 UTF-8。当前设置和交互使用通用的 `character.poster`、`character.name`、`interaction.click`，分别提供角色图、名称与点击音效。

包内文件采用以 `/` 分隔的相对路径，每段仅允许 ASCII 字母、数字和 `._-`，且不能是空段、`.` 或 `..`。解析后的文件须为包根内的普通文件。所有 SHA-256 使用 64 位小写十六进制字符。清单字段和校验规则见 [Rust 类型](../src/media/manifest.rs)及[读取端](../src/media/pack.rs)。320×320 代表页参考 4–5 帧横排，预算见 [G-06](G-06-validation.md#3-性能实验与预算)。

v1 读取器拒绝未知字段。产品包使用循环 `idle` 作为必备待机。可选语义 ID 为 `interact`、`drag`、`land`、`move-left`、`move-right`、`edge-left`、`edge-right`、`peek-left`、`peek-right`。缺失探头姿态先尝试同侧吸边，再尝试待机。缺失互动保持当前动画，缺失或运行时失败的预览在菜单中禁用。已有的循环单动作 `move` 和 `reference` 包仍可播放。动作列表顺序对绑定无影响。启动依次尝试配置包和当前配置来源对应的默认包。清单、载荷路径和首个待机页通过加载并能上传后才允许显示角色，两处都失败则退出。后续页按需校验。

`logical_size` 定义画布宽高，`anchor` 是相对画布的归一化坐标，`offset` 以适配后的画布宽高为单位，正值向右、向下。先将画布等比居中适配视口，再绕锚点应用 `scale`，最后应用偏移并量化到像素边界。呈现和 alpha 命中共用目标矩形及像素中心采样。自主移动要求一次性动作同时声明对应方向与非空 `movement_us` 窗口。手动预览可原地播放这些动作。

运行时页限制比整包静态校验更严格。静态页 RGBA8 展开上限 32 MiB，当前单播放页上限 6 MiB，BC7 同时计入压缩块和 CPU 命中用 RGBA。PNG 输入和解码器工作区各有 3 MiB 限额。清单及单个音频/文本载荷各有 1 MiB 限额。P2 制包和 P4 附件装配均使用 `validate-pack --distribution` 检查声明的全部页档。可选资源未声明时按语义键回退，已声明载荷的有效性仍按包校验规则检查。

## 素材工作区

| 路径 | 用途 |
| --- | --- |
| `assets/source/<素材集>/` | 本地素材原件、来源说明、清单和许可文件 |
| `assets/packs/<pack-id>/<pack-version>/` | 已转换、可校验和待装配的解包目录 |
| `assets/packs/white-dragon/0.1.0/` | 开发 TOML 生效时的原型默认包 |
| `<exe-dir>/resources/default/` | 发布程序约定的默认包根 |
| `<exe-dir>/resources/<其他目录>/` | 可在产品配置中选择的其他本地包 |

素材转换统一以根 `assets/` 为工作区。素材原件、转换包与独立发行 ZIP 由 Git 忽略。转换或重新分页应保留动作 ID、时序、资源映射和来源记录。资源/依赖台账登记 ID、来源、作者或权利主体、获取日期、SHA-256、许可依据、允许用途、转换链和拟入包范围。私人通信证据保存在受控位置，公开文档只留脱敏索引。

通过配置的 `pack_path` 指向解包根。开发 TOML 和产品 TOML 的默认包分别以配置目录和 EXE 目录定位。显式路径按下节解析。安装或更新先解到新目录并校验，再切换目录，或在退出后替换默认包。保留上一版本用于回退。[本地原型素材装配](../assets/README.md)

## 配置分层

| 层 | 入口 | 职责 |
| --- | --- | --- |
| 构建 | `Cargo.toml`、`rust-toolchain.toml` | 工具链、依赖和 features |
| 产品偏好 | `%LOCALAPPDATA%/DesktopPetChatGPT/config.toml` | 用户可选的启动值与设置偏好 |
| 开发配置 | 启动工作目录的 `desktop-pet.dev.toml` | 启用 Cargo feature `development` 时为唯一默认配置来源，缺失时使用内置默认值 |

两个文件共用 UTF-8 TOML 模型、解析器和校验规则，最多 64 KiB，接受 UTF-8 BOM。构建标记决定唯一配置来源，缺项由内置默认值补全。开发构建的普通启动、设置和配置恢复均在工程目录范围内。产品构建将偏好写入用户目录。开发配置和诊断参数各由对应 feature 控制。可从 [desktop-pet.example.toml](../desktop-pet.example.toml) 复制模板。旧 `config.json` 由[显式迁移](#旧数据迁移)读取。

| 字段 | 规则 |
| --- | --- |
| `schema_version` | 整数，当前为 1，省略按 1 处理 |
| `initial_scale_percent` | 50–200 的整数，默认 70。无已保存尺寸时乘以物理探测基准，探测不可用时基准为 256 DIP |
| `[saved_size]` | 可选表，`base_dip` 为 1–4096 的逻辑基准，`percent` 为 50–200 的整数。共同确定逻辑边长，优先于初始比例和物理探测 |
| `preferences.start_hidden` | 布尔值，默认 false。启动时设为手动隐藏，需要可用托盘。「显示/隐藏」仅影响当次运行 |
| `preferences.fullscreen_hide` | 布尔值，默认 true。角色所在屏幕的前台窗口全屏时自动隐藏，需要可用托盘 |
| `preferences.auto_move`、`preferences.sound` | 布尔值，默认 true，分别控制自移动和音效 |
| `preferences.passthrough` | 布尔值，默认 false。开启全窗口鼠标穿透，需要可用托盘 |
| `preferences.task_bubbles` | 布尔值，默认 true。任务提醒气泡开关，运行效果随外部任务集成接入 |
| `preferences.bubble_scale_percent` | 50–150 的整数，默认 100。任务提醒气泡大小 |
| `pack_path` | 含 `manifest.json` 的本地目录。相对路径以所选配置文件所在目录为准。省略或指定包加载失败时，开发配置加载配置目录内的 `assets/packs/white-dragon/0.1.0/`，产品配置加载 EXE 旁的 `resources/default/` |
| `[development]` | 仅允许出现在开发构建读取的 `desktop-pet.dev.toml` 中。可选 `format` 为 `"png"`、`"rgba8"`、`"bc7"`，`trace_playback` 和 `trace_input` 为布尔值。`ffmpeg-decoder` 构建还可指定 `"ffmpeg"` |

配置文件缺失时使用默认值。选中的文件读取、解析或校验失败时，启动弹窗显示原因并询问是否备份和覆盖。拒绝会保留原件并退出。确认后先将可读取的原始字节备份到配置目录的 `backups/`，再以默认 TOML 原子替换同一配置。无法安全备份或写入时提示失败、保留原文件并退出。未知字段与不支持的版本按无效配置处理。

产品设置中的「应用并保存」提交确认值，关闭窗口撤销预览。未改动大小时保留首次尺寸推导策略。「恢复默认设置」清除已保存的尺寸、素材包等偏好。写入使用同目录独立临时文件，刷盘后替换正式文件。开发配置生效时，设置只应用到本次运行。登录自启使用当前 Windows 用户 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 中的 `DesktopPetChatGPT` 项，注册值是带引号的绝对 EXE 路径。仅真实用户目录的产品设置确认或恢复默认会修改该项。配置替换失败时尝试恢复原注册值并报告回滚错误。

开发构建的诊断脚本可用 `DESKTOP_PET_FORMAT`、`DESKTOP_PET_TRACE` 和 `DESKTOP_PET_TRACE_INPUT` 提供缺省诊断参数，`[development]` 中对应的显式设置优先。仅启用窗口诊断时，可用后两项收集日志，素材格式保持 PNG。窗口诊断以 `DESKTOP_PET_WINDOW_GEOMETRY=模式,x,y` 指定物理像素坐标，`auto` 使用普通尺寸逻辑，`dip` 跳过 EDID 探测，`256`、`320`、`512` 固定客户区像素边长。FFmpeg 对照使用 `DESKTOP_PET_FFMPEG_EXE` 和 `DESKTOP_PET_FFMPEG_VIDEO`，可通过 `DESKTOP_PET_FFMPEG_VERIFY` 启用 RGBA8 页哈希核验。

## 旧数据迁移

本项目的存储兼容承诺仅覆盖本仓库的历史版本。P2 提供的 `bailong-preferences` 与 `bailong-history` 是针对已核验上游 v0.5.1 样本的临时导入能力。两仓库的存储设计继续分化时，跨仓库导入能力可能在后续几次迭代后移除。

使用 `desktop-pet-chatgpt.exe migrate` 查看命令。迁移独立运行，要求同一用户目录下的桌宠已退出。迁移始终以产品用户目录为目标，普通启动只读取已经生成的产品配置。

| 子命令 | 来源与转换 |
| --- | --- |
| `config-json [path]` | 默认读取 `%LOCALAPPDATA%/DesktopPetChatGPT/config.json`。支持上一阶段 schema 1 的 JSON 字段，并按当前产品规则校验。异目录导入会把相对素材路径解析至源配置目录 |
| `bailong-preferences` | 只读取当前用户的 `HKCU\Software\BailongPet\DesktopPet`。导入已保存的桌宠与气泡比例、自移动、全屏隐藏、声音和任务气泡开关。按旧版规则限制比例，未设置项采用产品默认值 |
| `bailong-history [path]` | 默认读取 `%LOCALAPPDATA%/BailongPet/conversations.json`。如有 `BAILONG_DATA_DIR` 则使用该目录，显式传入文件时使用指定路径。读取旧版会话数组，生成独立的会话快照 |
| `rollback <migration-id>` | 核验该次原件备份和生成文件的哈希，将仍与导入结果一致的目标移入该备份的 `rolled-back` 项 |

配置导入以 `config.toml` 为目标。历史存入 `history/legacy-bailong-codex/<session-id>.json`，文件名取旧会话 ID 的 SHA-256。正文、结果、事件、公开摘要和旧线程标识随会话归档。未确认完成的状态转为 `interrupted`，保留原状态。真实会话恢复、历史界面和通知水位随 P3 实现。迁移命令只输出状态、迁移 ID 和脱敏错误。

每次导入把原始文件字节或已读取注册表值的类型与原始字节写入 `backups/migrations/<migration-id>/source.json`。解析或校验失败时保留该备份。已生成的迁移凭据 `receipt.json` 记录版本、来源 SHA-256 和输出哈希。新目标经原子移动发布，重读验证后写入 `applied` 标记。原始偏好文件、注册表和历史文件全程只读。

同一来源内容的重试使用同一迁移 ID，配置导入还将原目录计入 ID，以保留相对路径语义。已有配置或已存在的旧历史命名空间由当前数据占用，导入会报告冲突。若导入在发布前或写入完成标记前中断，同一命令可重试。回滚会保留被编辑、缺失或有新增文件的目标。同一已回滚的迁移继续保留回滚状态。配置源与转换结果各限 64 KiB，历史源限 64 MiB，转换后单会话限 16 MiB。

## 源码、程序与素材附件

- 源码归档来自标记的 Git 树，包含常驻规范和阶段文档。
- 程序 ZIP 按明确清单装配。收录默认 feature 的 EXE、已审定的运行依赖、许可通知、版本与校验清单和用户说明。P4 核实 CRT、字体、干净机及回退；历史文档和阶段验证产物留在开发侧。
- 素材 ZIP 是独立附件，ZIP 根直接放 `manifest.json` 和清单引用的文件。经确认可分发的包才附到 release，并在发布说明中列出程序版本、适用的素材包版本、附件 SHA-256、来源与分发依据。安装时把 ZIP 内容解到指定包根。

发布时从确定的提交进行独立默认 feature 构建，使用 `--target-dir artifacts/dist/<版本>/build`，再按清单把 EXE 和运行依赖放入独立暂存目录。素材从审定的解包根单独装配，安装后解到发布 EXE 旁的 `resources/default/`。日常开发使用工程的 `assets/`，日常 `target/` 的编译输出及其中曾存放的历史样本均不直接用于发布。归档中的 `target/` 路径只记录当时的实验。

在本地装配时使用新输出文件名，命令会输出整个 ZIP 的 SHA-256。工具校验输入及装配快照，包括整包载荷和实际播放的页预算，成功后才写入目标附件。`validate-pack <目录>` 检查清单和整包载荷，`--distribution` 额外试读全部已声明的页档。

```powershell
python scripts/release/assemble-asset-pack.py --validator "$build/validate-pack.exe" --pack $pack --output artifacts/dist/local/pet-assets-dawn-0.1.0.zip
```

用合成包或已获相应权限的素材验证装配。现有素材的公开发布资格按 [许可说明](../README.md#许可) 和 P4 清单审定。
