# GPT 白龙娘桌宠

面向 Windows 的非官方桌宠项目。基础桌宠可离线运行，计划支持透明动画、拖拽、自移动、吸边、缩放、托盘、隐藏和音效。外部任务提醒、额度和会话交互是可选集成。首发目标为 Windows x64，优先验证 Windows 11。

## 当前运行方式

使用 rustup、Windows x64 MSVC 构建工具和 Windows SDK。Rust 版本由 `rust-toolchain.toml` 固定。在仓库根目录运行：

```powershell
cargo run --locked
```

默认显示内置透明色阶样本。按住可见区域拖动，右键打开菜单，可打开独立的中文输入预览窗口。开启鼠标穿透后，使用系统托盘里的 Desktop Pet ChatGPT 菜单恢复交互或退出。

可在启动前将 `DESKTOP_PET_PACK` 指向包含 `manifest.json` 的本地解包目录。`DESKTOP_PET_FORMAT` 默认为 `png`，可选 `rgba8` 或 `bc7`。合成包验证、素材工作区和附件装配见 [G-05](docs/G-05-contracts.md)。

## 文档索引

| 常驻文档 | 维护范围 |
| --- | --- |
| [G-01 总体设计](docs/G-01-master-plan.md) | 产品范围、技术方向、风险 |
| [G-02 功能规格](docs/G-02-feature-spec.md) | 用户行为、需求 ID、产品缺省值 |
| [G-03 模块架构](docs/G-03-architecture.md) | 模块职责、线程、资源与数据流 |
| [G-04 主线计划](docs/G-04-delivery-plan.md) | 阶段依赖、交付物和验收关口 |
| [G-05 工程与交付约定](docs/G-05-contracts.md) | 文档维护、工程布局、配置、素材与发布装配 |
| [G-06 验证矩阵](docs/G-06-validation.md) | 用例、测量口径、预算与发布门槛 |

[当前阶段](docs/stages/README.md)维护任务进度和验证记录。[阶段归档](docs/archive/README.md)保留已结项的记录。

| 路径 | 用途 |
| --- | --- |
| `src/`、`tests/` | 程序、素材校验器与测试 |
| `scripts/validation/`、`scripts/release/` | 合成输入、验证采样与附件装配 |
| [`assets/`](assets/README.md) | 本地素材原件和转换包 |
| `target/` | Cargo 默认输出 |
| [`artifacts/`](artifacts/README.md) | 阶段验证产物与待验收附件 |

## 版本规则

`Cargo.toml` 的包版本是版本来源。修复和兼容性小改动通常增加补丁位，新增能力或阶段性迁移通常增加次版本位。`0.x` 阶段的破坏性变更通过次版本和迁移说明标记。发布时同步更新锁文件、带 `v` 前缀的 Git 标签和发布说明。

## 许可


源码尚未指定开源许可证。第三方组件遵循各自许可证。

本仓库是基于上游项目进行技术研究与改进的非官方 fork。本仓库尚未收到原作者（[bilibili @風-隐，UID 290239611](https://space.bilibili.com/290239611)）的明确授权。我们尊重原作者及相关权利人的权益，不将未回复视为授权，也不据此推断作者拒绝引用。角色、美术、动画、音效及其转换版本的权利范围尚待逐项确认；本仓库不因公开展示、技术修改或署名而额外授予这些资源的使用、修改、商用或再分发许可。上游代码的许可状态与第三方软件许可另行说明。如权利人对相关内容有疑问，我们愿意沟通核实，并据核实结果调整展示、分发或移除相关内容。本项目非 OpenAI 官方产品。
