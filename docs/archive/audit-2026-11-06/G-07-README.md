# 原始工程与本地发布包调查

记录日期：2026-11-06。阶段范围：盘点与分析，不修改程序、依赖配置、构建脚本，不移动或导入素材，不提交或推送。

## 1. 调查基线与结论

本次以本地工作区和用户下载的 `release/BailongPet` 为证据，没有拉取远端或验证远端最新版本。源码 HEAD 为 `b102e734f20bb50889af12df5778f1499109f41b`，提交说明为 `Publish v0.5.1 WebM desktop pet source and build instructions`。源码与发布包均标示 v0.5.1。

本地 `origin` 的 fetch/push 地址均为 `https://github.com/myonku/desktop-pet-chatgpt.git`，`upstream` 均为 `https://github.com/jingh12312-rgb/bailong-desktop-pet.git`。本次没有改变远端设置。后续按用户要求只向 fork 开发，不向 upstream 提交阶段性 PR；注意 upstream 当前仍配置了 push URL，隔离目前依靠操作约束。

主要结果：

- 当前是 Python + PySide6/Qt Widgets 原生桌面程序，使用 PyInstaller onedir 打包，没有浏览器渲染层。
- 发布包中的 35 段动画全部与清单一一对应，没有发现清单外动画或缺失动画。全部可经随包 FFmpeg 的 libvpx-vp9 解码出透明 RGBA，共 4,426 帧。
- 动画压缩文件仅 146.76 MiB，但启动时全部展开为 QPixmap 并常驻。320×320 下像素载荷约 1,728.91 MiB，与用户观察的 1760+ MB 十分接近。该机制有明确代码证据，尚未做实时进程内存归因或泄漏测试。
- 优先验证帧缓存和播放策略的改变是否足够，再判断语言迁移。相同缓存策略搬到 C++/Rust 仍会承担相同数量级的像素内存。
- 当前源码没有项目 LICENSE；ASSETS.md 明确没有额外授予美术素材商用或再分发许可。这会影响后续把素材推入 fork 的安排，不能把“公开源码”视为已经获得开源许可。

配套记录：

- [素材明细与迁移边界](G-06-assets.md)：35 段动画表、必要文件、清单字段、未来归档方案。
- [素材测量原始记录](asset-inventory.json)：逐动画 SHA-256、字节数、尺寸、帧率、实解码帧数与透明度采样。
- [发布包文件清单](release-inventory.json)：本地发布目录 230 个文件的相对路径、字节数和 SHA-256。包含已经存在的运行状态文件，属于调查时快照，不等同于原始下载包校验清单。
- [源码结构和构建记录](source-inventory.json)：模块统计、导入关系、构建哈希对照。

## 2. 现有目录结构

| 位置 | 真实职责与现状 |
| --- | --- |
| `app/` | 24 个 Python 文件、3,384 行；生产代码、10 个测试文件和音效混放；没有 `__init__.py` |
| `app/pet.py` | 970 行，入口、桌宠窗口、动画、缓存、气泡、菜单、交互、设置、任务通知等集中在同一模块 |
| `app/sounds/` | click.wav 及两个说明文本；源码和发布包的 click.wav 字节一致 |
| `work/` | 3 个构建脚本共 167 行，另有发布说明模板；并非统一开发任务入口 |
| 根目录文档 | README、DEVELOPMENT、安装说明、Mac 说明、素材说明、版本及构建记录；README 与 DEVELOPMENT 当前字节完全相同 |
| `release/BailongPet/` | 用户自行下载并放入的完整 Windows 包；调查开始时即未跟踪，当前 `.gitignore` 不忽略 `release/` |
| 缺失的源码运行资源 | 根目录没有 `outputs/`、`tools/`，因此新 clone 不能直接按源码启动 |

没有发现 AGENTS.md、pyproject.toml、包锁文件、CI 工作流、类型检查或格式检查配置。构建与资源定位主要依赖硬编码目录和 `sys.path`，不是可安装的 Python 包。全部 app 函数/方法均未发现参数或返回类型注解。

## 3. 模块分工与调用逻辑

| 模块 | 行数 | 职责与边界 |
| --- | ---: | --- |
| pet.py | 970 | QApplication 入口、Pet/Bubble、托盘与菜单、帧加载和绘制、动作状态切换、移动、吸边、缩放、设置、通知和诊断写入 |
| video_frames.py | 28 | 找到 FFmpeg，强制 libvpx-vp9，按帧从 stdout 读取缩放后的 RGBA |
| prepare_assets.py | 30 | 顶层直接执行的离线缓存生成脚本，写 `app/cache/*-hq.rgba.gz` 和元数据 |
| movement.py | 54 | 移动路线与时间插值计算，和 Qt 窗口操作相对分离 |
| chat_ui.py | 396 | 输入与历史窗口、ChatController、TaskWorker，UI 更新、任务审批交互、历史保存调度和 session 复用 |
| chat_backend.py | 276 | HistoryStore、本地会话持久化、CodexSession，子进程 stdio JSON 通信及流式事件整理 |
| quota.py | 133 | 定位 Codex、启动 app-server、查询额度、错误分类、超时/重试及脱敏日志 |
| desktop_task_sync.py | 186 | 只读观察本地 Codex SQLite、投影任务状态、去重通知、同步到历史窗口 |
| desktop_visibility.py | 62 | Windows 前台全屏检测及 macOS 分支 |
| platform_support.py | 46 | 资源/状态/用户数据路径、Mac 窗口和全屏辅助 |
| autostart.py / mac_autostart.py | 94 / 24 | 进程观察、Windows 启动文件夹脚本或 Mac LaunchAgent、自启守护进程 |
| selftest.py / version.py | 96 / 1 | 发布程序自检入口 / 版本常量 |

普通启动流程为 `pet.main()` → QApplication 与 QLocalSocket/QLocalServer 单实例检查 → `Pet()` 读取素材清单 → 播放基础 idle → 同步预解码全部动作 → 托盘及定时器 → Qt 事件循环。预加载期间用 `QApplication.processEvents()` 更新进度窗口，初始化和大量状态设置仍在主线程上。

播放链为清单 item → `asset_path()` 优先取 playbackFile → `load()` → 有效 gzip 缓存或 FFmpeg/WebP 解码 → QImage.copy → QPixmap 列表 → `play()` 设置当前状态 → `animate()` 按清单 durationMs 和帧数选索引 → QPainter 绘制。不是持续视频流播放，也没有使用视频时间戳逐帧调度。清单中的 `renderScale` 和 `renderOffset` 用于动作间对齐；当前绘制采用底部居中计算，不能仅搬走视频而丢弃这些配置。

主要时间任务：动画 tick 16ms，全屏检测 500ms，桌面任务同步 1s，诊断 JSON 写入 2s；聊天保存 1s、渲染节流 80ms。隐藏时停止动画 tick，但不会释放 pinned_frames，任务同步等其他周期工作仍可能继续。

聊天与额度是两个入口，共同依赖用户自己的 Codex 可执行文件。普通动画不需要账号。聊天 session 使用独立线程和子进程，支持连接复用；额度查询另起 app-server。桌面同步则在 Qt 定时器回调里直接查询 SQLite，固定依赖 `state_5.sqlite`、`thread_history_1.sqlite` 及其表结构，不是稳定的抽象适配层。这里记录的是仓库实现，没有验证其与当前 Codex 版本的兼容性。

## 4. 依赖、打包与可复现性

| 配置/组件 | 当前值 | 作用及问题 |
| --- | --- | --- |
| requirements.txt | PySide6 6.9.2 / Pillow 12.0.0 / psutil 7.0.0 | Qt UI/图像及兼容 WebP/进程观察；固定了直接版本，没有锁定完整依赖图 |
| requirements-macos.txt | 同上 + PyInstaller 6.22.3 + Cocoa/Quartz | 混合运行和构建依赖；PyObjC 没有固定版本 |
| Windows 构建 | Python 3.13，脚本要求 `.build-tools` 中 PyInstaller 6.22.3 | 没有统一环境锁或干净环境构建验证；发布组件记录显示 Python 3.13.9 |
| 视频解码 | `tools/ffmpeg.exe`，87,638,016 字节 | 源码 requirements 不包含；需手动从发布包提取或另行准备 |
| 发布运行库 | `_internal/`，143 文件，116.07 MiB | Python/Qt/Pillow/psutil/native DLL 和插件，并不是源码素材 |

Windows `build_windows.py` 只构建 EXE 和依赖目录；`build_release.py` 再复制素材、音效、FFmpeg、说明和许可，重写清单，生成 ZIP。源脚本用白名单维护模块，并动态生成一套 README/DEVELOPMENT/requirements，这些内容和仓库现有文档并不完全一致，后续改文档可能被打包模板覆盖。

发布脚本依赖 `outputs`、`work/video-deps/imageio_ffmpeg/binaries/ffmpeg-win-x86_64-v7.1.exe`、已生成的 EXE、`.build` TOC、依赖分发元数据及可选 Conda 元数据。当前仓库不具备完整输入。许可收集还借用了当前 Python 环境，现有发布包确实收集了 Conda 来源组件，不能假设只用 requirements.txt 的干净 pip 环境就能字节复现当前发布物。

Mac 构建脚本复制素材并临时签名，但没有打包 FFmpeg，原文档也承认未实机验收。Windows 包中的插件与 DLL 不能拿给 Mac 使用。源码 Windows 音效主要走 winsound；Mac 分支导入 QtMultimedia，Windows 发布 `_internal` 没有 QtMultimedia 模块，不应仅凭源码导入就认为随包实际包含它。

`licenses/components.json` 中存在 qtwebengine 组件记录，目录中可见 Qt6Pdf/qpdf 等文件，但未发现 QtWebEngine 浏览器运行库。该记录不能用来断言程序使用 Chromium，也不能把磁盘 DLL 大小直接当成常驻内存。后续可以在干净构建中追踪插件收集原因，不能直接删除未知 DLL。

FFmpeg 实际版本输出是 `7.1-essentials_build-www.gyan.dev`，配置包含 `--enable-gpl --enable-version3`。随包 FFmpeg.txt 尾部标记的 imageio/BtbN 来源链接不能证明该具体二进制来自这些构建，后续应按实物重新记录下载来源、构建参数、哈希和相应许可。

### 源码与发布记录是否匹配

根目录和发布包 BUILD_INFO.json 内容相同，声明了 24 个源码哈希。按工作区原始字节核对，12 项匹配；另外 10 项仅 CRLF/LF 不同，归一化换行后匹配，和本地 `core.autocrlf=true` 相符。剩余 `test_preferences.py`、`test_desktop_task_sync.py` 即使归一化也不匹配，直接读取 HEAD 中 Git blob 的哈希同样不匹配。

因此所有生产 Python 模块均为原始字节匹配或仅存在换行差异，但两个测试文件的记录已经失配。BUILD_INFO 是声明性元数据，没有独立证明 EXE 确由这份源码构建；本次没有反编译 EXE 或验证下载来源签名。

## 5. 内存解释与其他性能线索

关键证据是 `app/pet.py:139` 的 `pinned_frames`、`:140` 的 frame_size、`:174–184` 的全动作预加载、`:406–438` 的帧加载，以及 `app/video_frames.py:16–28` 的 raw RGBA 解码。

frame_size 使用所有显示器中最大的 devicePixelRatio：`min(512, max(256, round(256 * max_DPR)))`。并不由桌宠窗口当前显示大小决定；高 DPI 的另一块显示器也可能提高整个缓存分辨率。

全部帧像素载荷估算为 `4,426 × frame_size² × 4`：

| 帧尺寸 | 对应 DPR 示例 | 像素载荷 MiB |
| --- | --- | ---: |
| 256×256 | 100% | 1,106.50 |
| 320×320 | 125% | 1,728.91 |
| 384×384 | 150% | 2,489.63 |
| 512×512 | 200% 或以上封顶 | 4,426.00 |

这里计算的是 4 字节/像素的存储量级，不含 Qt/Python 对象、运行库、窗口缓冲及驱动等开销，也不是进程 working set 的测量值。动画压缩文件约 146.76 MiB，压缩比不会自动降低解码后的像素存储。

现有发布目录 `app/运行状态.json` 记录 displaySize 256×256、frameSize 320×320。它是用户之前运行留下的静态记录，不是本次实时采样；本次查询时没有正在运行的 BailongPet 进程。用户报告的“1760+MB”未明确任务管理器列及单位，因此只能说量级高度吻合，不能声称已经证明全部内存的来源或排除了泄漏。

`cache` 虽限制最多两个动作，启动预加载会把每段帧列表转入不淘汰的 `pinned_frames`，所以这个 LRU 不构成总内存上限。当前 `self.frames`、cache 和 pinned_frames 的引用不应机械算成三份像素拷贝；真正额外拷贝、QPixmap 的后端资源应通过运行剖析确认。

其他线索：

- `prepare_assets.py` 在 512 分辨率下先 list 全段帧，再 join，再 gzip.compress，生成缓存时会有显著瞬时内存峰值；读取 gzip 缓存也会整体解压单段数据。发布包当前没有 app/cache，不能把本次既有占用归因于 gzip 分支。
- 隐藏只停止动画播放，不释放 pinned_frames，预计隐藏后不能显著降低常驻像素量。
- 同步任务的 SQLite 查询、历史 JSON 全量保存和部分 UI 更新运行在主线程；长历史、较多任务可能带来卡顿。known、会话列表和会话内容没有整体保留上限，属于长时间运行待验证项，尚无泄漏实测。
- 诊断每两秒写安装目录；资源目录、可变状态和缓存混在一起，影响只读安装、升级与打包隔离。
- FFmpeg 通过 stdout 输出、stderr 另管道最后读取；异常输出量大时的管道阻塞、失败超时和取消行为值得后续审查，本次未故障注入。

## 6. 测试与本次验证范围

现有 10 个 test_*.py 覆盖聊天、桌面同步、移动、桌宠交互、平台路径、偏好、额度、session 复用、任务通知和可见性；不能笼统说项目“没有测试”。不过多项测试直接构造 Pet，会依赖整套素材并触发全量预加载，测试与资源、GUI 的边界较重，缺少性能预算测试和统一测试入口。

本次执行：

- 对 app 和 work 的 27 个 Python 文件做 AST 语法解析，全部成功，未导入构建脚本。
- `PYTHONPATH=app python -B -m unittest test_quota test_session_reuse -v`：6 项全部通过；使用 mock/临时文件，没有实际发起模型请求。
- 对全部 35 段视频逐段完整解码为 16×16 RGBA，`-vsync 0` 保留帧数量，全部返回成功；记录原始尺寸/帧率及透明度采样。该验证不等于原尺寸逐像素视觉验收。
- 记录发布目录逐文件 SHA-256，核对素材引用闭合与源码构建哈希。

本机当前 Python 没有 PySide6、Pillow、psutil，本次没有安装依赖、复制运行库、启动发布程序、运行 GUI 测试或发布程序自检。未检查个人 Codex 数据库或历史内容，也未执行自启设置。发布目录已有状态 JSON 没有被修改。

## 7. 下一阶段的输入与建议顺序（尚未实施）

1. 先固定本次基线与资源来源。把资源目录从安装状态文件中分离，确定可以纳入 fork 的许可范围；用清单与哈希保证 35 段视频及校准参数完整。
2. 建立独立、可复制的源码运行环境：明确 Python 支持版本，区分开发/运行/打包依赖，给 FFmpeg 单独的来源和获取规则，消除手工从 release 拼环境的步骤。
3. 先做可重复内存基准：分别在 100%/125%/150%/200% DPI，记录启动峰值、稳定 private bytes/working set、进程树、首次动作延迟、隐藏及长时间运行。当前用户观察只能作为基线线索。
4. 用最小实验比较按需解码、有字节预算的缓存、有限帧环形缓冲与当前全帧驻留，保留 VP9 alpha、24/30fps、动作对齐和即时互动要求。不能用牺牲透明通道的硬解或错误默认 VP9 解码来换取低占用。
5. 再拆分动画解码/缓存、动作状态机、窗口与气泡、Codex 适配、状态持久化和平台接口，补类型与边界测试；统一 docs、tests、构建入口。具体目录设计属于下一阶段，不在本次搬动。
6. 在同一素材、同一帧缓存策略下比较 Python/Qt、C++/Qt 或 Rust GUI 的内存、透明窗口/托盘/多屏实现、部署和维护成本，再决定是否迁移。语言选择目前未定。

下一阶段资源引入与依赖优化应各自独立可验收；无需为本次调查向原作者仓库发 PR。
