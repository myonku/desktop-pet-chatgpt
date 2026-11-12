# P2-01 · 核心行为

2026-11-12 完成本地实施与验收。实现提交为 `9f578ce`，基于 `main` 的启动配置提交 `0fba128`。

## 行为和缺省值

| 范围 | 本次实现 |
| --- | --- |
| F-02 播放 | 按稳定 ID 选择动作，预览播一次后返回当前基础姿态。首帧和后续缺页各等至多 750 ms，等待时冻结播放时钟并保留上一帧。读取失败回退到当前姿态或待机，必备待机失败显示占位。取消后按页源及代次丢弃旧结果 |
| F-03 手势 | 单击等待 250 ms，系统双击矩形内的双击抑制单击并提示连接客户端。使用系统 DPI 拖动阈值，越过后立即由原生拖动接管。取消捕获时丢弃单击与吸边判定。空闲位置松手后可选播 `land` |
| F-04 自移动 | 默认启用。首次或被用户打断后等 15 s，正常走完后冷却 10 s。按空间选可用方向，沿工作区水平线性移动，至少有 16 DIP 空间，单次最多 144 DIP。只在声明的 `movement_us` 内移动，合成左右路线时窗为 250–950 ms。禁用时仍可在原地预览移动动作 |
| F-05 吸边 | 左右工作区边缘 16 DIP 内松手触发。吸边暂停自移动，单击切换探头，菜单展开向内退 32 DIP，双击保留姿态。探头缺失可保留同侧吸边姿态，吸边姿态缺失回待机 |
| F-06 尺寸 | 初始基准取 EDID 可视高度的 16%，限于 36–90 mm，再乘初始缩放 70%。探测最多等 120 ms，未知时采用 256 DIP 基准。跨屏保留逻辑边长，按目标屏 DPI 和工作区约束窗口 |
| F-07 可见性 | 手动隐藏和当前屏前台全屏分别判定，托盘恢复手动隐藏。隐藏和最小化停止动画更新和位移，恢复从基础姿态起播。全屏及工作区最多每 250 ms 复查，窗口移动和变更也会触发复查 |

## 素材绑定与代表动作

合成包为 10 个动作、20 页，首项为 `interact`，`idle` 排第五。另生成 1 个动作、2 页的待机包。合成互动的锚点为底边中央，缩放 0.75，偏移 `[0.125, -0.25]`，用于验证变换后透明命中。完整绑定与变换顺序见 [G-05](../../G-05-contracts.md#素材包-v1)。

本地代表包从 P0 已盘点素材中选择 6 个动作，24 fps，320×320，四帧横排分页。均采用底边中央锚点，表中缩放与偏移作显示舍入，装配时从旧清单读入原值。移动采用源视频各自的朝向与时窗，位移上限按本节点规则计算。旧素材缺少独立吸边和探头姿态，代表包通过待机降级。

| 产品 ID | 旧 ID | 帧数 | 缩放 | 归一化偏移 | 方向 / 位移时窗 |
| --- | --- | ---: | ---: | --- | --- |
| `idle` | `idle` | 96 | 1.000000 | [0, 0] | — |
| `interact` | `01` | 96 | 1.011589 | [0.001397, 0.013434] | — |
| `move-left` | `move02left` | 217 | 0.998366 | [-0.009705, 0.009597] | 左，1,917–5,250 ms |
| `move-right` | `move02` | 264 | 1.014950 | [0.001399, 0.010864] | 右，4,750–7,500 ms |
| `drag` | `drag` | 96 | 1.004934 | [-0.003493, 0.007315] | — |
| `land` | `landing` | 48 | 1.004934 | [-0.003493, 0.007315] | — |

代表包共 205 页。PNG 载荷 134,974,091 字节，三档解包共 553,547,693 字节，已超过 P1b 三段素材的 80 MiB 原型包体预算。该输入用于本地验证，正式素材范围与分发体积随 P2-04 和 P4 核定。

| 包 | 清单 SHA-256 |
| --- | --- |
| `p2-behavior` | `c90a175afc90beb414ba4de18067864b4f1248f593ae4a81046a7654ef68b8b9` |
| `p2-idle-only` | `71ebdff60b13ef86d485db6bf6c097a4ed42781ef58526588e5d5519815b0bae` |
| `white-dragon-p2-local` | `7f76dea75008907e26b0d66557faf97d53f47056e539f7884ab02233d4c132fe` |

## 复测命令

在仓库根运行。行为脚本使用交互桌面及自己的窗口，测试过程中会短暂显示测试菜单和全屏窗口。默认使用 PNG。

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked
cargo build --release --locked
cargo test --locked
cargo test --locked --all-features
cargo build --release --locked --features p1b-reference --target-dir target/reference
python scripts/validation/p2/generate-fixtures.py
$fixtures = 'artifacts/validation/p2/fixtures'
$run = 'artifacts/validation/p2/runs/p2-01-recheck'
target/release/validate-pack.exe --distribution "$fixtures/p2-behavior"
target/release/validate-pack.exe --distribution "$fixtures/p2-idle-only"
python scripts/validation/p2/check-windows.py --exe target/reference/release/desktop-pet-chatgpt.exe --fixtures $fixtures --output $run
```

代表包额外需要本地旧素材和 FFmpeg。示例使用本机保留的旧版工具，使用独立输出目录。

```powershell
$env:PYTHONPATH = Join-Path $PWD 'artifacts/validation/p2/tools'
python -m pip install --target "$env:PYTHONPATH" ispc_texcomp==1.0.1 texture2ddecoder==1.0.6
python scripts/validation/p2/assemble-representative.py --legacy-manifest 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' --ffmpeg '.legacy-v0.5.1/tools/ffmpeg.exe' --output "$fixtures/white-dragon-p2-local"
target/release/validate-pack.exe --distribution "$fixtures/white-dragon-p2-local"
python scripts/validation/p1/measure-p1b-playback.py --exe target/reference/release/desktop-pet-chatgpt.exe --pack "$fixtures/white-dragon-p2-local" --output "$run/representative" --formats png --rounds 1 --warmup 2 --seconds 3
```

## 本地结果

Windows 11 专业版 22631，Core Ultra 7 155H、Intel 核显驱动 32.0.101.8860、DX12 DirectComposition。Rust 1.99.1 MSVC，Python 3.14.7。行为与尺寸使用 `p1b-reference` 构建的显式初始定位，常规冒烟使用默认 Release。参照构建 SHA-256 为 `b65280364103647dfc34d110e7306cb6a52e1f4991d8ac2aa454fce9f7a1cefe`，默认 Release 为 `de723a09af252fcff4d11a05670832e9702e9a3ad88fc434c9b20c58bfd692a8`。

| 显示器 | 显示模式 | 缩放 | EDID 可视尺寸 | 初始 DIP | 实际像素 |
| --- | --- | ---: | --- | ---: | ---: |
| DISPLAY1，内屏 | 1920×1080 | 125% | 310×170 mm | 128.08 | 160×160 |
| DISPLAY2，外屏 | 1920×1080 | 100% | 530×300 mm | 120.96 | 121×121 |
| DISPLAY1，强制回退 | 1920×1080 | 125% | 跳过探测 | 179.20 | 224×224 |

两块屏的 EDID 读数与本机 WMI 尺寸字段一致。行为脚本同时验证从 125% 屏拖到 100% 屏后保留逻辑边长，并按新屏工作区约束窗口。负坐标、工作区角落、EDID 校验和、零尺寸、缺页超时及迟到结果由确定性测试覆盖。双屏本次未拆屏，面板未用尺复测，150%/200% 和另一 GPU 的实机行为仍待设备矩阵。

格式、Clippy、锁定 Debug/Release 构建通过，默认与全 feature 测试各 14 项通过。两个合成包与六动作本地代表包均通过 `validate-pack --distribution`。已保存尺寸优先级、设置写回和重启恢复随 P2-02 验证。托管 Windows CI 仍待触发。

窗口验收通过动作按 ID 绑定、单击与双击互斥、变换后命中、取消拖拽、手动移动预览、双侧吸边及展开、混合 DPI 拖动、托盘恢复、同屏和异屏全屏判定。自主路线实走 180 像素，返回待机后约 10 秒再开始，第二次由拖拽打断，落地后位置保持。手动隐藏在退出全屏后继续生效，自动隐藏期间没有动画帧，恢复后位置保持。坏互动后续页回到待机，坏待机首帧回到占位。

默认 Release 的 PNG 短测预热 2 秒、采样 3 秒。代表包待机首个素材帧 472.67 ms，呈现 73/73 帧。P1 的 `dawn` 单动作兼容包首帧 349.66 ms，呈现 90/90 帧。

本地证据位于 `artifacts/validation/p2/runs/p2-01-final-1794443416424/`。其中 `result.json` 为双屏尺寸及行为结果，`behavior.log` 与坏页日志包含动作和可见性事件，`representative/` 和 `p1-dawn/` 包含短测参数、输入哈希和采样结果。
