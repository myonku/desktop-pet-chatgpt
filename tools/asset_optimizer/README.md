# 离线素材优化

从仓库根运行。初版接收一个动作的一组直通 RGBA PNG 帧。按文件名排序，使用补零序号确保顺序。所有帧尺寸一致。输出目录需是输入目录之外的新目录。结果包含 `frames/` 和 `report.json`，其中记录顺序、输入及输出哈希、优化参数与依赖版本。动作时序和位移仍来自上游素材清单。

```powershell
uv run --locked --group optimize python -m tools.asset_optimizer --source assets/work/input/idle --output assets/work/idle-alpha
```

默认采用保守的 alpha 清理，参考 fork 公开的颜色、连通区域和主体距离方法。加 `--protect X,Y,W,H` 可为当前动作保护一个像素矩形，可重复指定。加 `--denoise` 对不透明区域做轻度 RGB 滤波，`--keep-alpha` 跳过 alpha 清理。比较不同处理时，每次使用新的输出目录。 [Fork 的 WebM 去噪](https://github.com/xiamoqingliang/bailong-codex-pet/blob/main/codex-integration/edge-repair/noise_webm.py)

可选超分入口使用本机的 Real-ESRGAN NCNN Vulkan 可执行文件和 `realesr-animevideov3-x2` 权重。提供 `--gpu-exe <exe>` 与 `--models <目录>`。可用 `--gpu-id` 指定 Real-ESRGAN 列出的设备索引，显存紧张时设置 `--tile 128`。RGB 进入模型前将可见颜色延伸到透明区域，alpha 单独缩放并合成回输出。程序与权重哈希记录在报告中。 [Real-ESRGAN 模型和命令](https://github.com/xinntao/Real-ESRGAN/blob/master/docs/anime_video_model.md)

## WebM 装配与对照

这是现有全量工具能力。新主线要求先确定默认内容表，再选择动作试制；不要因工具默认收录全部动作就把全部动作作为默认交付。[素材设计](../../docs/design/assets.md)

`python -m tools.asset_optimizer.pack` 读取本地白龙素材清单和透明 WebM，保留原始动作时序、锚点、缩放、方向与移动窗口。默认收录全部动作，保留本仓库的行为绑定，并从 `idle` 派生四个吸边和探头动作。离线对完整待机帧做左右 45° 旋转并按探头宽度裁切。同侧两个姿态共享透明 PNG 页，吸边姿态通过画布偏移缩小露出范围，播放器可在原动画时钟上连续切换。指定 `--action` 时只装配列出的动作及 `idle`，用于同尺寸对照。

```powershell
cargo build --locked --release --bin validate-pack
uv run --locked --group optimize python -m tools.asset_optimizer.pack `
  --source 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' `
  --ffmpeg assets/work/tools/ffmpeg.exe --validator target/release/validate-pack.exe `
  --upstream-release https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1 `
  --pack-version 0.1.0 --size 320 --batch 4 --profile despeckle `
  --click assets/source/white-dragon/sounds/click.wav `
  --app-icon desktop-icon.png --tray-icon tray-icon.png `
  --output assets/work/white-dragon-despeckle-320/pack
```

`--app-icon` 与 `--tray-icon` 均可省略。工具把方形静态 PNG 缩放为多尺寸 RGBA8 ICO，保留输入的透明度和底色，并在来源记录中登记哈希与尺寸。

| 配方 | 处理 |
| --- | --- |
| `resize` | 双三次缩放，作为对照 |
| `alpha` | 缩放和保守 alpha 清理 |
| `despeckle` | 在 `alpha` 基础上，结合角色轮廓和相邻帧清理外围弱残留与单帧小连通块，覆盖高 alpha 噪点 |
| `contour` | 在 `despeckle` 基础上，清理可信轮廓之外的黄褐色及低亮度残留，覆盖贴边、跨帧及完全不透明的噪点 |
| `denoise` | 在 `alpha` 基础上对不透明区域做轻度 RGB 滤波 |
| `gpu` | 在 `alpha` 基础上做 RGB 2× 超分和 Lanczos 缩回，使用清理后的原尺寸 alpha |

GPU 配方需要 `--gpu-exe` 与 `--models`。`--protect ACTION:X,Y,W,H` 可为指定动作保留缩放后的像素区域。各配方在 PNG 入包前清零完全透明像素的 RGB，以减少无效载荷。产物经 `validate-pack --distribution` 校验后写入新目录。`provenance.json` 绑定成品清单哈希并记录逐帧输入和输出 RGBA 哈希、清理像素计数、处理工具、权重及派生参数。使用示例和实测见 [审查摘要](../../docs/plan/baseline.md)。

使用 `--profile contour` 时，先用高 alpha 且颜色不符合残留特征的像素构造可信轮廓，再从画布边界识别外部区域。轮廓旁的一圈抗锯齿和被轮廓包围的区域有保护，`--protect` 也适用于此步骤。真实的暗色或黄褐色外部特效可能与残留同色，试播时应比较这类特效和细线。报告的 `contour_alpha_pixels_cleared` 单独计数。

运行 `uv run --locked --group optimize python scripts/validation/check-asset-optimizer.py` 可复验 CPU 路径。追加 `--gpu-exe` 和 `--models` 可复验 2× 输出、alpha 缩放和 GPU 失败隔离。
