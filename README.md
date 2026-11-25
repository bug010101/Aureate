# Aureate

本地优先的 Markdown 写作器：**屏幕上预览，纸上只出 PDF**。桌面三端（macOS / Windows / Linux）。

- **纯 Rust 后端**：`pulldown-cmark` → 自有 AST → **Typst 0.15** 排版 → PDF。所有平台产出字节级一致的 PDF，字体随包嵌入（Noto Serif/Sans SC + Typst 自带数学/西文字体），不依赖系统环境。
- **零构建前端**：`ui/` 目录是原生 HTML/CSS/JS，无 npm、无打包器。全部逻辑在 Rust 命令层（30+ 个 Tauri 命令）。
- **公式**：预览与 PDF 用同一个 Typst 编译器渲染（`preview_math` 把 TeX 转成行内 SVG），所见即所得。
- **研究卡**：实验卡/论文卡/想法卡等六段骨架卡，SQLite FTS5 全文检索、反向链接、进度台账；`[[卡ID]]` 双链。
- **导出**：单文档 PDF / 整仓批量 PDF / 卡册 PDF（一卡一页 A6/A5）。
- **不落数据**：运行时数据（索引缓存、偏好设置、回收站）全部写在应用包内的 `data/` 目录，电脑上其他位置不留任何文件；启动时永远由你自己选文件夹，程序不会自动打开或创建任何目录。
- **单文件打开**：可以直接打开一个 `.md` 文件（引导页按钮、侧栏「打开文件」、拖拽均可），所在文件夹自动成为本次工作目录。
- **目录边界**：程序只会打开你工作目录里的文件和本次导出的 PDF，绝不打开应用自身目录；拖拽 `.md` 进窗口同样直接打开。

## 工作区结构

```
crates/aur-core      Markdown/frontmatter/卡片领域模型（无 Tauri/Typst 依赖）
crates/aur-vault     仓库存储：原子写、回收站、文件监听
crates/aur-index     SQLite FTS5 索引、反链、台账
crates/aur-render    AST → 预览 HTML（syntect 高亮，XSS 转义）
crates/aur-export    AST → Typst 源码 → PDF/SVG 公式
crates/aur-app       Tauri 壳：命令层、字体、图标（桌面三端）
ui/                  零构建前端（编辑器/预览/卡片/搜索/导出/设置）
.github/workflows    ci.yml（三平台测试）+ release.yml（三端打包）
```

## 本地开发（只需 Rust）

```bash
cargo test --workspace          # 41 个测试
cargo run -p aur-app            # 桌面调试运行
```

启动后自己选一个文件夹当仓库，或直接选一个 `.md` 文件（其所在文件夹即工作目录）。每次启动都要手动选，程序不会记住路径、不会自动打开。Aureate 只往这个文件夹写你自己的 `.md`，不建索引目录、不建 `docs/`、不建回收站。

## GitHub Actions 打包（唯一出包方式）

- 推送 tag `v*` 或手动触发 **release** workflow：
  - **macOS**：universal dmg（aarch64 + x86_64）
  - **Windows**：msi + nsis 安装器
  - **Linux**：deb + AppImage
  产物在各 job 的 **Artifacts** 里下载。
- 每次 push/PR 会跑 **ci** workflow：三平台 fmt + clippy(-D warnings) + 全量测试。

## macOS 首次打开未签名应用

Action 产物没有开发者签名。下载 dmg 安装后若提示"无法打开"：

```bash
xattr -cr /Applications/Aureate.app
```

## 数据放在哪

| 内容 | 位置 |
| --- | --- |
| 你的文档 | 你自己选的那个文件夹，只有 `.md` |
| 索引缓存 / 偏好 / 回收站 | 应用包内 `data/`（macOS 为 `Aureate.app/Contents/Resources/data`） |

装在 `/Applications`（只读）时无法往包内写，才会退回系统用户数据目录，界面会提示"索引暂不可用"。开发态和放进 `~/Applications` 时都符合上表。

## 已知边界

- 公式：TeX → Typst 转换覆盖 frac/sqrt/上下标/矩阵/cases/希腊字母/关系符；未知宏按字面量降级显示，不会导致整篇编译失败。
