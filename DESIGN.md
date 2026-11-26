# Aureate 设计文档

> 一个本地 Markdown 写作器，只有一种输出：PDF。写下的每个文档、每张卡片，都能编译成排版精确、字体嵌入的 PDF。

| 项 | 值 |
|---|---|
| 应用名 | Aureate |
| 定位 | Markdown 写作器 + 研究生卡片系统 |
| bundle id | `com.zhoujin.aureate` |
| 平台 | macOS（universal：arm64 + x86_64） |
| 技术栈 | 100% Rust —— `egui 0.37`（eframe）界面 + `Typst 0.15` 排版 |
| 输出格式 | 仅 PDF |
| 参考实现 | [M2P](https://github.com/bug010101/M2P)（Electron + Chromium printToPDF） |
| 版本 | 1.0.7 |

---

## 1. 项目定位

Aureate 不是又一个 Typora 替代品。它解决的是两个不同层面的问题：

**第一层，写作**：左边写 Markdown，右边看 A4 纸张效果，一键导出 PDF。这一层是基础设施，要求稳、要求排版准。

**第二层，沉淀**：写下的每个文档都能被组织成卡片，卡片能被检索、按时间回看、按状态看板管理、编译成卡册。这一层是 Aureate 的核心差异。

一句话原则：**卡片是产品，编辑器只是载体。**

---

## 2. 平台与产物

| 平台 | 产物 | 最低版本 |
|---|---|---|
| macOS | `.dmg`（universal，arm64 + x64） | 11.0 |

不做的平台：Windows / Linux（CI 算力有限，`release.yml` 只保留 macOS）、Android（原方案已整体删除）、iOS、Web 版（与本地文件优先原则冲突）。

---

## 3. 技术选型

### 3.1 为什么是 egui，而不是 Tauri 或 Electron

硬约束是「全栈只用 Rust」。Tauri 的界面层本质上是 HTML + 系统 WKWebView，Electron 直接带一整个 Chromium，两者都引入了非 Rust 的界面技术栈。要做到字面意义上的纯 Rust，界面只能用 Rust 原生 GUI 框架。

| 维度 | egui（当前） | Tauri 2 | Electron |
|---|---|---|---|
| 界面技术 | 纯 Rust（GPU 立即模式） | HTML + WKWebView | HTML + Chromium |
| 依赖链 | 无 npm / 无 Node | 需前端资源 | 完整 Node 运行时 |
| 包体 | 几十 MB 级（含 Typst 与字体） | 40–55 MB | 150–200 MB+ |
| 预览一致性 | 与 PDF 同一引擎，逐页一致 | 需第二套 HTML 渲染，天然不一致 | 依赖 Chromium 打印 |

代价：egui 是立即模式 GUI，没有 CSS 生态，虚拟滚动、复杂文本光标、树形折叠都要手写；排版能力也只能靠 Typst 重建，不能白拿浏览器的。这是本项目最大的工作量来源。

### 3.2 核心依赖

| 用途 | crate |
|---|---|
| GUI | `eframe` / `egui` 0.37 |
| 原生文件对话框 | `rfd` 0.18 |
| Markdown 解析 | `pulldown-cmark` 0.13 |
| frontmatter | `gray_matter` + `serde_yaml` |
| 排版与 PDF | `typst` / `typst-pdf` 0.15.1 |
| 预览栅格化 | `typst-render` 0.15.1 |
| 代码高亮 | `syntect` 5 |
| 索引与检索 | `rusqlite`（FTS5） |
| 文件监听 | `notify` |

---

## 4. 功能范围

### 4.1 写作

- 编辑器：等宽字体文本编辑、`Cmd+S` 保存、`Cmd+E` 导出、`Cmd+F` 搜索、`Cmd+N` 新建
- 三种视图：纯编辑 / 分屏 / 纯预览，预览可缩放
- 文件树：层级折叠、右键重命名 / 删除（进应用内回收站）
- 单文件打开：引导页按钮或侧栏「打开文件」，父目录自动成为工作目录
- 保存：切换文档前、导出前自动落盘；状态栏显示「已保存 / 未保存」与字数

### 4.2 排版

- 四套主题：GitHub（代码/表格/引用）、学术论文（宋体、首行缩进、居中标题）、商务报告（深蓝标题）、极简（无边框、紧凑留白）
- 页面：A4 / A5 / Letter / Legal（卡册另用 A6），三档页边距
- 页眉取文档标题，页脚页码，自动目录，PDF 书签
- 一级标题另起一页；表格跨页重复表头

### 4.3 Markdown 能力

- CommonMark 基础语法；GFM 表格、任务清单、删除线、脚注
- 围栏代码块 + 语法高亮（`syntect` → Typst `raw` 颜色 span）
- 行内公式 `$...$`、块级公式 `$$...$$`（TeX → Typst 映射）
- 相对路径图片，以文档所在目录为基准
- 段内硬换行保留（SoftBreak 在 Typst 里转成段内换行，不并成空格）

### 4.4 导出

- 当前文档 PDF
- 整目录批量导出（递归，失败不中断，最后汇总成功 / 失败数）
- 卡册 PDF：一卡一页

### 4.5 卡片

- 五类卡：实验 / 论文 / 想法 / 会议 / 周
- 四视图：表单编辑 / 时间线 / 看板 / 周历
- SQLite FTS5 全文检索（标题 + 正文，带高亮片段）
- `[[卡ID]]` 双链与反向链接

### 4.6 明确不做

| 删除项 | 原因 |
|---|---|
| HTML 单文件导出 | 只保留 PDF 一种输出 |
| 内嵌 HTML 渲染（`<table>` 等） | 不做第二套 DOM 语义，表格一律用 Markdown 管道语法 |
| 命令行导出 | 不做 CLI |
| Windows / Linux / Android | 见第 2 节 |

---

## 5. 系统架构

### 5.1 crate 划分

| crate | 职责 | 对外接口 |
|---|---|---|
| `aur-core` | 文档与卡片领域模型、frontmatter、md AST | `Document`, `Card`, `CardKind` |
| `aur-vault` | 存储抽象、原子写、回收站、监听 | `trait Storage`, `FsStorage` |
| `aur-index` | SQLite 索引、FTS5 检索、反链、台账 | `Index::open()`, `search()`, `ledger()` |
| `aur-export` | AST → Typst → PDF；同一源码栅格化成预览页 | `export_markdown()`, `export_folder()`, `export_cardbook()`, `render_pages()` |
| `aur-app` | egui 界面 + 业务操作层 | `ops::*`（无框架类型，UI 直接调） |

### 5.2 依赖方向

```
aur-app
  ├── aur-core
  ├── aur-vault ── aur-core
  ├── aur-index ── aur-core
  └── aur-export ── aur-core
```

`aur-core` 不依赖本项目任何其他 crate；`aur-app` 的 `ops.rs` 里不放任何 egui 类型，界面与业务严格分离。

---

## 6. 导出管线：唯一一条

```
.md 源文件
  → aur-core 解析成 AST
  → aur-export 转成 Typst markup
  → typst 编译（Resources/fonts 的 Noto SC 四字体 + Typst 内置字体）
  → PDF（书签、页码、目录）
```

设计约束：

- 导出不经过任何平台打印 API
- 字体必须随包嵌入，禁止依赖系统字体，否则换机掉字
- 导出在后台线程执行（`std::thread` + 通道回传），界面不阻塞
- 编译失败不静默：错误与警告回传界面，状态栏提示

### 6.1 批量导出

输入一个目录，递归找 `.md`，逐个编译为同名 `.pdf`，失败不中断整批，结束后汇总成功与失败清单。

---

## 7. 预览与导出的分界（已取消）

早期方案里预览走 HTML、导出走 Typst，天然会有差异。现在的做法是**只有一条路径**：

| | 预览 | 导出 |
|---|---|---|
| 渲染器 | Typst（同一份源码） | Typst |
| 输出 | 页面栅格图（`typst-render` → 纹理） | PDF 文件 |
| 一致性 | 逐页一致 | 基准 |
| 延迟 | 停止输入后 600 ms 重编译 | 导出时一次 |

代价是预览不能选中文字、不能复制；换来的是「预览和 PDF 不一致」这类问题从根上不存在。

防抖策略：编辑后记 `preview_stale`，600 ms 无新输入才提交编译任务，避免每次按键都编译。

---

## 8. 卡片系统

### 8.1 卡片定义

一张卡 = 一个 `.md` 文件 + 固定 frontmatter。

```yaml
---
id: exp-001
kind: 实验卡        # 实验 / 论文 / 想法 / 会议 / 周
date: 2026-11-25
project: 课题一
status: done        # todo / doing / done / blocked
tags: [anomalydiffusion, 冗余验证]
---
## 问题
## 设计
## 关键命令
## 结果
## 结论
## 下一步
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | 是 | 全局唯一，创建后不可变，双链与索引都基于它 |
| `kind` | 是 | 决定表单布局与默认栏位 |
| `date` | 是 | 所属日期，时间线排序依据 |
| `project` | 否 | 归属项目，用于分组 |
| `status` | 是 | 默认 todo |
| `tags` | 否 | 字符串数组 |

### 8.2 四视图

| 视图 | 功能 |
|---|---|
| 表单编辑 | 按 section 分块编辑，保存回写 frontmatter 与正文 |
| 时间线 | 按日期倒排，看一段时间做了什么 |
| 看板 | todo / doing / done / blocked 四列，右键「移到 …」换状态 |
| 周历 | 本周卡片状态变化摘要（按周起止统计） |

### 8.3 卡册

- 单卡导出：A6 或 A5 一页
- 集合导出：按时间区间 / 项目 / tag 筛选后一卡一页拼成册
- 用途：周报、阶段总结、给导师看的进展材料

### 8.4 双向链接

- 正文中 `[[exp-001]]` 建立引用
- 反向链接基于 `id` 解析，重命名不断链

---

## 9. 数据与存储

### 9.1 文件优先原则

- 工作目录就是一个普通文件夹，内容全是 `.md` 纯文本
- 可以被 git 管理，可以被 Obsidian 等工具直接打开
- SQLite 索引是缓存，删掉能重建，不作为数据源
- 卸载 Aureate 不损失任何数据

### 9.2 运行时数据的位置

所有运行时数据都在应用包内，不污染用户目录：

| 内容 | 位置 |
|---|---|
| 索引缓存 | `data/index/<vault 路径哈希>.db` |
| 偏好设置 | `data/settings.json` |
| 回收站 | `data/trash/<vault 路径哈希>/` |

路径解析顺序：`AUREATE_DATA_DIR` → 可执行文件旁（`Contents/Resources/data`）→ `~/.aureate/data` 兜底。写不进去时索引降级为内存模式，状态栏显示「索引：内存」。

索引按工作目录路径哈希分文件，多个目录互不干扰；打开失败时自动删除 `-wal/-shm/-journal` 再试一次（崩溃自愈）。

### 9.3 存储抽象

```rust
trait Storage {
    fn read(&self, id: &str) -> Result<String>;
    fn write_atomic(&self, id: &str, content: &str) -> Result<()>;
    fn list(&self) -> Result<Vec<Entry>>;
    fn watch(&self, cb: impl Fn(Change)) -> Result<WatchHandle>;
    fn trash(&self, id: &str) -> Result<()>;
}
```

只有 `FsStorage` 一个实现：原子写（临时文件 + rename），回收站跨卷时降级为复制后删除。

---

## 10. 平台细节

| 平台 | 注意 |
|---|---|
| macOS | universal 二进制；ad-hoc 签名，未签名安装需 `xattr -cr` |

界面渲染后端：`eframe` 默认 `glow`（OpenGL）。macOS 已弃用 OpenGL 但仍在工作；若未来出现兼容问题，切到 `wgpu`（Metal）只需改 Cargo feature，代码不动。

---

## 11. 里程碑

| 阶段 | 内容 | 状态 |
|---|---|---|
| M1 | 写作器骨架 + Typst 导出（公式、高亮、表头） | 完成 |
| M2 | 四主题 + 目录 + 书签 + 页眉页脚 + 批量导出 | 完成 |
| M3 | 卡片系统（frontmatter + 四视图 + FTS + 双链） | 完成 |
| M4 | macOS 打包（GitHub Actions → dmg → Release） | 完成 |
| M5 | 界面从 Tauri/HTML 整体重写为 egui 纯 Rust | 1.0.7 |
| M6 | 打磨：暗色模式、字体管理、崩溃日志 | 进行中 |

---

## 12. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| TeX → Typst 公式映射不完整 | 学术文档错版 | 常见符号映射表 + 未知宏按字面量降级；只影响该公式，不炸整篇 |
| egui 立即模式生态不成熟 | 交互细节要自己写 | 复杂控件（树、表格）手写并保持单一实现；不用第三方 widget 库 |
| 预览每次重编译开销 | 大文档编辑卡顿 | 600 ms 防抖 + 预览在后台线程 + 结果用通道回传，主线程只贴纹理 |
| 未签名 dmg 被 Gatekeeper 拦 | 用户无法打开 | README 给出 `xattr -cr` 步骤；后续考虑开发者签名 |
| 包内写数据在只读安装时失败 | 索引不可用 | 降级内存索引 + 状态栏提示 + `~/.aureate/data` 兜底 |
| 单平台 CI 覆盖不足 | 回归遗漏 | 只保 macOS；fmt + clippy(-D warnings) + 全量测试作为合并门槛 |

---

## 13. 命名与术语

| 中文 | 英文 | 使用场景 |
|---|---|---|
| 卡片 | Card | 领域模型、界面文案 |
| 卡册 | Cardbook | 一卡一页的 PDF 产物 |
| 工作目录 / 仓库 | Vault | 用户选择的根目录 |
| 周历 | Ledger | 本周卡片状态变化摘要视图 |
| 实验卡 / 论文卡 / 想法卡 / 会议卡 / 周卡 | Experiment / Paper / Idea / Meeting / Week Card | `kind` 字段取值 |

crate 前缀统一 `aur-`。

---

## 14. 非目标

- 云同步与账号体系
- 多人协作与评论
- 网页版、移动端
- 插件市场
- 命令行工具
- HTML、Word、EPUB 等其他导出格式
- 自动更新服务（首版用 dmg 手动更新）
