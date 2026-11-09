# 模块与架构设计

基线日期为 2026-11-07，补充设计于 2026-11-09 更新。下文为目标结构，现有 P1a 原型入口仍在 `src/app.rs`、`src/render.rs`、`src/sample.rs` 和 `src/platform/windows.rs`。

## 1. 依赖与职责

```text
winit / Windows events → app coordinator → core (纯状态与命令)
                              ↓ effects       ↑ domain events
                 ui / render / media / storage / integrations
                       ↓                         ↓
                 platform/windows       provider-specific adapters
```

core 不导入 winit、wgpu、Win32、SQLite 或具体 provider。app 编排模块依赖、调度及生命周期；模块通过数据与窄接口交互，不通过共享可变的“大 Pet 对象”。

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

### 首次尺寸与跨屏 DPI

`PlatformServices` 提供目标显示器标识、工作区、当前像素范围、有效 DPI，以及可选的可视区宽高（毫米）和数据来源。P2 可调查 Windows 公开的 EDID/WMI 尺寸数据，以实际显示目标为准匹配监视器实例并考虑旋转。遇到缺失、零值、异常、实例匹配不唯一或查询超时，返回尺寸未知并采用 256×256 DIP。探测应有界，启动回退可直接出首帧。依据 [Microsoft Learn 的 EDID 尺寸字段](https://learn.microsoft.com/en-us/windows/win32/wmicoreprov/wmimonitorbasicdisplayparams)。

候选尺寸算法以可视区物理高度乘目标比例，先限制在可用的物理大小区间，再按当前显示模式下的像素密度换算到窗口像素与 DIP。保持画布纵横比，并按工作区约束尺寸和位置。比例、物理上下限及数据可靠性在 P2 实测后决定。Windows 有效 DPI 用于像素和 DIP 换算及后续跨屏呈现。依据 [Microsoft Learn 的 DPI 换算说明](https://learn.microsoft.com/en-us/windows/win32/learnwin32/dpi-and-device-independent-pixels)。

## 3. 线程、背压和取消

一个 winit 事件线程拥有全部窗口、egui 状态和 GPU 生命周期。media worker 负责读取/解码；storage worker 顺序写入；每个启用 provider 的 worker 处理阻塞传输与只读数据库。先标准线程，不引入完整异步运行时。

初始设计上限：控制通道 64 项；媒体完成队列 8 项且同时受 CPU 32 MiB 字节预算约束；进度采用每 turn 最新值合并。取消和退出使用独立原子标记/唤醒，不能排在大量进度之后。控制队列满时 UI 不阻塞等待：新命令返回忙并保留输入；不可丢的终态由 worker 保留待投递，超时连接失败并要求重同步。重同步按水位去重，不自动重发写操作。逐 provider 限单次活动命令与有界观察任务集，避免 worker 本地待发列表无界。

动作切换、窗口隐藏、资源更换增加 generation；旧结果允许回收但不展示。CPU 字节预算包括在读、解码目标和待上传数据；申请预算成功后才分配，释放计费遵循所有权。GPU 素材预算初始 64 MiB；不包含 surface/UI/驱动，分别测量。单页大于预算直接拒绝该资源档，不以暂时超限加载解决。

事件循环等待输入或最早 deadline：素材下一帧、移动步进、必要的 UI 动画和后台结果。隐藏暂停媒体请求和上传；后台会话可继续，UI 隐藏不等于取消业务。恢复重置时间基准。最小化/零尺寸 surface 跳过提交。

## 4. 资源契约 v1 草案

manifest 使用 JSON，必须有 `schema_version`、`pack_id`、`pack_version`、源文件哈希、转换工具版本与参数、色彩空间、alpha 约定、动作列表和页校验值。每动作包含 ID、逻辑尺寸、帧数、逐帧时间/持续时间、循环策略、锚点/偏移、方向能力与移动时窗。每帧引用页和页内区域，分页格式由 P1b 冻结。

manifest 通过稳定语义键索引静态图、动画、音效和纯文本，文件路径相对包根。由 app 在启动时按配置选择素材包，读取端校验完整性后才启用。core、渲染和 UI 传递语义键或已解析资源，文件路径与包内目录由读取端处理。保留内置自有占位资源作为启动回退。素材包采用版本化的数据格式，由程序内置的读取器校验。首期通过配置和重启验证整包替换，运行中切换沿用 generation 机制作为后续扩展点。

展示资源按通用界面、角色素材和客户端专属资源分开。客户端显示名、标识、连接提示和任务文案可用 `client_id` 选用对应覆盖项，再回退通用资源。缺少可选声音时静音。审批权限说明与错误语义由程序根据结构化状态生成，素材包提供的展示文案不参与权限或能力判定。

读取端拒绝未知主 schema、重复动作 ID、越界帧/UV、负或零帧时长、整数溢出、超限尺寸、错误校验值以及逃出素材根的路径。压缩包在 P1b 决策前按“本地解包目录”实现最小基线；若引入解包，禁止绝对路径、`..` 穿越、链接逃逸和无界展开量。

工作色彩/alpha 约定必须从解码到 shader 到 surface 成套记录，不在规格里猜测某个 API 默认值。BC7/PNG 共用动作和时间元数据；缺 BC 支持选择已随包提供的兼容档，未提供则回到占位并提示，不运行时静默转换整包。

## 5. 存储与迁移

默认用户根拟定 `%LOCALAPPDATA%/DesktopPetChatGPT/`；通过平台服务解析，安装目录只读。`config.json`（schema_version）、`history/`（按 provider/session 隔离）、`state/`（通知水位）、`logs/`、`cache/` 与 `backups/` 分离。素材包位置通过配置指定，默认从安装目录旁的 `resources/` 解析。开发目录 `assets/` 用作本地工作区。

首期历史按会话 JSON 快照实现，单 worker 写入，内存/UI 只载入选中会话及有限预览；单会话文件初始上限 16 MiB，超限提示归档并新建会话，不静默删旧消息。日志初始 3×2 MiB 滚动；正文不进入日志。文件路径使用内部 ID 编码，不能直接拼接外部 session 字符串。

保存：同目录临时文件 → 写完并 flush → 平台原子替换 → 成功后更新内存持久化版本。失败保留原文件、提示未保存，不无限重试。迁移由用户明确触发：识别旧来源/格式 → 备份与哈希 → 转换到新目录 → 重读校验 → 标记迁移版本；幂等重试。旧版 QSettings、JSON 与实际路径在 P0 盘点，不臆测全部位于一个目录。中断恢复时将旧 running 标为 interrupted，不继续工作。新 schema 不供旧程序读取；回退靠旧文件和备份。

## 6. 集成与隐私边界

连接状态：Disabled → Probing → Ready（带 capability set）或 Unavailable/NeedsAuth/Unsupported；失败可退避重试，认证/协议不支持不自动循环重试。会话状态：Idle → Running → WaitingApproval/WaitingInput → Completed/Failed/Cancelled；本地取消先标 Cancelling，等待确认；断连未确认则 Interrupted，不能伪称成功取消。

沿用 `IntegrationProvider`，由 app 组装具体适配器。用稳定 `provider_id` 隔离连接、历史和事件，用 `client_id` 选择展示资源，显示名称可独立变化。ChatGPT 桌面端为首个产品目标，Codex 是需验证的接入路径。P3 用合成适配器验证同一命令、事件与能力模型可接入第二种实现，DSH 在后续独立验证。适配器负责把协议与版本差异映射为领域状态，核心与 UI 根据统一能力和结构化事件工作。断连或切换适配器时失效旧请求与实时状态，迟到事件按来源和连接代次过滤。

观察源始终只读、短查询、有超时；schema 探测失败禁用观察能力，不创建/迁移外部数据库，不读取不必要字段。主动会话与外部观察分开命名空间，并排除本桌宠已追踪会话的重复提醒。每次事件应用前核对 session/turn/generation，过期响应不串入新会话。

请求审批展示来源、目标与权限，绑定 approval_id + session + turn，仅用户回答该次请求后发送响应；未知请求、窗口关闭或缺失处理器均不自动同意。凭据沿用用户客户端登录或系统凭据引用，不复制到本项目配置；不记录原始协议帧。P3 对锁定的接口编写契约测试，协议字段未核实前不视为正式 API。

## 7. 故障恢复与生命周期

启动顺序：单实例 → 路径/配置 → 资源校验 → 窗口/托盘 → GPU/占位首帧 → 可选 worker。GPU 丢失暂停提交并重建设备/页池一次；重复失败进入可见诊断状态，不忙循环重启。具体 surface 错误分类在 P1a 按版本确认。

退出停止接收命令 → 取消媒体/集成 → 保存状态 → 回收自有子进程与 worker → 释放 GPU/窗口。退出等待设计上限 5s，写入与子进程各有有限期限；仅终止本程序创建的子进程，不关闭用户外部客户端。超时记录脱敏原因，保留已有稳定数据。测试涵盖 stderr 持续读取、stdout 关闭、进程崩溃及迟到事件。
