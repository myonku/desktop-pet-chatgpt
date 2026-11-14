# 程序默认图标

`desktop.png` 和 `tray.png` 由内置 ImageGen 分别编辑根目录的 `desktop-icon.png` 和 `tray-icon.png` 得到。多尺寸 `app.icon.ico` 和 `tray.icon.ico` 由素材装配器使用 Lanczos 缩放生成。Windows 构建通过 `icons.rc` 嵌入两个图标组，应用图标排在首位，供 Explorer 和桌面快捷方式提取。程序与设置窗口和托盘也各自读取对应图标组。

源图和加工来源需随正式发行清单核对；编译进 EXE 不代表额外获得分发许可。旧图像编辑提示保存在文档重组的本地快照中，不作为工程规范维护。
