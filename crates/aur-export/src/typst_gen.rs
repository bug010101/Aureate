//! Markdown AST → Typst markup.
//!
//! Everything the PDF needs is decided here: page geometry, theme, headings,
//! tables, code, formulas, footnotes and images.  The Typst source is also
//! kept when a compile fails, so the error can point at real generated code.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use aur_core::ast::{plain_all, walk_blocks, Align, Block, Inline};
use aur_core::Document;

use crate::fonts::FontSet;
use crate::math;
use crate::options::{ExportOptions, Margin, Paper, Theme};

/// Real files mapped to virtual Typst paths (`assets/<hash>.png`).
#[derive(Debug, Default)]
pub struct Assets {
    pub map: HashMap<String, PathBuf>,
    base_dir: PathBuf,
    warnings: Vec<String>,
}

impl Assets {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Assets {
            map: HashMap::new(),
            base_dir: base_dir.into(),
            warnings: Vec::new(),
        }
    }

    pub fn register(&mut self, src: &str) -> Option<String> {
        let src = src.trim();
        if src.is_empty() {
            return None;
        }
        if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("data:") {
            self.warnings
                .push(format!("跳过远程图片（导出不联网）：{src}"));
            return None;
        }
        let raw = src.trim_start_matches("file://");
        let path = Path::new(raw);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        };
        if !resolved.exists() {
            self.warnings
                .push(format!("图片不存在：{}", resolved.display()));
            return None;
        }
        let ext = resolved
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let mut hasher = DefaultHasher::new();
        resolved.to_string_lossy().hash(&mut hasher);
        let vpath = format!("assets/{:016x}.{ext}", hasher.finish());
        self.map.insert(vpath.clone(), resolved);
        Some(vpath)
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

struct Palette {
    heading: &'static str,
    accent: &'static str,
    body: &'static str,
    muted: &'static str,
    rule: &'static str,
    code_bg: &'static str,
    code_border: &'static str,
    table_border: &'static str,
    table_head_bg: &'static str,
    quote_bar: &'static str,
    heading_serif: bool,
    h1: f32,
    h2: f32,
    h3: f32,
    h1_center: bool,
    h1_rule: bool,
    justify: bool,
}

fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Github => Palette {
            heading: "#1f2328",
            accent: "#0969da",
            body: "#1f2328",
            muted: "#656d76",
            rule: "#d8dee4",
            code_bg: "#f6f8fa",
            code_border: "#e4e8ec",
            table_border: "#d0d7de",
            table_head_bg: "#f6f8fa",
            quote_bar: "#d0d7de",
            heading_serif: false,
            h1: 19.0,
            h2: 15.0,
            h3: 13.0,
            h1_center: false,
            h1_rule: true,
            justify: true,
        },
        Theme::Academic => Palette {
            heading: "#000000",
            accent: "#000000",
            body: "#000000",
            muted: "#444444",
            rule: "#000000",
            code_bg: "#f7f7f7",
            code_border: "#dddddd",
            table_border: "#000000",
            table_head_bg: "#ffffff",
            quote_bar: "#000000",
            heading_serif: true,
            h1: 16.0,
            h2: 14.0,
            h3: 12.0,
            h1_center: true,
            h1_rule: false,
            justify: true,
        },
        Theme::Business => Palette {
            heading: "#1f3864",
            accent: "#2e5c9a",
            body: "#1a1a1a",
            muted: "#5a6b85",
            rule: "#c9d5e8",
            code_bg: "#f5f7fb",
            code_border: "#dbe3f0",
            table_border: "#b8c8e0",
            table_head_bg: "#dfe7f3",
            quote_bar: "#2e5c9a",
            heading_serif: false,
            h1: 20.0,
            h2: 15.0,
            h3: 12.5,
            h1_center: false,
            h1_rule: true,
            justify: true,
        },
        Theme::Minimal => Palette {
            heading: "#111111",
            accent: "#111111",
            body: "#222222",
            muted: "#777777",
            rule: "#e5e5e5",
            code_bg: "#fafafa",
            code_border: "#eeeeee",
            table_border: "#e0e0e0",
            table_head_bg: "#ffffff",
            quote_bar: "#dddddd",
            heading_serif: false,
            h1: 18.0,
            h2: 14.0,
            h3: 12.0,
            h1_center: false,
            h1_rule: false,
            justify: false,
        },
    }
}

pub struct GenResult {
    pub source: String,
    pub assets: Assets,
    pub warnings: Vec<String>,
    pub headings: usize,
}

pub struct Generator<'a> {
    opts: &'a ExportOptions,
    fonts: &'a FontSet,
    pal: Palette,
    assets: Assets,
    warnings: Vec<String>,
    footnote_numbers: HashMap<String, usize>,
    footnote_defs: HashMap<String, Vec<Block>>,
    footnotes_used: Vec<String>,
    headings: usize,
}

pub fn generate(
    doc: &Document,
    opts: &ExportOptions,
    fonts: &FontSet,
    base_dir: &Path,
) -> GenResult {
    let mut g = Generator {
        opts,
        fonts,
        pal: palette(opts.theme),
        assets: Assets::new(base_dir),
        warnings: Vec::new(),
        footnote_numbers: HashMap::new(),
        footnote_defs: HashMap::new(),
        footnotes_used: Vec::new(),
        headings: 0,
    };
    g.scan(doc);

    let title = if opts.title.trim().is_empty() {
        doc.title.clone()
    } else {
        opts.title.trim().to_string()
    };

    let mut out = String::new();
    out.push_str("// Generated by Aureate. Do not edit.\n");
    out.push_str(&g.preamble(&title, doc));

    if opts.toc && g.headings > 1 {
        out.push_str("#outline(title: [目录], depth: 3, indent: auto)\n");
        if opts.toc_own_page {
            out.push_str("#pagebreak()\n");
        } else {
            out.push_str("#v(12pt)\n");
        }
    }

    for block in &doc.blocks {
        g.block(block, &mut out, 0);
    }

    g.endnotes(&mut out);

    let warnings = g
        .warnings
        .iter()
        .chain(g.assets.warnings().iter())
        .cloned()
        .collect();

    GenResult {
        source: out,
        assets: g.assets,
        warnings,
        headings: g.headings,
    }
}

impl<'a> Generator<'a> {
    fn scan(&mut self, doc: &Document) {
        let mut visit = |b: &Block| match b {
            Block::Heading { level, .. } => {
                if *level <= 3 {
                    self.headings += 1;
                }
            }
            Block::FootnoteDef { label, blocks } => {
                self.footnote_defs.insert(label.clone(), blocks.clone());
            }
            _ => {}
        };
        walk_blocks(&doc.blocks, &mut visit);

        // Number notes by order of first reference, like a paper.
        let mut used = Vec::new();
        let mut visit = |b: &Block| {
            if let Block::Paragraph(inlines) | Block::Heading { inlines, .. } = b {
                collect_refs(inlines, &mut used);
            }
        };
        walk_blocks(&doc.blocks, &mut visit);
        for (i, label) in used.iter().enumerate() {
            self.footnote_numbers.insert(label.clone(), i + 1);
        }
        self.footnotes_used = used;
    }

    fn font_list(&self, serif: bool) -> String {
        if serif {
            format!(
                "(\"{}\", \"{}\", \"Libertinus Serif\")",
                self.fonts.serif, self.fonts.sans
            )
        } else {
            format!(
                "(\"{}\", \"{}\", \"Libertinus Serif\")",
                self.fonts.sans, self.fonts.serif
            )
        }
    }

    fn preamble(&self, title: &str, _doc: &Document) -> String {
        let p = &self.pal;
        let (mx, my) = self.opts.margin.cm();
        let body_font = if self.opts.theme == Theme::Academic {
            self.font_list(true)
        } else {
            self.font_list(false)
        };
        let heading_font = if p.heading_serif {
            self.font_list(true)
        } else {
            self.font_list(false)
        };

        let mut s = String::new();
        s.push_str(&format!(
            "#set page(\n  paper: \"{}\",\n  flipped: {},\n  margin: (x: {mx}cm, y: {my}cm),\n",
            self.opts.paper.typst_name(),
            if self.opts.landscape { "true" } else { "false" },
        ));

        let header_text = self.header_text(title);
        if !header_text.is_empty() {
            s.push_str(&format!(
                "  header: context {{\n    set text(size: 8.5pt, fill: rgb(\"{}\"), font: {})\n    align(right)[{}]\n  }},\n",
                p.muted, body_font, esc(&header_text)
            ));
        }
        if self.opts.footer_page_numbers {
            s.push_str(&format!(
                "  footer: context {{\n    set text(size: 8.5pt, fill: rgb(\"{}\"), font: {})\n    let total = counter(page).final().first()\n    align(center)[第 #counter(page).display() 页 / 共 #total 页]\n  }},\n",
                p.muted, body_font
            ));
        }
        s.push_str(")\n\n");

        s.push_str(&format!(
            "#set text(font: {body_font}, size: {}pt, lang: \"zh\", region: \"cn\")\n",
            self.opts.font_size
        ));
        s.push_str(&format!(
            "#set par(justify: {}, leading: {}em, spacing: 1.0em)\n",
            p.justify, self.opts.line_height
        ));
        if self.opts.first_line_indent {
            s.push_str("#set par(first-line-indent: (amount: 2em, all: false))\n");
        }
        s.push_str(&format!("#set text(fill: rgb(\"{}\"))\n", p.body));
        s.push_str("#set list(indent: 1.6em, body-indent: 0.6em, spacing: 0.85em)\n");
        s.push_str(
            "#set enum(indent: 1.8em, body-indent: 0.6em, spacing: 0.85em, numbering: \"1.\")\n",
        );
        s.push_str("#show link: set text(fill: rgb(\"#0969da\"))\n");

        // Headings
        let heading_color = p.heading;
        for (level, size) in [(1u8, p.h1), (2, p.h2), (3, p.h3), (4, p.h3 - 1.0)] {
            let above = if level == 1 { 0.0 } else { 16.0 };
            if level == 1 && p.h1_rule {
                s.push_str(&format!(
                    "#show heading.where(level: 1): it => block(above: 0pt, below: 14pt, width: 100%)[\n  #align({})[#text(font: {heading_font}, size: {size}pt, weight: \"bold\", fill: rgb(\"{heading_color}\"))[#it.body]]\n  #v(5pt)\n  #line(length: 100%, stroke: 0.7pt + rgb(\"{}\"))\n]\n",
                    if p.h1_center { "center" } else { "left" },
                    p.rule
                ));
            } else {
                s.push_str(&format!(
                    "#show heading.where(level: {level}): it => block(above: {above}pt, below: {}pt, width: 100%)[#align({})[#text(font: {heading_font}, size: {size}pt, weight: \"bold\", fill: rgb(\"{heading_color}\"))[#it.body]]]\n",
                    if level == 1 { 16 } else { 8 },
                    if level == 1 && p.h1_center { "center" } else { "left" }
                ));
            }
        }

        // Raw code
        s.push_str(&format!(
            "#show raw: set text(size: 0.86em, font: (\"DejaVu Sans Mono\", \"{}\"))\n",
            self.fonts.mono
        ));
        s.push_str(&format!(
            "#show raw.where(block: true): block.with(width: 100%, fill: rgb(\"{}\"), inset: (x: 12pt, y: 10pt), radius: 3pt, stroke: 0.4pt + rgb(\"{}\"))\n",
            p.code_bg, p.code_border
        ));
        s.push_str(&format!(
            "#show raw.where(block: false): box.with(fill: rgb(\"{}\"), inset: (x: 3pt, y: 1pt), radius: 2pt)\n",
            p.code_bg
        ));

        // Tables
        if self.opts.theme == Theme::Academic {
            s.push_str("#set table(stroke: none, inset: (x: 9pt, y: 7pt), align: horizon)\n");
        } else {
            s.push_str(&format!(
                "#set table(stroke: 0.4pt + rgb(\"{}\"), inset: (x: 8pt, y: 6pt), fill: (col, row) => if row == 0 {{ rgb(\"{}\") }} else {{ none }})\n",
                p.table_border, p.table_head_bg
            ));
        }

        // Quotes
        s.push_str(&format!(
            "#show quote.where(block: true): it => block(width: 100%, inset: (left: 14pt, right: 8pt, y: 4pt), stroke: (left: 2.4pt + rgb(\"{}\")), text(fill: rgb(\"{}\"), it.body))\n",
            p.quote_bar, p.muted
        ));
        s.push_str("#set figure(gap: 10pt)\n");
        s.push_str(&format!(
            "#show figure.caption: set text(size: 0.85em, fill: rgb(\"{}\"))\n",
            p.muted
        ));
        s.push_str("#set footnote(numbering: \"1\")\n\n");
        s
    }

    fn header_text(&self, title: &str) -> String {
        let mut t = self.opts.header.trim().to_string();
        if t.is_empty() {
            return t;
        }
        let date = if self.opts.date.trim().is_empty() {
            chrono::Local::now().format("%Y-%m-%d").to_string()
        } else {
            self.opts.date.trim().to_string()
        };
        t = t.replace("{title}", title).replace("{date}", &date);
        t
    }

    fn block(&mut self, block: &Block, out: &mut String, _depth: usize) {
        match block {
            Block::None => {}
            Block::Heading { level, inlines } => {
                if *level == 1 && self.opts.h1_page_break {
                    out.push_str("#pagebreak(weak: true)\n");
                }
                out.push_str(&format!(
                    "#heading(level: {}, outlined: true)[{}]\n\n",
                    (*level).clamp(1, 6),
                    self.inlines(inlines)
                ));
            }
            Block::Paragraph(inlines) => {
                if inlines.is_empty() {
                    return;
                }
                out.push_str("#h(0pt)");
                out.push_str(&self.inlines(inlines));
                out.push_str("\n\n");
            }
            Block::CodeBlock { lang, code } => {
                let lang = lang
                    .as_deref()
                    .map(|l| l.split(',').next().unwrap_or(l).trim())
                    .filter(|l| {
                        !l.is_empty()
                            && l.chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    });
                match lang {
                    Some(lang) => out.push_str(&format!(
                        "#raw({}, block: true, lang: \"{}\")\n\n",
                        typst_string(code),
                        lang
                    )),
                    None => out.push_str(&format!("#raw({}, block: true)\n\n", typst_string(code))),
                }
            }
            Block::List {
                ordered,
                start,
                items,
                ..
            } => {
                let func = if *ordered { "enum" } else { "list" };
                out.push_str(&format!("#{func}("));
                if *ordered && *start > 1 {
                    out.push_str(&format!("start: {start}, "));
                }
                for item in items {
                    out.push_str("\n  [");
                    if let Some(checked) = item.checked {
                        out.push_str(&self.checkbox(checked));
                        out.push_str("#h(0.45em)");
                    }
                    for (i, b) in item.blocks.iter().enumerate() {
                        if i > 0 {
                            out.push_str("\n\n");
                        }
                        let mut inner = String::new();
                        self.block(b, &mut inner, _depth + 1);
                        out.push_str(inner.trim_end());
                    }
                    out.push_str("],");
                }
                out.push_str("\n)\n\n");
            }
            Block::Table { aligns, head, rows } => {
                self.table(aligns, head, rows, out);
            }
            Block::Quote(children) => {
                out.push_str("#quote(block: true)[\n");
                for child in children {
                    self.block(child, out, _depth + 1);
                }
                out.push_str("]\n\n");
            }
            Block::Rule => {
                out.push_str(&format!(
                    "#v(8pt)\n#line(length: 100%, stroke: 0.5pt + rgb(\"{}\"))\n#v(8pt)\n\n",
                    self.pal.rule
                ));
            }
            Block::Image { src, title, alt } => {
                let Some(vpath) = self.assets.register(src) else {
                    return;
                };
                let alt_text = title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| plain_all(alt));
                if alt_text.trim().is_empty() {
                    out.push_str(&format!(
                        "#align(center)[#image(\"{}\", width: 78%)]\n\n",
                        vpath
                    ));
                } else {
                    out.push_str(&format!(
                        "#figure(image(\"{}\", width: 78%), caption: [{}])\n\n",
                        vpath,
                        esc(&alt_text)
                    ));
                }
            }
            Block::Math { tex } => {
                let converted = math::convert(tex);
                if self.opts.math_as_text {
                    out.push_str(&format!(
                        "#block(above: 8pt, below: 8pt)[#align(center)[#text(font: (\"DejaVu Sans Mono\",))[{}]]]\n\n",
                        esc(tex)
                    ));
                } else {
                    out.push_str(&format!(
                        "#block(above: 10pt, below: 10pt, width: 100%)[#align(center)[$ {} $]]\n\n",
                        converted
                    ));
                }
            }
            Block::Html(html) => {
                if !html.trim().is_empty() {
                    self.warnings
                        .push("已忽略内嵌 HTML（PDF 只渲染 Markdown 结构）".into());
                }
            }
            Block::FootnoteDef { .. } => {
                // Rendered at the end.
            }
        }
    }

    fn checkbox(&self, checked: bool) -> String {
        if checked {
            format!(
                "#box(width: 0.78em, height: 0.78em, fill: rgb(\"{}\"), radius: 1.5pt, baseline: 0.08em)[#align(center + horizon)[#text(fill: white, size: 0.6em)[✓]]]",
                self.pal.accent
            )
        } else {
            format!(
                "#box(width: 0.78em, height: 0.78em, stroke: 0.5pt + rgb(\"{}\"), radius: 1.5pt, baseline: 0.08em)",
                self.pal.muted
            )
        }
    }

    fn table(
        &mut self,
        aligns: &[Align],
        head: &[Vec<Vec<Inline>>],
        rows: &[Vec<Vec<Inline>>],
        out: &mut String,
    ) {
        let cols = head
            .iter()
            .chain(rows.iter())
            .map(|r| r.len())
            .max()
            .unwrap_or(0);
        if cols == 0 {
            return;
        }

        out.push_str("#align(center)[#table(\n");
        out.push_str(&format!(
            "  columns: {},\n  align: ({}),\n",
            cols,
            (0..cols)
                .map(|i| match aligns.get(i).copied().unwrap_or(Align::None) {
                    Align::Left => "left",
                    Align::Center => "center",
                    Align::Right => "right",
                    Align::None => "left",
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));

        if self.opts.theme == Theme::Academic {
            out.push_str("  table.hline(stroke: 1pt),\n");
        }

        if let Some(row) = head.first() {
            out.push_str(&format!("  table.header({}),\n", self.cells(row, cols)));
        }
        for row in head.iter().skip(1) {
            out.push_str(&format!("  {},\n", self.cells(row, cols)));
        }
        if self.opts.theme == Theme::Academic {
            out.push_str("  table.hline(stroke: 0.6pt),\n");
        }

        for row in rows {
            out.push_str(&format!("  {},\n", self.cells(row, cols)));
        }

        if self.opts.theme == Theme::Academic {
            out.push_str("  table.hline(stroke: 1pt),\n");
        }
        out.push_str(")]\n\n");
    }

    /// One comma-separated argument per cell: Typst's `table` takes cells as
    /// separate positional arguments, not as one concatenated content block.
    fn cells(&mut self, row: &[Vec<Inline>], cols: usize) -> String {
        let mut cells: Vec<String> = Vec::with_capacity(cols);
        for cell in row {
            cells.push(format!("[{}]", self.inlines(cell)));
        }
        for _ in row.len()..cols {
            cells.push("[]".to_string());
        }
        cells.join(", ")
    }

    fn inlines(&mut self, inlines: &[Inline]) -> String {
        let mut s = String::new();
        for inline in inlines {
            s.push_str(&self.inline(inline));
        }
        s
    }

    fn inline(&mut self, inline: &Inline) -> String {
        match inline {
            Inline::Text(t) => esc(t),
            Inline::Code(code) => format!("#raw({})", typst_string(code)),
            Inline::Strong(c) => format!("#strong[{}]", self.inlines(c)),
            Inline::Emph(c) => format!("#emph[{}]", self.inlines(c)),
            Inline::Strike(c) => format!("#strike[{}]", self.inlines(c)),
            Inline::Link { url, children, .. } => {
                let text = if children.is_empty() {
                    esc(url)
                } else {
                    self.inlines(children)
                };
                if url.starts_with("http://")
                    || url.starts_with("https://")
                    || url.starts_with("mailto:")
                {
                    format!("#link({})[{}]", typst_string(url), text)
                } else {
                    // Vault-relative links are not resolvable inside a PDF.
                    text
                }
            }
            Inline::Image { src, alt, .. } => {
                if let Some(vpath) = self.assets.register(src) {
                    format!(
                        "#box(image(\"{}\", height: 1.1em, fit: \"contain\"))",
                        vpath
                    )
                } else {
                    esc(&plain_all(alt))
                }
            }
            Inline::Math(tex) => {
                if self.opts.math_as_text {
                    format!(
                        "#text(font: (\"DejaVu Sans Mono\",), size: 0.92em)[{}]",
                        esc(tex)
                    )
                } else {
                    format!("${}$", math::convert(tex))
                }
            }
            Inline::FootnoteRef(label) => {
                let n = self.footnote_numbers.get(label).copied().unwrap_or(0);
                if n == 0 {
                    return format!("#super[{}]", esc(label));
                }
                format!(
                    "#super[#link(<fn-{n}>)[#text(fill: rgb(\"{}\"))[{}]]#label(\"fnref-{n}\")]",
                    self.pal.accent, n
                )
            }
            Inline::WikiLink(id) => {
                format!(
                    "#text(fill: rgb(\"{}\"), weight: \"medium\")[{}]",
                    self.pal.accent,
                    esc(id)
                )
            }
            // `\` + space is Typst's in-paragraph line break — the same
            // promise the preview makes with <br/>.
            Inline::SoftBreak => "\\ ".into(),
            Inline::HardBreak => "#linebreak()#h(0pt)".into(),
        }
    }

    fn endnotes(&mut self, out: &mut String) {
        let used = self.footnotes_used.clone();
        if used.is_empty() {
            return;
        }

        out.push_str("#v(24pt)\n");
        out.push_str("#heading(level: 2, outlined: false)[注]\n");
        out.push_str("#set par(first-line-indent: 0pt)\n");

        for label in &used {
            let Some(n) = self.footnote_numbers.get(label).copied() else {
                continue;
            };
            let Some(blocks) = self.footnote_defs.get(label).cloned() else {
                continue;
            };
            out.push_str(&format!(
                "#block(above: 4pt, below: 4pt, width: 100%)[#label(\"fn-{n}\")\n"
            ));
            out.push_str(&format!(
                "#grid(columns: (1.4em, 1fr), gutter: 0.4em,\n  [#text(size: 0.82em, fill: rgb(\"{}\"))[{}]],\n  [#text(size: 0.86em)][\n",
                self.pal.accent, n
            ));
            for b in &blocks {
                self.block(b, out, 1);
            }
            out.push_str(&format!(
                "  #link(<fnref-{n}>)[#text(size: 0.8em, fill: rgb(\"{}\"))[↩]]\n  ]\n)]\n\n",
                self.pal.accent
            ));
        }
    }
}

fn collect_refs(inlines: &[Inline], out: &mut Vec<String>) {
    for inline in inlines {
        match inline {
            Inline::FootnoteRef(label) => {
                if !out.contains(label) {
                    out.push(label.clone());
                }
            }
            Inline::Strong(c) | Inline::Emph(c) | Inline::Strike(c) => collect_refs(c, out),
            Inline::Link { children, .. } => collect_refs(children, out),
            _ => {}
        }
    }
}

/// Escape markdown text for Typst markup.  Brackets matter because the text
/// often lands inside `[content]` blocks.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '#' => out.push_str("\\#"),
            '*' => out.push_str("\\*"),
            '_' => out.push_str("\\_"),
            '$' => out.push_str("\\$"),
            '`' => out.push_str("\\`"),
            '<' => out.push_str("\\<"),
            '>' => out.push_str("\\>"),
            '@' => out.push_str("\\@"),
            '~' => out.push_str("\\~"),
            '[' => out.push_str("\\["),
            ']' => out.push_str("\\]"),
            '\u{00a0}' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// A Typst string literal from arbitrary text.
pub fn typst_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Collect the headings of a document for the outline preview / page estimate.
pub fn heading_outline(blocks: &[Block]) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    let mut visit = |b: &Block| {
        if let Block::Heading { level, inlines } = b {
            if *level <= 3 {
                out.push((*level, plain_all(inlines)));
            }
        }
    };
    walk_blocks(blocks, &mut visit);
    out
}

/// Used by tests to assert every referenced image is registered once.
pub fn unique_assets(map: &HashMap<String, PathBuf>) -> HashSet<String> {
    map.keys().cloned().collect()
}

/// Paper size in points, for a page-count estimate in the status bar.
pub fn paper_points(paper: Paper, landscape: bool, margin: Margin) -> (f32, f32) {
    let (w, h) = paper.size_mm();
    let (mx, my) = margin.cm();
    let (w, h) = if landscape { (h, w) } else { (w, h) };
    let w = (w - 2.0 * mx).max(40.0) / 25.4 * 72.0;
    let h = (h - 2.0 * my).max(40.0) / 25.4 * 72.0;
    (w, h)
}

// ---------------------------------------------------------------------------
// Cardbook: one card per page.
// ---------------------------------------------------------------------------

pub struct CardbookResult {
    pub source: String,
    pub assets: Assets,
    pub warnings: Vec<String>,
}

impl<'a> Generator<'a> {
    fn new(opts: &'a ExportOptions, fonts: &'a FontSet, base_dir: &Path) -> Self {
        Generator {
            opts,
            fonts,
            pal: palette(opts.theme),
            assets: Assets::new(base_dir),
            warnings: Vec::new(),
            footnote_numbers: HashMap::new(),
            footnote_defs: HashMap::new(),
            footnotes_used: Vec::new(),
            headings: 0,
        }
    }
}

pub fn generate_cardbook(
    cards: &[aur_core::Card],
    root: &Path,
    opts: &crate::options::CardbookOptions,
    fonts: &FontSet,
) -> CardbookResult {
    use crate::options::Margin as M;

    let pal = palette(opts.theme);
    let paper = match opts.page {
        crate::options::CardPage::A6 => Paper::A6,
        crate::options::CardPage::A5 => Paper::A5,
    };
    let eo = ExportOptions {
        theme: opts.theme,
        paper,
        landscape: false,
        margin: M::Narrow,
        header: String::new(),
        footer_page_numbers: opts.footer_page_numbers,
        toc: false,
        toc_own_page: false,
        h1_page_break: false,
        font_size: opts.font_size,
        line_height: 0.72,
        first_line_indent: false,
        date: String::new(),
        title: String::new(),
        math_as_text: false,
    };

    let body_font = format!(
        "(\"{}\", \"{}\", \"Libertinus Serif\")",
        fonts.sans, fonts.serif
    );
    let serif_font = format!(
        "(\"{}\", \"{}\", \"Libertinus Serif\")",
        fonts.serif, fonts.sans
    );

    let mut out = String::new();
    out.push_str("// Generated by Aureate — cardbook.\n");
    out.push_str(&format!(
        "#set page(paper: \"{}\", margin: (x: 1.15cm, y: 1.25cm), footer: context {{\n  set text(size: 6.5pt, fill: rgb(\"{}\"), font: {body_font})\n  let total = counter(page).final().first()\n  align(center)[#counter(page).display() / #total]\n}})\n",
        paper.typst_name(),
        pal.muted
    ));
    out.push_str(&format!(
        "#set text(font: {body_font}, size: {}pt, lang: \"zh\", region: \"cn\", fill: rgb(\"{}\"))\n",
        opts.font_size, pal.body
    ));
    out.push_str("#set par(justify: false, leading: 0.72em, spacing: 0.75em)\n");
    out.push_str(&format!(
        "#show heading.where(level: 1): it => block(above: 0pt, below: 4pt, width: 100%)[#text(font: {serif_font}, size: 15pt, weight: \"bold\", fill: rgb(\"{}\"))[#it.body]]\n",
        pal.heading
    ));
    out.push_str(&format!(
        "#show heading.where(level: 2): it => block(above: 8pt, below: 3pt)[#text(size: 10pt, weight: \"bold\", fill: rgb(\"{}\"))[#it.body]]\n",
        pal.heading
    ));
    out.push_str(&format!(
        "#show raw: set text(size: 0.8em, font: (\"DejaVu Sans Mono\", \"{}\"))\n",
        fonts.mono
    ));
    out.push_str(&format!(
        "#show raw.where(block: true): block.with(width: 100%, fill: rgb(\"{}\"), inset: (x: 7pt, y: 6pt), radius: 2pt, stroke: 0.3pt + rgb(\"{}\"))\n",
        pal.code_bg, pal.code_border
    ));
    out.push_str(&format!(
        "#set table(stroke: 0.3pt + rgb(\"{}\"), inset: (x: 4pt, y: 3pt), fill: (col, row) => if row == 0 {{ rgb(\"{}\") }} else {{ none }})\n",
        pal.table_border, pal.table_head_bg
    ));
    out.push_str(&format!(
        "#show quote.where(block: true): it => block(width: 100%, inset: (left: 8pt, right: 4pt, y: 2pt), stroke: (left: 2pt + rgb(\"{}\")), text(fill: rgb(\"{}\"), it.body))\n\n",
        pal.quote_bar, pal.muted
    ));

    let mut all_assets = Assets::new(root.to_path_buf());
    let mut warnings: Vec<String> = Vec::new();

    for card in cards {
        let card_file = root.join(&card.path);
        let dir = card_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| root.to_path_buf());

        let mut g = Generator::new(&eo, fonts, &dir);

        let meta = {
            let mut parts = vec![card.kind.label().to_string(), card.date.clone()];
            if let Some(project) = &card.project {
                if !project.trim().is_empty() {
                    parts.push(project.clone());
                }
            }
            parts.push(card.status.label().to_string());
            parts.join(" · ")
        };

        out.push_str("#pagebreak(weak: true)\n");
        out.push_str(&format!(
            "#set page(header: context {{\n  set text(size: 6.5pt, fill: rgb(\"{}\"), font: {body_font})\n  align(right)[{}]\n}})\n",
            pal.muted,
            esc(&meta)
        ));
        out.push_str("#block(width: 100%)[\n");
        out.push_str(&format!(
            "#text(size: 6.8pt, fill: rgb(\"{}\"), font: {body_font})[{}]\n",
            pal.muted,
            esc(&meta)
        ));
        out.push_str(&format!(
            "#v(3pt)\n#heading(level: 1)[{}]\n",
            esc(&card.title)
        ));
        out.push_str(&format!(
            "#line(length: 100%, stroke: 0.6pt + rgb(\"{}\"))\n",
            pal.rule
        ));
        if !card.tags.is_empty() {
            out.push_str(&format!(
                "#v(3pt)\n#text(size: 6.8pt, fill: rgb(\"{}\"))[{}]\n",
                pal.accent,
                esc(&card
                    .tags
                    .iter()
                    .map(|t| format!("#{t}"))
                    .collect::<Vec<_>>()
                    .join("  "))
            ));
        }
        out.push_str("#v(6pt)\n");

        for section in &card.sections {
            if section.title != "正文" && opts.show_sections {
                out.push_str(&format!(
                    "#v(4pt)\n#text(size: 8.2pt, weight: \"bold\", fill: rgb(\"{}\"))[{}]\n#v(1pt)\n",
                    pal.accent,
                    esc(&section.title)
                ));
            }
            for b in &section.blocks {
                g.block(b, &mut out, 0);
            }
        }
        out.push_str("\n]\n");

        warnings.extend(g.assets_warnings());
        warnings.extend(g.warnings);
        all_assets.map.extend(g.assets.map);
    }

    CardbookResult {
        source: out,
        assets: all_assets,
        warnings,
    }
}

impl<'a> Generator<'a> {
    fn assets_warnings(&self) -> Vec<String> {
        self.assets.warnings().to_vec()
    }
}
