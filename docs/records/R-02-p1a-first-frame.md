# R-02 · P1a-01 透明首帧

日期 2026-11-07。P0 基线为 `53d0ba6`，验收前置见 [R-01](R-01-p0-baseline.md)。本报告随 P1a-01 实现提交。

## 实现与呈现约定

- winit 创建 256 逻辑像素的无边框置顶窗口，使用 `WS_EX_NOREDIRECTIONBITMAP`。wgpu 27.0.1 选择 DX12，设置 `Dx12BackendOptions::presentation_system = DxgiFromVisual`，由 wgpu 管理 DirectComposition visual 与交换链。
- 内嵌 P0 自有 `alpha-color.png`，可用 `python scripts/generate-p0-fixtures.py` 重建。嵌入文件 SHA-256 为 `d5478d2b6c91686110fb16601a59645255bc5993905ab4ea2c2e2008400a1ec5`。由锁定的 png 0.18.1 解码，pollster 0.4.0 等待设备初始化。
- 输入是 sRGB 编码的 straight RGBA8。上传到 `Rgba8Unorm` 后按最近像素取样，WGSL 在编码值上将 RGB 乘以 alpha。交换链显式要求 `Bgra8Unorm` 和 `PreMultiplied`，以透明值清屏且关闭渲染混合。该约定用于本次 SDR Windows 合成实验。参见 [DXGI alpha 模式](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ne-dxgi1_2-dxgi_alpha_mode)。
- 首次绘制、窗口重绘和尺寸/DPI 变更触发提交，零尺寸跳过。窗口关闭和初始化或绘制错误退出事件循环，错误写入 stderr。

## 实机验收

Windows 11 家庭中文版 22631，RTX 4060 Laptop GPU，驱动 32.0.15.9186，DPI 175%，客户区 448×448 物理像素。

| 查询项 | 实测 |
| --- | --- |
| Backend | Dx12 |
| 支持的 surface 格式 | Bgra8UnormSrgb、Rgba8UnormSrgb、Bgra8Unorm、Rgba8Unorm、Rgb10a2Unorm、Rgba16Float |
| 支持的 alpha mode | Auto、Inherit、Opaque、PostMultiplied、PreMultiplied |
| 支持的 present mode | Mailbox、Fifo、Immediate |
| 实际采用 | DxgiFromVisual、Bgra8Unorm、PreMultiplied、Fifo |

1. 在参考机运行 `cargo run --locked`，用纯色窗口覆盖原型下方区域，依次切换黑色、白色和蓝色（`#23589C`）。
2. 通过 Windows Computer Use 查看实际窗口的三张合成截图。每张图上部四条色带左端显示底色，向右平滑过渡至不透明色。下半部灰阶遮住底色，边缘无可见黑框或错色。选中原型后按 Alt+F4，确认窗口退出。
3. 本地通过 `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo build --locked`、`cargo build --release --locked` 和 `cargo test --locked`（0 项 Rust 测试）。

初次 GDI 区域采样只取得背景，故以实机窗口截图做视觉判定，未取得逐像素误差数据。P1a-01 的透明首帧验收通过。窗口输入与混合 DPI 在 P1a-02 验证，休眠、设备恢复及 egui 中文输入在 P1a-03 验证。另一 GPU/驱动环境仍待覆盖。
