> 归档整理日期：2026-11-07。归档编号：A-03；原始版本/日期见正文，未注明的原始日期不作推定。

# GPT白龙娘动态桌宠 v0.5.1

Windows轻量版：自移动、吸边探头、大小调节、Codex公开任务进展提醒，35段透明WebM动画。

普通使用者请到[发布页](https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1)下载 `bailong-desktop-pet-v0.5.1-windows-x64-webm.zip`，全部解压后双击 `BailongPet.exe`。

## 开发者运行

此仓库提供v0.5.1轻量版源码；动画与音效位于 `assets/`，Windows解码器位于 `tools/ffmpeg.exe`。原作者的补充文档与构建记录保留在 `docs/upstream/`，素材调查记录位于 `docs/audit-2026-11-06/`。

1. 安装64位Python（当前构建使用Python 3.13）。
2. 在源码目录执行 `python -m pip install -r requirements.txt`。
3. 保留源码中的 `assets/白龙桌宠素材/`、`assets/sounds/` 和 `tools/ffmpeg.exe`，无需再从发布包复制资源。
4. 执行 `python app/pet.py`；自检执行 `python app/pet.py --self-test`。

程序通过FFmpeg的libvpx-vp9解码透明通道，保留动作预加载；请不要用丢弃透明通道的默认VP9解码方式替换。

## 构建Windows包

在Windows上安装PyInstaller 6.22.3至 `.build-tools`：`python -m pip install --target .build-tools pyinstaller==6.22.3`。

运行 `python scripts/build_windows.py` 生成EXE。重新整理完整ZIP使用 `python scripts/build_release.py`，使用 `tools/ffmpeg.exe` 和 `assets/` 中的资源，输出到 `release/`。素材预处理入口为 `python scripts/prepare_assets.py`；测试已移至 `tests/`，在PowerShell中先执行 `$env:PYTHONPATH="app"`，再执行 `python -m unittest discover -s tests`。随包第三方许可保留在 `third_party/licenses/`。

## 说明

没有Codex也能使用普通桌宠功能。额度、对话与任务同步需要使用者自己的Codex安装和登录。仓库不包含账号、密钥、个人聊天记录。

macOS适配源码尚未完成Mac实机验收，不提供已验证Mac安装包。当前Mac构建脚本仍需补齐FFmpeg打包，请勿将其当作可直接发布的构建流程。

源码尚未指定开源许可证；公开源码不代表授予任意使用或再分发许可。美术素材说明见ASSETS.md，运行库遵循各自许可证。本项目非OpenAI官方产品。
