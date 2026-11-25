//! Markdown AST shared by the preview renderer and the PDF exporter.
//!
//! Both output paths consume exactly this tree.  That is the only mechanism
//! guaranteeing that what you see in the preview and what lands in the PDF
//! describe the same document.

/// Horizontal alignment of a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// A block-level element.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Block {
    #[default]
    None,
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    List {
        ordered: bool,
        start: u64,
        tight: bool,
        items: Vec<ListItem>,
    },
    Table {
        aligns: Vec<Align>,
        head: Vec<Vec<Vec<Inline>>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Quote(Vec<Block>),
    Rule,
    Image {
        src: String,
        title: Option<String>,
        alt: Vec<Inline>,
    },
    Math {
        tex: String,
    },
    Html(String),
    FootnoteDef {
        label: String,
        blocks: Vec<Block>,
    },
}

/// One item of an ordered or unordered list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListItem {
    /// `Some(true)` = checked checkbox, `Some(false)` = unchecked.
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

/// An inline-level element.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Strike(Vec<Inline>),
    Link {
        url: String,
        title: Option<String>,
        children: Vec<Inline>,
    },
    Image {
        src: String,
        title: Option<String>,
        alt: Vec<Inline>,
    },
    Math(String),
    FootnoteRef(String),
    /// `[[exp-001]]` — resolved against the vault index, not the file name.
    WikiLink(String),
    SoftBreak,
    HardBreak,
}

impl Inline {
    pub fn text(s: impl Into<String>) -> Self {
        Inline::Text(s.into())
    }

    /// Flatten to plain text, used for titles, PDF bookmarks and search.
    pub fn plain(&self) -> String {
        match self {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => t.clone(),
            Inline::Strong(c) | Inline::Emph(c) | Inline::Strike(c) => plain_all(c),
            Inline::Link { children, .. } => plain_all(children),
            Inline::Image { alt, .. } => plain_all(alt),
            Inline::FootnoteRef(l) => l.clone(),
            Inline::WikiLink(id) => id.clone(),
            Inline::SoftBreak => " ".into(),
            Inline::HardBreak => " ".into(),
        }
    }

    /// Walk the inline tree, collecting wiki link targets.
    pub fn collect_wiki_links(&self, out: &mut Vec<String>) {
        match self {
            Inline::WikiLink(id) => out.push(id.clone()),
            Inline::Strong(c) | Inline::Emph(c) | Inline::Strike(c) => {
                for i in c {
                    i.collect_wiki_links(out);
                }
            }
            Inline::Link { children, .. } => {
                for i in children {
                    i.collect_wiki_links(out);
                }
            }
            _ => {}
        }
    }
}

pub fn plain_all(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        s.push_str(&i.plain());
    }
    s
}

impl Block {
    /// Plain text of a block, without children of containers.
    pub fn plain(&self) -> String {
        match self {
            Block::Heading { inlines, .. } | Block::Paragraph(inlines) => plain_all(inlines),
            Block::CodeBlock { code, .. } | Block::Math { tex: code } => code.clone(),
            Block::Image { alt, .. } => plain_all(alt),
            _ => String::new(),
        }
    }

    pub fn is_heading(&self) -> bool {
        matches!(self, Block::Heading { .. })
    }

    pub fn heading_level(&self) -> Option<u8> {
        match self {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        }
    }

    /// Every block in document order, depth first.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Block)) {
        f(self);
        match self {
            Block::Quote(children) => {
                for c in children {
                    c.walk(f);
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    for c in &item.blocks {
                        c.walk(f);
                    }
                }
            }
            Block::FootnoteDef { blocks, .. } => {
                for c in blocks {
                    c.walk(f);
                }
            }
            _ => {}
        }
    }
}

/// Walk all blocks, depth first.
pub fn walk_blocks<'a>(blocks: &'a [Block], f: &mut impl FnMut(&'a Block)) {
    for b in blocks {
        b.walk(f);
    }
}

/// All wiki links in the document, de-duplicated, in order of appearance.
pub fn wiki_links(blocks: &[Block]) -> Vec<String> {
    let mut raw = Vec::new();
    let mut visit = |b: &Block| match b {
        Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
            for i in inlines {
                i.collect_wiki_links(&mut raw);
            }
        }
        _ => {}
    };
    walk_blocks(blocks, &mut visit);
    let mut out: Vec<String> = Vec::new();
    for id in raw {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// First heading of the document, used as fallback title.
pub fn first_heading(blocks: &[Block]) -> Option<String> {
    blocks.iter().find_map(|b| match b {
        Block::Heading { level, inlines } if *level <= 2 => Some(plain_all(inlines)),
        _ => None,
    })
}
