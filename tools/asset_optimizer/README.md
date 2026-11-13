# 离线素材优化

从仓库根运行。初版接收一个动作的一组直通 RGBA PNG 帧。按文件名排序，使用补零序号确保顺序。所有帧尺寸一致。输出目录需是输入目录之外的新目录。结果包含 `frames/` 和 `report.json`，其中记录顺序、输入及输出哈希、优化参数与依赖版本。动作时序和位移仍来自上游素材清单。

```powershell
uv run --locked --group optimize python -m tools.asset_optimizer --source assets/work/input/idle --output assets/work/idle-alpha
```

默认采用保守的 alpha 清理，参考 fork 公开的颜色、连通区域和主体距离方法。加 `--protect X,Y,W,H` 可为当前动作保护一个像素矩形，可重复指定。加 `--denoise` 对不透明区域做轻度 RGB 滤波，`--keep-alpha` 跳过 alpha 清理。比较不同处理时，每次使用新的输出目录。 [Fork 的 WebM 去噪](https://github.com/xiamoqingliang/bailong-codex-pet/blob/main/codex-integration/edge-repair/noise_webm.py)

可选超分入口使用本机的 Real-ESRGAN NCNN Vulkan 可执行文件和 `realesr-animevideov3-x2` 权重。提供 `--gpu-exe <exe>` 与 `--models <目录>`。可用 `--gpu-id` 指定 Real-ESRGAN 列出的设备索引，显存紧张时设置 `--tile 128`。RGB 进入模型前将可见颜色延伸到透明区域，alpha 单独缩放并合成回输出。程序与权重哈希记录在报告中。 [Real-ESRGAN 模型和命令](https://github.com/xinntao/Real-ESRGAN/blob/master/docs/anime_video_model.md)

初版输出供逐帧和播放对照。WebM 提取、候选接入制包和姿态派生按 [临时计划](../../docs/stages/p2-p3/README.md#后续素材实验) 推进。

运行 `uv run --locked --group optimize python scripts/validation/check-asset-optimizer.py` 可复验 CPU 路径。追加 `--gpu-exe` 和 `--models` 可复验 2× 输出、alpha 缩放和 GPU 失败隔离。
