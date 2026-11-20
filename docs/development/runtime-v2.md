# 当前运行时契约（素材 v2）

Windows 运行时、`validate-pack` 和装配器使用 v2 PNG 目录包。素材 schema 与 TOML 配置版本独立。实现见 [manifest.rs](../../src/media/manifest.rs)、[pack.rs](../../src/media/pack.rs) 与 [storage](../../src/storage/mod.rs)。

## 配置与启动

| 项目 | 当前规则 |
| --- | --- |
| 产品配置 | `%LOCALAPPDATA%/DesktopPetChatGPT/config.toml`；设置保存到该文件 |
| 开发配置 | `development` feature 只从启动工作目录读取 `desktop-pet.dev.toml`；缺失用默认值；设置只影响本次运行 |
| 格式 | UTF-8 TOML，可有 BOM；上限 64 KiB；schema_version = 1；未知字段或超前版本拒绝 |
| 包路径 | pack_path 指向含 manifest.json 的目录；相对路径相对于配置文件目录 |
| 默认包 | 开发使用配置目录内的 `assets/packs/white-dragon/0.1.0-v2/`，产品使用 EXE 旁的 `resources/default/` |
| 加载失败 | 先配置包，后默认包；都失败则首帧前退出；配置损坏时询问是否备份并恢复，拒绝保留原件并退出 |
| 单实例 | 同一配置根/用户会话下限制重复运行，第二次启动尝试打开已有实例设置 |

省略 `initial_scale_percent` 时使用默认值 **100**，可配置范围为 50–200。已保存的 `saved_size.base_dip`（1–4096）和 `saved_size.percent`（50–200）优先。无已保存尺寸时，Windows 根据面板物理尺寸推导基准，探测失败时用 256 DIP，再按初始百分比缩放。

默认启用 `auto_move`、`fullscreen_hide`、`sound` 和 `task_bubbles`，关闭 `start_hidden` 和 `passthrough`。`bubble_scale_percent` 默认为 100（50–150）。`preferences.chatgpt_connection` 默认是 `true`，应用设置后生效。旧值 `off` 仍表示关闭，`manual` 和 `automatic` 均按启用读取；保存时写入布尔值。套餐和 Codex 额度快照只保留在内存。任务气泡目前仅有设置项。隐藏与穿透需要可用的恢复入口。

设置支持预览、应用并保存、撤销未确认值和恢复默认。快捷面板与托盘的操作保留设置草稿，应用时合并未冲突的字段，冲突字段提示确认。产品保存使用同目录临时文件刷盘再替换；检测外部改写/文件占用。登录自启使用当前用户 Run 项，配置写失败时尝试恢复原注册值。

设置可选择本地目录或直接输入路径。「检查素材包」在后台运行分发校验并显示包名称、版本和动作数。保存不同包路径会重新校验，被拒绝的路径保留在草稿供修正，已保存路径继续有效。校验期间修改草稿或关闭设置会作废当前检查。保存成功后重启应用新包。

开发配置中的 `trace_playback` 和 `trace_input` 为布尔值，分别优先于 `DESKTOP_PET_TRACE` 和 `DESKTOP_PET_TRACE_INPUT`。`window-diagnostics` 可启用诊断日志和几何测试。

`preferences.auto_diagonal` 默认开启，自移动开启且素材支持时才生效。设置中心默认使用「全方向」，「关闭」只关闭 `auto_move` 并保留斜向偏好，「仅水平」开启 `auto_move` 并关闭 `auto_diagonal`。托盘和快捷设置只切换 `auto_move`。旧程序会把 `auto_diagonal` 视为未知项，回退前可从配置中移除此键。开发构建可在 `[development]` 用 `diagonal_angle_degrees` 调整相对水平的角度，范围 1–60°，默认 25°。

## 素材清单

| 字段 | 当前约束 |
| --- | --- |
| `schema_version`, `pack_id`, `pack_version` | schema 为 `2`，包 ID 非空，包版本为 SemVer |
| `engine_api`, `required_features` | 引擎版本为 `2`，当前必需能力列表应为空。旧开发包须以新清单重新装配 |
| `encoding`, `pages` | `encoding` 为 `png`。页含 `id`、`size`、`payload`，载荷含 `path`、`sha256`、正整数 `bytes`。读取时匹配声明长度和 SHA-256 |
| `clips` | `id`、可选 `preview_name`、`logical_size`、`looping`、`anchor`、`offset`、`scale`、`directions`、可选 `movement_us` 及 `frames`。每帧声明 `page`、`rect = [x,y,w,h]` 和 `duration_us` |
| `bindings` | 固定语义键映射到 `{ "default": "<变体名>", "variants": { "<变体名>": "<clip ID>" } }`。每组必须声明至少一个变体，每个变体引用实际 clip，默认名必须在本组的 `variants` 中 |
| 素材元数据 | `color_space`、`alpha`、`source_sha256`、`tools` 分别要求 `sRGB`、`straight`、非空来源 SHA-256 列表及非空的制备工具版本表 |

同组的变体名使用 1–48 个 ASCII 小写字母、数字和单个分隔连字符，首尾不能是连字符。组内变体名与 clip 引用必须各自唯一；不同组可共用 clip。未知语义、重复键、悬空引用和未知必需能力均报错。未绑定的 clip 仍可预览。`preview_name` 允许最多 24 个字符，不得为空白或含控制字符，建议约 6 个汉字或 12 个半角字符。省略时显示 clip ID，吸边和探头默认项使用对应方向的姿态名称。

帧持续时间为正微秒，累计不得溢出；矩形须位于引用页内；移动时窗满足 0≤start<end≤动作时长。尺寸为 1–4096；scale 正且有限，anchor/offset 有限。解析按 ID 绑定，不依赖数组顺序。

## 引擎行为语义

`validate-pack --behaviors` 从引擎列出所有受理的语义键。`idle` 为唯一必需语义，默认变体须循环。其余语义可按素材能力提供。

| 语义键 | 程序入口与选择行为 | 白龙全包默认变体 |
| --- | --- | --- |
| `idle` | 启动、恢复与基础待机持续播放默认变体。空闲时可随机播一次其他变体，自移动关闭时仍可原地换待机动作 | `rest`（待机） |
| `interact` | 单击未吸边角色后随机播一次。缺失时保留姿态 | `wave`（招手） |
| `drag`、`land` | 拖拽期间使用默认拖拽动作，未吸边落地时播一次默认落地动作。缺失时使用基础姿态 | `held`、`land` |
| `move.left`、`move.right` | 优先选择一步后仍与边界有余量的方向。两侧条件相同时随机选向。各变体必须非循环、声明对应方向和合法移动时窗。缺失方向不参与选路 | `run`（跑） |
| `move.up-left`、`move.up-right`、`move.down-left`、`move.down-right` | 可选。采用对应左右朝向和移动时窗。缺失时复用同侧 `move.left` 或 `move.right` 明确命名的 `fly` 变体 | 39 动作包复用 `fly`，标准十动作包仅支持水平移动 |
| `dock.left`、`dock.right` | 贴边停靠使用同侧默认姿态。缺失时显示基础待机 | `hold` |
| `peek.left`、`peek.right` | 单击已吸边的角色时切换默认探头姿态。缺失时回退同侧吸边，再到基础待机 | `peek` |

当前只定义普通桌宠语义。工作、额度和反应等专用素材可保留短显示名，供手动预览。面板命令和客户端事件独立于这些素材。

当自移动与原地待机变体均可用时，两类动作轮流获得自动播放机会，首轮优先自移动。同类内可随机选用可用变体。

水平方向的 `walk` 变体单步最多移动 72 DIP，其余移动沿用 144 DIP 的水平步长上限。靠近边界时按剩余空间缩短步长。不同变体仍各用素材声明的移动时窗。

启用斜向移动后，每次选择自移动路线有 25% 的概率尝试斜走。左右选向和水平步长沿用水平移动规则。竖直位移取水平步长乘以角度的正切，沿同一动作时窗插值。上下方向都放不下该步或缺少可用素材时，本次改走水平路线。现有 v2 包无需改清单。新增语义键只适用于支持它们的引擎，旧引擎遇到未知语义会拒绝加载。

程序按场景请求默认、随机或指定变体。随机从对应组中可用的独立 clip 选择；指定变体缺失或无法播放时回退到本组默认，默认也不可用则由表中各行为决定。变体名只在自己的语义组和素材包内有意义。其他素材包可以为相同语义提供不同变体，不必复刻白龙或保留它的 clip ID。播放预览时直接选中该条 clip，移动在原地预览，默认吸边／探头项按对应边缘动作演示。

```json
{
  "idle": {"default": "rest", "variants": {"rest": "rest-a", "read": "ambient-book"}},
  "move.left": {"default": "run", "variants": {"run": "run-left", "walk": "walk-left"}}
}
```

上例是 `bindings` 的内容。向左自移动可从 `move.left` 随机选取动作；若某场景需要走路，可请求 `walk` 变体，包内未提供时使用 `run`。 [白龙全包映射](../../tools/asset_optimizer/white_dragon_bindings.py)

## 播放与载荷

先将逻辑画布等比居中适配视口，再绕归一化 anchor 缩放，offset 按适配后的画布宽高偏移，正方向右/下。呈现与 alpha 命中共用像素中心采样。同侧 edge/peek 循环时间线一致可保留播放相位；共用帧且仅横向偏移不同可在 180 ms 内滑动。分别裁切的素材可在时间线一致时保留相位。

每项资源声明 `key`、可选 `client_id` 及 `content`。内容由 `kind` 区分，`image` 引用 `page`，`audio`、`text`、`icon` 的 `file` 声明 `path` 和 `sha256`。常见通用资源包括 character.poster、character.name、interaction.click；按 client_id 优先，缺失回退通用项。音频是规范 PCM16 WAV，单/双声道；文本 UTF-8。app.icon / tray.icon 仅允许通用 ICO 资源，缺失/损坏各自回退 EXE 图标；一个 ICO 含 1–8 个边长不同的 16–256 方形 RGBA8 PNG，单平面 32 位、范围不重叠。应用图标覆盖是现状，是否继续是设计选择。

路径以 `/` 分隔，每段仅 ASCII 字母、数字、`._-`，拒绝空段、`.`、`..`；规范化后须为包根内普通文件，读取时再次检查。SHA-256 为 64 位小写十六进制。

| 边界 | 当前限制 |
| --- | --- |
| manifest / 单音频、文本、图标 | 各 ≤1 MiB |
| 静态页展开 RGBA | ≤32 MiB |
| 运行单页 | ≤6 MiB |
| 运行 PNG 输入 / 解码器工作区 | 各 ≤3 MiB |
| 页请求 | 一个未完成请求，当前页 + 预取页，GPU 单纹理槽 |
| 页等待 | 首帧与后续缺页分别有 750 ms 期限；后续保留有效帧，有界回退 |

PNG 必须静态 RGBA8、尺寸匹配。可选资源未声明可回退；已声明载荷损坏运行时可独立回退，分发校验仍拒绝。动画页缺失或损坏按播放失败处理，普通动作失败时回到待机。若已选素材包的基础待机动作失败，当前进程尝试一次默认包，更新角色与设置显示，并保留原配置。默认包无法继续时停止运行并报告错误。结构或引用无效时拒绝整个包。

## 校验器与装配器

| 命令模式 | 检查范围 |
| --- | --- |
| validate-pack --behaviors | 列出引擎受理的全部语义键，无需素材包 |
| validate-pack `<dir>` | 清单和全部声明载荷；不代表满足播放预算 |
| validate-pack --playback `<dir>` | 额外检查 PNG 页的运行预算 |
| validate-pack --distribution `<dir>` | 与 `--playback` 使用同一校验，供分发装配调用 |

三种校验模式均要求 `idle` 默认变体指向循环 clip。应用和素材版本独立。ZIP 总体积、许可、实际 GUI 和性能另行验收。`assemble-asset-pack.py` 复制唯一声明载荷、保留 manifest 原字节、验证装配快照、生成 ZIP/哈希并拒绝覆盖已有输出；provenance 不自动入运行包。

## 旧数据迁移

独立 `migrate` 命令要求目标用户目录无运行实例；始终写产品目录，原件只读。命令见[使用说明](../user/guide.md)。

config-json 导入历史 schema 1 JSON；bailong-preferences 读取 `HKCU/Software/BailongPet/DesktopPet`；bailong-history 默认读 `%LOCALAPPDATA%/BailongPet/conversations.json`，支持 BAILONG_DATA_DIR 或显式文件。历史目标为 `history/legacy-bailong-codex/<源会话ID的SHA256>.json`，未完成状态记 interrupted，不恢复执行。

备份和 receipt 位于 `backups/migrations/<migration-id>/`。重试核对同源内容，已存在目标报告冲突；回滚只处理哈希仍匹配的结果，保留用户后续修改。配置上限 64 KiB，历史输入 64 MiB，转换后单会话 16 MiB。跨仓库导入仅针对已核验 v0.5.1，不承诺跟随上游变化。
