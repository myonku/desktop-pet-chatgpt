# Rust / wgpu 与无 FFmpeg 运行时评估

日期：2026-11-07。状态：技术方向已定，性能与兼容性待原型验证。本轮不实现播放器或转换素材。

## 1. 确定的技术栈

语言确定为 Rust stable，目标 `x86_64-pc-windows-msvc`；窗口/事件循环使用 winit，渲染使用 wgpu + WGSL，优先 D3D12 后端；Windows 特有能力由 `windows` crate 封装。设置和聊天窗口选择 egui + egui-winit + egui-wgpu，桌宠本体直接绘制纹理四边形，不经过整套控件树。首期不引入 Qt、WebView、Bevy 或 C++ 应用核心。

这里的“原生”是本地编译的桌面窗口与 GPU 渲染，不表示所有控件都是 Win32 系统控件。wgpu 不是 GUI 工具包，也不是视频解码器；winit 提供窗口/输入事件而不提供绘制。egui 的中文输入、选区、剪贴板、滚动、长文本和字体仍须验收，不能因采用它便认定与原 Qt 控件等价。[winit](https://docs.rs/winit/latest/winit/)、[egui](https://docs.rs/egui/latest/egui/)

选择同时满足学习 Rust 与控制常驻负载两个目标；不需要先证明 Rust 胜过 C++ 才允许推进。但必须证明最终程序达到项目性能和功能预算。采用稳定版本的相容依赖组合，在 P0 固定 rust-toolchain.toml、MSRV、Cargo.lock 和启用的 features，不盲目混用各 crate 的最新主版本。

应用代码优先安全 Rust；COM、窗口消息及句柄操作集中在小型平台模块，写明 unsafe 的线程、生命周期和释放约束。开始用标准线程、有界通道和单一窗口事件循环；没有明确需求先不引入完整异步运行时。借此把学习重点限定在 ownership、错误处理、消息传递、模块测试和基础 GPU 管线。

## 2. 透明窗口是第一个技术门槛

Windows 窗口透明标志不足以保证 GPU 内容透明。查阅时 wgpu 的 DX12 文档区分：`DxgiFromHwnd` 不支持透明，`DxgiFromVisual` 使用 DirectComposition visual 并支持透明。原型须确认锁定的 wgpu 版本确有该路径，查询 surface capabilities，选择受支持的 alpha mode，并统一 shader 输出、纹理和合成器的预乘 alpha 约定。[wgpu Dx12SwapchainKind](https://docs.rs/wgpu/latest/wgpu/enum.Dx12SwapchainKind.html)、[CompositeAlphaMode](https://docs.rs/wgpu/latest/wgpu/enum.CompositeAlphaMode.html)

P1a 使用自有透明测试图验证：逐像素半透明、黑/白/彩色背景下边缘、置顶且不抢焦点、拖动/菜单、混合 DPI、显示器断开、锁屏及睡眠恢复。透明区域鼠标穿透与视觉透明是两个问题；命中测试使用离线生成的低分辨率 alpha mask/角色轮廓，避免每帧 GPU 回读。全窗口穿透作为单独用户开关，不未经验证假定 WM_NCHITTEST 能完成所有跨进程穿透场景。

窗口无法正常合成时，先定位 wgpu/驱动/平台适配。限时失败后可研究 Windows 专用呈现适配或 CPU layered-window 兼容模式，保留 Rust 与核心接口；不因此自动回到 Qt。兼容模式须单独报告 CPU 拷贝成本，不作为预先承诺。不能为求“通用封装”在首版同时维护多个完整渲染引擎。

渲染按素材下一帧时间、移动或输入事件唤醒，不持续空转 60/144Hz。隐藏时暂停上传和动画提交，设置/聊天窗口没有变化时休眠。GPU 绘制本身也有上下文、显存和唤醒成本，须测整机能耗趋势和 GPU 活跃时间，不能以 CPU 降低就宣称更省电。

## 3. 当前播放链的可替换位置

现实现：WebM 容器 → FFmpeg 内的 libvpx-vp9 → CPU 缩放/转 RGBA → stdout 管道 → QImage/QPixmap → Qt 绘制。换 Rust 不会使 VP9 运算自动变快；wgpu 能缩放、颜色转换和合成，但不会直接把 WebM 解成图像。

透明 WebM 的颜色与 alpha 需要容器层配合。WebM 的 alpha 设计包含 BlockAdditional 和独立 alpha 编解码数据；官方设计页主要说明 VP8，不能单凭它推断每个 VP9 API 都能返回 alpha。Chromium 的 WebM 写入代码也记录了该附加数据结构。任何替代方案必须对本仓库 VP9 文件验证 alpha、时间戳与帧对应，而不是仅凭“支持 VP9”标签。[WebM alpha 设计](https://wiki.webmproject.org/alpha-channel)、[Chromium WebM muxer](https://chromium.googlesource.com/chromium/src/+/1713e6bc0307c8fe1d0adba2b8c557b66c2d8747/media/muxers/webm_muxer.cc)

根据本地历史调查，现有 35 段共 4,426 帧，多数 720×720，24/30fps，WebM 合计约 146.76 MiB；不是原始无损创作资产。离线转码无法恢复已经丢失的细节，新增有损压缩必须检查累积损伤。

## 4. 替代方案比较

| 方案 | 消除/减少什么 | 代价和风险 | 本项目取舍 |
| --- | --- | --- | --- |
| 保留 FFmpeg，改为有界流水线 | 可减少全缓存、阻塞和无效工作；GPU 承担缩放 | 运行时仍有 FFmpeg/管道，解码本身没有被消除 | 原型参照和后备实验，不是默认发布目标 |
| Rust 容器解析 + libvpx FFI | 去掉 CLI 进程与管道，可把 YUV/alpha 上传后在 shader 转换 | 仍是同类 VP9 解码；必须支持 alpha 附加块、双路同步、颜色矩阵、seek/loop；增加 C 构建和 unsafe 边界 | 仅在必须直接播放 WebM 时评估，不优先自写 demuxer |
| Media Foundation / D3D 视频解码 | 可能使用系统和硬件解码能力 | VP9 支持不等于透明 WebM alpha 端到端支持；设备、扩展、GPU 互操作与 wgpu 导入有额外成本 | 不作为即插即用替代，须独立 alpha 样本实证 |
| 动画 WebP/APNG/PNG 序列 | 小范围解码依赖，透明通道易检查；PNG 可作无损基准 | 仍有 CPU 解压、逐帧上传；PNG/APNG 不自动更快，包体/IO 可能增大；图集全驻留会重现内存问题 | PNG 分帧作为原型正确性基准与兼容资源候选 |
| 离线 BC7 分帧纹理包 | 运行时无视频解码，可上传压缩块并由 GPU 纹理采样处理 | 有损，丢失视频帧间压缩，磁盘体积可能明显增加；需 BC 功能支持与显存预算 | **优先性能实验，拟作为默认运行资源路径** |
| 原始 RGBA 分块包（可逐块 LZ4/Zstd） | 无视频预测，随机访问简单；无新增有损压缩 | 未压缩磁盘与带宽较大；压缩包仍需 CPU 解压、GPU 上传 | BC7 质量不达标时的备选，先测再加 codec |
| 颜色/alpha 分离视频或重新编码 AV1 等 | 可研究硬解和压缩率 | 重做同步、色彩/alpha、兼容性与制作流程；AV1 解码器并非 VP9 替代，必须重编码 | 暂缓，当前规模收益不确定 |

Media Foundation 文档列举了 WebM/VP9 容器映射，但不足以证明当前素材的透明通道能完整通过硬解链；本计划没有认定它必然不支持，只是不把未经验证的路径作为发布依赖。[Microsoft MKV 支持](https://learn.microsoft.com/en-us/windows/win32/medfound/mkv-support)

## 5. 推荐的激进路径：把解码移出用户运行期

**制作工具：** 原 WebM → 明确支持 alpha 的参考解码 → 校验帧与时间 → 离线尺寸档/透明边缘处理 → BC7 分帧/分页编码 + manifest。

**用户程序：** 读 manifest → 异步读取当前/下一组块 → 上传 BC7 纹理 → WGSL 缩放/合成 → DirectComposition 呈现。BC7 是 GPU 纹理压缩，不是视频硬解；没有帧间预测，可直接定位帧。使用驱动支持的 BC 功能后不必先在 CPU 展开整套 RGBA。[Microsoft BC7：RGBA、4×4 块和 16 字节块大小](https://learn.microsoft.com/en-us/windows/win32/direct3d11/bc7-format)、[wgpu BC 格式与功能要求](https://docs.rs/wgpu/latest/wgpu/enum.TextureFormat.html)

目标是**发布程序运行时不带 FFmpeg**。首期素材制作工具可以继续使用 FFmpeg，记录版本、哈希及参数；开发工具依赖与最终用户运行依赖分开。不为“纯 Rust”口号自行实现 VP9/PNG/BC7 编码器。若以后也要移除制作工具中的 FFmpeg，再另行选择可靠解码库。

建议第一版仅定义项目内部、带版本的分帧资源清单和按动作分页文件，避免先开发通用媒体容器。记录 source hash、转换器版本、尺寸、帧数、每帧时间、格式、alpha/色彩约定、页索引及校验和；保留动作 ID、renderScale/renderOffset/renderAnchor 和移动时窗。尺寸按 4×4 块对齐，裁透明边界时保留锚点与 UV，必须避免图集边缘串色。暂不承诺图集、纹理数组或 KTX2 的最终封装，P1b 按显卡 limits 与读取结果选一种。

采用有界的 CPU staging 与 GPU 活跃页池，优先保留当前帧/近期帧及下一动作首帧。CPU、GPU 分别计费并设上限；超预算逐出，动作切换可取消过期任务。初始实验可设 CPU 素材缓冲 32 MiB、GPU 素材池 64 MiB（不含 UI/交换链/驱动），按实测调整。不要把 35 段全塞进显存，也不要让单个动作必须完整装入才能播放。纹理复用/分页与时序逻辑分开验收。

**容量量级（推算，不是转换测量）：** 不计 mipmap、页填充、文件元数据与裁边，BC7 每像素约 1 字节、RGBA8 每像素 4 字节。

| 全部 4,426 帧统一尺寸 | RGBA8 原始载荷 | BC7 块载荷 |
| --- | ---: | ---: |
| 320×320 | 1,728.91 MiB | 432.23 MiB |
| 512×512 | 4,426.00 MiB | 1,106.50 MiB |

这只是尺寸归一化算例，不是全部原素材的实际包体，也不是内存目标。BC7 在同分辨率下约为 RGBA8 的 1/4，仍可能比现有 146.76 MiB 视频包大很多。mipmap、多档分辨率、页填充还会增加容量；文件级再压缩和透明裁边能省多少必须实际测。以包体换 CPU 解码成本是否值得，由 P1b 决定。

320×320@30fps 的 RGBA 纯像素上传约 11.72 MiB/s、BC7 约 2.93 MiB/s；这是忽略 API/行对齐/重复上传/同步的理论值，不能据此直接推算 FPS。核显的共享内存也要记录，不能只看任务管理器的主进程 RAM。

BC7 不支持或画质不过关时，优先验证 PNG/RGBA 分页的有界播放；参考机上若无 FFmpeg 路径均不能达标，则记录阻塞原因，重新选择资源格式。语言和窗口栈不因此重开全部比较。

## 6. 原型与选择门槛

P1a 先用自有透明 PNG/渐变验证 Rust / winit / wgpu 的呈现、输入和睡眠调度；P1b 再比较下列等价资源路径，不把转码与语言收益混为一谈：

1. 相同帧序列的 PNG/RGBA 有界基线；用于像素、时间和 alpha 正确性。
2. 原 WebM + 参考 FFmpeg 的有界输出；记录现状可替代成本，不作为必须内置的正式依赖。
3. 同源同尺寸 BC7 分页；测转换时间、包体、CPU、GPU 时间、上传/IO、显存、首次可交互和动作切换 p95。

以上逐对固定分辨率、帧数、时间、显示窗口和预取预算；旧全缓存版只用于整产品基线，不能声称它与有界 GPU 流水线的差值全部来自 Rust。语言、渲染器、资源格式和缓存各项变化在报告中分别标注；无法隔离时只报告组合方案差异。

先用自有测试图和许可明确样本进行公共 CI；本地现有素材对比在明确来源条件下进行，不上传或发布转换后的未明许可资源。测试至少覆盖 idle、移动、长动作、半透明发丝/轮廓、淡入淡出；候选发布再验证全部 35 段。

视觉检查使用黑/白/棋盘背景、100%/125%/200% DPI；分别比较 RGB 与 alpha，重点看预乘错误、色彩空间/伽马、透明边缘发黑和纹理块伪影。无损 PNG 基准也要先与原 WebM 的参考解码核对，BC7 不以平均误差掩盖边缘可见损伤。

初始门槛：沿用主计划的 ≤2s 首次可交互、p95≤100ms 交互响应、稳定播放丢帧率 <1%；无持续 GPU 忙等和隐藏后解码/上传；页池确实不超冻结预算。BC7 必须无可接受范围外的视觉退化，且相对 FFmpeg 有界基线在 CPU/唤醒或帧延迟至少一项有稳定收益、其他关键指标无未解释回退。若收益不明显而包体明显增加，默认选更简单的 PNG/RGBA 路径或重新评估，不能为了舍弃工具而牺牲实际表现。

P0 先记录参考设备和基线包体，再冻结包体上限；至少测试一台核显/普通笔记本和一台不同 GPU/驱动环境。无法取得第二台时标注未验证，不宣称全 Windows 兼容。

## 7. 本轮明确决定与保留事项

已定：Rust、winit、wgpu/WGSL、Windows API 适配、egui 辅助 UI；Windows x64 首发；运行时移除 Qt/Python；追求无 FFmpeg 运行时；BC7 离线纹理作为优先实验。

留给原型：相容精确版本、DirectComposition 实机表现、BC7 质量和包体、分页封装、兼容素材格式、是否需要系统呈现后备。这些是栈内实现选择，不再维持 C++/Qt 与 Rust 并行候选工程。

移除 Qt/FFmpeg 运行时不会改变现有素材和上游代码的许可状态。新依赖、离线工具及其构建产物仍需登记许可；主计划的作者联系状态及 README 声明安排继续有效。
