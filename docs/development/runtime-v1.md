# 当前实现契约（v1）

本文描述当前工作树支持的行为，不是下一代协议。改代码时同步此文；产品提案见 [design](../README.md)。类型和边界检查的最终依据是 [manifest.rs](../../src/media/manifest.rs)、[pack.rs](../../src/media/pack.rs)、[storage](../../src/storage/mod.rs)。

## 配置与启动

| 项目 | 当前规则 |
| --- | --- |
| 产品配置 | `%LOCALAPPDATA%/DesktopPetChatGPT/config.toml`；设置保存到该文件 |
| 开发配置 | `development` feature 只从启动工作目录读取 `desktop-pet.dev.toml`；缺失用默认值；设置只影响本次运行 |
| 格式 | UTF-8 TOML，可有 BOM；上限 64 KiB；schema_version = 1；未知字段或超前版本拒绝 |
| 包路径 | pack_path 指向含 manifest.json 的目录；相对路径相对于配置文件目录 |
| 默认包 | 开发：配置目录内 `assets/packs/white-dragon/0.1.0/`；产品：EXE 旁 `resources/default/` |
| 加载失败 | 先配置包，后默认包；都失败则首帧前退出；配置损坏时询问是否备份并恢复，拒绝保留原件并退出 |
| 单实例 | 同一配置根/用户会话下限制重复运行，第二次启动尝试打开已有实例设置 |

省略 `initial_scale_percent` 时使用默认值 **100**，可配置范围为 50–200。已保存的 `saved_size.base_dip`（1–4096）和 `saved_size.percent`（50–200）优先。无已保存尺寸时，Windows 根据面板物理尺寸推导基准，探测失败时用 256 DIP，再按初始百分比缩放。

当前偏好默认：auto_move、fullscreen_hide、sound、task_bubbles = true；start_hidden、passthrough = false；bubble_scale_percent = 100（50–150）。任务气泡目前仅有设置项，真实集成未实现。隐藏/穿透需要恢复入口，托盘不可用时限制相关行为。

设置支持预览、应用并保存、撤销未确认值、恢复默认。打开菜单会先关闭设置并撤销未确认预览，菜单偏好修改单独生效。产品保存使用同目录临时文件刷盘再替换；检测外部改写/文件占用。登录自启使用当前用户 Run 项，配置写失败时尝试恢复原注册值。

开发表字段：format = png / rgba8 / bc7，trace_playback、trace_input 为布尔；ffmpeg-decoder feature 另外允许 ffmpeg，且依赖 development。显式 TOML 值优先于 `DESKTOP_PET_FORMAT`、`DESKTOP_PET_TRACE`、`DESKTOP_PET_TRACE_INPUT`。window-diagnostics 可启用诊断日志和几何测试，默认发布格式仍为 PNG。

## 素材清单

顶层字段：schema_version、pack_id、pack_version、color_space、alpha、source_sha256、tools、pages、actions、resources。当前仅 schema 1、sRGB、straight alpha；ID/版本为非空字符串，pack_version 尚未按 SemVer 校验。未知字段拒绝；source_sha256 和 tools 必须非空。

| 对象 | 字段 |
| --- | --- |
| Page | id、size = [width,height]、必需 png、可选 rgba8/bc7 |
| Payload | path、sha256 |
| Action | id、logical_size、looping、anchor、offset、scale、directions、可选 movement_us、frames |
| Frame | page、rect = [x,y,w,h]、duration_us |
| Resource | key、可选 client_id、content |
| content | kind 为 image（page）、audio/text/icon（file 载荷） |

帧持续时间为正微秒，累计不得溢出；矩形须位于引用页内；移动时窗满足 0≤start<end≤动作时长。尺寸为 1–4096；scale 正且有限，anchor/offset 有限；BC7 页宽高为 4 的倍数。解析按 ID 绑定，不依赖数组顺序。

产品必需循环 `idle`。已识别可选语义 ID：interact、drag、land、move-left/right、edge-left/right、peek-left/right。探头缺失先回退同侧吸边再待机；缺互动不强制切动画；自动移动要求一次性动作、方向和合法移动时窗。额外动作可手动预览，移动预览通常原地播放，吸边/探头预览按对应边缘动作处理。

先将逻辑画布等比居中适配视口，再绕归一化 anchor 缩放，offset 按适配后的画布宽高偏移，正方向右/下。呈现与 alpha 命中共用像素中心采样。同侧 edge/peek 循环时间线一致可保留播放相位；共用帧且仅横向偏移不同可在 180 ms 内滑动。分别裁切的旧包仍可读。

通用资源：character.poster、character.name、interaction.click；按 client_id 优先，缺失回退通用项。音频是规范 PCM16 WAV，单/双声道；文本 UTF-8。app.icon / tray.icon 仅允许通用 ICO 资源，缺失/损坏各自回退 EXE 图标；一个 ICO 含 1–8 个边长不同的 16–256 方形 RGBA8 PNG，单平面 32 位、范围不重叠。应用图标覆盖是现状，是否继续是设计选择。

路径以 `/` 分隔，每段仅 ASCII 字母、数字、`._-`，拒绝空段、`.`、`..`；规范化后须为包根内普通文件，读取时再次检查。SHA-256 为 64 位小写十六进制。

| 边界 | 当前限制 |
| --- | --- |
| manifest / 单音频、文本、图标 | 各 ≤1 MiB |
| 静态页展开 RGBA | ≤32 MiB |
| 运行单页 | ≤6 MiB；BC7 同时计入 RGBA 命中数据和压缩块 |
| 运行 PNG 输入 / 解码器工作区 | 各 ≤3 MiB |
| 页请求 | 一个未完成请求，当前页 + 预取页，GPU 单纹理槽 |
| 页等待 | 首帧与后续缺页分别有 750 ms 期限；后续保留有效帧，有界回退 |

PNG 必须静态 RGBA8、尺寸匹配；RGBA 载荷需与 PNG 解码一致；BC7 校验长度/块模式但不是视觉质量验收。可选资源未声明可回退；已声明载荷损坏运行时可独立回退，分发校验仍拒绝。动画页缺失/错误按播放失败处理；结构或引用无效拒绝整个包。

## 校验器与装配器

| 命令模式 | 检查范围 |
| --- | --- |
| validate-pack `<dir>` | 清单和全部声明载荷；不代表满足播放预算 |
| validate-pack --playback `<dir>` | 再检查所有声明编码的运行页预算及启动动作；允许历史单动作 move/reference |
| validate-pack --distribution `<dir>` | 所有载荷与运行预算，强制循环 idle |

校验器不检查应用/素材版本相同，也不检查总 ZIP 大小、许可、真实 GUI 与性能。`assemble-asset-pack.py` 复制唯一声明载荷、保留 manifest 原字节、验证装配快照、生成 ZIP/哈希并拒绝覆盖已有输出；provenance 不自动入运行包。旧“同版发布”属于约定，不是校验器保证。

## 旧数据迁移

独立 `migrate` 命令要求目标用户目录无运行实例；始终写产品目录，原件只读。命令见[使用说明](../user/guide.md)。

config-json 导入历史 schema 1 JSON；bailong-preferences 读取 `HKCU/Software/BailongPet/DesktopPet`；bailong-history 默认读 `%LOCALAPPDATA%/BailongPet/conversations.json`，支持 BAILONG_DATA_DIR 或显式文件。历史目标为 `history/legacy-bailong-codex/<源会话ID的SHA256>.json`，未完成状态记 interrupted，不恢复执行。

备份和 receipt 位于 `backups/migrations/<migration-id>/`。重试核对同源内容，已存在目标报告冲突；回滚只处理哈希仍匹配的结果，保留用户后续修改。配置上限 64 KiB，历史输入 64 MiB，转换后单会话 16 MiB。跨仓库导入仅针对已核验 v0.5.1，不承诺跟随上游变化。
