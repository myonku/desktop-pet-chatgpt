# 本地素材工作区

本文说明 v2 制备工具和本地路径。默认动作范围及分发档位见[素材设计](../docs/design/assets.md)。

- `source/white-dragon/` 存放当前角色的原始动画、音效、素材清单和来源及许可说明。制备时从这里选择输入。
- `work/<候选>/` 存放中间帧、优化对照和报告。
- `packs/<pack-id>/<pack-version>/` 存放解包的 v2 素材，包根直接包含 `manifest.json`。
- `icon/` 存放项目默认图标的透明 PNG、ICO 和 Windows 资源脚本。Cargo 构建时把两个 ICO 编译进 EXE。

白龙素材发布源为 `jingh12312-rgb/bailong-desktop-pet`。保留下载版本、输入哈希和原素材许可说明。 [上游发布](https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1)

v0.5.1 的源素材清单引用 35 段不同的 720 × 720 VP9 WebM，合计 153,885,547 B。既有完整解码盘点共 4,426 帧，并在缩小到 16 × 16 的 RGBA 样本中确认每段都保留透明和不透明像素。本地源副本的 35 段文件大小和 SHA-256 均与盘点记录一致。重制时使用 `provenance.json` 留存实际使用的输入哈希和配方。

`sounds/来源.txt` 将 `click.wav` 描述为程序合成音效，同目录另有 Kenney Interface Sounds 的 CC0 说明。两份说明未给出该 WAV 与 Kenney 音效的对应关系，公开装配前须逐项核对来源与许可。 [发布管理](../docs/development/release.md)

将包含 `素材清单.json` 和 WebM 的素材目录放在 `source/white-dragon/白龙桌宠素材/`，点击音效放在 `source/white-dragon/sounds/click.wav`。FFmpeg 需支持 `libvpx-vp9` 透明解码，示例使用 `work/tools/ffmpeg.exe`，可按本机路径替换。没有本地源素材时，可使用已制备并校验的正式 v2 包。

开发默认加载 `assets/packs/white-dragon/0.1.0-v2/`，由原始 WebM 生成六个基础动作及四个吸边、探头姿态。可通过 `pack_path` 选择其他 v2 包。发布默认素材解压至 EXE 旁的 `resources/default/`。 [十动作包制备命令](../docs/development/workflow.md#检查与回归)

## 画质参照与图标

720 px 画质参照目录为 `assets/packs/white-dragon/0.1.0-baseline/`。将下方全动作命令的尺寸和版本参数替换为 `--size 720 --batch 2 --pack-version 0.1.0-baseline` 并更新输出目录，即可制备带四个吸边和探头动作的全尺寸 contour 对照。程序可通过开发配置显式加载。

若需验证桌面快捷方式图标，向 [New-DesktopPetShortcut.ps1](../scripts/release/New-DesktopPetShortcut.ps1) 传入 `-Exe`、`-Pack` 和 `-Validator`，可用 `-Output` 指定新快捷方式的路径。素材制备的 `provenance.json` 记录源文件、配方和成品清单哈希。

## v2 全动作开发包

`assets/packs/white-dragon/0.1.0-full-v2/` 是独立的 320 px PNG 全动作包，保留 39 个动作及原始时序。普通桌宠使用 10 个语义组，包含待机、互动、走／跑／飞等动作变体。工作、额度和反应素材保留短名称，供手动预览。首发默认包沿用十动作方案。 [制作契约与语义表](../docs/development/runtime-v2.md#引擎行为语义) · [白龙变体及短名称](../tools/asset_optimizer/white_dragon_bindings.py)

在 `desktop-pet.dev.toml` 顶层设 `pack_path = "assets/packs/white-dragon/0.1.0-full-v2"` 后重启即可试用。从原始素材可直接重建到新目录。

示例使用 `assets/icon/` 下带透明边角的两张源图。ICO 转换保留 PNG 的透明度和底色，自制图标需要在源图中提供透明区域。

```powershell
cargo build --locked --release --bin validate-pack
uv run --locked --group optimize python -m tools.asset_optimizer.pack `
  --source 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' `
  --ffmpeg assets/work/tools/ffmpeg.exe --validator target/release/validate-pack.exe `
  --upstream-release https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1 `
  --pack-version 0.1.0-full-v2 --size 320 --batch 4 --profile contour `
  --click assets/source/white-dragon/sounds/click.wav `
  --app-icon assets/icon/desktop.png --tray-icon assets/icon/tray.png `
  --output assets/packs/white-dragon/0.1.0-full-v2
```

不同尺寸、配方和局部动作对照见[离线素材优化模块](../tools/asset_optimizer/README.md#webm-装配与对照)。
