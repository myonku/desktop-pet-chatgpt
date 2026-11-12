# P2-02 · 平台与设置

2026-11-12 完成本地实施与验收。实现提交为 `3588f1d`，基线为 `a17ce31`。

## 配置与平台行为

- 产品配置使用 Windows 本地应用数据目录下的 `DesktopPetChatGPT/config.json`。schema 保持 1，读取旧配置时补齐新字段。设置预览立即生效，「应用并保存」写入确认值，关闭窗口撤回未确认的预览。
- 保存手动尺寸时记录基准 DIP 与缩放百分比。重启优先使用已保存尺寸，跳过 EDID 探测，跨屏继续按每屏 DPI 换算。未保存尺寸时沿用 P2-01 的探测和回退。恢复默认清除已保存尺寸和启动比例。
- 配置由同目录新文件刷盘后替换。坏配置启动使用默认值，普通保存保留原件，明确恢复默认时备份可读原件。检测到外部编辑或替换失败时显示未保存。
- 托盘和角色右键共用动作与设置菜单。穿透只能在有托盘恢复入口时开启，重启保留确认值，托盘可撤销。托盘不可用时保持角色可交互，并提供设置退出入口。
- 同一 Windows 会话内按用户配置目录限制实例。重复启动打开既有实例的设置，窗口暂不可达时显示已运行提示。退出关闭音效，取消页请求并有界等待媒体 worker。
- 登录自启仅在产品设置中确认，使用当前用户的 Run 注册项。开发配置和独立验证目录不改动系统自启。设置写入失败时报告错误，必要时尝试恢复先前的注册值。注册机制见 [Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)。
- 设置显示包内 `character.poster` 和通用 `character.name`，单击和试听使用 `interaction.click`。缺图回退内置图，缺文案使用通用名称，缺音效静音。保存素材目录后在重启时切换。任务气泡开关和缩放独立保存，真实任务与详情随 P3 接入。

## 复测

复测使用独立用户目录，测试鼠标指针会短暂定位到自建窗口，按键与按钮消息发给被测窗口。自启读写测试使用隔离的 HKCU 子键。

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked
cargo build --release --locked
cargo test --locked
cargo test --locked --all-features
cargo build --release --locked --features p1b-reference --target-dir target/reference
python scripts/validation/p2/generate-fixtures.py
$packs = 'artifacts/validation/p1/fixtures'
$run = 'artifacts/validation/p2/runs/p2-02-recheck'
target/release/validate-pack.exe --distribution "$packs/dawn"
target/release/validate-pack.exe --distribution "$packs/dusk"
python scripts/validation/p2/check-settings.py --exe target/reference/release/desktop-pet-chatgpt.exe --packs $packs --output $run
python scripts/validation/p2/check-windows.py --exe target/reference/release/desktop-pet-chatgpt.exe --fixtures artifacts/validation/p2/fixtures --output "$run/core-regression"
```

`dawn`、`dusk` 由 [P1 合成包脚本](../../../scripts/validation/p1/generate-p1b-fixtures.py) 生成，依赖安装见 [G-05](../../G-05-contracts.md#工程与产物)。两包使用不同资源路径和内容。

## 本地验收

Windows 11 专业版 22631，Intel Core Ultra 7 155H，Intel 核显 32.0.101.8860，Rust 1.99.1 MSVC，Python 3.14.7。设置实测使用 125% DPI；核心回归覆盖 125% 与 100% 双屏。

| 检查 | 本地结果 |
| --- | --- |
| 构建与测试 | 格式、Clippy、锁定 Debug 与 Release、默认与全 feature 测试通过，两组各 18 项。隔离 HKCU 测试覆盖引号和 Unicode 路径、启用、读取和撤销 |
| 单实例与共享菜单 | 第二个进程正常退出并打开已有实例的设置；角色右键可打开设置；托盘穿透在重启后恢复并能由托盘撤销 |
| 尺寸与持久化 | 启动从已保存的 264 DIP 得到 330 像素，跳过 EDID。缩放预览时设置窗原位且配置不变，关闭撤销。应用后为 156 像素，重启一致。恢复默认清除已保存尺寸、素材路径和覆盖比例 |
| 坏配置与写失败 | 未来版本配置使用默认值，显式恢复原始字节完整备份。禁止删除共享的文件锁触发 Windows 错误 5，原件和已确认值保留，无遗留临时文件。开发覆盖优先于已保存尺寸，本次应用未写用户文件 |
| 素材与音效 | dawn 和 dusk 通过整包校验，重启后实际加载的图片、WAVE 和通用角色名指纹均不同。任务气泡关闭时仍可试听，音效静音后单击未触发播放 |
| P2-01 回归 | 合成包的手势、位移、吸边、同屏与异屏全屏、双屏 EDID/DPI、缺页及必备待机损坏回退均通过 |

默认 Release SHA-256 为 `b1fab54ce68e1aab5b8457a8577028e48e0985a36ec11242c230ca1dedaeef61`，行为参照构建为 `4ccb537fa117a1e4b3ca02337f4de2bd130ee7faf51658229fec435c69e551c3`。dawn 清单 SHA-256 为 `92dee7e152fbe8524fcadf7ca11571ba33dc53ec198626585753343d4d2ef640`，dusk 清单为 `faba509b4166b98ada8d84018362cefff41d6503b54f60cf8aa74494d4a787c5`。

运行证据位于 `artifacts/validation/p2/runs/p2-02-acceptance-1794448149518/`。`result.json` 记录设置检查和资源指纹，`core-regression/result.json` 记录核心行为和显示器数据。

登录自启验证覆盖隔离注册表的注册与撤销，真实 Windows 注销后登录、Explorer 托盘创建故障注入和托管 Windows CI 留待后续实机复验。
