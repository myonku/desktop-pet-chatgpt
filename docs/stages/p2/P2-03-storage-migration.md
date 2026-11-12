# P2-03 · 存储迁移

2026-11-12 在 `5ee3324` 上实施。范围覆盖 F-09、F-15 和 F-16，包含偏好与历史的显式导入、备份、回滚和中断重试。

## 来源与映射

上一阶段 Rust 配置源由 `ebf6b58:src/storage/mod.rs` 核对。JSON schema 1 中的启动比例、保存尺寸、素材路径和功能偏好沿用当前产品规则。迁移到不同目录时按原配置目录解析相对素材路径。

白龙桌宠 v0.5.1 来源由 `852f4f4` 的 `app/pet.py`、`app/platform_support.py`、`app/chat_backend.py` 和 `app/chat_ui.py` 核对。旧版固定 PySide6 6.9.2。在独立 Windows 注册表键用该版本实际写入，确认比例为 `REG_DWORD`、布尔值为字符串 `true` 与 `false`。读取只涉及当前用户应用键。Qt 的注册表编码见 [Qt 6.9.2 源码](https://raw.githubusercontent.com/qt/qtbase/v6.9.2/src/corelib/io/qsettings_win.cpp)。

| 旧偏好 | TOML 字段 |
| --- | --- |
| `sizePercent` | `initial_scale_percent`，范围 50–200 |
| `bubblePercent` | `preferences.bubble_scale_percent`，范围 50–150 |
| `autoHideFullscreen` | `preferences.fullscreen_hide` |
| `autoMovementDisabled`、`soundDisabled`、`taskBubblesDisabled` | 取反后分别写入 `preferences.auto_move`、`preferences.sound`、`preferences.task_bubbles` |

历史按旧会话 ID 的 SHA-256 命名，存入独立 provider 目录。转存的会话保留标题、工作目录、原线程标识、模式、提示、结果和可读事件。未完成的状态转为 `interrupted` 并保留原状态。通知水位等字段留在原始备份，真实会话恢复与历史界面随 P3 实现。完整命令与冲突语义见 [G-05](../../G-05-contracts.md#旧数据迁移)。

## 复测

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --all-features
cargo build --locked
cargo build --locked --release
$run = 'artifacts/validation/p2/runs/migration-recheck'
python scripts/validation/p2/check-migration.py --exe target/release/desktop-pet-chatgpt.exe --pack artifacts/validation/p1/fixtures/dawn --output $run
```

`dawn` 由 [P1 合成包脚本](../../../scripts/validation/p1/generate-p1b-fixtures.py) 生成。历史样本由旧版 `HistoryStore` 写入合成对话。复测 Qt 写入格式时，在独立目录安装旧版固定的 `PySide6-Essentials==6.9.2`，并给脚本传入 `--qt-runtime <目录>`。验收使用隔离的产品目录和注册表键。

## 本地验收

Windows 11 22631，Rust 1.99.1 MSVC，Python 3.14.7。默认及全 feature 测试各 28 项。专项结果见 `artifacts/validation/p2/runs/migration-qt-1794466348529/result.json`，发布构建 SHA-256 为 `69e2367e46344b5b321e73d6dfce85eb0317ae810475784e61089b9fc8285173`。

| 验收项 | 结果 |
| --- | --- |
| 偏好 | JSON 导入后发布构建读取保存的 264 DIP 尺寸。Qt 原始值在隔离注册表读写复测通过。已保存比例及开关转换正确，跨目录解析相对素材路径通过 |
| 历史 | 原写入器生成两份会话，完成结果保留，`running` 转为 `interrupted`。路径分隔符和穿越形式的会话 ID 仍生成固定长度的散列文件名 |
| 冲突与重试 | 运行中的桌宠阻止同目录迁移，已有 TOML 或历史目标保留。发布前中断和发布后未标记的两种状态均可重试。同一输入不产生重复结果 |
| 损坏和回滚 | 截断的历史备份逐字节一致，修复后可导入。超前配置、重复会话 ID、被占用的来源及损坏的备份均安全停止。回滚保留原件，并在目标存在编辑或新增文件时拒绝移动 |

验证使用合成数据和隔离用户目录，托管 Windows CI 待触发。P2-04 负责断网整体验收。

同一实现的 Windows 设置与双屏行为回归均通过。报告位于 `artifacts/validation/p2/runs/migration-regression-1794466416673/` 的 `settings/result.json` 和 `core/result.json`。
