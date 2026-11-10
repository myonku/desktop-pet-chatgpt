# R-05 · P1b-01 离线资源包

> 阶段归档。命令与路径保留当时的语境，复测使用[现行工程约定](../../G-05-contracts.md#工程与产物)。

2026-11-09，状态为完成。实现基于 `c110e66`，校验器、样本生成器与本记录随同一提交交付。

## 清单 v1

素材包是本地目录，入口为 UTF-8 `manifest.json`。清单记录 `schema_version`、包标识与版本、源素材 SHA-256、转换工具和参数、色彩空间与 alpha。当前实验使用 sRGB 与 straight alpha。

- `pages` 中每个 ID 标识同一逻辑页。PNG 与原始 RGBA8 必需，BC7 可选。每档有包内相对路径和 SHA-256。RGBA8 按行紧密存放，BC7 是按行存放的原始 4×4 块，每块 16 字节。样本将若干完整帧横排在一页。
- `actions` 中每个 ID 指向逻辑尺寸、循环策略、缩放、锚点、偏移、方向、移动时窗及帧列表。帧通过页 ID、像素矩形 `[x, y, width, height]` 和微秒持续时间引用同源像素。各格式共享同一时间表。
- `resources` 用语义键查找静态图片、PCM16 WAVE 和 UTF-8 文本。可选的 `client_id` 指定覆盖项，缺项查通用键。文件路径只存在于包内清单。两个样本包使用相同键和不同素材及布局。
- 校验器限制清单 1 MiB、页载荷 32 MiB、单边 4096 像素、单个文本或音频 1 MiB。检查版本、重复键、页引用、范围与时间溢出、路径及哈希。PNG 解码结果必须逐字节等于 RGBA8，BC7 检查块数与保留模式，WAVE 检查 PCM16 布局和长度。实验上限与播放器全局预算分别验证。

## 复现

在 Windows PowerShell 从仓库根目录运行。生成工具仅用于开发，输出落在忽略目录 `target/p1b-fixtures`。

```powershell
python -m pip install --target target/p1b-tools ispc_texcomp==1.0.1 texture2ddecoder==1.0.6
$env:PYTHONPATH = (Resolve-Path target/p1b-tools).Path
python scripts/generate-p1b-fixtures.py
cargo run --locked --bin validate-pack -- target/p1b-fixtures/dawn
cargo run --locked --bin validate-pack -- target/p1b-fixtures/dusk
```

生成器沿用 P0 自有渐变和移动轮廓。两包分别采用 4 帧与 5 帧横排，后者调整配色、音调和文案。BC7 采用 ISPC 的 `alpha_basic`，独立 CPU 解码后将 BGRA 与源 RGBA 通道逐一对齐。依据 [ISPC Python 绑定](https://k0lb3.github.io/ispc_texcomp_py/ispc_texcomp.html)和 [Texture2DDecoder](https://github.com/K0lb3/texture2ddecoder)。

## 结果

Windows 本机，Python 3.14.7。表内字节只统计页文件，不含清单和辅助资源。时间为一次热运行的开发机采样。

| 样本包 | 页／动作帧 | PNG 字节 | RGBA8 字节 | BC7 字节 | BC7 编码／CPU 解码 | 总生成时间 |
| --- | --- | ---: | ---: | ---: | --- | --- |
| dawn | 9／30 | 14,956 | 2,031,616 | 507,904 | 806／7 ms | 1.42 s |
| dusk | 7／30 | 12,975 | 2,031,616 | 507,904 | 802／7 ms | 1.43 s |

两次生成的清单 SHA-256 一致。两包均通过校验，PNG 解码与原始 RGBA8 一致。BC7 CPU 解码相对源 RGBA8 的最大单通道绝对误差为 RGB 1、alpha 9（8 位）。这批合成图像的 PNG 压缩率较高，正式素材上的包体、画质及运行收益在后续对照实验中测量。

- dawn 清单为 `566248d3654fe63801b0e04c0c4cde1e385c6e6e05d5de8d3dc54881c2ab77cc`。
- dusk 清单为 `a86347e39d19a2e908ee983631e6cb81389d5e7b78defbe26865dccf0d81ed5f`。
- 13 个坏包用例均以退出码 1 拒绝，覆盖未知版本、重复动作、重复资源、路径穿越与 Windows 目录联接逃逸、越界矩形、零帧时长、超大页、坏哈希、损坏 PNG、哈希有效但像素不一致的 RGBA8、非法 BC7 块及截断 WAVE。
- Windows locked 构建、格式、Clippy 和 3 项 Rust 测试通过，包括客户端回退与非法路径回归。

下一节点 P1b-02 将清单接入播放器，验证有界读取、页池、取消与重启换包。最终媒体默认格式在 P1b-03 决定。
