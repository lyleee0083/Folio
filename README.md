# Folio

Folio 是 Windows 10／11 x64 的本地 Markdown 预览与源码编辑工具。当前版本使用 Rust、windows-rs、Win32、Direct2D 和 DirectWrite。

## 安装与启动

下载 `Folio-版本-win-x64.msi`，在资源管理器双击它并选择「安装」。软件安装到当前用户的 `%LOCALAPPDATA%\Folio`，可从开始菜单搜索「Folio」，也可在 Windows 的「设置 → 应用 → 已安装的应用」（Windows 10 为「应用和功能」）中卸载。应用的阅读偏好保存在单独的 `%APPDATA%\Folio\native` 目录。

需要从当前源码生成安装包时，在 GitHub 仓库打开 **Actions → Build Folio Installer → Run workflow**。运行成功后，从对应运行的 **Artifacts** 下载 `Folio-Windows-x64` ZIP，解压得到 MSI。产物保留 30 天，安装包未签名。

直接运行时，双击根目录的 `Folio.exe` 或 `Folio.cmd`，也可以传入 Markdown 文件或目录。

```powershell
.\Folio.exe "D:\Notes\笔记.md"
```

后续启动会把文件转交给同一 Windows 用户和会话中已有的窗口。应用使用 Windows 系统组件，运行时仅需 Windows x64。

## Preview 和 Raw

- 默认在 Preview 显示。点击工具栏的铅笔图标进入 Raw，点击眼睛图标返回 Preview。工具栏图标采用 Lucide。
- Preview 默认使用 16 DIP 正文和 1.6 倍行距。自动正文宽度随预览区伸缩，两侧保留窄边距。可以在阅读设置调整字号、行距和正文宽度。
- GFM 支持标题、引用、列表、表格、任务清单、删除线、链接和带语言标记的代码块。Preview 提供大纲、查找、连续选择、代码复制和宽内容的局部横向滚动。
- Codex 公开终端的数学排版规则用于可转换的公式，超出该范围的公式显示源码。Antigravity 的文件预览行为用于文件顶部的 YAML 键值摘要。
- 原始 HTML 显示为文字；围栏源码作为代码处理。本地栅格图片从文档目录及其下级目录读取。
- Raw 支持行号、自动换行、查找、撤销和 GFM 语法着色。点击保存图标或按 `Ctrl+S` 写回原文件。返回 Preview、打开其他文档、重读和关闭窗口前会处理未保存的改动。
- 保存核对原文件哈希，保留编码、BOM 和未改动行的换行。单个文档最大 10 MiB，单批上限 256 MiB，库最多 2000 项。支持 UTF-8、带 BOM 的 UTF-16／UTF-32 和 GB18030。

常用快捷键为 `Ctrl+O` 打开，`Ctrl+Shift+O` 导入，`Ctrl+F` 查找，`F3 / Shift+F3` 切换匹配，`Ctrl+R` 重读，`Ctrl+P` 打印 Preview，`Ctrl+,` 阅读设置，`F11` 专注模式，`Ctrl++ / Ctrl+-` 调整字号。

应用设置位于 `%APPDATA%\Folio\native\reader-library.json`。正常关闭会清空打开历史并保留阅读及窗口设置。旧版配置可迁移，工作中的 Raw 草稿保存在内存。

## 构建

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\Build-Native.ps1 -Test
```

脚本使用项目内 Rust 与 GNU binutils，也支持系统中的 Rust 和 Visual Studio MSVC 工具链。源码和锁文件在 `src/rust/`，编译缓存位于 `work/rust-build/`。`-Test` 使用隔离配置验证实际窗口，`-UnitTest` 则运行 Rust 单元测试，适用于 CI。

生成安装包时，先构建根目录程序，再运行 `scripts/Build-Installer.ps1 -RestoreTools -Verify`。`-RestoreTools` 使用 .NET SDK 9 或以上版本还原 WiX 5.0.2 及同版本 UI 扩展。`-Verify` 解包并核对内置程序。MSI 写入 `generated/`。

`scripts/Measure-ReaderPerformance.ps1` 使用本地历史基准样本，统计 Direct2D 提交首帧及切换帧的时间。基准样本和结果属于本地验证数据。

更多内容见 [原生架构](docs/native-design.md) 和 [第三方声明](docs/THIRD-PARTY-NOTICES.md)。
