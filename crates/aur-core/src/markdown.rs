//! Markdown → AST.
//!
//! `pulldown-cmark` gives us a flat event stream; this module turns it into
//! the small tree that both renderers consume.  Keeping the tree explicit is
//! what makes "preview and PDF agree on structure" checkable in code.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::borrow::Cow;

use crate::ast::{Align, Block, Inline, ListItem};

fn options() -> Options {
    let mut o = Options::empty();
    o.insert(Options::ENABLE_TABLES);
    o.insert(Options::ENABLE_FOOTNOTES);
    o.insert(Options::ENABLE_STRIKETHROUGH);
    o.insert(Options::ENABLE_TASKLISTS);
    o.insert(Options::ENABLE_MATH);
    o
}

/// Parse markdown body (frontmatter must already be stripped).
pub fn parse_blocks(markdown: &str) -> Vec<Block> {
    let events: Vec<Event> = Parser::new_ext(markdown, options()).collect();
    let mut p = Parser2 { events, pos: 0 };
    let mut blocks = p.blocks(None);
    normalize(&mut blocks);
    blocks
}

struct Parser2<'a> {
    events: Vec<Event<'a>>,
    pos: usize,
}

#[derive(Debug)]
enum Frame {
    Strong,
    Emph,
    Strike,
    Link { url: String, title: Option<String> },
    Image { src: String, title: Option<String> },
}

impl Frame {
    fn build(self, children: Vec<Inline>) -> Inline {
        match self {
            Frame::Strong => Inline::Strong(children),
            Frame::Emph => Inline::Emph(children),
            Frame::Strike => Inline::Strike(children),
            Frame::Link { url, title } => Inline::Link {
                url,
                title,
                children,
            },
            Frame::Image { src, title } => Inline::Image {
                src,
                title,
                alt: children,
            },
        }
    }
}

impl<'a> Parser2<'a> {
    fn peek(&self) -> Option<&Event<'a>> {
        self.events.get(self.pos)
    }

    fn next(&mut self) -> Option<Event<'a>> {
        let ev = self.events.get(self.pos).cloned();
        if ev.is_some() {
            self.pos += 1;
        }
        ev
    }

    fn skip_to_end(&mut self, end: &TagEnd) {
        let mut depth = 1;
        while let Some(ev) = self.next() {
            match ev {
                Event::Start(t) if t.to_end() == *end => depth += 1,
                Event::End(t) if t == *end => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn blocks(&mut self, stop: Option<TagEnd>) -> Vec<Block> {
        let mut out: Vec<Block> = Vec::new();
        while let Some(ev) = self.next() {
            match ev {
                Event::End(t) => {
                    if stop.as_ref() == Some(&t) {
                        break;
                    }
                }
                Event::Start(tag) => match tag {
                    Tag::Paragraph => {
                        let inlines = self.inlines(TagEnd::Paragraph);
                        out.push(Block::Paragraph(inlines));
                    }
                    Tag::Heading { level, .. } => {
                        let inlines = self.inlines(TagEnd::Heading(level));
                        out.push(Block::Heading {
                            level: level as u8,
                            inlines,
                        });
                    }
                    Tag::CodeBlock(kind) => {
                        let lang = match &kind {
                            CodeBlockKind::Fenced(info) => info
                                .split_whitespace()
                                .next()
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string()),
                            CodeBlockKind::Indented => None,
                        };
                        let mut code = String::new();
                        while let Some(ev) = self.next() {
                            match ev {
                                Event::Text(t) => code.push_str(&t),
                                Event::End(TagEnd::CodeBlock) => break,
                                _ => {}
                            }
                        }
                        if code.ends_with('\n') {
                            code.pop();
                        }
                        out.push(Block::CodeBlock { lang, code });
                    }
                    Tag::BlockQuote(kind) => {
                        let inner = self.blocks(Some(TagEnd::BlockQuote(kind)));
                        out.push(Block::Quote(inner));
                    }
                    Tag::List(start) => {
                        out.push(self.list(start));
                    }
                    Tag::Item => {
                        let blocks = self.blocks(Some(TagEnd::Item));
                        out.extend(blocks);
                    }
                    Tag::Table(aligns) => {
                        out.push(self.table(aligns));
                    }
                    Tag::FootnoteDefinition(label) => {
                        let blocks = self.blocks(Some(TagEnd::FootnoteDefinition));
                        out.push(Block::FootnoteDef {
                            label: label.to_string(),
                            blocks,
                        });
                    }
                    Tag::HtmlBlock => {
                        let mut html = String::new();
                        while let Some(ev) = self.next() {
                            match ev {
                                Event::Html(t) | Event::Text(t) => html.push_str(&t),
                                Event::End(TagEnd::HtmlBlock) => break,
                                _ => {}
                            }
                        }
                        out.push(Block::Html(html));
                    }
                    Tag::Image {
                        dest_url, title, ..
                    } => {
                        let alt = self.inlines(TagEnd::Image);
                        out.push(Block::Image {
                            src: dest_url.to_string(),
                            title: non_empty(&title),
                            alt,
                        });
                    }
                    Tag::MetadataBlock(_) => {
                        // YAML metadata blocks are already handled by frontmatter.
                        self.skip_tag_block(&tag);
                    }
                    _ => {}
                },
                Event::Rule => out.push(Block::Rule),
                Event::DisplayMath(tex) => out.push(Block::Math {
                    tex: tex.to_string(),
                }),
                Event::Html(html) => out.push(Block::Html(html.to_string())),
                Event::Text(text) => {
                    // Loose text outside a paragraph (can happen inside tight
                    // lists); keep it rather than dropping content.
                    let mut inlines = vec![Inline::Text(text.to_string())];
                    while let Some(Event::Text(t)) = self.peek() {
                        let t = t.to_string();
                        inlines.push(Inline::Text(t));
                        self.pos += 1;
                    }
                    out.push(Block::Paragraph(inlines));
                }
                _ => {}
            }
        }
        out
    }

    fn skip_tag_block(&mut self, tag: &Tag) {
        let end = tag.to_end();
        self.skip_to_end(&end);
    }

    fn list(&mut self, start: Option<u64>) -> Block {
        let mut items: Vec<ListItem> = Vec::new();
        loop {
            match self.next() {
                None => break,
                Some(Event::End(TagEnd::List(_))) => break,
                Some(Event::Start(Tag::Item)) => {
                    let mut checked = None;
                    if let Some(Event::TaskListMarker(v)) = self.peek() {
                        checked = Some(*v);
                        self.pos += 1;
                    }
                    let blocks = self.blocks(Some(TagEnd::Item));
                    items.push(ListItem { checked, blocks });
                }
                Some(_) => {}
            }
        }
        let tight = items
            .iter()
            .all(|i| i.blocks.len() == 1 && matches!(i.blocks[0], Block::Paragraph(_)));
        Block::List {
            ordered: start.is_some(),
            start: start.unwrap_or(1),
            tight,
            items,
        }
    }

    fn table(&mut self, aligns: Vec<pulldown_cmark::Alignment>) -> Block {
        let aligns = aligns
            .into_iter()
            .map(|a| match a {
                pulldown_cmark::Alignment::Left => Align::Left,
                pulldown_cmark::Alignment::Center => Align::Center,
                pulldown_cmark::Alignment::Right => Align::Right,
                pulldown_cmark::Alignment::None => Align::None,
            })
            .collect();

        // pulldown-cmark puts the header cells straight inside `TableHead`
        // (there is no row wrapper there), so collect cells until it closes.
        let mut head: Vec<Vec<Vec<Inline>>> = Vec::new();
        let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();

        loop {
            match self.next() {
                None => break,
                Some(Event::End(TagEnd::Table)) => break,
                Some(Event::Start(Tag::TableHead)) => {
                    let mut header_row: Vec<Vec<Inline>> = Vec::new();
                    while let Some(ev) = self.peek() {
                        match ev {
                            Event::Start(Tag::TableCell) => {
                                self.pos += 1;
                                header_row.push(self.inlines(TagEnd::TableCell));
                            }
                            Event::End(TagEnd::TableHead) => {
                                self.pos += 1;
                                break;
                            }
                            _ => {
                                self.pos += 1;
                            }
                        }
                    }
                    if !header_row.is_empty() {
                        head.push(header_row);
                    }
                }
                Some(Event::Start(Tag::TableRow)) => {
                    rows.push(self.row());
                }
                Some(_) => {}
            }
        }

        Block::Table { aligns, head, rows }
    }

    fn row(&mut self) -> Vec<Vec<Inline>> {
        let mut cells = Vec::new();
        loop {
            match self.next() {
                None => break,
                Some(Event::End(TagEnd::TableRow)) => break,
                Some(Event::Start(Tag::TableCell)) => {
                    cells.push(self.inlines(TagEnd::TableCell));
                }
                Some(_) => {}
            }
        }
        cells
    }

    fn inlines(&mut self, stop: TagEnd) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        let mut stack: Vec<(Frame, Vec<Inline>)> = Vec::new();

        macro_rules! push {
            ($inl:expr) => {{
                let inl: Inline = $inl;
                match stack.last_mut() {
                    Some((_, children)) => children.push(inl),
                    None => out.push(inl),
                }
            }};
        }

        while let Some(ev) = self.next() {
            match ev {
                Event::End(t) => {
                    if t == stop && stack.is_empty() {
                        break;
                    }
                    if let Some((frame, children)) = stack.pop() {
                        let built = frame.build(children);
                        match stack.last_mut() {
                            Some((_, parent)) => parent.push(built),
                            None => out.push(built),
                        }
                    } else if t == stop {
                        break;
                    }
                }
                Event::Start(tag) => match tag {
                    Tag::Strong => stack.push((Frame::Strong, Vec::new())),
                    Tag::Emphasis => stack.push((Frame::Emph, Vec::new())),
                    Tag::Strikethrough => stack.push((Frame::Strike, Vec::new())),
                    Tag::Link {
                        dest_url, title, ..
                    } => stack.push((
                        Frame::Link {
                            url: dest_url.to_string(),
                            title: non_empty(&title),
                        },
                        Vec::new(),
                    )),
                    Tag::Image {
                        dest_url, title, ..
                    } => stack.push((
                        Frame::Image {
                            src: dest_url.to_string(),
                            title: non_empty(&title),
                        },
                        Vec::new(),
                    )),
                    _ => {}
                },
                Event::Text(t) => push!(Inline::Text(t.to_string())),
                Event::Code(t) => push!(Inline::Code(t.to_string())),
                Event::InlineMath(t) | Event::DisplayMath(t) => {
                    push!(Inline::Math(t.to_string()))
                }
                Event::FootnoteReference(label) => {
                    push!(Inline::FootnoteRef(label.to_string()))
                }
                Event::SoftBreak => push!(Inline::SoftBreak),
                Event::HardBreak => push!(Inline::HardBreak),
                // Raw inline HTML is kept as literal text: the preview must
                // never execute scripts from a document, and the PDF shows
                // the same literal text, so both stay in agreement.
                Event::InlineHtml(html) | Event::Html(html) => {
                    push!(Inline::Text(html.to_string()))
                }
                _ => {}
            }
        }
        out
    }
}

/// pulldown-cmark 0.13 models an absent title as an empty `CowStr`, not `None`.
fn non_empty(s: &pulldown_cmark::CowStr<'_>) -> Option<String> {
    let t = s.to_string();
    if t.trim().is_empty() {
        None
    } else {
        Some(t)
    }
}

/// Turn `[[card-id]]` inside text runs into wiki links.  Done as a
/// post-processing pass so nested emphasis still works.
fn normalize(blocks: &mut [Block]) {
    // `$$ … $$` arrives as a paragraph holding one inline-math node whose TeX
    // is newline-wrapped.  Promote that to a real block so the preview and the
    // PDF both centre it.
    for b in blocks.iter_mut() {
        if let Block::Paragraph(inlines) = b {
            if inlines.len() == 1 {
                if let Inline::Math(tex) = &inlines[0] {
                    if tex.starts_with('\n') && tex.ends_with('\n') && !tex.trim().is_empty() {
                        *b = Block::Math {
                            tex: tex.trim().to_string(),
                        };
                    }
                }
            }
        }
    }

    for b in blocks.iter_mut() {
        match b {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                *inlines = normalize_inlines(std::mem::take(inlines))
            }
            Block::Quote(children) => normalize(children),
            Block::List { items, .. } => {
                for item in items {
                    normalize(&mut item.blocks);
                }
            }
            Block::FootnoteDef { blocks, .. } => normalize(blocks),
            Block::Table { head, rows, .. } => {
                for row in head.iter_mut().chain(rows.iter_mut()) {
                    for cell in row.iter_mut() {
                        *cell = normalize_inlines(std::mem::take(cell));
                    }
                }
            }
            _ => {}
        }
    }
}

fn normalize_inlines(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::new();
    // pulldown-cmark breaks text at every potential link opener, so `[[id]]`
    // arrives as "[", "[", "id", "]", "]" — glue adjacent text back together
    // before looking for wiki links.
    let mut pending = String::new();
    let flush = |pending: &mut String, out: &mut Vec<Inline>| {
        if !pending.is_empty() {
            out.extend(split_wiki(&std::mem::take(pending)));
        }
    };
    for inl in inlines {
        let inl = match inl {
            Inline::Text(text) => {
                pending.push_str(&text);
                continue;
            }
            other => other,
        };
        flush(&mut pending, &mut out);
        match inl {
            Inline::Strong(c) => out.push(Inline::Strong(normalize_inlines(c))),
            Inline::Emph(c) => out.push(Inline::Emph(normalize_inlines(c))),
            Inline::Strike(c) => out.push(Inline::Strike(normalize_inlines(c))),
            Inline::Link {
                url,
                title,
                children,
            } => out.push(Inline::Link {
                url,
                title,
                children: normalize_inlines(children),
            }),
            other => out.push(other),
        }
    }
    flush(&mut pending, &mut out);
    out
}

fn split_wiki(text: &str) -> Vec<Inline> {
    if !text.contains("[[") {
        return vec![Inline::Text(text.to_string())];
    }
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        if start > 0 {
            out.push(Inline::Text(rest[..start].to_string()));
        }
        let after = &rest[start + 2..];
        if let Some(end) = after.find("]]") {
            let id = after[..end].trim();
            if !id.is_empty() {
                out.push(Inline::WikiLink(id.to_string()));
            }
            rest = &after[end + 2..];
        } else {
            out.push(Inline::Text(rest[start..].to_string()));
            rest = "";
            break;
        }
    }
    if !rest.is_empty() {
        out.push(Inline::Text(rest.to_string()));
    }
    out
}

/// Keep the compiler honest about the `Cow` import used by the event types.
#[allow(dead_code)]
fn _assert_cow(_: Cow<'_, str>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::plain_all;

    #[test]
    fn headings_paragraphs_and_emphasis() {
        let blocks = parse_blocks("# 标题\n\n一段 **粗体** 和 *斜体*。\n");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            Block::Heading { level, inlines } => {
                assert_eq!(*level, 1);
                assert_eq!(plain_all(inlines), "标题");
            }
            _ => panic!("expected heading"),
        }
        match &blocks[1] {
            Block::Paragraph(i) => assert!(i.iter().any(|x| matches!(x, Inline::Strong(_)))),
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn table_with_header_and_alignment() {
        let md = "| A | B |\n|:--|--:|\n| 1 | 2 |\n";
        let blocks = parse_blocks(md);
        match &blocks[0] {
            Block::Table { head, rows, aligns } => {
                assert_eq!(head.len(), 1);
                assert_eq!(rows.len(), 1);
                assert_eq!(aligns[0], Align::Left);
                assert_eq!(aligns[1], Align::Right);
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn nested_and_task_lists() {
        let md = "- [ ] a\n- [x] b\n  - c\n";
        let blocks = parse_blocks(md);
        match &blocks[0] {
            Block::List { items, ordered, .. } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].checked, Some(false));
                assert_eq!(items[1].checked, Some(true));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn math_and_wikilinks() {
        let blocks = parse_blocks("行内 $a^2$ 和 [[exp-001]]\n\n$$\n\\frac{1}{2}\n$$\n");
        match &blocks[0] {
            Block::Paragraph(i) => {
                assert!(i.iter().any(|x| matches!(x, Inline::Math(_))));
                assert!(i
                    .iter()
                    .any(|x| matches!(x, Inline::WikiLink(id) if id == "exp-001")));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
        assert!(matches!(blocks[1], Block::Math { .. }));
    }

    #[test]
    fn code_fence_language_and_trim() {
        let blocks = parse_blocks("```rust\nfn main() {}\n```\n");
        match &blocks[0] {
            Block::CodeBlock { lang, code } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(code, "fn main() {}");
            }
            other => panic!("expected code, got {other:?}"),
        }
    }

    #[test]
    fn footnotes() {
        let md = "正文[^1]\n\n[^1]: 注释\n";
        let blocks = parse_blocks(md);
        assert!(
            matches!(&blocks[0], Block::Paragraph(i) if i.iter().any(|x| matches!(x, Inline::FootnoteRef(_))))
        );
        assert!(matches!(blocks[1], Block::FootnoteDef { .. }));
    }
}
