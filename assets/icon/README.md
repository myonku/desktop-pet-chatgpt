# 程序默认图标

`desktop.png` 和 `tray.png` 是经 ImageGen 编辑后的透明图，编辑前的输入文件名分别为 `desktop-icon.png` 和 `tray-icon.png`。现行素材装配器从这两张透明图用 Lanczos 缩放生成多尺寸 `app.icon.ico` 和 `tray.icon.ico`，白龙全动作开发包也使用同一输入。Windows 构建通过 `icons.rc` 嵌入两个图标组，应用图标排在首位，供 Explorer 和桌面快捷方式提取。设置中心使用内置应用图标。角色窗口和托盘优先使用素材包声明的图标，缺省时读取对应内置图标组。

正式发行清单须核对编辑前的源图、加工来源及内嵌 ICO 的分发许可。 [发行内容](../../docs/development/release.md#发行内容)
