# 离线素材优化

从仓库根运行。初版接收一个动作的一组直通 RGBA PNG 帧。按文件名排序，使用补零序号确保顺序。所有帧尺寸一致。输出目录需是输入目录之外的新目录。结果包含 `frames/` 和 `report.json`，其中记录顺序、输入及输出哈希、优化参数与依赖版本。动作时序和位移仍来自上游素材清单。

```powershell
uv run --locked --group optimize python -m tools.asset_optimizer --source assets/work/input/idle --output assets/work/idle-alpha
```

默认采用保守的 alpha 清理，参考 fork 公开的颜色、连通区域和主体距离方法。加 `--protect X,Y,W,H` 可为当前动作保护一个像素矩形，可重复指定。加 `--denoise` 对不透明区域做轻度 RGB 滤波，`--keep-alpha` 跳过 alpha 清理。比较不同处理时，每次使用新的输出目录。 [Fork 的 WebM 去噪](https://github.com/xiamoqingliang/bailong-codex-pet/blob/main/codex-integration/edge-repair/noise_webm.py)

可选超分入口使用本机的 Real-ESRGAN NCNN Vulkan 可执行文件和 `realesr-animevideov3-x2` 权重。提供 `--gpu-exe <exe>` 与 `--models <目录>`。可用 `--gpu-id` 指定 Real-ESRGAN 列出的设备索引，显存紧张时设置 `--tile 128`。RGB 进入模型前将可见颜色延伸到透明区域，alpha 单独缩放并合成回输出。程序与权重哈希记录在报告中。 [Real-ESRGAN 模型和命令](https://github.com/xinntao/Real-ESRGAN/blob/master/docs/anime_video_model.md)

## WebM 装配与对照

这是现有全量工具能力。新主线要求先确定默认内容表，再选择动作试制；不要因工具默认收录全部动作就把全部动作作为默认交付。[素材设计](../../docs/design/assets.md)

`python -m tools.asset_optimizer.pack` 读取本地白龙素材清单和透明 WebM，保留原始动作时序、锚点、缩放、方向与移动窗口。默认收录全部动作，保留本仓库的行为绑定，并从 `idle` 派生四个吸边和探头动作。离线对完整待机帧做左右 45° 旋转并按探头宽度裁切。同侧两个姿态共享透明 PNG 页，吸边姿态通过画布偏移缩小露出范围，播放器可在原动画时钟上连续切换。使用 `--default-actions` 生成六个基础 clip 和四个派生姿态。指定 `--action` 时只装配列出的源动作和 `idle`，用于同尺寸对照。

工具直接生成 `engine_api = 2` 的 v2 PNG 包，为普通桌宠动作写入语义组和默认变体，为已知动作写入预览短名，再用 `validate-pack --distribution` 校验产物。`validate-pack --behaviors` 可列出支持的语义。 [协议与示例](../../docs/development/runtime-v2.md#引擎行为语义) · [白龙映射](white_dragon_bindings.py) · [制备与装配命令](../../docs/development/workflow.md#检查与回归)

```powershell
cargo build --locked --release --bin validate-pack
uv run --locked --group optimize python -m tools.asset_optimizer.pack `
  --source 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' `
  --ffmpeg assets/work/tools/ffmpeg.exe --validator target/release/validate-pack.exe `
  --upstream-release https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1 `
  --pack-version 0.1.0 --size 320 --batch 4 --profile despeckle `
  --click assets/source/white-dragon/sounds/click.wav `
  --app-icon assets/icon/desktop.png --tray-icon assets/icon/tray.png `
  --output assets/work/white-dragon-despeckle-320/pack
```

`--app-icon` 与 `--tray-icon` 均可省略。工具把方形静态 PNG 缩放为多尺寸 RGBA8 ICO，保留输入的透明度和底色，并在来源记录中登记哈希与尺寸。白龙包使用 `assets/icon/` 下带透明边角的两张源图。

| 配方 | 处理 |
| --- | --- |
| `resize` | 双三次缩放，作为对照 |
| `alpha` | 缩放和保守 alpha 清理 |
| `despeckle` | 在 `alpha` 基础上，结合角色轮廓和相邻帧清理外围弱残留与单帧小连通块，覆盖高 alpha 噪点 |
| `contour` | 在 `despeckle` 基础上，清理可信轮廓之外的黄褐色及低亮度残留，覆盖贴边、跨帧及完全不透明的噪点 |
| `denoise` | 在 `alpha` 基础上对不透明区域做轻度 RGB 滤波 |
| `gpu` | 在 `alpha` 基础上做 RGB 2× 超分和 Lanczos 缩回，使用清理后的原尺寸 alpha |

GPU 配方需要 `--gpu-exe` 与 `--models`。`--protect ACTION:X,Y,W,H` 可为指定动作保留缩放后的像素区域。各配方在 PNG 入包前清零完全透明像素的 RGB，以减少无效载荷。产物经 `validate-pack --distribution` 校验后写入新目录。`provenance.json` 绑定成品清单哈希并记录逐帧输入和输出 RGBA 哈希、清理像素计数、处理工具、权重及派生参数。 [Windows 画质与呈现实测](../../docs/plan/validation.md#m1-高-dpi-与设置成本验证)

使用 `--profile contour` 时，先用高 alpha 且颜色不符合残留特征的像素构造可信轮廓，再从画布边界识别外部区域。轮廓旁的一圈抗锯齿和被轮廓包围的区域有保护，`--protect` 也适用于此步骤。真实的暗色或黄褐色外部特效可能与残留同色，试播时应比较这类特效和细线。报告的 `contour_alpha_pixels_cleared` 单独计数。

运行 `uv run --locked --group optimize python scripts/validation/check-asset-optimizer.py` 可复验 CPU 路径。追加 `--gpu-exe` 和 `--models` 可复验 2× 输出、alpha 缩放和 GPU 失败隔离。

## 2026-11-18 白龙十动作画质实验

输入为上游 v0.5.1 的透明 WebM 与素材清单，保留六个源动作和四个由待机派生的吸边、探头姿态。全部产物为 v2 直通 alpha PNG。本机工作目录为 `assets/work/white-dragon-720-gpu-2026-11-18/`。下表中的体积只计清单声明的 PNG 页。

| 本地包目录 | 画布 | PNG 页 | PNG 载荷 | 制备顺序 |
| --- | --- | --- | --- | --- |
| `01-source` | 720 × 720 | 506 | 430.2 MiB | 用 `--profile resize --size 720 --batch 2 --default-actions` 解码原片，作为画质基线。 |
| `02-contour-sr` | 720 × 720 | 506 | 487.7 MiB | 先做 `clean_alpha`、相邻帧噪点抑制和 contour 杂边清理。将最近可见 RGB 延拓进透明区，送入 Real-ESRGAN NCNN Vulkan `realesr-animevideov3-x2`，从 1440 Lanczos 缩回 720，再合回清理后的 alpha。 |
| `03-contour-sr-cuda` | 720 × 720 | 506 | 463.7 MiB | 复用 02 的超分输出和 alpha。RGB 转 YUV444，在 CUDA 执行 `bilateral_cuda=window_size=5:sigmaS=1.4:sigmaR=12` 并转回 RGB。只在全不透明掩膜经 5 × 5 腐蚀后的区域以 `115/255` 混合去噪结果。 |
| `04-outer-fringe` | 720 × 720 | 506 | 464.4 MiB | 基于 03 衰减深色描线外的浅色锯齿和杂边。六个源动作保留可见 RGB，吸边及探头按 03 的变换重烘。 |
| `04-outer-fringe-320` | 320 × 320 | 254 | 117.9 MiB | 以 04 为母版，逐帧先做浮点预乘 alpha，再以 Lanczos 同步缩放 RGB 和 alpha。恢复直通 alpha，清零完全透明 RGB，按每页最多四帧重新装配。 |
| `04-outer-fringe-480` | 480 × 480 | 254 | 237.7 MiB | 同 320 档，由同一 720 母版直接缩到 480，保留相同的动作时序和归一化几何。 |

实测 GPU 为 NVIDIA RTX 500 Ada Generation Laptop GPU，Real-ESRGAN 使用 20220424 便携版，Vulkan 设备索引为 1，单块大小 720，流水线线程 `2:1:2`，FFmpeg 9.0.2 的 CUDA 设备索引为 0。换机重制时按实际设备编号选择 GPU。源档、模型和工具哈希记录在各包的 `provenance.json`。

### 04 外沿处理参数

1. 取 `alpha >= 112` 的最大八连通区域为主体，八连通面积至少 32 px 的 `alpha <= 12` 区域为外侧参照。亮度取 `Y = 0.213R + 0.715G + 0.072B`。描线种子位于主体内，同时满足 `alpha >= 154`、`Y <= 103`，并满足 `B - R >= 16` 或 `Y <= 39`。
2. 在 `alpha > 12` 区域中保护与主体分离的八连通区，同时保护主体以外所有 `alpha >= 112` 的像素。对这两类保护区应用 7 × 7 椭圆膨胀。对 `Y >= 138`、`alpha >= 160` 的非描线像素，以 9 × 9 椭圆腐蚀找浅色实体核心，再在 `alpha >= 36` 的非描线区域内进行七轮 3 × 3 膨胀，保留亮色实体和相连特效。
3. 只处理同时距透明参照和描线不超过 7 px 的非保护像素。用 OpenCV `DIST_L2` 5 × 5 距离变换寻找最近种子。两者方向的反向余弦 `c` 须大于 0.30，且描线种子比候选更深入主体，离透明区域的距离差 `Δd` 须大于 0.45 px。记亮度高于描线的差值为 `ΔY`，距描线和透明参照的距离为 `di` 和 `do`。
4. 将下列六项分别限在 0–1 后相乘作为衰减权重，再将原 alpha 乘以一减权重并取整。中性色（RGB 极差 ≤ 24）或黄褐色（`min(R,G) - B >= 5`）用 `t = 12`，其余用 `t = 17`。

   ```text
   (ΔY - t) / 27
   (c - 0.30) / 0.47
   (Δd - 0.45) / 0.77
   (8 - do) / 2
   (8 - di) / 2
   (di + 0.7) / 2
   ```

实验脚本 `optimize_720.py`、`fringe_filter.py`、`build_fringe.py` 和 `build_standard_sizes.py` 随本机工作目录保留。现有 `--profile gpu` 只提供 alpha 清理和超分入口，表中的完整配方由上述实验脚本串联。

### 校验与异机复测

六套均通过 `validate-pack --distribution`，各有十个动作、1,201 个逻辑帧。逐帧核对了 04 的源动作可见 RGB 与独立于主体的前景像素。320 和 480 档核对了各动作的首帧、跨页和末帧。以下为三份 04 成品的 `manifest.json` SHA-256，便于换机核对副本。

```text
a6f2557010bcd8aff2263e87bcedd360c7f7f8a818f908ad19205d627f0d9e10  04-outer-fringe
a72f4b6055b51676423354a8bd8dab06c7b8782f94ed9c0684f63b81d8233632  04-outer-fringe-320
0a37ee41a1cbff33fe34bd9f1cc4dc999aa66c655754b9e64598fe0d77d2843c  04-outer-fringe-480
```

维护者观察到各档在对应素材分辨率下线条较清晰，缩小显示后仍有失真，观感像过度锐化。当前观察设备为 1080p，分辨率的影响尚待异机复测。当前渲染在缩小时按目标像素中心选择单个源纹素，使用 `textureLoad` 直接取样。素材超分后的线条特征、运行时缩小及显示设置都应纳入对照。 [当前渲染着色器](../../src/render/first_frame.wgsl)

换机时复制工作目录里的三个 04 包，保持这些目录下的 `manifest.json` 与载荷完整。设置 `desktop-pet.dev.toml` 顶层 `pack_path = "assets/work/white-dragon-720-gpu-2026-11-18/04-outer-fringe-320"`，以包目录名切换另外两档。用同一动作和帧在两台设备分别比较接近 1∶1 的显示和日常缩小显示，记录屏幕分辨率、Windows DPI 和角色窗口的实际物理像素尺寸。固定物理像素维度后，再对照相同尺寸的离线缩小图，分辨素材与运行时采样的影响。根 README 的展示更新暂缓，等待复测。
