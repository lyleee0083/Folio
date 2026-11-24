# Folio

Folio 是面向 Windows 10／11 x64 的原生 GFM Markdown 预览与源码编辑工具，使用 Rust、windows-rs、Win32、Direct2D 和 DirectWrite。

## 功能

- **Preview** 提供 GFM Markdown 预览。预览仅解码可见区域的图片，并限制图像缓存大小。
- **Raw** 提供 Markdown 源码编辑、行号和语法着色。
- 相对 Markdown 链接支持跳转到目标文件的标题。

## 快速开始

1. 在 [Releases](https://github.com/lyleee0083/Folio/releases) 下载 Windows x64 MSI，并使用同版本的 SHA-256 校验文件核对安装包。
2. 运行 MSI 并按提示完成安装，然后启动 Folio。
3. 打开 Markdown 文件。使用 **Preview** 阅读，或切换到 **Raw** 编辑并保存。

### 保存与恢复

**Raw** 有未保存的编辑时，Folio 会阻止普通注销或重启，让用户先保存或放弃修改。阅读状态仅保存窗口设置、文档路径和位置。

保存源码前，Folio 会检查原文件并暂存恢复副本。系统中断后如需找回原内容，可在原文件所在目录查看 `.folio-*.recovery.bak`。

## 构建与测试

1. 打开 [Build Folio Installer](https://github.com/lyleee0083/Folio/actions/workflows/build-installer.yml)。
2. 点击 **Run workflow**，选择 `main`，然后启动构建。
3. 构建成功后打开该次运行，在 **Artifacts** 下载 `Folio-win-x64-x.x.x`，其中包含同名 MSI 和 `SHA256SUMS.txt`。

`main` 分支推送和以 `main` 为目标的 Pull Request 也会触发工作流。工作流使用 Windows x64 MSVC 工具链编译并测试 Folio，使用 WiX 生成 MSI，并验证按用户安装、从 `0.0.2` 升级和卸载。Actions 产物保留 30 天。

## 仓库结构

| 路径 | 用途 |
| --- | --- |
| [`src/rust/src/main.rs`](src/rust/src/main.rs)、[`icons.rs`](src/rust/src/icons.rs) | 窗口、工具栏、文档库、查找、大纲和 Preview／Raw 切换 |
| [`document.rs`](src/rust/src/document.rs)、[`math/`](src/rust/src/math) | GFM 解析、YAML 文件头摘要和 Unicode 公式排版 |
| [`preview.rs`](src/rust/src/preview.rs)、[`raw.rs`](src/rust/src/raw.rs) | Direct2D／DirectWrite 预览，以及 Win32 RichEdit 源码编辑、行号和语法着色 |
| [`files.rs`](src/rust/src/files.rs)、[`platform.rs`](src/rust/src/platform.rs) | 文件编码、换行保留、保存冲突、设置和 Windows 平台集成 |
| [`src/rust/assets/`](src/rust/assets)、[`build.rs`](src/rust/build.rs) | 程序图标、Windows 清单和资源嵌入 |
| [`src/installer/`](src/installer)、[`scripts/`](scripts) | MSI 定义和构建脚本 |
| [`.github/workflows/`](.github/workflows) | 验证主分支和 PR 的 Windows x64 构建流程 |

## 版本发布

在 [Releases](https://github.com/lyleee0083/Folio/releases) 下载 `Folio-win-x64-x.x.x.msi`，并使用同版本的 SHA-256 校验文件核对安装包。

[工作流](.github/workflows/build-installer.yml) 根据 `main` 的提交历史递增安装包版本。`main` 构建和测试通过后自动发布，同一提交重跑沿用原版本。

## 许可与声明

Folio 自有源码保留版权并公开可阅。第三方内容适用各自的许可，详见 [LICENSE](LICENSE) 和 [第三方声明](docs/THIRD-PARTY-NOTICES.md)。
