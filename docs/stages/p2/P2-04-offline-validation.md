# P2-04 · 离线验收

2026-11-12，在 P2-03 检查点 `051d2e7` 上完成离线构建、Windows 桌面回归和本地运行检查点。断网环境实测待执行。

## 检查点与复测

`artifacts/validation/p2/runs/offline-checkpoint-1794468940996/p2-checkpoint.zip` 包含默认 feature 的 Windows x64 Release 和自有十动作合成包。整包解压后运行 `desktop-pet-chatgpt.exe`。该 ZIP 用于本地验收。ZIP 为 4,144,307 字节，SHA-256 为 `9f396b17093263256d8a578bfdeb39c74906e9dc1ba74d918fceeb8364ae90b6`。程序 SHA-256 为 `69e2367e46344b5b321e73d6dfce85eb0317ae810475784e61089b9fc8285173`。

使用已有依赖缓存和合成包，在外部客户端不可用的交互式 Windows 环境中从仓库根运行。严格断网模式要求运行前后均无活动物理网卡和已连接接口的默认路由。测试使用独立配置与环境。

```powershell
cargo fmt --all --check
cargo clippy --offline --locked --all-targets --all-features -- -D warnings
cargo test --offline --locked
cargo test --offline --locked --all-features
cargo build --offline --release --locked
python scripts/validation/p2/generate-fixtures.py
$run = 'artifacts/validation/p2/runs/offline-recheck'
python scripts/validation/p2/check-offline.py --exe target/release/desktop-pet-chatgpt.exe --validator target/release/validate-pack.exe --fixtures artifacts/validation/p2/fixtures --output $run --require-disconnected
```

带 `--representative <目录>` 时，资源采样使用指定包。默认启动与功能检查仍使用合成包。代表包的本地构建方式见 [P2-01](P2-01-core-behavior.md#复测命令)。设置操作、音效、混合 DPI 和坏包路径使用 [P2-02 的回归命令](P2-02-platform-settings.md#复测)。迁移使用 [P2-03 的复测命令](P2-03-storage-migration.md#复测)。

## 本地结果

Windows 11 22631，Core Ultra 7 155H，Intel 核显驱动 32.0.101.8860，DX12 DirectComposition，主屏 125% DPI。Rust 1.99.1 MSVC，Python 3.14.7。格式、Clippy、锁定离线构建和两组各 28 项测试通过。

默认发布构建在独立用户目录及禁止写入的安装目录通过十次启动、透明命中、拖拽、吸边和展开、自移动、托盘与全屏隐藏及恢复、设置打开、偏好写入和重启。测得首个可见素材帧中位数 316.65 ms，范围 307.97–642.73 ms，系统文件缓存保留。十次拖拽在 25.20–26.21 ms 内观察到窗口位移，采样以约 25 ms 轮询，正式输入到可见反馈的延迟分位数随 P4 测量。设置、音效、双屏与坏包回归沿用同一源码的 [P2-03 回归证据](P2-03-storage-migration.md#本地验收)。

| 默认发布构建，代表包 PNG | 单核 CPU | Private bytes 峰值 | Working set 峰值 |
| --- | ---: | ---: | ---: |
| 可见待机，15.21 秒 | 2.26% | 465.15 MiB | 447.60 MiB |
| 设置打开，15.18 秒 | 1.24% | 976.45 MiB | 663.73 MiB |
| 关闭设置后隐藏，15.19 秒 | 0.00% | 620.83 MiB | 569.37 MiB |

代表包参照构建的 PNG 短测预热 2 秒、采样 12 秒，呈现 288/288 帧，跨页准备 p95 为 976 µs。WDDM 13 个样本中，3D 引擎均值 2.25%，共享显存计数 394.95 MiB。此计数包含应用和驱动开销。隐藏阶段保留内存及多次开关的稳态留到 P4 复测。

| 素材覆盖 | 产品核心动作 ID | 代表素材体积 |
| --- | --- | --- |
| 自有合成包 | 10/10 | 见检查点及合成附件 |
| 本地六动作代表包 | 6/10。覆盖待机、互动、拖拽、落地和左右移动。缺少左右吸边与探头素材，使用现行待机降级 | 三档解包 527.90 MiB、ZIP 301.12 MiB。仅 PNG 解包 128.93 MiB、ZIP 128.72 MiB |

两套代表包均为 205 页、817 帧，通过 `validate-pack --distribution`。当前代表包的解包与 ZIP 均超过 80 MiB 原型包体预算。P4 按实际获准的素材范围核定全量动作、压缩、装配、授权、另一 GPU、干净机和两小时常驻。

## 断网关口

本地验证通过隔离目录、仅系统目录的 PATH 和默认构建完成。代表包采样时未观察到桌宠子进程、TCP 连接或 UDP 端点。宿主 WLAN 与隧道路由仍活动。严格模式实测在启动桌宠前正确停止。试用无网络能力的 AppContainer 时，系统拒绝素材路径的规范化查询，桌宠停在素材加载阶段。当前权限无法设置针对测试程序的防火墙规则。

证据在 `artifacts/validation/p2/runs/offline-checkpoint-1794468940996/`，包括 `result.json`、进程采样 CSV、`representative-size.json` 和 `playback/result.json`。断网门槛的预检证据在 `offline-guard-1794469173177/network-before.json`。实际断网、无外部客户端的交互验证和托管 Windows CI 尚待执行。
