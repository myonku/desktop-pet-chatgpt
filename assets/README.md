# 本地素材工作区

- `source/white-dragon/` 存放当前角色的原始动画、音效、素材清单和来源及许可说明。转换时从这里选择输入。
- `work/<候选>/` 存放中间帧、优化对照和报告。
- `packs/<pack-id>/<pack-version>/` 存放转换后的解包素材包，包根直接包含 `manifest.json`。

白龙素材发布源为 `jingh12312-rgb/bailong-desktop-pet`。保留下载版本、输入哈希和原素材许可说明。 [上游发布](https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1)

开发构建默认从启动工作目录加载 `assets/packs/white-dragon/0.1.0/`。两套设备可从同一份本地包复制此目录，或用已有的源素材和支持 VP9 alpha 的 FFmpeg 重新装配。当前 PNG 包含待机、互动、拖拽、落地和左右移动。吸边和探头姿态缺失时使用程序的基础姿态回退。发布安装包的默认素材位置为 EXE 旁的 `resources/default/`，由独立的素材附件安装到该位置。

在仓库根运行，按本机位置替换 FFmpeg 路径。

```powershell
uv run --locked python scripts/assets/assemble-white-dragon.py --legacy-manifest 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' --ffmpeg 'C:/path/to/ffmpeg.exe' --output 'assets/packs/white-dragon/0.1.0' --png-only
cargo run --locked --bin validate-pack -- --distribution 'assets/packs/white-dragon/0.1.0'
```

暂时没有白龙源素材时，可运行 `uv run --locked python scripts/validation/p2/generate-fixtures.py`，再将开发配置的 `pack_path` 指向 `artifacts/validation/p2/fixtures/p2-behavior`。BC7 转换需加 `--group media` 并去掉 `--png-only`。本地候选用 `--pack-version` 和新的输出目录区分。正式附件与源码同版。 [素材包 v1](../docs/G-05-contracts.md#素材包-v1) · [离线优化工具](../tools/asset_optimizer/README.md)

候选包可通过 `--size` 和 `--batch` 调整帧尺寸及每页帧数，默认 320 像素和 4 帧。装配器检查清单字节数并生成本地 `provenance.json`。保留该文件，用它关联输入清单、逐动作源视频、转换工具和成品清单。
