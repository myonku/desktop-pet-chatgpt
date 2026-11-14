# 本地素材工作区

本文说明现有工具和路径。下一代格式、默认动作范围和分发档位仍是[设计提案](../docs/design/assets.md)；现有全量装配命令不代表默认发布选择。

- `source/white-dragon/` 存放当前角色的原始动画、音效、素材清单和来源及许可说明。转换时从这里选择输入。
- `work/<候选>/` 存放中间帧、优化对照和报告。
- `packs/<pack-id>/<pack-version>/` 存放转换后的解包素材包，包根直接包含 `manifest.json`。
- `icon/` 存放项目默认图标的透明 PNG、ICO 和 Windows 资源脚本。Cargo 构建时把两个 ICO 编译进 EXE。

白龙素材发布源为 `jingh12312-rgb/bailong-desktop-pet`。保留下载版本、输入哈希和原素材许可说明。 [上游发布](https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1)

开发构建默认从启动工作目录加载 `assets/packs/white-dragon/0.1.0/`。两套设备可从同一份本地包复制此目录，或用已有的源素材和支持 VP9 alpha 的 FFmpeg 重新装配。当前 PNG 包含待机、互动、拖拽、落地和左右移动。吸边和探头姿态缺失时使用程序的基础姿态回退。发布安装包的默认素材位置为 EXE 旁的 `resources/default/`，由独立的素材附件安装到该位置。

在仓库根运行，按本机位置替换 FFmpeg 路径。

```powershell
uv run --locked python scripts/assets/assemble-white-dragon.py --legacy-manifest 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' --ffmpeg 'C:/path/to/ffmpeg.exe' --output 'assets/packs/white-dragon/0.1.0' --png-only
cargo run --locked --bin validate-pack -- --distribution 'assets/packs/white-dragon/0.1.0'
```

暂时没有白龙源素材时，可运行 `uv run --locked python scripts/validation/p2/generate-fixtures.py`，再将开发配置的 `pack_path` 指向 `artifacts/validation/p2/fixtures/p2-behavior`。BC7 转换需加 `--group media` 并去掉 `--png-only`。本地候选用 `--pack-version` 和新的输出目录区分。现有读取器不要求素材版本等于程序版本；新发布方案采用兼容清单，见[发布管理](../docs/development/release.md)。 [素材包 v1](../docs/development/runtime-v1.md) · [离线优化工具](../tools/asset_optimizer/README.md)

候选包可通过 `--size` 和 `--batch` 调整帧尺寸及每页帧数，默认 320 像素和 4 帧。装配器检查清单字节数并生成本地 `provenance.json`。保留该文件，用它关联输入清单、逐动作源视频、转换工具和成品清单。

全量候选可用 `--app-icon desktop-icon.png --tray-icon tray-icon.png` 装配两类可选图标。开发配置指向 `assets/work/white-dragon-contour-icons-320/pack` 时，重启即可查看窗口和托盘图标。桌面快捷方式用 [PowerShell 脚本](../scripts/release/New-DesktopPetShortcut.ps1)创建，传入 `-Exe`、所选 `-Pack` 和 `-Validator`，可用 `-Output` 指定新 `.lnk` 路径。

全尺寸画质参照位于 `assets/packs/white-dragon/0.1.0-baseline/`，保留源动画的 720×720 分辨率，每页 2 帧，采用 `contour` 去噪，并包含四个吸边和探头动作。可在开发配置中显式指定该路径。它不是建议的日常运行或默认发行包；运行档位和成本按[新素材设计](../docs/design/assets.md)与[性能方案](../docs/design/performance.md)另行选定。

```powershell
cargo build --locked --release --bin validate-pack
uv run --locked --group optimize python -m tools.asset_optimizer.pack `
  --source 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' `
  --ffmpeg assets/work/tools/ffmpeg.exe --validator target/release/validate-pack.exe `
  --upstream-release https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1 `
  --pack-version 0.1.0-baseline --size 720 --batch 2 --profile contour `
  --click assets/source/white-dragon/sounds/click.wav `
  --app-icon assets/icon/desktop.png --tray-icon assets/icon/tray.png `
  --output assets/packs/white-dragon/0.1.0-baseline
```

全动作候选通过 [离线素材优化模块](../tools/asset_optimizer/README.md#webm-装配与对照)生成。个人设备上的全动作包和画质对照放在 `work/`。从仓库根目录启动时，可在 `desktop-pet.dev.toml` 顶层指定 `pack_path = "assets/work/white-dragon-despeckle-320/pack"`，或把路径指向 `assets/work/white-dragon-comparison/` 下的同尺寸对照。 [现状与历史证据摘要](../docs/plan/baseline.md)
