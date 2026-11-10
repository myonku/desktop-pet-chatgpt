# R-07 · P1b-03 格式对照初测

> 阶段归档。命令与路径保留当时的语境，复测使用[现行工程约定](../../G-05-contracts.md#工程与产物)。

日期：2026-11-10。状态：进行中。保留 PNG 为原型缺省，RGBA8 为无损对照，BC7 为性能候选。当前证据不足以冻结发布格式、分页或完整包体预算。

## 环境与复测

在 `d6bac21` 加本轮跟踪代码上构建 Windows x64 Release（Rust 1.99.1、锁定依赖）。Windows 11 专业版 22631，Core Ultra 7 155H，实际 DX12 适配器为 Intel 核显（32.0.101.8860）；桌面 125% 缩放，客户区 320×320，FIFO。两套自制素材的动作均为 128×128、30 fps、30 帧；dawn 每页最多 4 帧，末页 2 帧；dusk 每页 5 帧。

```powershell
cargo build --release --locked
python -m pip install --target target/p1b-tools ispc_texcomp==1.0.1 texture2ddecoder==1.0.6 psutil==7.1.3
$env:PYTHONPATH = 'target/p1b-tools'
python scripts/generate-p1b-fixtures.py
python scripts/measure-p1b-playback.py --pack target/p1b-fixtures/dawn --output target/p1b-dawn-short --rounds 10 --warmup 0.5 --seconds 2
python scripts/measure-p1b-playback.py --pack target/p1b-fixtures/dusk --output target/p1b-dusk-smoke --rounds 3 --warmup 1 --seconds 5
python scripts/measure-p1b-playback.py --pack target/p1b-fixtures/dawn --output target/p1b-dawn-resources --rounds 1 --warmup 2 --seconds 10 --process-metrics
foreach ($pack in @('dawn','dusk')) {
    python scripts/compare-p1b-media.py --pack "target/p1b-fixtures/$pack" --ffmpeg .legacy-v0.5.1/tools/ffmpeg.exe --output "target/p1b-$pack-media"
}
```

每次使用新的输出目录。工具输出逐轮日志、JSON 和本地对照图；不提交生成文件或旧版二进制。稳定采样可省略时长参数，默认预热 120 秒、采样 600 秒、每档三轮。脚本轮换格式顺序，检查实际播放格式，按呈现序号统计跳帧；跟踪仅在设置 `DESKTOP_PET_TRACE` 后启用。帧序号在单次连续播放中跨循环递增，隐藏或恢复后的独立片段不合并。这里的“呈现”指程序成功提交，未测显示器实际扫描时间；跨页准备包括同步等待和提交，不等于 GPU 实际耗时。

## 播放与资源

dawn 每档 10 次进程启动。启动前未清系统文件缓存；首轮 PNG 超过 2 秒，随后各次明显下降。占位与素材首帧独立记录，启动数值不代替拖拽和菜单可用时间。

| 档位 | 占位首帧中位数（范围），ms | 素材首帧中位数（范围），ms | 帧间隔 p95 各轮范围，ms | 跨页准备 p95 各轮范围，ms | 跳帧 |
| --- | ---: | ---: | ---: | ---: | ---: |
| PNG | 397（322–2098） | 400（328–2101） | 33.72–34.67 | 0.46–0.92 | 0 / 603 |
| RGBA8 | 400（385–442） | 404（388–446） | 33.90–34.57 | 0.52–1.37 | 0 / 603 |
| BC7 | 394（384–415） | 397（387–418） | 33.73–35.51 | 0.47–3.87 | 0 / 603 |

dusk 每档三次 5 秒短测。PNG、RGBA8、BC7 的素材首帧中位数分别为 372、359、382 ms；跳帧分别为 0/451、0/450、0/450。

dawn 另测每档单轮 10 秒资源快照（psutil 7.1.3，约每秒一次）。各档 300/300 帧；同尺寸复用/变尺寸重建准备 p95 分列：

| 档位 | 复用 / 重建，ms | CPU，% 单核 | 采样 private 最大 / 工作集峰值，MiB | 进程读取计数，bytes |
| --- | ---: | ---: | ---: | ---: |
| PNG | 0.579 / 0.664 | 0.47 | 465.2 / 447.4 | 130,560 |
| RGBA8 | 0.582 / 0.706 | 0.16 | 464.1 / 447.0 | 19,660,800 |
| BC7 | 0.502 / 0.728 | 0.31 | 466.4 / 447.8 | 4,915,200 |

进程读取计数可能含缓存命中，不是物理磁盘流量。全进程 private bytes 包含渲染等开销，与 32 MiB CPU 素材缓冲预算口径不同。最大页为 dawn 512×128、dusk 640×128；单张 RGBA8 纹理分别为 256/320 KiB，BC7 为 64/80 KiB。BC7 仍需 CPU 侧 RGBA 数据做透明命中。这里的页体积是理论载荷，不含驱动开销；单轮 CPU 和工作集仅供排查。

## 画质与包体

逐页核对 FFmpeg 解出的 PNG 与原始 RGBA8，结果逐字节相同。对同一动作另用本地 FFmpeg 7.1 的 libvpx-vp9 以 30 fps 编码透明 WebM，再显式用 libvpx-vp9 解成 RGBA；两套均得到 30 帧且保留 alpha。动作 WebM 各 2,841 bytes；离线编码 84/94 ms，解码各约 30 ms（含进程启动）。此项没有限速播放、GPU 上传或输入路径，只是离线参照；VP9 的 YUV420 转换即使设为 lossless 也不保证 RGBA 无损。

逐帧计算黑、白、棋盘底 RGB 合成误差，并抽看棋盘底和白底对照图。对照图按 RGBA8、PNG、BC7、VP9 从左到右排列。两包的 PNG alpha 和合成最大误差均为 0；BC7 alpha 最大误差为 9/255，黑/白/棋盘底 RGB 最大误差为 9/8/7；VP9 分别为 1/255 和 2/3/3。合成图基于 CPU 的 8 位通道混合，只覆盖合成样本，真实素材边缘、颜色空间与多 DPI 的 GPU 呈现需另验。

| 项目（bytes） | dawn | dusk |
| --- | ---: | ---: |
| PNG 素材页 | 14,956 | 12,975 |
| RGBA8 素材页 | 2,031,616 | 2,031,616 |
| BC7 素材页 | 507,904 | 507,904 |
| manifest 与文本、音频 | 15,605 | 14,810 |
| 三档实验包总计 | 2,570,081 | 2,567,305 |

Release 主程序 8,955,904 bytes。两套样本的平涂图像很适合 PNG，不能按比例外推到完整素材。当前实验包校验要求 PNG 与 RGBA8 双档，BC7 可选；发布时能否省去对照档仍属分页与分发决策。上述体积没有纳入全部动作、字体、运行库、许可文件或部署压缩。

## 放行前补测

- 用具备分发依据的代表性动作，按 G-06/G-07 进行 100%/125%/200% DPI 的实机画质检查；同一呈现路径增加 FFmpeg 有界参照，保留独立进程采样。
- 完成每档 2 分钟预热、10 分钟采样、至少 3 轮。复测不同缓存条件下的首次可交互及首轮 PNG 异常，测输入到反馈 p50/p95/p99、隐藏后的停止行为、GPU 活跃和内存、唤醒及 I/O。
- 用代表素材和实际部署内容验证页预算、全进程内存及包体，再冻结默认/兼容格式和分页。P2 产品行为、P3 客户端接入、P4 全动作与干净机验收仍按 G-04 推进。

本轮通过 `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked`（5 项）和 `cargo build --release --locked`。P1b-03 尚未放行。
