# R-01 · P0 工具链、旧行为和实验基线

> 阶段归档。命令与路径保留当时的语境，复测使用[现行工程约定](../../G-05-contracts.md#工程与产物)。

日期 2026-11-07。设计基线为 `256d3d9`，旧版证据来自 `852f4f4`。P0 实施产物与本报告一起提交。

## P0-01 与 P0-04 · 工具链和构建

- 固定 Rust/Cargo 1.99.1、`x86_64-pc-windows-msvc`、rustfmt、Clippy。该版本是本轮验证的构建下限。构建机安装 VS 2026，默认 MSVC 工具集 14.51.36231，Windows SDK 10.0.26100.0。
- 直接依赖精确锁定 winit 0.30.12（rwh_06）、wgpu 27.0.1（std、dx12、wgsl）、egui / egui-winit / egui-wgpu 0.33.3（默认字体、剪贴板、winit 适配）及 windows 0.58.0（DirectComposition、DXGI、窗口消息）。依赖树中应用和适配器共享相同的 winit/wgpu/egui 版本，wgpu-hal 与应用共享 windows 0.58.0。依据 [egui-wgpu 版本清单](https://docs.rs/crate/egui-wgpu/0.33.3/source/Cargo.toml) 和本地 `cargo tree --locked -d`。
- 核对已下载的 wgpu-types 27.0.1 与 wgpu-hal 27.0.4 源码，存在 `Dx12SwapchainKind::DxgiFromVisual` 及 visual surface 路径。临时类型探针用锁定产物编译通过，验证 `egui_winit::State::new` 接受本项目 winit 窗口和 egui context，`egui_wgpu::Renderer::new` 接受本项目 wgpu device/format。该版本枚举的访问路径为 `wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual`。P1a 仍须在实机查询 capabilities、选择 alpha mode 并验证合成结果。
- PowerShell 中设置 `$env:CARGO_BUILD_TARGET='x86_64-pc-windows-msvc'`，以空 `CARGO_TARGET_DIR` 执行 `cargo build --locked`，通过（26.33 秒，依赖已下载）。`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked` 均通过，Rust 测试数为 0。干净 Release 构建 36.48 秒，EXE 130,560 字节并能运行。这个体积只作为骨架基线。
- Windows Actions 配置运行相同检查，检出内容只读，无素材上传步骤。提交前以 `x86_64-pc-windows-msvc` 复跑格式、Clippy、locked 构建和测试命令，全部通过（0 项 Rust 测试）。远端 `main` 尚未包含该工作流。P0-04 需以包含本次产物的提交触发托管运行，并记录提交 SHA、运行链接和结论后验收。

## P0-02 · 旧行为

[动作清单](p0-legacy-actions.json) 直接提取自历史 Git manifest，保留 35 个非空动作及空槽、时长、方向、渲染和移动参数。缺省字段按消费代码解释，避免给未声明的字段补造数值。

| 分类 | 旧行为与迁移决定 |
| --- | --- |
| 保留，F-02～F-07 | 动作 ID、时长、时序、方向、缩放/偏移/锚点，及左右吸边、拖后不触发点击、手动隐藏独立于全屏隐藏。旧点击窗口 250 ms，左右吸边阈值 24 逻辑像素，自移动等待 15 s、冷却 10 s，路线不镜像。 |
| 默认值，F-02/F-04 | 旧绘制缺省 scale 1.0、offset [0,0]。返回过渡 ID 为 return-加动作 ID，scale/offset 使用缺省值。旧自测帧数参照 24 fps，带角度范围的编号 03 动作为 30 fps，显式 fps 优先。移动路线缺省时窗 3000～5583 ms，实际插值缺省 0～durationMs，速率缺省 1.0（编号 01 为 0.5）。保留显式时窗并用源时间戳验证新播放调度。 |
| 默认值，F-06～F-09 | 尺寸 256 逻辑像素，sizePercent 100（50～200），bubblePercent 100（50～150），soundDisabled、taskBubblesDisabled、autoMovementDisabled 默认 false，autoHideFullscreen 默认 true。旧配置命名空间为 QSettings BailongPet/DesktopPet。 |
| 变更，F-01/F-03/F-09/F-15～F-17 | 单实例、系统拖动阈值取代旧固定 8 像素曼哈顿距离、托盘可撤销穿透、按需有界播放取代启动全量解码。迁移走显式备份导入，旧历史默认文件为 `%LOCALAPPDATA%/BailongPet/conversations.json`，可用 `BAILONG_DATA_DIR` 指定目录。实际用户数据导入与迁移回退留给 P2。 |
| 延后，F-10～F-15 | 旧 syncCodexTasks 默认 true，新版由用户开启后探测。实际 Codex 数据库与接口、审批和额度能力留给 P3；多平台和 DSH 留给 P5。 |

| 旧测试（tests/） | 需求 |
| --- | --- |
| test_pet.py | F-01～F-03、F-06、F-14 |
| test_movement.py | F-03～F-07 |
| test_visibility.py | F-07 |
| test_preferences.py | F-08、F-09 |
| test_platform_support.py | F-01、F-09、F-16 |
| test_desktop_task_sync.py | F-11、F-15 |
| test_task_notifications.py | F-08、F-11 |
| test_chat.py | F-12、F-13、F-15 |
| test_session_reuse.py | F-12、F-13 |
| test_quota.py | F-14 |

### 旧版执行结果

在本机 Python 3.13.14 临时 venv 中按旧 requirements 安装 PySide6 6.9.2、Pillow 12.0.0、psutil 7.0.0。配置镜像超时后使用 PyPI。将 `PYTHONPATH` 指向旧 `app/`，Qt 使用 offscreen，QSettings 使用 INI 格式并重定向应用数据和 Codex 目录到临时目录。以 unittest discover 执行 `tests/`，83 项通过（214.915 秒），详细日志在 `target/p0-legacy-tests.log`。

另执行旧源码 `selftest.run()` 的离线分支，35 段均可解码，9 项检查通过。原始报告在 `target/p0-legacy-selftest/检查报告.json`。未执行旧冻结 EXE、真实 Codex 聊天和额度、自启与前台全屏实机检查；这些分别需要旧分发包、授权会话或交互桌面。上述结果用于旧行为基线。

## P0-03 · 参考设备、样本台账与口径

参考机为 Windows 11 家庭中文版 22631、Ryzen 7 8845H，同机 Radeon 780M（31.0.22032.1）和 RTX 4060 Laptop GPU（32.0.15.9186）。桌面分辨率 2560×1600。P1a 每轮记录实际选中的适配器、显示器和 DPI。第二台参考机待取得。

| ID | 来源与用途 | 可复现证据 |
| --- | --- | --- |
| SYN-01 / SYN-02 | 2026-11-07 本项目通过标准库脚本生成的几何样本，分别为 alpha / 色阶和 30 fps 移动。无外部图片、字体、音效输入。用于本地、CI 实验；正式资源包范围留待 P1b。 | `python scripts/generate-p0-fixtures.py` 生成 128×128 straight RGBA、sRGB PNG。逐文件 SHA-256 在 `target/p0-fixtures/manifest.json`，清单 SHA-256 为 `f8c51decd60d948011e84cbf78850772cf63fce9497ad2c400b8099c3db407b4`。Pillow 解码 31 张并核对尺寸、alpha 范围、端点像素、运动变化及全部哈希。 |
| LEG-01 | 历史角色素材，来源与既有权利线索见 旧资源审计 `docs/archive/audit-2026-11-06/A-04-assets.md`（从 [归档索引](../README.md) 查阅）。 | 历史 WebM 约 146.76 MiB，逐文件哈希见旧审计。只作为本地旧行为参照，分发权限待确认。 |

沿用并固定 [G-06](../../G-06-validation.md) 口径。首次可交互 ≤2 s（首帧且拖拽和菜单可用），启动至少 10 次；输入到可见反馈 p95 ≤100 ms，同时报告 p50/p99；稳定播放丢帧率 <1%，按 24/30 fps 时间序列统计。P1b 初始 CPU 素材缓冲 32 MiB、GPU 素材页池 64 MiB，完整内存和包体上限由 P1b 实测后冻结。骨架退出过快，暂无常驻内存和交互基线。

稳定场景预热 2 分钟、采样 10 分钟、至少三轮。每轮保留 Release 版本、依赖、样本哈希、DPI、适配器、场景、进程树。使用 PowerShell 7 运行以下命令，传入主进程及已识别的相关子进程 PID，按进程分列累计 CPU 时间、区间单核百分比、private bytes、working set 和峰值 working set。采样进程提前退出时报错，CSV 可能留下部分数据。GPU、唤醒、I/O、帧间隔及上传延迟由 P1 用 ETW/WPR 或应用事件采集。

```powershell
./scripts/measure-process.ps1 -ProcessIds $petPid -DurationSeconds 600 -OutputCsv ./target/p0-pet.csv
```

提交前重新生成并核对 31 张 PNG 的 SHA-256，清单哈希与初次生成相同。采样脚本再次对当前 PowerShell 进程进行 1 秒冒烟，输出两条可解析的采样记录。原始 PNG、清单和旧测试日志留在本地 `target/`。

## P0-04 · 托管验收

2026-11-07，提交 `53d0ba629543ff76e6ddd8150d71753ff4b09949` 推送至 `main` 后，[Windows Rust checks #1](https://github.com/myonku/desktop-pet-chatgpt/actions/runs/42341652991) 成功，耗时 1 分 36 秒。P0-04 验收通过。
