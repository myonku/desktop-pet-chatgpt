# R-08 · P1b-03 代表素材与同路径对照

日期为 2026-11-10。状态为进行中。基线为 `13fad82`，本轮增加仅供开发的 FFmpeg 参照、PNG-only 清单支持和采样工具。继续使用 PNG 缺省，RGBA8 与 BC7 可选。当前 BC7 编码未通过透明边缘和命中一致性检查。

## 复现环境

Windows 11 专业版 22631、Core Ultra 7 155H、Intel 核显 32.0.101.8860、DX12 DirectComposition、125% DPI、320×320 客户区、FIFO。Rust 1.99.1 锁定 Release、Python 3.14.7、FFmpeg 7.1 essentials、`ispc_texcomp 1.0.1 alpha_basic`、`texture2ddecoder 1.0.6`、`psutil 7.1.3`。实测日志记录实际适配器和窗口大小。单独构建 `p1b-reference` 才能启动 FFmpeg，普通 Release 不需要该工具。

三段历史 WebM 仅作本地验证。用 libvpx-vp9 解码透明通道，双三次缩放至 320×320，保持 24 fps。页内整帧横排，待机与落地每页最多 4 帧，移动每页最多 5 帧且末页 4 帧。三档共用时间表和同一参考解码结果。移动素材 FFmpeg 参照在有界 worker 上按需读管道，一次只组装请求页并另用一帧缓冲，通过相同的纹理上传、命中、时间调度和呈现路径。

| 本地输入 | 帧数 | WebM SHA-256 | 清单 SHA-256 |
| --- | ---: | --- | --- |
| `idle.webm` | 96 | `d8dd3130631c0572caa5b6c4cc6e7b89f373d893d59eb53f1a214a804d2d7a76` | `465fb11a18547dea66642999a61993621905f197f793e1f6487661db652b1ae3` |
| `move02.webm` | 264 | `e6de4681409bfe8c2d8cd277206d05c492ce5854f075fd2b6d1724d7dd0d9d0d` | `6ba24f96afbaf8798c20085b8d3dcf88c4a5f4ce222269228e03cd72060b1032` |
| `landing.webm` | 48 | `5824eb23fffcae7b3834fa2f589b0e18525b33d73e918bcbc7b50777accc05ac` | `a4b56ee938bb9591b70dc31153f877bdbcdbd958505bca533c7afae0f398ecf1` |

```powershell
$env:PYTHONPATH = (Resolve-Path target/p1b-tools).Path
$ffmpeg = (Resolve-Path .legacy-v0.5.1/tools/ffmpeg.exe).Path
$videos = (Get-ChildItem .legacy-v0.5.1/assets -Recurse -Filter idle.webm).Directory.FullName
$video = Join-Path $videos move02.webm
python scripts/import-p1b-reference.py --ffmpeg $ffmpeg --source $video --fps 24 --batch 5 --output target/p1b-representative/move02-batch5
cargo build --release --locked --features p1b-reference
target/release/validate-pack.exe target/p1b-representative/move02-batch5
python scripts/compare-p1b-media.py --imported --pack target/p1b-representative/move02-batch5 --output target/p1b-quality-frames/move02-batch5
python scripts/measure-p1b-playback.py --pack target/p1b-representative/move02-batch5 --output target/p1b-move02-long --formats png ffmpeg rgba8 bc7 --ffmpeg $ffmpeg --video $video --process-metrics --gpu-metrics
```

首次冒烟可设置 `DESKTOP_PET_REFERENCE_VERIFY=1`，逐页将管道输出的 RGBA8 与素材包页哈希比较。长测前移除此环境变量，避免参照路径额外做逐页哈希。运行时记录在忽略目录 `target/`，使用新的输出目录复测。

## 画质、时间与交互

三个素材包均通过离线校验，PNG 与参考 RGBA8 逐字节一致。待机与移动的 FFmpeg 参照分别连续播放 12 秒和 24 秒，跨多个循环时逐页哈希一致。30 fps 的两套自有样本结果见 [R-07](R-07-p1b-format-comparison.md)。

独立 CPU 解码 BC7，逐页检查 alpha，抽看各动作首、中、末帧的黑、白、棋盘及蓝底合成。移动素材的白底合成平均通道误差约 1.12/255，局部最大值达 88/255。新增非零 alpha 可把透明像素纳入点击区域。三段素材均存在此类变化，当前 BC7 档保留作实验。

| 素材 | 最大 alpha 绝对误差 | 透明变非零 | 非零变透明 | PNG 载荷 | BC7 载荷 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 待机 | 106/255 | 363,045 / 9,830,400 | 33,152 | 14,838,066 B | 9,830,400 B |
| 移动 | 166/255 | 1,751,235 / 27,033,600 | 63,202 | 43,432,298 B | 27,033,600 B |
| 落地 | 129/255 | 139,259 / 4,915,200 | 15,958 | 7,737,863 B | 4,915,200 B |

125% DPI 下实看待机 PNG，轮廓及半透明区域可见桌面背景。100% DPI 第二屏同样看到正确的透明合成，测试客户区 205×205，蓝、白、灰底可透出。两块 1920×1080 屏幕的 Windows 标准缩放选项均止于 175%，实际 200% DPI 留到 P4 的设备矩阵。用参照构建设置 `DESKTOP_PET_REFERENCE_CLIENT=512,480,165`，在 125% DPI 下对 512×512 物理客户区实看黑、白、蓝及棋盘底透明边缘，覆盖 256 DIP 在 200% 下的等效呈现尺寸。日志为 `target/p1b-visual-client512.log`。

播放中拖拽和右键菜单可用。点「隐藏桌宠」后窗口消失，`target/p1b-interaction.log` 在隐藏标记后无帧记录。320×320 与 512×512 客户区合计八次窗口位置更新用 `GetMessageTime` 至 `DwmFlush` 返回取样，p50/p95/p99 均为 16 ms，范围 15–16 ms。此小样本测的是 Windows 消息到合成器刷新，计时粒度约 10–16 ms。显示器扫描延迟未测。 [Windows Time](https://learn.microsoft.com/windows/win32/sysinfo/windows-time) · [DwmFlush](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmflush)

## 移动场景短测与长测

`target/p1b-move02-comparison-smoke/rounds.jsonl` 记录四档各预热 2 秒、采样 24 秒。全部呈现 576/576 帧。启动栏为素材首帧提交，未把占位首帧混入。跨页准备计时包括纹理复用/重建、同步等待和上传提交。跨页重建与循环回首均有采样。

| 档位 | 素材首帧 | 帧间隔 p95 | 跨页准备 p95 | CPU，% 单核 | 3D 引擎平均 |
| --- | ---: | ---: | ---: | ---: | ---: |
| PNG | 332 ms | 44.57 ms | 1.10 ms | 0.20 | 2.44% |
| RGBA8 | 381 ms | 45.14 ms | 1.12 ms | 1.63 | 2.46% |
| BC7 | 412 ms | 50.67 ms | 0.88 ms | 8.98 | 2.62% |
| FFmpeg 参照 | 523 ms | 45.01 ms | 0.91 ms | 主进程 0.65，解码 1.04 | 2.54% |

主进程采样 private 峰值约 469–470 MiB，工作集峰值约 451–452 MiB，WDDM 共享显存峰值约 396 MiB。参照解码进程 private 峰值约 60 MiB。WPR 会话因权限返回 `0xc5585011`，改用 PDH 按 PID 采集 GPU 引擎和显存。进程读取计数含文件缓存及管道，进程上下文切换数仅作调度频度代理。上表是短测，CPU、GPU 与准备时间不能据此冻结预算或声明稳定收益。 [PDH 格式化计数器数组](https://learn.microsoft.com/en-us/windows/win32/api/pdh/nf-pdh-pdhgetformattedcounterarrayw)

`target/p1b-move02-long` 为每档预热 120 秒、采样 600 秒、3 轮的串行对照。Release SHA-256 为 `d4d28b7a475e5ad387a99355c662db08ba28e3a438cfa5b6d87b7d4c57900b36`。每轮完成后在 `rounds.jsonl` 落盘，所有轮次完成后生成 `result.json`。PNG 首轮呈现 14,401/14,401 帧，帧间隔 p95 46.15 ms，跨页准备 p95 1.19 ms，单核 CPU 2.22%，3D 引擎平均 2.19%。采样 private 峰值 463.6 MiB，工作集峰值 454.9 MiB，共享显存峰值 396.2 MiB。FFmpeg 首轮呈现 14,400/14,400 帧，帧间隔 p95 46.36 ms，主进程和解码进程的单核 CPU 分别为 2.13% 和 1.84%。

## 包体与回归

本地组装包包含普通 Release 主程序、三段共 408 帧的 PNG-only 资源、样本静态图、音效、文案及 x64 `VCRUNTIME140.dll`。97 个文件，解包 75,193,342 B（71.71 MiB），ZIP 74,409,438 B（70.96 MiB）。主程序为 8,954,368 B，CRT 为 178,616 B。字体取自本机 Windows 的 `msyh.ttc`，分发统计只计实际入包文件。ZIP 仅为本地验证装配，原图及转换产物均留在忽略目录。维护者确认 P1 按该本地原型装配验收，正式分发清单留到 P4。按实测加约 8.3 MiB 余量，冻结本地原型解包和 ZIP 各 ≤80 MiB 的预算。

解压 ZIP 到新目录，以仅含 Windows System32 的 `PATH` 启动 10 次。素材首帧中位数 404 ms，范围 345–746 ms。占位首帧中位数 390 ms，范围 335–725 ms。未再出现 R-07 的 2.1 秒现象。复测中未清系统文件缓存，R-07 的异常时间主要落在占位首帧以前。请求 BC7 时实机记录 PNG 回退并继续播放。运行时加载包内 CRT 和 Windows 的 D3D12 组件，进程树中观察到 Windows 控制台宿主。证据位于 `target/p1b-packaged-startup` 和 `target/p1b-packaged-fallback.log`。

同一机器设置 `WGPU_POWER_PREF=high` 后，实际选择 NVIDIA RTX 500 Ada Generation Laptop GPU（32.0.15.7344）。解包应用在该适配器上呈现 192/192 帧，素材首帧 530 ms，帧间隔 p95 45.02 ms，采样 private 峰值 488.7 MiB。证据位于 `target/p1b-high-power-gpu`，为同机第二 GPU 短测。

打开中文预览并保留动画后，单次采样 private 967.6 MiB、工作集 653.1 MiB，WDDM 共享与专用分别为 507.0 和 314.0 MiB，见 `target/p1b-preview-resource.json`。预览使用独立渲染设备。P1 本地原型 private bytes 预算为单独播放 ≤550 MiB、同时打开预览 ≤1,100 MiB，分别以目前最高 488.7 MiB 与 967.6 MiB 加约 12.5% 和 13.7% 余量。预览数据为单点，P4 按完整 UI 和设备矩阵重新测量。

`validate-pack` 已通过 PNG-only 装配与三个三档素材包。`cargo fmt --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`cargo test --locked`、`cargo test --locked --all-features` 和两种 Release 构建均通过，默认和参照测试各 6 项。新增回归验证 PNG-only 包在请求 RGBA8 或 BC7 时仍返回与缺省一致的像素。原型现有 CPU、GPU 素材池限制继续生效。320×320 的 5 帧 PNG 页展开占 1.95 MiB，最多两个展开页和一个 PNG 读取/解码工作区。活动 GPU 纹理为单槽，纹理本身为 1.95 MiB，驱动与交换链独立计量。

## 放行差项

待完成剩余长测统计和分页决策。系统冷缓存启动、实际 200% DPI、更多交互样本及正式分发清单分别记录验证范围。P1b-03 保持进行中。
