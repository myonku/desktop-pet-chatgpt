# 开发与维护工作流

## 构建当前 Windows 版本

使用 rustup、MSVC 构建工具和 Windows SDK。版本由 rust-toolchain.toml / Cargo.lock 固定。Python 工具由 pyproject.toml / uv.lock 管理，普通程序运行无需 Python。

GUI、换包和安装回归使用 `schema_version = 2`、`engine_api = 2` 的 PNG 素材包。将已制备的白龙十动作默认包放在 `assets/packs/white-dragon/0.1.0-v2/`，或按下文从上游源素材制备。产品构建从 EXE 旁的 `resources/default/` 加载默认包。旧开发 ZIP 须使用现行清单重新装配。 [素材契约](runtime-v2.md#素材清单)

```powershell
if (-not (Test-Path -LiteralPath desktop-pet.dev.toml)) { Copy-Item desktop-pet.example.toml desktop-pet.dev.toml }
cargo run --locked --features development
```

已有开发配置保留，`pack_path` 可选择另一份已制备的 v2 包。开发设置只影响本次运行。持久化验证使用产品构建和隔离测试目录。 [素材制备](../../assets/README.md)

在 Linux 或 WSL 装好 Rust 1.99.1 和本机链接器后，可用 `cargo +1.99.1 test --locked --all-features` 运行共享核心、配置和素材测试。显式工具链选择绕开仓库工具链文件的 Windows 目标列表。Windows 专有的登记、窗口与托盘测试在 Windows 运行。

## 检查与回归

```powershell
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --all-features
uv run --locked --group optimize python scripts/validation/check-asset-optimizer.py
```

依赖已缓存时可加 `--offline`。按变更选择检查：文档检查链接与事实；纯逻辑跑相关 Rust 测试；素材工具跑合成检查；窗口/调度/命中变化补对应 GUI 场景。通过后不无理由重复全套；每个测试应覆盖行为或风险，不复制实现细节。

| Windows 实机入口 | 用途 |
| --- | --- |
| check-winui.py | 设置、快捷面板首显、草稿、托盘和反复开关，支持启动与管道故障注入 |
| check-chatgpt.py | ChatGPT 连接复选框、双击速览与自动刷新、真实套餐与额度、需认证状态、详情与清除缓存。可加第二个合成来源复验切页与隔离 |
| check-pack-switch.py | 正式包检查、保存与重启，及已选包的后续待机页损坏恢复 |
| check-offline.py | 正式包解包、只读安装、启动和托盘，支持 `--require-disconnected` |
| check-modal-playback.py | 快捷面板、原生托盘菜单和拖动设置窗口期间的播放 |
| check-peek-slide.py | 吸边、探头相位和实际命中 |
| check-multi-display.py | 扩展桌面和混合 DPI，设备变更时补测 |

实机脚本位于 `scripts/validation/windows/`。一般用 `--pack` 指定正式 v2 包，换包回归用 `--default-pack` 和 `--alternate-pack` 指定两份正式 v2 包。操作 WinUI 控件的脚本使用 `uiautomation` 和 `psutil`。输出写新运行目录。编解码和存储单元测试可构造最小输入；这些输入不作为运行素材包。

从原始素材清单和透明 WebM 直接生成十动作默认 v2 包。先按 [素材目录](../../assets/README.md) 准备源素材和本地 FFmpeg。输出版本使用 SemVer，输出目录和 ZIP 均使用新路径。

```powershell
cargo build --locked --release --bin validate-pack
uv run --locked --group optimize python -m tools.asset_optimizer.pack `
  --source 'assets/source/white-dragon/白龙桌宠素材/素材清单.json' `
  --ffmpeg assets/work/tools/ffmpeg.exe --validator target/release/validate-pack.exe `
  --upstream-release https://github.com/jingh12312-rgb/bailong-desktop-pet/releases/tag/v0.5.1 `
  --pack-version 0.1.0-v2 --size 320 --batch 4 --profile contour --default-actions `
  --click assets/source/white-dragon/sounds/click.wav `
  --output assets/packs/white-dragon/0.1.0-v2
target/release/validate-pack.exe --distribution assets/packs/white-dragon/0.1.0-v2
uv run --locked python scripts/release/assemble-asset-pack.py --validator target/release/validate-pack.exe --pack assets/packs/white-dragon/0.1.0-v2 --output artifacts/dist/local/white-dragon-0.1.0-v2.zip
```

默认发布二进制用 `cargo build --release --locked` 构建，安装时把素材解压至 EXE 旁的 `resources/default/`。正式产物的独立构建与证据要求见 [发布管理](release.md)。

## WinUI 3 面板验证

Windows 构建默认使用 `windows-reactor` 0.100.0。运行面板需要与 EXE 架构一致的 Windows App Runtime 2（最低框架版本 2.4）。本地开发附件使用预装运行时。 [Windows App SDK 下载](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/downloads)

```powershell
Get-AppxPackage Microsoft.WindowsAppRuntime.2 | Select-Object Version, Architecture
cargo build --release --locked --features window-diagnostics
uv run --isolated --with uiautomation==2.0.29 --with psutil==7.1.3 python scripts/validation/windows/check-winui.py `
  --exe target/release/desktop-pet-chatgpt.exe --pack assets/packs/white-dragon/0.1.0-v2 `
  --output artifacts/validation/local/winui-panels --functional
```

`--functional` 同时检查快捷栏首显、子菜单、拖动跟随、收起规则和设置草稿。设置中心可从快捷栏的「快捷设置」→「全部设置」或托盘的「设置」打开。 [正式替换实测](../plan/validation.md#m1-winui-3-正式替换)

## Windows ChatGPT 套餐查询验证

确认本地配套 `codex.exe` 的来源和版本，再将其绝对路径传给开发诊断命令。该命令单次查询 App Server 使用的账户与 Codex 额度，输出来源、获取时间、套餐与额度状态。

```powershell
cargo run --locked --features development --bin check-chatgpt-account -- 'C:\path\to\codex.exe'
```

使用临时空目录作为该命令的 `CODEX_HOME` 可复验 `needs_auth`。已有登录和桌面客户端进程继续由客户端管理。 [实测版本与结果](../plan/validation.md#m2-windows-套餐适配器)

面板回归使用隔离桌宠配置和当前 Windows 会话中的 ChatGPT 客户端。默认验证自动连接、速览自动刷新、真实套餐和本地额度重置时间。追加 `--needs-auth` 可用临时空 App Server 配置验证需认证及空额度状态。每次运行指定新输出目录。

```powershell
cargo build --locked --features window-diagnostics --bin desktop-pet-chatgpt
uv run --isolated --with uiautomation==2.0.29 --with psutil==7.1.3 python scripts/validation/windows/check-chatgpt.py `
  --exe target/debug/desktop-pet-chatgpt.exe --pack assets/packs/white-dragon/0.1.0-v2 `
  --output artifacts/validation/local/chatgpt-live
```

用带 `window-diagnostics` 的构建追加 `--fixture`，可并列显示合成来源，验证刷新、详情切换、清除缓存、晚到响应和合成断连重连。脚本仅为本次子进程注入合成来源。

### 真实桌面进程生命周期

在独立可重启的 Windows 测试会话中，先打开并登录 ChatGPT 桌面客户端。用上面的面板回归命令追加 `--lifecycle`，将 `--output` 设为新目录。保持终端和桌宠运行，按脚本提示正常退出 ChatGPT（托盘菜单「Exit」），待脚本确认来源消失后再重开（`shell:AppsFolder\OpenAI.Codex_2p2nqsd0c76g0!App`）。脚本检查退出后清除套餐与获取时间、重开后自动重连，以及从详情刷新后实际返回套餐。

结果写入各自的 `result.json`，记录重启前后的桌面版本。另记录本机 Windows 版本及配套 `codex.exe --version`。网络异常单独验证并记结果。

### Windows 同会话接口探针

用已确认来源的配套 CLI 运行下列两条命令。每次指定新输出目录。第一条用当前 App Server 的 ChatGPT 登录发送一条固定的内存测试消息，检查线程和轮次归属、文字增量及完成事件。第二条只对明确授权的客户端测试会话查询元数据。结果不含会话 ID、消息正文或凭据。

```powershell
uv run --no-project python scripts/validation/windows/check-codex-conversation.py --cli 'C:\path\to\codex.exe' --ephemeral-turn --output artifacts/validation/local/codex-ephemeral
uv run --no-project python scripts/validation/windows/check-codex-conversation.py --cli 'C:\path\to\codex.exe' --thread-id '<test-id>' --output artifacts/validation/local/desktop-thread
```

单独启动的 App Server 返回的运行状态仅代表该进程。正式验收需要客户端与桌宠核对已授权会话的相同标识、消息往返和并发写入状态。 [接口实验结果](../plan/validation.md#m6-普通-chat-与-codex-接口实验)

### Windows 桌面 IPC 只读探针

使用 PowerShell 7.4 或更高版本，在已运行目标 MSIX 客户端的 Windows 会话执行。每次指定新输出目录。默认只握手并查询随机 ID。按需指定明确授权的测试会话原始 ID，普通 Chat、Codex 和 Work 分别使用 `-ChatTestId`、`-CodexTestId` 和 `-WorkTestId`。分享链接的 `/share/` 标识应排除在会话路由输入之外。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -OutputDirectory artifacts/validation/local/desktop-ipc-control
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -ChatTestId '<authorized-chat-id>' -CodexTestId '<authorized-codex-id>' `
  -OutputDirectory artifacts/validation/local/desktop-ipc-selected
```

每条测试会话先在桌面端打开，再查属主。若只能同时打开一条，分别执行并记录窗口状态。`owner_found` 表示存在能处理该 ID 的属主，仍需独立验证账号和工作区、快照、消息收发及断线恢复。`no_owner_found` 与客户端版本和当前加载状态有关。默认模式只握手并查属主。`status: ok` 仅表示 `probe_scope` 声明的协议检查通过，`desktop_ui_verified` 固定为 false；客户端人工核对另记，不能把协议通过作为 M6 完成。

#### Codex 快照与受控写入实验

当前安装实际使用流协议 v13，参考 fork 使用 v11。`-CodexSnapshot` 默认为 v11，遇到其他版本拒绝处理；必须明确指定实测版本。本机 v13 快照中的分页实体优先于旧 `turns` 数组，后者可能为空。探针在内存核对会话和属主，不记录历史正文或账号信息。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -CodexTestId '<authorized-codex-id>' -CodexSnapshot -SnapshotVersion 13 `
  -OutputDirectory artifacts/validation/local/codex-snapshot
```

首次写入曾导致客户端通用错误；2026-11-17 通过历史异常堆栈和安装包原函数定位为探针文本缺少 `text_elements`，补齐空数组后新会话单次双端复验通过。`-CodexTestTurn` 仍仅用于已授权的专用 Codex 测试会话：只发送一次随机固定标记，继承设置并要求不使用工具，须先取得 v13 完整历史并确认空闲。发现该会话曾有 IPC 测试标记即拒绝再发，发送前落盘关联信息；超时或断线不自动重试。实验通过不代表产品发送功能已接入或 M6 已完成。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -CodexTestId '<new-authorized-codex-id>' -CodexTestTurn -SnapshotVersion 13 `
  -OutputDirectory artifacts/validation/local/codex-fixed-test-turn
```

已发出的消息仅以报告中的测试标记和消息关联 ID 核对，无需再次发送：

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -CodexTestId '<same-authorized-codex-id>' -SnapshotVersion 13 `
  -ExpectedTestMarker '<expected_test_marker>' -ExpectedMessageId '<client_message_id>' `
  -OutputDirectory artifacts/validation/local/codex-reply-verification
```

该操作会建立新连接、重新发现属主并取快照，按分页实体核对原会话、用户测试文字、完成状态和助手标记。仅运行命令验证的是探针重新连接；测试客户端重启时，先由维护者正常退出并重开，再运行该命令。本轮已通过这种重启后的原消息回读，客户端人工确认历史可见。

反向轮次由维护者在同一客户端手动发送「只回复 DESKTOP_PET_M6_CLIENT_OK，不调用工具」，完成后执行只读核对：

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -CodexTestId '<same-authorized-codex-id>' -SnapshotVersion 13 -VerifyClientTurn `
  -OutputDirectory artifacts/validation/local/codex-client-reverse
```

2026-11-17 已通过文字流重建和指定轮次停止。只读监听使用尚未在该会话出现的新标记，先启动探针，看到 `READY` 后再由维护者从客户端发送「只回复 DESKTOP_PET_M6_CLIENT_STREAM_01，不调用工具」。默认监听 60 秒，可用 `-ObserveSeconds` 指定 5–120 秒。每次选新标记，避免把旧回复误判为本次流。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -CodexTestId '<authorized-codex-id>' -ObserveClientTurn -SnapshotVersion 13 `
  -ClientTestMarker DESKTOP_PET_M6_CLIENT_STREAM_01 `
  -OutputDirectory artifacts/validation/local/codex-client-stream
```

探针核对会话、属主、流版本及 revision，在内存应用混合快照/补丁。报告分别统计补丁、快照、文字更新和未完成文字，不记录正文；最终补丁完成状态另与新快照核对。只计补丁事件不能代表文字重建。发现 revision 缺口、属主断开或版本变化即失败，不猜测缺失内容。监听及等待快照期间也须应答其他连接的发现请求，避免阻塞客户端路由。

停止实验使用新的 `DESKTOP_PET_M6_CLIENT_STOP_` 标记。先启动下列探针，看到 `READY` 后由维护者在同一客户端发送「DESKTOP_PET_M6_CLIENT_STOP_01：请从 1 开始逐行数到 10000，不调用工具」。探针只选择该标记对应的活动轮次，通过属主的停止 v4 携带 `expectedTurnId` 停止一次；发送前落盘关联回执，随后重取快照核对 `interrupted`，再由维护者核对界面。历史已存在该标记时拒绝运行。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc.ps1 `
  -CodexTestId '<authorized-codex-id>' -StopClientTestTurn -SnapshotVersion 13 `
  -ClientTestMarker DESKTOP_PET_M6_CLIENT_STOP_01 `
  -OutputDirectory artifacts/validation/local/codex-client-stop
```

并行只读连接不等于并发写入协调已通过。2026-11-17 的后续实验已通过探针在途重连与未读确认后的送达核对，但双写入复现界面卡住，见下节。账号/工作区、属主在途退出及升级兼容仍需独立验收；这些结果不替代普通 Chat 或 Work 的支持证据。

#### Codex 断线恢复与并发诊断

`check-desktop-ipc-recovery.ps1` 固定使用已验证安装版本 `26.1111.2228.0` 和流 v13。每次使用新的输出目录，已有 `result.json` 时拒绝覆盖；未知版本、来源不符、缺失 revision 或属主断开均拒绝继续，恢复不会自动重发。

`UnconfirmedSend` 要求新的、已授权专用测试会话，完整历史为空闲且没有进行中行、未确认提交或已有 IPC 测试标记。发送前保存回执，写出一条固定标记请求帧后立即关闭探针连接，不读取发送确认，再重新发现属主、取快照并按原消息 ID 核对唯一完成轮次。此模式只发一次，不应以新输出目录重跑同一已写会话。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc-recovery.ps1 `
  -Mode UnconfirmedSend -CodexTestId '<new-authorized-codex-id>' `
  -OutputDirectory artifacts/validation/local/codex-unconfirmed-send
```

`InFlightReadReconnect` 不发送消息。先启动并等待 `READY`，再由维护者从客户端发起含新恢复标记的文本轮次，例如「DESKTOP_PET_M6_CLIENT_RECOVERY_01：请从 1 到 300 逐行只输出数字，每行一个，不调用工具」。探针在非空助手文字与活动状态同时存在时断开自己的连接，重新核对来源和原轮次，再核对文字前缀保留及最终回复与独立快照一致。报告的 `active_after_reconnect` 表明恢复时轮次是否仍在进行。该探针验证文字连续性，不校验模型是否遵守计数指令，不能直接把运行成功算作长文本覆盖。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc-recovery.ps1 `
  -Mode InFlightReadReconnect -CodexTestId '<authorized-codex-id>' `
  -ClientTestMarker DESKTOP_PET_M6_CLIENT_RECOVERY_01 `
  -OutputDirectory artifacts/validation/local/codex-inflight-reconnect
```

`check-desktop-ipc-concurrency.ps1` 是双写入故障诊断，**已在本版复现残留进行中占位和客户端卡住**，只在明确授权的新专用会话运行；正常单次发送验收不包含它。它建立两个独立连接，各写一条不同固定标记后才读取确认，记录发送间隔和每条请求的确认及历史关联。准备连接时向已核对的属主定向发现，避免两个尚未读取控制帧的预备连接互等。两条确认指向同一轮次，或任一消息/回复关联未通过，都不能记为两个独立请求成功。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc-concurrency.ps1 `
  -CodexTestId '<new-authorized-codex-id>' `
  -OutputDirectory artifacts/validation/local/codex-two-writers
```

出现卡住或不明确送达时停止追加写入，保留回执，使用下列命令只读检查；不会重发原消息。检查结果只输出对应用户消息/助手标记的索引、轮次状态及项类型。`status: ok` 仅表示检查成功，不代表并发发送通过。本机曾出现 runtime idle 但历史仍有 `turnId = null` 的进行中占位；单次发送探针现在拒绝这种状态和任何未确认提交。客户端切页未恢复时，由维护者正常退出并重启，随后重新打开测试会话，再用新输出目录核对占位清除和原消息保留。

```powershell
pwsh -NoProfile -File scripts/validation/windows/inspect-desktop-ipc-concurrency.ps1 `
  -CodexTestId '<same-authorized-codex-id>' `
  -ReceiptDirectory artifacts/validation/local/codex-two-writers `
  -OutputDirectory artifacts/validation/local/codex-two-writers-readback
```

产品侧须按属主写入契约发送，两个初始空闲快照不构成互斥。后续已验证下节队列候选的双连接同时提交；直接并发 `start-turn` 仍是已知失败路径。上述恢复实验只断开探针自己的管道，不模拟系统断网、属主在途重启或账号切换。

#### Codex 属主队列仲裁候选

`check-desktop-ipc-queue.ps1` 在已授权的新专用会话中，使用本版队列 v3 的 `preparationMessageId` 增量预留与完成提交，避免提交整队列状态。它只携带固定标记、当前 cwd、空附件及继承上下文，不提供权限或模型覆盖。只准许本次消息 ID 的单条更新和空丢弃列表。必须空闲、历史完整且不存在进行中行、未确认提交或旧 IPC/队列测试标记，已有输出回执也拒绝覆盖。

默认用一个连接先预留两条消息，再完成提交；`-TwoWriters` 使用两个独立连接分别预留，在读取任何完成确认之前写出两条完成请求。报告记录间隔和 `completions_sent_before_any_ack_read`，最终核对两条唯一消息、独立真实轮次、准确回复、队列清空及无进行中/未确认占位。2026-11-17 同时提交实测间隔为 2 ms，通过双端核对。两个连接但顺序完成的旧样本也保留，不算同时在途覆盖。

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc-queue.ps1 `
  -CodexTestId '<new-authorized-codex-id>' -TwoWriters `
  -OutputDirectory artifacts/validation/local/codex-owner-queue
```

队列写入确认只表示条目变更获属主接受，不表示消息已完成。探针按消息 ID 跟踪实际轮次；本版原生文本制备会添加一个末尾 LF，验证仅对固定队列测试文字接受这一个 LF，不做任意 trim，也不放宽普通直接发送的文本核对。发现问题不重放，按原回执只读核对：

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc-queue.ps1 `
  -CodexTestId '<same-authorized-codex-id>' `
  -VerifyReceiptDirectory artifacts/validation/local/codex-owner-queue `
  -OutputDirectory artifacts/validation/local/codex-owner-queue-readback
```

只读模式的 `queue_mutation_attempts` 为 0。队列清空以对应广播为准，历史快照用于核对轮次。探针异常清理只针对本轮已确认预留且从未尝试完成提交的条目。完成确认不明确时保留回执，按原 ID 核对送达。

队列生命周期使用 `check-desktop-ipc-queue-lifecycle.ps1`。各次运行传入授权测试会话的 `-CodexTestId` 和独立的 `-OutputDirectory`。

| 参数 | 复验方式 |
| --- | --- |
| 默认 | 在专用会话中分别模拟预留确认和完成提交确认丢失。断开写入探针并重新连接，核对按 ID 取消和另一条预留保留情况 |
| `-ReadbackReceipt <result.json>` | 按原回执只读核对取消和送达。`delivery_confirmed` 表示唯一完成轮次与原消息匹配，`retrying_error_items` 记录该快照中的可重试错误数 |
| `-DesktopInterleave -TimeoutSeconds 120` | 在测试会话手动发送以 `READY` 输出标记开头的请求，请模型回显标记并生成一段足够长的纯文本。探针在该桌面轮次仍活动时提交另一条消息 |
| `-PrepareRestart` | 预留一条测试消息并保存回执，确认该条目尚未启动轮次 |
| `-RestartReceipt <result.json>` | 维护者正常重启桌面客户端并重新打开同一会话后运行。核对进程已更换，检查原预留并按 ID 取消 |

实测结果及未覆盖场景见[队列生命周期验证](../plan/validation.md#2026-11-18-队列确认丢失取消与重启)。

离线检查不会连接真实客户端：

```powershell
pwsh -NoProfile -File scripts/validation/windows/check-desktop-ipc-protocol.ps1
python -m unittest discover -s scripts/validation/windows -p test_codex_conversation.py -v
```

IPC 检查覆盖分片、严格 UTF-8、帧大小、超时、错误属主、版本变化、来源过滤、分页历史、测试回复关联和禁止重发。App Server 检查会拼接同一项的文字增量与完成正文核对，并排除其他线程或轮次。

## 工程规范化提案

继续单 Cargo 包，按模块边界拆职责。只在确有独立复用/发布需求时拆 workspace；不以文件行数或“跨平台”名义提前建立大量空 trait。

**开发期迭代**。未对外发布的设计迭代不要求兼容迁移。直接更新实现、制备工具、测试和文档，清理可再生的旧产物。已发布的兼容承诺与显式外部数据导入单独决策。 [D19](../plan/decisions.md)

新增错误区分用户可处理、暂时不可用、永久不兼容和内部故障；展示给用户原因/下一步，日志记录结构化类别，避免泄漏路径、账户或正文。unsafe 局限平台边界并说明句柄所有权、线程要求和释放责任。

新增 CI 提案：PR/push 执行 fmt、clippy、默认与全 feature 测试、CPU 素材合成校验；平台抽离后在 Windows/Linux 跑纯逻辑与适用构建。GUI、性能和发布测试分开，不让每次文档修改启动两小时验收。当前工作流仍仅手动触发 Windows，本轮未改它。

## 文档与任务纪律

| 内容 | 放置位置 / 维护方式 |
| --- | --- |
| 用户当前能力、安装与限制 | README、docs/user；仅写已实现并声明支持的内容 |
| 当前格式和配置 | docs/development/runtime-v2.md，与代码同改 |
| 目标架构和取舍 | docs/design；明确提案/已决定，避免重复维护参数 |
| 当前路线与阻塞选择 | docs/plan/roadmap.md、decisions.md，滚动更新，不按每阶段另开日记 |
| 实测结论与证据位置 | docs/plan/validation.md，按主题持续更新 |
| 单次任务执行 | Issue/PR/工作记录；没有外部服务时放 artifacts/work-notes，不自动发送外部消息 |
| 原始验证日志/图像/实验包 | artifacts/validation、assets/work，Git 忽略 |
| 需长期共享的发布证据 | Release 或可持久访问的附件存储；仓库只留支持结论与哈希/定位 |

新任务先确定用户结果和停止条件，再写必要测试。方案试验结束将结论并入所属设计，不保留一串相互矛盾的“最新修订”。只有长寿命、跨模块且难以从代码理解的决策才值得独立 ADR；其余直接维护对应设计段落。

PR 描述写问题、最终行为、验证与限制；不复述调试流水账。提交粒度按可回退能力切片；遇到已有暂存修改，先明确基准并保留，不擅自 reset、混合提交或覆盖用户成果。
