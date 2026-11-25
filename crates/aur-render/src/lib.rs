//! `aur-render` — HTML for the screen preview only.
//!
//! This renderer is deliberately separate from the PDF pipeline.  It exists
//! to be fast: no layout engine, no pagination, just semantic HTML with the
//! same AST the exporter consumes.  The preview promises structure, not
//! pixel-identical layout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use aur_core::ast::{plain_all, walk_blocks, Align, Block, Inline};
use aur_core::Document;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::html::IncludeBackground;
use syntect::parsing::SyntaxSet;

#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub dark: bool,
    /// Document folder; relative image paths resolve against it.
    pub base_dir: Option<PathBuf>,
}

pub fn render_document(doc: &Document, opts: &RenderOptions) -> String {
    render_blocks(&doc.blocks, opts)
}

pub fn render_blocks(blocks: &[Block], opts: &RenderOptions) -> String {
    let mut r = Renderer::new(opts);
    r.prepare(blocks);
    let mut out = String::new();
    for b in blocks {
        r.block(b, &mut out);
    }
    r.endnotes(&mut out);
    out
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn themes() -> &'static ThemeSet {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    SET.get_or_init(ThemeSet::load_defaults)
}

struct Renderer<'a> {
    opts: &'a RenderOptions,
    footnote_numbers: HashMap<String, usize>,
    footnote_defs: HashMap<String, Vec<Block>>,
    footnote_order: Vec<String>,
    heading_seq: usize,
    list_depth: usize,
}

impl<'a> Renderer<'a> {
    fn new(opts: &'a RenderOptions) -> Self {
        Renderer {
            opts,
            footnote_numbers: HashMap::new(),
            footnote_defs: HashMap::new(),
            footnote_order: Vec::new(),
            heading_seq: 0,
            list_depth: 0,
        }
    }

    fn prepare(&mut self, blocks: &[Block]) {
        let mut visit = |b: &Block| {
            if let Block::FootnoteDef { label, blocks } = b {
                self.footnote_defs.insert(label.clone(), blocks.clone());
            }
        };
        walk_blocks(blocks, &mut visit);

        let mut order = Vec::new();
        let mut visit = |b: &Block| {
            if let Block::Paragraph(inlines) | Block::Heading { inlines, .. } = b {
                collect_refs(inlines, &mut order);
            }
        };
        walk_blocks(blocks, &mut visit);
        for (i, label) in order.iter().enumerate() {
            self.footnote_numbers.insert(label.clone(), i + 1);
        }
        self.footnote_order = order;
    }

    fn block(&mut self, block: &Block, out: &mut String) {
        match block {
            Block::None => {}
            Block::Heading { level, inlines } => {
                self.heading_seq += 1;
                let lvl = (*level).clamp(1, 6);
                let seq = self.heading_seq;
                let inner = self.inlines(inlines);
                out.push_str(&format!(
                    "<h{lvl} id=\"h{seq}\" data-level=\"{lvl}\">{inner}</h{lvl}>"
                ));
            }
            Block::Paragraph(inlines) => {
                out.push_str("<p>");
                out.push_str(&self.inlines(inlines));
                out.push_str("</p>");
            }
            Block::CodeBlock { lang, code } => {
                out.push_str(&self.highlight(lang.as_deref(), code));
            }
            Block::List {
                ordered,
                start,
                items,
                ..
            } => {
                let tag = if *ordered { "ol" } else { "ul" };
                out.push_str(&format!("<{tag}"));
                if *ordered && *start > 1 {
                    out.push_str(&format!(" start=\"{start}\""));
                }
                out.push('>');
                self.list_depth += 1;
                for item in items {
                    out.push_str("<li");
                    if item.checked.is_some() {
                        out.push_str(" class=\"task\"");
                    }
                    out.push('>');
                    if let Some(checked) = item.checked {
                        out.push_str(&format!(
                            "<input type=\"checkbox\" disabled {}/>",
                            if checked { "checked" } else { "" }
                        ));
                    }
                    for b in &item.blocks {
                        self.block(b, out);
                    }
                    out.push_str("</li>");
                }
                self.list_depth -= 1;
                out.push_str(&format!("</{tag}>"));
            }
            Block::Table { aligns, head, rows } => {
                out.push_str("<div class=\"table-wrap\"><table><thead>");
                for row in head {
                    out.push_str("<tr>");
                    for (i, cell) in row.iter().enumerate() {
                        out.push_str(&format!(
                            "<th{}>{}</th>",
                            align_attr(aligns.get(i).copied()),
                            self.inlines(cell)
                        ));
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</thead><tbody>");
                for row in rows {
                    out.push_str("<tr>");
                    for (i, cell) in row.iter().enumerate() {
                        out.push_str(&format!(
                            "<td{}>{}</td>",
                            align_attr(aligns.get(i).copied()),
                            self.inlines(cell)
                        ));
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</tbody></table></div>");
            }
            Block::Quote(children) => {
                out.push_str("<blockquote>");
                for child in children {
                    self.block(child, out);
                }
                out.push_str("</blockquote>");
            }
            Block::Rule => out.push_str("<hr/>"),
            Block::Image { src, title, alt } => {
                out.push_str(&self.image(src, title.as_deref(), alt, false));
            }
            Block::Math { tex } => {
                out.push_str(&format!(
                    "<div class=\"math-block\" data-tex=\"{}\">{}</div>",
                    esc_attr(tex),
                    esc_html(tex)
                ));
            }
            Block::Html(html) => {
                // Show raw HTML as text: the preview must never execute
                // scripts coming from a document.
                out.push_str(&format!("<pre class=\"raw-html\">{}</pre>", esc_html(html)));
            }
            Block::FootnoteDef { .. } => {}
        }
    }

    fn inlines(&mut self, inlines: &[Inline]) -> String {
        let mut out = String::new();
        for i in inlines {
            self.inline(i, &mut out);
        }
        out
    }

    fn inline(&mut self, inline: &Inline, out: &mut String) {
        match inline {
            Inline::Text(t) => out.push_str(&esc_html(t)),
            Inline::Code(c) => out.push_str(&format!("<code>{}</code>", esc_html(c))),
            Inline::Strong(c) => {
                out.push_str("<strong>");
                for i in c {
                    self.inline(i, out);
                }
                out.push_str("</strong>");
            }
            Inline::Emph(c) => {
                out.push_str("<em>");
                for i in c {
                    self.inline(i, out);
                }
                out.push_str("</em>");
            }
            Inline::Strike(c) => {
                out.push_str("<del>");
                for i in c {
                    self.inline(i, out);
                }
                out.push_str("</del>");
            }
            Inline::Link { url, children, .. } => {
                out.push_str(&format!(
                    "<a href=\"{}\" target=\"_blank\" rel=\"noreferrer\">{}</a>",
                    esc_attr(url),
                    self.inlines(children)
                ));
            }
            Inline::Image { src, title, alt } => {
                out.push_str(&self.image(src, title.as_deref(), alt, true));
            }
            Inline::Math(tex) => out.push_str(&format!(
                "<span class=\"math-inline\" data-tex=\"{}\">{}</span>",
                esc_attr(tex),
                esc_html(tex)
            )),
            Inline::FootnoteRef(label) => {
                let n = self.footnote_numbers.get(label).copied().unwrap_or(0);
                if n == 0 {
                    out.push_str(&format!("<sup>{}</sup>", esc_html(label)));
                } else {
                    out.push_str(&format!(
                        "<sup class=\"fnref\"><a href=\"#fn-{n}\">{n}</a></sup>"
                    ));
                }
            }
            Inline::WikiLink(id) => out.push_str(&format!(
                "<a class=\"wikilink\" data-card=\"{}\" href=\"#\">{}</a>",
                esc_attr(id),
                esc_html(id)
            )),
            // A single newline in the source is a line break on paper too:
            // Chinese notes routinely put each sentence on its own line, and
            // silently gluing them together changes what the author wrote.
            Inline::SoftBreak => out.push_str("<br/>"),
            Inline::HardBreak => out.push_str("<br/>"),
        }
    }

    fn image(&self, src: &str, title: Option<&str>, alt: &[Inline], inline: bool) -> String {
        let alt_text = title
            .map(|t| t.to_string())
            .unwrap_or_else(|| plain_all(alt));
        let resolved = self.resolve_image(src);
        let Some(path) = resolved else {
            return format!(
                "<span class=\"missing-image\">图片缺失：{}</span>",
                esc_html(src)
            );
        };
        let caption = if inline || alt_text.trim().is_empty() {
            String::new()
        } else {
            format!("<figcaption>{}</figcaption>", esc_html(&alt_text))
        };
        let img = format!(
            "<img data-asset=\"{}\" alt=\"{}\" loading=\"lazy\"/>",
            esc_attr(&path.to_string_lossy()),
            esc_attr(&alt_text)
        );
        if inline {
            format!("<span class=\"inline-image\">{img}</span>")
        } else {
            format!("<figure>{img}{caption}</figure>")
        }
    }

    fn resolve_image(&self, src: &str) -> Option<PathBuf> {
        if src.starts_with("http://") || src.starts_with("https://") {
            return None;
        }
        let raw = src.trim_start_matches("file://");
        let path = Path::new(raw);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.opts.base_dir.as_ref()?.join(path)
        };
        if resolved.exists() {
            Some(resolved)
        } else {
            None
        }
    }

    fn highlight(&self, lang: Option<&str>, code: &str) -> String {
        let ss = syntaxes();
        let syntax = lang
            .and_then(|l| ss.find_syntax_by_token(l))
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let theme: &Theme = if self.opts.dark {
            themes()
                .themes
                .get("base16-ocean.dark")
                .or_else(|| themes().themes.values().next())
                .unwrap()
        } else {
            themes()
                .themes
                .get("InspiredGitHub")
                .or_else(|| themes().themes.values().next())
                .unwrap()
        };

        let mut h = syntect::easy::HighlightLines::new(syntax, theme);
        let mut body = String::new();
        for line in syntect::util::LinesWithEndings::from(code) {
            match h.highlight_line(line, ss) {
                Ok(ranges) => {
                    let escaped = syntect::html::styled_line_to_highlighted_html(
                        &ranges,
                        IncludeBackground::No,
                    )
                    .unwrap_or_else(|_| esc_html(line));
                    body.push_str(escaped.trim_end_matches('\n'));
                }
                Err(_) => body.push_str(&esc_html(line)),
            }
            body.push('\n');
        }

        let lang_label = lang.unwrap_or("text");
        format!(
            "<div class=\"code-block\" data-lang=\"{}\"><div class=\"code-head\"><span class=\"code-lang\">{}</span><button class=\"code-copy\" data-action=\"copy-code\">复制</button></div><pre><code>{}</code></pre></div>",
            esc_attr(lang_label),
            esc_html(lang_label),
            body
        )
    }

    fn endnotes(&mut self, out: &mut String) {
        if self.footnote_order.is_empty() {
            return;
        }
        let order = self.footnote_order.clone();
        out.push_str("<section class=\"endnotes\"><h2>注</h2><ol>");
        for label in &order {
            let n = self.footnote_numbers.get(label).copied().unwrap_or(0);
            let Some(blocks) = self.footnote_defs.get(label).cloned() else {
                continue;
            };
            out.push_str(&format!("<li id=\"fn-{n}\">"));
            for b in &blocks {
                self.block(b, out);
            }
            out.push_str(" <a class=\"fn-back\" href=\"#h-0\">↩</a></li>");
        }
        out.push_str("</ol></section>");
    }
}

fn align_attr(a: Option<Align>) -> &'static str {
    match a {
        Some(Align::Center) => " style=\"text-align:center\"",
        Some(Align::Right) => " style=\"text-align:right\"",
        _ => "",
    }
}

fn collect_refs(inlines: &[Inline], out: &mut Vec<String>) {
    for i in inlines {
        match i {
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

pub fn esc_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

pub fn esc_attr(s: &str) -> String {
    esc_html(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_from_markdown() {
        let doc = Document::parse("普通 <script>alert(1)</script>\n", None);
        let html = render_document(&doc, &RenderOptions::default());
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn renders_table_and_tasklist() {
        let md = "| A | B |\n|:-:|--:|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n";
        let doc = Document::parse(md, None);
        let html = render_document(&doc, &RenderOptions::default());
        assert!(html.contains("<table>"));
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("checked"));
    }

    #[test]
    fn code_is_highlighted() {
        let doc = Document::parse("```rust\nfn main() {}\n```\n", None);
        let html = render_document(&doc, &RenderOptions::default());
        assert!(html.contains("code-block"));
        assert!(html.contains("style=\"color:"));
    }

    #[test]
    fn math_carries_tex() {
        let doc = Document::parse("行内 $a^2$\n", None);
        let html = render_document(&doc, &RenderOptions::default());
        assert!(html.contains("data-tex=\"a^2\""));
    }
}
