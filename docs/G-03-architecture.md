# 模块与架构设计

本文维护模块职责、数据契约和生命周期约束。

## 1. 依赖与职责

```text
winit / Windows events → app coordinator → core (纯状态与命令)
                              ↓ effects       ↑ domain events
                 ui / render / media / storage / integrations
                       ↓                         ↓
                 platform/windows       provider-specific adapters
```

core 不导入 winit、wgpu、Win32、SQLite 或具体 provider。app 编排模块依赖、调度及生命周期；模块通过数据与窄接口交互，不通过共享可变的“大 Pet 对象”。

| 现有入口 | 职责 |
| --- | --- |
| `src/main.rs`、`src/app/` | 启动参数、事件路由、窗口生命周期与恢复 |
| `src/media/`、`src/bin/validate-pack.rs` | 清单、资源解析、按页校验和解码、时间表、后台读页及离线整包校验 |
| `src/render/` | DX12 单槽纹理页、帧矩形采样、透明合成与内置占位 |
| `src/platform/windows/`、`src/ui/` | Win32 输入、DPI 与托盘，独立 egui 设置与中文预览 |
| `tests/`、`scripts/validation/` | 公开接口回归测试、合成输入及阶段采样 |

启动配置从文件选包，播放器按稳定动作 ID 选择时间表。应用将手势、时钟和窗口几何交给 core 的状态与路线规则，将结果应用到播放器、窗口和菜单。Windows 层负责抓取、屏幕探测、坐标换算和即时拖动。拖拽期间由原生输入独占位置写入，结束后应用用最新窗口位置判定吸边和后续路线。

P2-01 引入 core，P2-02 引入配置存储。迁移和 integrations 随后续节点实现。

| 模块（拟定路径） | 拥有的数据与职责 | 不承担 |
| --- | --- | --- |
| `src/main.rs`, `src/app/` | 启动、依赖组装、事件路由、退出与恢复 | 业务规则和协议解析 |
| `src/core/` | PetState、动画选择、移动轨迹、通知规则、配置模型 | IO、窗口/GPU 句柄 |
| `src/ui/` | 角色窗口输入、菜单命令、egui 设置/聊天/详情视图模型 | 数据库查询、阻塞解码 |
| `src/render/` | surface/device、WGSL、纹理页池、上传与绘制 | 动作选择、文件读取 |
| `src/media/` | 素材包解析、资源键解析、帧索引、预取、CPU staging | Win32 命中、任务状态 |
| `src/platform/windows/` | 托盘、命中、DPI/屏幕、全屏、单实例、自启、窗口适配 | 角色和 provider 业务 |
| `src/storage/` | 原子配置写、历史、迁移、水位、限量日志 | 直接更改 UI |
| `src/integrations/` | 能力与事件模型、适配器生命周期 | 桌宠移动和渲染 |
| `src/integrations/chatgpt_codex/` | 客户端探测、版本映射、只读任务源、会话传输 | 向 core 泄露私有 SQL/协议类型 |
| `src/bin/` | 离线资源转换/校验入口（P1b 按需增加） | 用户程序启动时转换所有素材 |

## 2. 核心契约（Rust 实现草案语义）

| 契约 | 输入 → 输出 | 不变量 |
| --- | --- | --- |
| Core reducer | `State + Event + monotonic_time → State + Effects + next_deadline` | 可注入时钟/随机源，无 IO；相同输入可复现 |
| AssetPack / ResourceResolver | `pack_root + semantic_key + optional client_id → resource / Missing` | 先校验 manifest；上层只依赖动作、图像、音效和文案的语义键 |
| AnimationCatalog | `action_id → metadata / Unavailable` | ID 稳定，检查移动时窗在动作时长内 |
| PlaybackController | `action + elapsed → FrameRequest` | 不以固定 16ms tick 决定素材帧率 |
| FrameSource | `Request(generation, action, frame, variant) → Ready / Failed` | 返回带 generation；取消后迟到结果可丢弃；有界字节预算 |
| Renderer | `FrameHandle + transform + viewport → Presented / SurfaceError` | 只在事件线程使用；不持有永久全量动作纹理 |
| PlatformServices | 屏幕/位置/命中/托盘命令 → 平台结果 | unsafe 留在边界，标明线程与释放责任 |
| IntegrationProvider | `probe() → identity + connection_state + capabilities`；`command → event` | 命令有 request_id；事件带 provider/session/turn；能力缺失明确返回 |
| HistoryStore | `load/append/checkpoint → result` | 单写者；坏文件不覆盖；版本超前拒绝写入 |

FrameHandle 含页 ID、页内区域、时间戳、逻辑画布与锚点；不能把纹理句柄暴露给 core。命中使用与显示变换一致的 CPU alpha mask，拖动命中区域和视觉透明独立验证。

core 提供手势判定、活动优先级、工作区约束和位移插值。应用协调素材目录、基础姿态回退和显示器状态。播放层根据选中动作的时间表发出帧请求，并报告一次性动作完成、资源就绪与失败。等待首帧和后续缺页各使用 750 ms 截止时间。等待时保留已呈现帧并冻结该动作时钟，失败后尝试可用基础动作，待机也失败时显示占位。动画变换由播放元数据和窗口几何共同确定，由呈现和命中共用。

### 首次尺寸与跨屏 DPI

`PlatformServices` 提供目标显示器标识、工作区、当前像素范围、有效 DPI，以及可选的可视区宽高（毫米）和数据来源。按窗口所在的 GDI 显示源查找唯一活动监视器接口，经 SetupAPI 读取设备 EDID 基本块并按当前模式旋转尺寸。校验 EDID 头、校验和、宽高范围与显示模式宽高比。探测在后台执行，首帧最多等 120 ms。未取到可信尺寸时采用 256×256 DIP。接口和尺寸字段见 [EnumDisplayDevicesW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumdisplaydevicesw)和 [WmiMonitorBasicDisplayParams](https://learn.microsoft.com/en-us/windows/win32/wmicoreprov/wmimonitorbasicdisplayparams)。

初始方形窗口基准取显示器可视高度的 16%，限于 36–90 mm，再根据显示模式和有效 DPI 换算。按初始缩放比例得到逻辑边长，窗口限制在工作区内。跨屏沿用该逻辑尺寸并应用新屏 DPI，画布在窗口内等比适配。Windows 有效 DPI 用于像素和 DIP 换算及后续跨屏呈现。依据 [Microsoft Learn 的 DPI 换算说明](https://learn.microsoft.com/en-us/windows/win32/learnwin32/dpi-and-device-independent-pixels)。

## 3. 线程、背压和取消

一个 winit 事件线程拥有全部窗口、egui 状态和 GPU 生命周期。media worker 负责读取/解码；storage worker 顺序写入；每个启用 provider 的 worker 处理阻塞传输与只读数据库。先标准线程，不引入完整异步运行时。

初始设计上限：控制通道 64 项；媒体完成队列 8 项且同时受 CPU 32 MiB 字节预算约束；进度采用每 turn 最新值合并。取消和退出使用独立原子标记/唤醒，不能排在大量进度之后。控制队列满时 UI 不阻塞等待：新命令返回忙并保留输入；不可丢的终态由 worker 保留待投递，超时连接失败并要求重同步。重同步按水位去重，不自动重发写操作。逐 provider 限单次活动命令与有界观察任务集，避免 worker 本地待发列表无界。

播放器同时允许一个未完成的媒体请求，常驻为显示页和一个待用页，GPU 使用一个纹理槽。更换槽前的同步 GPU 等待每次上限 250 ms，跨页等待与输入延迟按 [G-06](G-06-validation.md#3-性能实验与预算) 测量。

动作切换、窗口隐藏、资源更换增加 generation；旧结果允许回收但不展示。现有 generation 在每个页源内从零开始。更换页源或包时，由 app 用页源实例与代次联合过滤结果，过期结果只结算其对应的请求。GPU 页缓存键也须区分页所属的包与实际格式，避免复用同索引的旧纹理。CPU 字节预算包括在读、解码目标和待上传数据；申请预算成功后才分配，释放计费遵循所有权。GPU 素材预算初始 64 MiB；不包含 surface/UI/驱动，分别测量。单页大于预算直接拒绝该资源档，不以暂时超限加载解决。

事件循环等待输入或最早 deadline：素材下一帧、移动步进、必要的 UI 动画和后台结果。隐藏暂停媒体请求和上传；后台会话可继续，UI 隐藏不等于取消业务。恢复重置时间基准。最小化/零尺寸 surface 跳过提交。

## 4. 资源契约 v1

[G-05](G-05-contracts.md#素材包-v1) 维护包格式、路径、校验与分发规则。core、渲染和 UI 使用语义键或解析后的数据，包内路径由读取端处理。应用在后台检查清单，媒体线程按页校验和解码，再提交 GPU 上传；内置样本承担启动和故障回退。重启时可换包。后续实现运行中切换时沿用 generation。

展示资源区分通用界面、角色素材和客户端覆盖项。权限说明与错误语义由程序从结构化状态生成，素材包提供的展示文案只用于呈现。

## 5. 存储与迁移

应用层区分已保存的配置和当前预览值，领域模块接收已校验的配置。关闭设置回到最近已确认的值。窗口缩放保存基准 DIP 与百分比，从而保留逻辑边长。设置界面通过同一配置模型应用运行中变更，由存储层保存用户值。一键恢复和首次启动共享内置默认值。开发覆盖随开发会话生效，与用户配置写入分离。

产品用户根为 `%LOCALAPPDATA%/DesktopPetChatGPT/`，通过平台服务解析。安装目录按只读处理。`config.json`、`history/`（按 provider/session 隔离）、`state/`（通知水位）、`logs/`、`cache/` 与 `backups/` 分离。开发 TOML 与产品 JSON 的用途、路径基准和覆盖规则见 [G-05](G-05-contracts.md#配置分层)。

首期历史按会话 JSON 快照实现，单 worker 写入，内存/UI 只载入选中会话及有限预览；单会话文件初始上限 16 MiB，超限提示归档并新建会话，不静默删旧消息。日志初始 3×2 MiB 滚动；正文不进入日志。文件路径使用内部 ID 编码，不能直接拼接外部 session 字符串。

保存：同目录临时文件 → 写完并 flush → 平台原子替换 → 成功后更新内存持久化版本。失败保留原文件、提示未保存，不无限重试。迁移由用户明确触发：识别旧来源/格式 → 备份与哈希 → 转换到新目录 → 重读校验 → 标记迁移版本；幂等重试。实施迁移时核验实际来源、路径和格式，按验证结果维护兼容范围。中断恢复时将旧 running 标为 interrupted，不继续工作。新 schema 不供旧程序读取；回退靠旧文件和备份。

## 6. 集成与隐私边界

连接状态：Disabled → Probing → Ready（带 capability set）或 Unavailable/NeedsAuth/Unsupported；失败可退避重试，认证/协议不支持不自动循环重试。会话状态：Idle → Running → WaitingApproval/WaitingInput → Completed/Failed/Cancelled；本地取消先标 Cancelling，等待确认；断连未确认则 Interrupted，不能伪称成功取消。

沿用 `IntegrationProvider`，由 app 组装具体适配器。用稳定 `provider_id` 隔离连接、历史和事件，用 `client_id` 选择展示资源，显示名称可独立变化。ChatGPT 桌面端为首个产品目标，Codex 是需验证的接入路径。P3 用合成适配器验证同一命令、事件与能力模型可接入第二种实现，DSH 在后续独立验证。适配器负责把协议与版本差异映射为领域状态，核心与 UI 根据统一能力和结构化事件工作。断连或切换适配器时失效旧请求与实时状态，迟到事件按来源和连接代次过滤。

P2 仅在共享菜单、任务状态选择和素材解析中预留能力状态、动作意图和可选客户端标识。P3 再按已验证接口确定传输及落盘字段。显示器、窗口与输入的领域数据由 Windows 边界转换。当前渲染后端和事件循环仍绑定 Windows，多平台立项需单独验证窗口合成、输入命中与恢复策略。

观察源始终只读、短查询、有超时；schema 探测失败禁用观察能力，不创建/迁移外部数据库，不读取不必要字段。主动会话与外部观察分开命名空间，并排除本桌宠已追踪会话的重复提醒。每次事件应用前核对 session/turn/generation，过期响应不串入新会话。

请求审批展示来源、目标与权限，绑定 approval_id + session + turn，仅用户回答该次请求后发送响应；未知请求、窗口关闭或缺失处理器均不自动同意。凭据沿用用户客户端登录或系统凭据引用，不复制到本项目配置；不记录原始协议帧。P3 对锁定的接口编写契约测试，协议字段未核实前不视为正式 API。

## 7. 故障恢复与生命周期

启动顺序：单实例 → 路径/配置 → 创建窗口/托盘和 GPU → 占位首帧。配置确定素材来源后并行检查包清单及读取首个待机页；待机就绪后替换占位。可选集成仅在用户启用后启动。GPU 丢失暂停提交并重建设备/页池一次；重复失败进入可见诊断状态，不忙循环重启。具体 surface 错误分类在 P1a 按版本确认。

退出停止接收命令 → 取消媒体/集成 → 保存状态 → 回收自有子进程与 worker → 释放 GPU/窗口。退出等待设计上限 5s，写入与子进程各有有限期限；仅终止本程序创建的子进程，不关闭用户外部客户端。超时记录脱敏原因，保留已有稳定数据。测试涵盖 stderr 持续读取、stdout 关闭、进程崩溃及迟到事件。

Windows 通过按用户配置根隔离的本地会话互斥量限制实例。第二次启动查找同一配置根的角色窗口，请求打开设置，窗口暂不可达时给出已运行提示。退出时停止音效、取消页请求、关闭请求通道并有界回收页读取与装包 worker。过期页结果仍由页源标识和代次拦截。

## 8. 延后扩展的接入边界

- 第二客户端及 DSH 沿用能力探测、来源隔离和展示资源解析边界。可行性以各自的接口与权限验证为准。
- 运行时换包可复用包校验与页源失效机制。多个角色同时运行需要另定实例、窗口、托盘命令路由及共享资源预算。首期保留单角色状态模型。
- 在线素材获取可沿用解包根、版本、校验和来源记录；安装器与程序更新依赖 P4 的版本、附件及回退约定。跨设备历史需单独设计数据同步及冲突处理。完整 Markdown 和终端视图需单独定义内容呈现边界。
