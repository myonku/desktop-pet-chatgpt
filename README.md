# GPT 白龙娘桌宠

面向 Windows 的非官方桌宠项目。基础桌宠可离线运行，已支持透明动画、拖拽、自移动、吸边、初始缩放、托盘和隐藏。外部任务提醒、额度和会话交互是可选集成。首发目标为 Windows x64，优先验证 Windows 11。

## 当前运行方式

使用 rustup、Windows x64 MSVC 构建工具和 Windows SDK。Rust 版本由 `rust-toolchain.toml` 固定。在仓库根目录装配好本地原型素材后，复制示例为开发配置并启动。已有 `desktop-pet.dev.toml` 时可直接运行第二行。[原型素材装配](assets/README.md)

```powershell
Copy-Item desktop-pet.example.toml desktop-pet.dev.toml
cargo run --locked --features development
```

开发构建只读取启动工作目录的 `desktop-pet.dev.toml`，文件缺失时使用内置默认值，设置改动只影响本次会话。发布构建在 `%LOCALAPPDATA%/DesktopPetChatGPT/config.toml` 读取和保存用户偏好。两种构建共用 TOML 解析规则，所用文件无效时弹窗询问是否备份并用默认值覆盖，拒绝则退出。[配置规则](docs/G-05-contracts.md#配置分层)

开发构建从工程的 `assets/packs/white-dragon/0.1.0/` 加载默认素材。发布构建在安装后的 EXE 旁查找 `resources/default/`。设置 `pack_path` 时先尝试指定包，失败时再试对应的默认包。两处均不可用则退出。发布附件通过独立构建和装配流程产出，开发用的 `target/` 只存放编译输出。[发布装配约定](docs/G-05-contracts.md#源码程序与素材附件)

单击角色可互动，按住可见区域可拖动，靠近工作区左右边缘松手可吸附。右键和托盘共用菜单，可预览动作、切换音效、自移动和隐藏状态，或打开设置。设置支持实时预览、保存、恢复默认和 Windows 登录自启。可用 `preferences.start_hidden` 指定下次启动隐藏，通过托盘显示角色。鼠标穿透也可从托盘撤销。示例 TOML 列出了全屏隐藏、任务气泡和其他公开偏好。[配置示例](desktop-pet.example.toml)

## 导入旧版数据

退出桌宠后，在 PowerShell 运行相应的导入命令。上一阶段的 JSON 偏好和白龙桌宠的注册表偏好都写入新 `config.toml`，可按来源选择其中一项。历史可独立导入。

```powershell
.\desktop-pet-chatgpt.exe migrate config-json
.\desktop-pet-chatgpt.exe migrate bailong-preferences
.\desktop-pet-chatgpt.exe migrate bailong-history
```

导入保留原件，遇到已有目标时会报告冲突。命令返回的迁移 ID 可用于 `migrate rollback <migration-id>`。长期存储兼容仅承诺覆盖本仓库的历史版本。上游白龙桌宠 v0.5.1 导入是临时能力，两仓库的存储设计继续分化时可能在后续迭代移除。支持的来源、备份位置及回滚规则见 [G-05](docs/G-05-contracts.md#旧数据迁移)。

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
| `scripts/assets/`、`scripts/release/` | 素材转换与附件装配 |
| `scripts/validation/` | 合成输入、单屏和多屏回归、性能采样 |
| `tools/asset_optimizer/` | 离线素材优化模块 |
| `pyproject.toml`、`uv.lock` | Python 工具环境与依赖锁定 |
| [`assets/`](assets/README.md) | 本地素材原件和转换包 |
| `target/` | Cargo 默认输出 |
| [`artifacts/`](artifacts/README.md) | 阶段验证产物与待验收附件 |

## 版本规则

`Cargo.toml` 的包版本是版本来源。修复和兼容性小改动通常增加补丁位，新增能力或阶段性迁移通常增加次版本位。`0.x` 阶段的破坏性变更通过次版本和迁移说明标记。发布时同步更新锁文件、带 `v` 前缀的 Git 标签和发布说明。

正式素材附件与本仓库源码使用同一发布版本。本地素材候选可以自定版本，素材发布源为 `jingh12312-rgb/bailong-desktop-pet`。 [版本、来源与装配](docs/G-05-contracts.md#源码程序与素材附件)

## 许可


源码尚未指定开源许可证。第三方组件遵循各自许可证。

本仓库是基于上游项目进行技术研究与改进的非官方 fork。本仓库尚未收到原作者（[bilibili @風-隐，UID 290239611](https://space.bilibili.com/290239611)）的明确授权。我们尊重原作者及相关权利人的权益，不将未回复视为授权，也不据此推断作者拒绝引用。角色、美术、动画、音效及其转换版本的权利范围尚待逐项确认；本仓库不因公开展示、技术修改或署名而额外授予这些资源的使用、修改、商用或再分发许可。上游代码的许可状态与第三方软件许可另行说明。如权利人对相关内容有疑问，我们愿意沟通核实，并据核实结果调整展示、分发或移除相关内容。本项目非 OpenAI 官方产品。
