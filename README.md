# Aureate

本地优先的 Markdown 写作器：**屏幕上预览，纸上只出 PDF**。当前平台：macOS（universal dmg）。

- **100% Rust**：界面是 `egui 0.37`（eframe），排版是 `Typst 0.15`。没有 HTML、没有 JavaScript、没有 Node、没有 WebView，也没有 Tauri —— 整个程序就是一个原生 Rust 二进制。
- **预览即 PDF**：预览不是另画一遍，而是用写 PDF 的**同一个 Typst 引擎**编译，再用 `typst-render` 栅格化成页面图贴到界面上。主题、表格、公式、页码、页眉，预览里什么样，PDF 就什么样。
- **排版**：四套主题（GitHub / 学术论文 / 商务 / 极简），纸张 A4 / A5 / Letter / Legal（卡册另用 A6），三档页边距；页眉页脚、页码、目录、PDF 书签。
- **导出**：单文档 PDF / 整目录批量 PDF / 卡册 PDF（一卡一页）。
- **研究卡**：frontmatter 驱动的六段骨架卡，四视图（表单 / 时间线 / 看板 / 周历），SQLite FTS5 全文检索，`[[卡ID]]` 双链。
- **字体自带**：4 个 Noto（思源黑体 / 宋体 × Regular / Bold）用 `include_bytes!` 打进二进制，配合 Typst 内置数学与西文字体，不依赖系统字体，换机不掉字。
- **保存时机**：`Cmd+S` 手动保存；切换文档前、导出 PDF 前自动落盘；状态栏显示「已保存 / 未保存」与字数。
- **不落数据**：索引、偏好、回收站全部写在应用包内 `data/`（`Aureate.app/Contents/Resources/data`）；只有包内不可写时才退回 `~/.aureate/data`，状态栏会显示「索引：内存」。
- **目录边界**：启动永远由你手动选文件夹或单个 `.md` 文件；程序不记路径、不自动打开任何目录，也不打开应用自身目录。

## 工作区结构

```
crates/aur-core      Markdown 解析 / frontmatter / 卡片领域模型（无 UI、无 Typst 依赖）
crates/aur-vault     文件仓库：原子写、回收站、文件监听
crates/aur-index     SQLite FTS5 索引、反向链接、周历台账
crates/aur-export    Markdown AST → Typst 源码 → PDF；同一套源码栅格化成预览页
crates/aur-app       egui 界面：文件树 / 编辑器 / 预览 / 卡片 / 搜索 / 导出 / 设置
.github/workflows    ci.yml（fmt + clippy + test）、release.yml（macOS universal dmg）
```

依赖方向：`aur-app` → {`aur-core`, `aur-vault`, `aur-index`, `aur-export`}，`aur-core` 不依赖本项目任何其他 crate。

## 本地开发（只需 Rust 工具链）

```bash
cargo run -p aur-app      # 调试运行
cargo test                # 31 个测试
cargo clippy --all-targets -- -D warnings
```

以 macOS 为例，建议给独立 target 目录避免和 IDE 的 rust-analyzer 抢锁：

```bash
CARGO_TARGET_DIR=target/run cargo run -p aur-app
```

## 使用

1. 启动后选一个文件夹当工作目录，或直接选一个 `.md`（其所在文件夹自动成为工作目录）
2. 左侧文件树管理文档（右键可重命名 / 删除，删除进应用内回收站，不进废纸篓）
3. 顶部切换「编辑 / 分屏 / 预览」；设置里改主题、纸张、页边距，预览 600 ms 内重编译
4. 快捷键：`Cmd+S` 保存、`Cmd+E` 导出 PDF、`Cmd+F` 搜索、`Cmd+N` 新建文档

## 打包（只有 GitHub Actions）

推送 tag `v*`（或手动触发 release workflow）：

1. `cargo build --release --target universal-apple-darwin`（arm64 + x86_64）
2. 手工组装 `Aureate.app`（Info.plist + 图标 + ad-hoc 签名）
3. `hdiutil` 出 dmg，上传 artifact，并自动挂到对应 tag 的 **Releases**

产物没有开发者签名。下载安装后若提示「无法打开」：

```bash
xattr -cr /Applications/Aureate.app
```

## 数据放在哪

| 内容 | 位置 |
| --- | --- |
| 你的文档 | 你自己选的那个文件夹，只有 `.md` |
| 索引缓存 / 偏好 / 回收站 | 应用包内 `data/` |
| 兜底（包内不可写时） | `~/.aureate/data` |

## 已知边界

- **内嵌 HTML 不渲染**：`<table>`、`<font color>` 这类 HTML 在预览和 PDF 里都不会变成表格或颜色。表格请用 Markdown 管道语法 `| a | b |`，两侧都正常。
- **公式**：`$...$` / `$$...$$` 经 TeX → Typst 转换，覆盖 frac / sqrt / 上下标 / 矩阵 / cases / 希腊字母 / 关系符；未知宏按字面量降级，不会让整篇编译失败。
- **预览与导出共用同一份 Typst 源码**，不存在「预览好看、PDF 崩掉」的两套排版，但预览是页面栅格图，不能选中文字。
