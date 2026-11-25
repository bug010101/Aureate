//! A document is a markdown file plus its parsed frontmatter and AST.

use crate::ast::{first_heading, Block};
use crate::frontmatter::{self, Frontmatter};
use crate::markdown;

#[derive(Debug, Clone)]
pub struct Document {
    pub raw: String,
    pub frontmatter: Frontmatter,
    /// Markdown source after the frontmatter fence.
    pub raw_body: String,
    pub blocks: Vec<Block>,
    pub title: String,
}

impl Document {
    pub fn parse(raw: &str, fallback_title: Option<&str>) -> Document {
        let (fm, body) = frontmatter::parse(raw);
        let blocks = markdown::parse_blocks(body);
        let title = fm
            .title
            .clone()
            .or_else(|| first_heading(&blocks))
            .or_else(|| fallback_title.map(title_from_path))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "未命名文档".to_string());

        Document {
            raw: raw.to_string(),
            frontmatter: fm,
            raw_body: body.to_string(),
            blocks,
            title,
        }
    }

    /// Replace the body while keeping the frontmatter untouched.
    pub fn set_body(&mut self, body: &str) {
        let raw = format!("---\n{}---\n{}", self.frontmatter.to_yaml(), body);
        *self = Document::parse(&raw, Some(&self.title));
    }

    pub fn to_markdown(&self) -> String {
        if self.frontmatter.present {
            format!("---\n{}---\n{}", self.frontmatter.to_yaml(), self.raw_body)
        } else {
            self.raw_body.clone()
        }
    }
}

pub fn title_from_path(path: &str) -> String {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .trim_end_matches(".markdown");
    if name.is_empty() {
        "未命名文档".to_string()
    } else {
        name.to_string()
    }
}

/// Word count that behaves sensibly for mixed CJK / latin text: CJK glyphs
/// count as one word each, latin runs count as one word.
pub fn word_count(text: &str) -> usize {
    let mut count = 0;
    let mut in_latin = false;
    for ch in text.chars() {
        let is_cjk = matches!(ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF);
        if is_cjk {
            count += 1;
            in_latin = false;
        } else if ch.is_alphanumeric() {
            if !in_latin {
                count += 1;
                in_latin = true;
            }
        } else {
            in_latin = false;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_priority() {
        let doc = Document::parse("---\ntitle: 显式标题\n---\n# 一级标题\n", None);
        assert_eq!(doc.title, "显式标题");

        let doc = Document::parse("# 一级标题\n", Some("file.md"));
        assert_eq!(doc.title, "一级标题");

        let doc = Document::parse("plain", Some("notes/读书笔记.md"));
        assert_eq!(doc.title, "读书笔记");
    }

    #[test]
    fn word_count_mixes_scripts() {
        // Han characters count one each; Latin runs count as one word.
        assert_eq!(word_count("你好 hello world"), 4);
        assert_eq!(word_count("中文 排版"), 4);
        assert_eq!(word_count("only latin words"), 3);
        assert_eq!(word_count(""), 0);
    }

    #[test]
    fn set_body_keeps_frontmatter() {
        let mut doc = Document::parse("---\nid: a\nkind: 想法卡\n---\nold\n", None);
        doc.set_body("new body\n");
        assert_eq!(doc.frontmatter.id.as_deref(), Some("a"));
        assert!(doc.raw.contains("new body"));
    }
}
