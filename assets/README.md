# 本地素材工作区

- `source/white-dragon/` 存放当前角色的原始动画、音效、素材清单和来源及许可说明。转换时从这里选择输入。
- `packs/<pack-id>/<pack-version>/` 存放转换后的解包素材包，包根直接包含 `manifest.json`。

开发构建默认从启动工作目录加载 `assets/packs/white-dragon/0.1.0/`。两套设备可从同一份本地包复制此目录，或用已有的源素材和支持 VP9 alpha 的 FFmpeg 重新装配。当前 PNG 包含待机、互动、拖拽、落地和左右移动。吸边和探头姿态缺失时使用程序的基础姿态回退。发布安装包的默认素材位置为 EXE 旁的 `resources/default/`，由独立的素材附件安装到该位置。

在仓库根运行，按本机位置替换 FFmpeg 路径。

```powershell
python scripts/assets/assemble-white-dragon.py --legacy-manifest 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' --ffmpeg '.legacy-v0.5.1/tools/ffmpeg.exe' --output 'assets/packs/white-dragon/0.1.0' --png-only
cargo run --locked --bin validate-pack -- --distribution 'assets/packs/white-dragon/0.1.0'
```

暂时没有白龙源素材时，可运行 `python scripts/validation/p2/generate-fixtures.py`，再将开发配置的 `pack_path` 指向 `artifacts/validation/p2/fixtures/p2-behavior`。素材本体与转换包留在本地。当前包根、必备文件和格式遵循本项目的版本化协议，由本项目独立维护。[素材包 v1](../docs/G-05-contracts.md#素材包-v1) · [素材工作区](../docs/G-05-contracts.md#素材工作区)
