# Folio 原生架构

## 运行与显示

- `src/rust/src/main.rs` 负责 Win32 消息循环、窗口、Lucide 工具栏、文档库、大纲、设置和 Preview／Raw 切换。原生文件选择、剪贴板、打印及单实例管道在 `platform.rs`。
- `document.rs` 将 Markdown 解析为原生行内样式与文档块。Preview 由 `preview.rs` 使用 DirectWrite 布局和命中测试，由 Direct2D 绘制文本、表格、代码块、选区及受限的本地图片。长文档滚动只绘制可见块。
- 默认正文优先采用 Segoe UI Variable Text，并为中文文字指定 Microsoft YaHei UI。自动宽度按当前预览区排版，左右各保留 20–44 DIP 边距。手动宽度可在阅读设置调整。Raw 保留编辑区的可用宽度。
- `raw.rs` 为 Win32 RichEdit 提供源码着色与段落行距。行号按 RichEdit 的实际文字基线绘制，仅在逻辑行首出现。编码转换、换行保留、保存冲突和偏好存储在 `files.rs`。
- 构建使用 Cargo 锁文件，以 `build.rs` 嵌入原有 Folio 图标及 Win32 清单。预览、工具栏和应用资源都随可执行文件提供。

## Markdown 处理范围

| 依据 | 应用行为 |
| --- | --- |
| [正式 GFM 规范](https://github.github.com/gfm/) | 按文档本身的 GFM 结构解析并渲染表格、删除线、任务清单和自动链接。 |
| [Codex 公开终端源码](https://github.com/openai/codex/tree/b07a3473813f101aeac446a012930aefe57e38f9/codex-rs/tui/src/markdown_render/math) | 采用公开的有界 Unicode 数学排版器。符号超出支持范围时显示源码。 |
| [Google Antigravity 发布记录](https://antigravity.google/changelog?version=2.12.2) | 采用文件顶部 YAML 键值摘要行为，复杂或无效值保留为源码。Preview／Raw 的切换沿用用户提供的操作参考。 |

原始 HTML 标签显示为文字。围栏中的 Markdown 示例保留其代码块属性。Mermaid 围栏显示为源码。采用 Codex 公开终端中能核实的行为，桌面客户端完整方言和渲染规则仍待公开确认。

## 存储和输入边界

应用只在显式保存时写回当前 Markdown，替换前校验 SHA-256。草稿只存内存。普通退出清除会话文档历史，保留偏好。旧版 JSON 的 PascalCase 字段仍可读取，损坏文件的恢复副本保留。

单实例互斥锁按 Windows 用户、会话和配置路径区分，命名管道只接收本地同一用户的路径。相对图片从文档目录树内读取，重解析点和外部图像请求被拒绝。外部链接按用户点击交给系统处理。

## 验证

`scripts/Build-Native.ps1 -Test` 运行 Rust 自检并启动隔离的 Win32 验证，覆盖 Preview、Raw 写回、文件编码、窗口复用和退出状态。此前 WPF/WebView2 性能报告仅对对应历史可执行文件有效。
