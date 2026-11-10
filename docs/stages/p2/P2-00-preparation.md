# P2-00 · 实施准备

2026-11-10 完成。以 `17e47dc` 为基线，工程调整和本记录同一提交。

- 拆分应用启动、媒体清单、包读取、播放与参照模块，分离公开接口测试和模块内部测试，提供独立的素材附件装配器。
- 建立连续的常驻文档编号及阶段文档目录。P0 和 P1 已验收记录归档，仍生效的素材、配置与验收约定同步到常驻文档。
- 清理原 `target/` 中的历史产物和构建缓存，恢复本地及 CI 的 Cargo 默认输出。验证和附件留在 `artifacts/`。
- 将 39 个素材及配套清单、来源和许可文件复制到 `assets/source/white-dragon/`，逐文件 SHA-256 一致。素材本体留本地。
- 约定产品偏好、运行时设置及独立的开发覆盖。字段与配置读取在对应功能实施时落地。

## 本地验证

Windows 11 10.0.22631、Rust 1.99.1 MSVC、Python 3.14.7。播放使用 Intel Core Ultra 7 155H 的核显，DX12 驱动 `32.0.101.8860`，DPI 125%。

| 检查 | 结果 |
| --- | --- |
| `cargo metadata --locked --no-deps --format-version 1` | `target_directory` 为仓库根 `target/` |
| `cargo fmt --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings` | 通过 |
| `cargo test --locked`、`cargo test --locked --all-features` | 各 6 项通过 |
| `cargo build --release --locked` | 通过 |
| `validate-pack --distribution`、附件装配及解包复验 | `dawn`、`dusk` 合成包校验通过。`dawn` 三档与 PNG-only 附件解包后均通过校验，PNG-only 附件排除了未声明的页载荷 |
| Release 合成包短测 | 预热 2 秒，采样 3 秒。三档附件选用 PNG，呈现 90/90 帧，素材首帧 536.75 ms。PNG-only 附件呈现 91/91 帧，素材首帧 362.41 ms |

装配与采样命令按 [G-05](../../G-05-contracts.md#工程与产物) 执行。本地输出在 `artifacts/validation/p2/runs/preparation-20261110/`，`playback/run.json` 和 `playback-png-only/run.json` 记录 EXE 及素材清单哈希。三档 ZIP SHA-256 为 `57ddb257154a18d71cf9c78dfcf095c573f9cf6b3f965d90b48c7af20f5829d6`，PNG-only ZIP 为 `029348bda81f9e21cfbb283ec16e4dda12f8ac62094c866ab8bfcc0f1c563e39`。

本次覆盖工程整理及合成包冒烟。P2 功能验收、正式分发验证和本次提交的托管 Windows CI 仍待执行。下一节点为 P2-01。
