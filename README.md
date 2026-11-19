# Folio

Folio 是 Windows 10／11 x64 的原生 GFM Markdown 预览和源码编辑工具，使用 Rust、windows-rs、Win32、Direct2D 和 DirectWrite。

## 源码结构

| 路径 | 负责的功能 |
| --- | --- |
| [`src/rust/src/main.rs`](src/rust/src/main.rs)、[`icons.rs`](src/rust/src/icons.rs) | 窗口、工具栏、文档库、查找、大纲和 Preview／Raw 切换 |
| [`document.rs`](src/rust/src/document.rs)、[`math/`](src/rust/src/math) | GFM 解析、YAML 文件头摘要和 Unicode 公式排版 |
| [`preview.rs`](src/rust/src/preview.rs)、[`raw.rs`](src/rust/src/raw.rs) | Direct2D／DirectWrite 预览，以及 Win32 RichEdit 源码编辑、行号和语法着色 |
| [`files.rs`](src/rust/src/files.rs)、[`platform.rs`](src/rust/src/platform.rs) | 文件编码、换行保留、保存冲突、设置和 Windows 平台集成 |
| [`src/rust/assets/`](src/rust/assets)、[`build.rs`](src/rust/build.rs) | 程序图标、Windows 清单和资源嵌入 |
| [`src/installer/`](src/installer)、[`scripts/`](scripts) | MSI 定义和构建脚本 |
| [`.github/workflows/`](.github/workflows) | 手动触发的 Windows x64 构建流程 |

## 构建

在 Windows 上使用 Rust 和 Visual Studio MSVC 工具链编译。生成 MSI 需要 .NET SDK 9 或以上版本，以还原 WiX 构建工具。安装包脚本会用 MSVC 重新构建它要打包的程序。

```powershell
.\scripts\Build-Installer.ps1 -RestoreTools
```

GitHub Actions 的 [Build Folio Installer](https://github.com/lyleee0083/Folio/actions/workflows/build-installer.yml) 支持手动构建。成功后可在该次运行的 **Artifacts** 直接下载 `Folio-版本-win-x64.msi`。MSI 作为构建产物存放。

[第三方声明](docs/THIRD-PARTY-NOTICES.md)

阅读状态仅保存窗口设置、文档路径和位置。保存源码前会检查原文件并暂存恢复副本；系统中断后如需找回原内容，可在原文件所在目录检查 `.folio-*.recovery.bak`。

Folio 自有源码保留版权并公开可阅。第三方内容适用各自许可，详见 [LICENSE](LICENSE) 和第三方声明。
