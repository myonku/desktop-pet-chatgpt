# 开发与维护工作流

## 构建当前 Windows 原型

使用 rustup、MSVC 构建工具和 Windows SDK；版本由 rust-toolchain.toml / Cargo.lock 固定。Python 工具由 pyproject.toml / uv.lock 管理；普通程序运行不需要 Python。

无白龙源素材也能用合成包运行，不应把私人素材目录作为新开发者的必要前置：

```powershell
uv run --locked python scripts/validation/p2/generate-fixtures.py
if (-not (Test-Path -LiteralPath desktop-pet.dev.toml)) { Copy-Item desktop-pet.example.toml desktop-pet.dev.toml }
```

只在开发配置不存在时复制示例；已有配置保留。在 TOML 顶层设 `pack_path = "artifacts/validation/p2/fixtures/p2-behavior"`，再运行：

```powershell
cargo run --locked --features development
```

真实素材制备见 [assets](../../assets/README.md)。开发配置默认不保存运行中的设置；需要验证持久化时使用产品构建和隔离测试目录，不能误用真实个人配置作破坏性测试。

## 检查与回归

```powershell
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --all-features
uv run --locked --group optimize python scripts/validation/check-asset-optimizer.py
```

依赖已缓存时可加 `--offline`。按变更选择检查：文档检查链接与事实；纯逻辑跑相关 Rust 测试；素材工具跑合成检查；窗口/调度/命中变化补对应 GUI 场景。通过后不无理由重复全套；每个测试应覆盖行为或风险，不复制实现细节。

| 实机入口（现有脚本） | 用途 / 额外条件 |
| --- | --- |
| scripts/validation/p2/check-windows.py | 主屏普通行为、拖拽、缩放、隐藏、边缘姿态 |
| check-settings.py | 预览/保存/重启、配置来源、坏配置、菜单与设置交接 |
| check-multi-display.py | 扩展桌面/混合 DPI；单屏只能记 skipped |
| check-offline.py --require-disconnected | 严格断网、独立用户目录、默认发布构建、离线普通功能 |
| check-migration.py | 隔离导入、冲突、中断重试和回滚 |
| check-modal-playback.py | 角色/托盘原生菜单、设置窗口拖动期间的播放 |
| check-peek-slide.py | 吸边/探头连续切换、相位与命中 |

所有脚本的精确参数先看 `--help`；文件位于 `scripts/validation/p2/`，阶段名称暂保留以兼容现有调用，不再新增 p3/p4 目录来复制相同用例。测试输出写新运行目录，避免覆盖证据。会自动操作桌面、注册表或隔离进程的脚本须在可控测试会话执行。

典型诊断构建与单屏回归：

```powershell
cargo build --release --locked --features development,window-diagnostics --target-dir target/window-diagnostics
uv run --locked python scripts/validation/p2/check-windows.py --exe target/window-diagnostics/release/desktop-pet-chatgpt.exe --fixtures artifacts/validation/p2/fixtures --output artifacts/validation/local/single-display-01
```

发布素材校验与装配：

```powershell
cargo build --release --locked --bin validate-pack
target/release/validate-pack.exe --distribution assets/packs/white-dragon/0.1.0
uv run --locked python scripts/release/assemble-asset-pack.py --validator target/release/validate-pack.exe --pack assets/packs/white-dragon/0.1.0 --output artifacts/dist/local/white-dragon-0.1.0.zip
```

输出名必须未占用。默认发布二进制另外用 `cargo build --release --locked` 构建；正式产物的独立构建与证据要求见[发布管理](release.md)。

## 工程规范化提案

继续单 Cargo 包，按模块边界拆职责。只在确有独立复用/发布需求时拆 workspace；不以文件行数或“跨平台”名义提前建立大量空 trait。

新增错误区分用户可处理、暂时不可用、永久不兼容和内部故障；展示给用户原因/下一步，日志记录结构化类别，避免泄漏路径、账户或正文。unsafe 局限平台边界并说明句柄所有权、线程要求和释放责任。

新增 CI 提案：PR/push 执行 fmt、clippy、默认与全 feature 测试、CPU 素材合成校验；平台抽离后在 Windows/Linux 跑纯逻辑与适用构建。GUI、性能和发布测试分开，不让每次文档修改启动两小时验收。当前工作流仍仅手动触发 Windows，本轮未改它。

## 文档与任务纪律

| 内容 | 放置位置 / 维护方式 |
| --- | --- |
| 用户当前能力、安装与限制 | README、docs/user；仅写已实现并声明支持的内容 |
| 当前格式和配置 | docs/development/runtime-v1.md，与代码同改 |
| 目标架构和取舍 | docs/design；明确提案/已决定，避免重复维护参数 |
| 当前路线与阻塞选择 | docs/plan/roadmap.md、decisions.md，滚动更新，不按每阶段另开日记 |
| 单次任务执行 | Issue/PR/工作记录；没有外部服务时放 artifacts/work-notes，不自动发送外部消息 |
| 原始验证日志/图像/实验包 | artifacts/validation、assets/work，Git 忽略 |
| 需长期共享的发布证据 | Release 或可持久访问的附件存储；仓库只留支持结论与哈希/定位 |

新任务先确定用户结果和停止条件，再写必要测试。方案试验结束将结论并入所属设计，不保留一串相互矛盾的“最新修订”。只有长寿命、跨模块且难以从代码理解的决策才值得独立 ADR；其余直接维护对应设计段落。

PR 描述写问题、最终行为、验证与限制；不复述调试流水账。提交粒度按可回退能力切片；遇到已有暂存修改，先明确基准并保留，不擅自 reset、混合提交或覆盖用户成果。
