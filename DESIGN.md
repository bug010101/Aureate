# Aureate 设计文档

> 一个四端本地 Markdown 写作器，只有一种输出：PDF。写下的每个文档、每张卡片，都能编译成排版精确、字体嵌入的 PDF。

| 项 | 值 |
|---|---|
| 应用名 | Aureate |
| 定位 | Markdown 写作器 + 研究生卡片系统 |
| bundle id | `com.zhoujin.aureate` |
| 平台 | Windows / macOS / Linux / Android |
| 输出格式 | 仅 PDF |
| 参考实现 | [M2P](https://github.com/bug010101/M2P)（Electron + Chromium printToPDF） |
| 文档版本 | v0.1 / 2026-11-25 |

---

## 1. 项目定位

Aureate 不是又一个 Typora 替代品。它解决的是两个不同层面的问题：

**第一层，写作**：跟 M2P 一样，左边写 Markdown，右边看 A4 纸张效果，一键导出 PDF。这一层是基础设施，要求稳、要求排版准。

**第二层，沉淀**：写下的每个文档都能被组织成卡片，卡片能被检索、按时间回看、按状态看板管理、编译成卡册。这一层是 Aureate 的核心差异。

一句话原则：**卡片是产品，编辑器只是载体。**

---

## 2. 平台与产物

| 平台 | 产物 | 最低版本 |
|---|---|---|
| macOS | `.dmg`（universal，arm64 + x64） | 11.0 |
| Windows | `.msi` / NSIS `.exe` | 10 x64 |
| Linux | `.AppImage` + `.deb` | glibc 2.28+
| Android | `.apk` / `.aab` | 8.0（API 26） |

包体目标：桌面 10 MB 级，Android 15 MB 级。

不做的平台：iOS（暂不规划）、Web 版（与本地文件优先原则冲突）。

---

## 3. 技术选型

### 3.1 为什么是 Tauri 2 而不是 Electron

Electron 没有 Android 运行时，M2P 那套架构天生到不了手机。四端要求把 Electron 排除在外，剩下的现实选择只有 Tauri 2。

| 维度 | Tauri 2 | Electron |
|---|---|---|
| Android | 支持 | 不支持 |
| 包体 | 10 MB 级 | 100 MB+ |
| 核心逻辑 | Rust 原生 | Node |
| PDF 导出 | 内嵌 Typst 编译 | Chromium printToPDF |

代价：Chromium 白拿的排版能力（打印分页、代码高亮、公式）在 Tauri 里要靠 Typst 重建。这是本项目最大的工作量来源，已在第 12 节列为风险。

### 3.2 核心依赖

| 用途 | crate |
|---|---|
| GUI 容器 | `tauri` 2 |
| Markdown 解析 | `pulldown-cmark` 或 `comrak` |
| frontmatter 解析 | `gray_matter` + `serde_yaml` |
| PDF 编译 | `typst` |
| 代码高亮 | `syntect` |
| 索引与检索 | `rusqlite`（FTS5） |
| 文件监听 | `notify` |
| YAML 序列化 | `serde_yaml` |

前端：CodeMirror 6（桌面）、原生 textarea + 高亮层（Android）。

---

## 4. 功能范围

### 4.1 从 M2P 继承

**写作体验**

- 实时预览，预览区按适应宽度自动缩放
- 编辑器：Markdown 语法高亮、行号、查找面板
- 自动保存草稿、最近文档、侧栏文件夹浏览、可拖拽分栏、界面缩放
- 拖拽文件或文件夹到窗口直接打开

**排版**

- 四套主题：GitHub（代码/表格/引用）、学术论文（宋体、首行缩进、居中标题）、商务报告（深蓝标题、蓝系表格）、极简（无边框、紧凑留白）
- 页面：A4 / A5 / Letter / Legal，纵向 / 横向，三档页边距
- 页眉取文档标题或自定义文案，支持 `{title}`、`{date}` 占位
- 页脚：第 X 页 / 共 Y 页

**文档结构**

- 自动目录（H1–H3），可单独占页
- 一级标题另起一页
- PDF 书签大纲
- 表格跨页重复表头，表格行不跨页切断

**Markdown 能力**

- CommonMark 基础语法
- GFM 表格、任务清单
- 脚注 `[^1]`，文末自动收集并附回链
- 围栏代码块，语言标签，语法高亮
- 行内公式 `$...$`、块级公式 `$$...$$`
- 相对路径图片，以文档所在目录为基准

**导出**

- PDF 导出（当前文档）
- 批量导出整个文件夹（含子目录），带进度

### 4.2 明确删除

| 删除项 | 原因 |
|---|---|
| HTML 单文件导出 | 只保留 PDF 一种输出 |
| WebView 打印 PDF（辅路径） | 双引擎等于双份 bug，四端行为还不一致 |
| 命令行导出与参数体系 | 不做 CLI，无 `--export` / `--out` / `--html` |
| HTML 内嵌 KaTeX 字体（base64） | 无 HTML 输出即无此需求 |

### 4.3 新增

- 卡片系统（frontmatter 驱动的结构化笔记）
- 卡片四视图：表单编辑 / Timeline / Board / Ledger
- Cardbook：卡册 PDF（一卡一页）
- 卡内命令可执行（存为脚本）
- 全文检索（SQLite FTS5）
- 双向链接与反向链接面板

---

## 5. 系统架构

### 5.1 crate 划分

| crate | 职责 | 对外接口 |
|---|---|---|
| `aur-core` | 文档与卡片领域模型、frontmatter 解析、md AST | `Document`, `Card`, `CardKind`, `parse()` |
| `aur-vault` | 存储抽象、原子写、目录监听 | `trait Storage` |
| `aur-index` | SQLite 索引、FTS5 检索、反链图 | `Index::open()`, `search()` |
| `aur-render` | md AST → HTML，仅服务屏幕预览 | `render_preview()` |
| `aur-export` | md AST → Typst markup → PDF | `export_pdf()`, `export_folder()` |
| `aur-app` | Tauri 窗口、命令、快捷键、分享接收 | Tauri commands |

### 5.2 依赖方向

```
aur-app
  ├── aur-core
  ├── aur-vault ── aur-core
  ├── aur-index ── aur-core
  ├── aur-render ── aur-core
  └── aur-export ── aur-core
```

底层 crate 不依赖上层，`aur-core` 不依赖任何其他本项目 crate。

### 5.3 平台差异只允许出现在三处

1. **存储**：桌面走文件系统路径，Android 走 SAF 的 `content://` URI
2. **输入**：桌面 CodeMirror 6 全功能，Android 降级为 textarea
3. **字体加载**：桌面读系统字体，Android 只能读打包进 APK 的字体

导出路径不在此列，因为 Typst 在 Rust 内编译，四端完全一致。

---

## 6. 导出管线：唯一一条

```
.md 源文件
  → aur-core 解析成 AST
  → aur-export 转成 Typst markup
  → typst crate 编译（嵌入思源宋体 / 思源黑体子集）
  → PDF（带书签、页码、目录）
```

设计约束：

- 导出过程不经过任何平台打印 API
- 四端输出逐字节一致
- 字体必须嵌入子集，禁止依赖系统字体，否则换机掉字
- 导出是同步阻塞操作，放在 Tauri 的 `spawn_blocking` 里，界面显示进度

### 6.1 批量导出

输入一个目录，递归（限 4 层）找出所有 `.md`，逐个编译为同名 `.pdf`，输出到同级或指定目录，全程显示进度与失败清单。失败不中断整批，最后汇总。

---

## 7. 预览与导出的分界

这是本设计里最需要守住的一条边界。

| | 预览 | 导出 |
|---|---|---|
| 渲染器 | HTML + `@page` CSS | Typst |
| 目的 | 毫秒级刷新，看结构 | 精确排版，交付 |
| 是否产出文件 | 否 | 是，PDF |
| 允许差异 | 与最终 PDF 有毫米级差异 | 基准 |

为什么预览不用 Typst：Typst 编译一次需数百毫秒，无法支撑实时预览。因此界面不承诺预览与 PDF 像素一致，只承诺内容与结构一致。

两条路径共用同一个 AST（`aur-core` 输出），这是保证内容一致的唯一手段。

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

字段约束：

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | 是 | 全局唯一，创建后不可变，用于双链与索引，不能用文件路径代替 |
| `kind` | 是 | 决定表单布局与默认栏位 |
| `date` | 是 | 卡片所属日期，Timeline 排序依据 |
| `project` | 否 | 归属项目，用于分组 |
| `status` | 是 | 默认 todo |
| `tags` | 否 | 字符串数组 |

### 8.2 四视图

| 视图 | 功能 | 桌面 | Android |
|---|---|---|---|
| 表单编辑 | 左侧六格字段，右侧实时预览卡片效果 | 分栏 | 上下切换 |
| Timeline | 按日期倒排，看见一段时间内做了什么 | 支持 | 主用 |
| Board | todo / doing / done / blocked 四列，拖拽换状态 | 拖拽 | 点按切换 |
| Ledger | 本周卡片状态变化摘要，零成本周结算 | 支持 | 支持 |

### 8.3 Cardbook

- 单卡导出：A6 或 A5 一页
- 集合导出：按时间区间、项目或 tag 筛选后，一卡一页拼成册
- 用途：周报、阶段总结、给导师看的进展材料

### 8.4 卡内命令可执行

`## 关键命令` 下的代码块渲染时附带操作：

- 复制：一键复制到剪贴板
- 存为脚本：写入 `vault/scripts/<card-id>-<n>.sh`，与卡绑定
- 查看历史：同一张卡的多次命令执行记录

### 8.5 双向链接

- 正文中 `[[exp-001]]` 建立引用
- 反向链接面板显示哪些卡引用了当前卡
- 链接解析基于 `id`，不依赖文件名，重命名不断链

---

## 9. 数据与存储

### 9.1 文件优先原则

- vault 就是一个普通文件夹，内容全是 `.md` 纯文本
- 可以被 git 管理，可以被 Obsidian 等其他工具直接打开
- SQLite 索引是缓存，删掉能重建，不作为数据源
- 卸载 Aureate 不损失任何数据

这条原则优先于任何功能便利性。工具不该绑架数据。

### 9.2 存储抽象

```rust
trait Storage {
    fn read(&self, id: &str) -> Result<String>;
    fn write(&self, id: &str, content: &str) -> Result<()>;
    fn list(&self, dir: Option<&str>) -> Result<Vec<Entry>>;
    fn watch(&self, cb: impl Fn(Change)) -> Result<WatchHandle>;
    fn remove(&self, id: &str) -> Result<()>;
}
```

| 实现 | 使用平台 | 要点 |
|---|---|---|
| `FsStorage` | 桌面三端 | 原子写：写临时文件再 rename |
| `SafStorage` | Android | `content://` URI，持久化权限需重新申请 |

### 9.3 索引

- 位置：桌面 `vault/.aureate/index.db`，Android 应用私有目录
- 表：`cards(id, kind, date, project, status, path, mtime)`、`cards_fts`（FTS5）
- 重建：启动时校验 mtime，变化则增量更新；提供手动全量重建入口

---

## 10. 平台细节

### 10.1 三端桌面

| 平台 | 注意 |
|---|---|
| macOS | universal 二进制；未签名需告知用户 `xattr -cr` |
| Windows | WebView2 运行时由安装器引导安装 |
| Linux | 中文字体需用户自备或打包 Noto CJK；WebKitGTK 版本差异需冒烟 |

### 10.2 Android 的定位

Android 版是**采集端，不是写作端**。

| 做 | 不做 |
|---|---|
| 系统分享菜单收文字/链接/图片，一键成卡 | 长文写作 |
| 看 Timeline、检索、改状态 | 复杂排版调整 |
| 单卡导出 A6 PDF | 批量导出、卡册 |

### 10.3 四端之间怎么同步

Aureate 不实现云同步。vault 是普通文件夹，用户自行选择：

- 电脑与手机之间：Syncthing
- 电脑之间：git

这样既保持了数据自由，也避免了一个人维护同步协议的巨大成本。冲突处理交给同步工具，Aureate 只负责读写本地文件。

---

## 11. 里程碑

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| M1 | 写作器骨架 + Typst 导出三件套（公式、高亮、表头） | macOS 导出的 PDF 与 M2P 产物对比无明显降级 |
| M2 | 四主题 + 目录 + 书签 + 页眉页脚 + 批量导出 | 20 页带目录文档导出正确，书签可跳转 |
| M3 | 卡片系统（frontmatter + 四视图 + FTS + 双链） | 记满 50 张卡，检索与 Board 操作流畅 |
| M4 | 三端打包 | Windows / Linux 安装包可装可跑 |
| M5 | Android（SAF + 分享接收 + 移动编辑器） | 手机能记卡，能导出单卡 PDF |
| M6 | 打磨（暗色模式、字体管理、本地崩溃日志） | 连续使用一周无阻断性问题 |

M1 的第一件事不是搭框架，而是**先做 Typst 三件套的技术验证**。这三件不通，整个导出方案要重议。

---

## 12. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| LaTeX 公式转 Typst 不完整 | 学术文档导出失败或错版 | 常见符号做映射表；复杂公式先用 KaTeX 渲成 SVG 再嵌入，保真但不可选中 |
| 代码高亮需重写 | 代码块无色 | 用 `syntect` 生成带颜色 span，映射到 Typst `raw` 元素 |
| 表格跨页表头映射 | 长表格断页后无表头 | 识别 AST 的表头行，映射为 Typst `table.header` |
| typst crate 交叉编译到 Android | Android 端无法导出 | 优先验证 NDK 工具链；不可行则 Android 降级为仅导出单卡 PDF |
| 四端测试矩阵过大 | 质量失控 | 只做 macOS 全组合回归，其余平台各一组冒烟 |
| SAF 权限失忆 | 重启后读不到 vault | 启动时校验授权，失效则引导重新选择目录 |

---

## 13. 命名与术语

| 中文 | 英文 | 使用场景 |
|---|---|---|
| 卡片 | Card | 领域模型、界面文案 |
| 卡册 | Cardbook | 一卡一页的 PDF 产物 |
| 仓库 | Vault | 用户选择的根目录 |
| 周历 | Ledger | 本周卡片状态变化摘要视图 |
| 实验卡 | Experiment Card | `kind: 实验卡` |
| 论文卡 | Paper Card | `kind: 论文卡` |
| 想法卡 | Idea Card | `kind: 想法卡` |
| 会议卡 | Meeting Card | `kind: 会议卡` |
| 周卡 | Week Card | `kind: 周` |

crate 前缀统一 `aur-`。

---

## 14. 非目标

明确不做，避免范围蔓延：

- 云同步与账号体系
- 多人协作与评论
- 网页版
- 移动端长文写作体验
- 插件市场
- 命令行工具
- HTML、Word、EPUB 等其他导出格式
- 自动更新服务（首版用安装包手动更新）
