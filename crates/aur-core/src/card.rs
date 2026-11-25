//! Card domain model.
//!
//! A card is nothing but a markdown file with a fixed frontmatter block.
//! Everything else in the app (timeline, board, ledger, cardbook) is a view
//! over this struct.

use crate::ast::{walk_blocks, Block};
use crate::document::Document;
use crate::frontmatter::Frontmatter;
use chrono::{Datelike, Local, NaiveDate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStatus {
    Todo,
    Doing,
    Done,
    Blocked,
}

impl CardStatus {
    pub const ALL: [CardStatus; 4] = [
        CardStatus::Todo,
        CardStatus::Doing,
        CardStatus::Done,
        CardStatus::Blocked,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            CardStatus::Todo => "todo",
            CardStatus::Doing => "doing",
            CardStatus::Done => "done",
            CardStatus::Blocked => "blocked",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            CardStatus::Todo => "待办",
            CardStatus::Doing => "进行中",
            CardStatus::Done => "已完成",
            CardStatus::Blocked => "受阻",
        }
    }

    pub fn parse_status(s: &str) -> CardStatus {
        match s.trim().to_ascii_lowercase().as_str() {
            "doing" | "进行中" | "wip" => CardStatus::Doing,
            "done" | "已完成" | "完成" => CardStatus::Done,
            "blocked" | "受阻" | "阻塞" => CardStatus::Blocked,
            _ => CardStatus::Todo,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardKind {
    Experiment,
    Paper,
    Idea,
    Meeting,
    Week,
    Other(String),
}

impl CardKind {
    pub const BUILTIN: [CardKind; 5] = [
        CardKind::Experiment,
        CardKind::Paper,
        CardKind::Idea,
        CardKind::Meeting,
        CardKind::Week,
    ];

    pub fn parse_kind(s: &str) -> CardKind {
        match s.trim() {
            "实验卡" | "实验" | "experiment" => CardKind::Experiment,
            "论文卡" | "论文" | "paper" => CardKind::Paper,
            "想法卡" | "想法" | "idea" => CardKind::Idea,
            "会议卡" | "会议" | "meeting" => CardKind::Meeting,
            "周卡" | "周" | "week" => CardKind::Week,
            other => CardKind::Other(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            CardKind::Experiment => "实验卡",
            CardKind::Paper => "论文卡",
            CardKind::Idea => "想法卡",
            CardKind::Meeting => "会议卡",
            CardKind::Week => "周卡",
            CardKind::Other(s) => s,
        }
    }

    pub fn slug(&self) -> &str {
        match self {
            CardKind::Experiment => "exp",
            CardKind::Paper => "paper",
            CardKind::Idea => "idea",
            CardKind::Meeting => "meeting",
            CardKind::Week => "week",
            CardKind::Other(_) => "card",
        }
    }

    /// Section skeleton offered by the form view.  `## 关键命令` is special:
    /// the preview attaches copy / save-as-script actions to its code blocks.
    pub fn sections(&self) -> &'static [&'static str] {
        match self {
            CardKind::Experiment => &["问题", "设计", "关键命令", "结果", "结论", "下一步"],
            CardKind::Paper => &["文献", "问题", "方法", "结论", "可复用点"],
            CardKind::Idea => &["一句话", "背景", "可能性", "下一步"],
            CardKind::Meeting => &["议题", "结论", "待办", "参与人"],
            CardKind::Week => &["本周完成", "问题", "下周计划"],
            CardKind::Other(_) => &["正文"],
        }
    }

    pub fn accent(&self) -> &'static str {
        match self {
            CardKind::Experiment => "exp",
            CardKind::Paper => "paper",
            CardKind::Idea => "idea",
            CardKind::Meeting => "meeting",
            CardKind::Week => "week",
            CardKind::Other(_) => "other",
        }
    }
}

/// A fully parsed card.
#[derive(Debug, Clone)]
pub struct Card {
    pub id: String,
    pub kind: CardKind,
    pub date: String,
    pub project: Option<String>,
    pub status: CardStatus,
    pub tags: Vec<String>,
    pub title: String,
    /// Vault-relative path.
    pub path: String,
    pub sections: Vec<CardSection>,
    pub body: String,
    /// Raw markdown body (after frontmatter), rendered when the card is
    /// exported without section-aware formatting.
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
pub struct CardSection {
    pub title: String,
    pub markdown: String,
    pub blocks: Vec<Block>,
}

impl Card {
    /// Returns `None` when the file is not a card (no id, or no kind).
    pub fn from_document(doc: &Document, path: &str) -> Option<Card> {
        let fm = &doc.frontmatter;
        let id = fm.id.clone()?;
        let kind = CardKind::parse_kind(fm.kind.as_deref()?);
        let status = CardStatus::parse_status(fm.status.as_deref().unwrap_or("todo"));
        let date = fm
            .date
            .clone()
            .filter(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
            .unwrap_or_else(today);
        let sections = sections_from_body(&doc.raw_body);

        Some(Card {
            id,
            kind,
            date,
            project: fm.project.clone(),
            status,
            tags: fm.tags.clone(),
            title: doc.title.clone(),
            path: path.to_string(),
            sections,
            body: doc.raw_body.clone(),
            blocks: doc.blocks.clone(),
        })
    }

    pub fn section(&self, title: &str) -> Option<&CardSection> {
        self.sections.iter().find(|s| s.title == title)
    }

    /// Frontmatter for this card, ready to serialise.
    pub fn frontmatter(&self) -> Frontmatter {
        Frontmatter {
            present: true,
            id: Some(self.id.clone()),
            kind: Some(self.kind.label().to_string()),
            date: Some(self.date.clone()),
            project: self.project.clone(),
            status: Some(self.status.as_str().to_string()),
            tags: self.tags.clone(),
            title: Some(self.title.clone()),
            extra: Vec::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        format!(
            "---\n{}---\n\n{}",
            self.frontmatter().to_yaml(),
            self.body.trim_start_matches('\n')
        )
    }

    pub fn date_naive(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()
    }

    pub fn plain_text(&self) -> String {
        let mut s = String::new();
        let mut push = |b: &Block| {
            let t = b.plain();
            if !t.is_empty() {
                s.push_str(&t);
                s.push('\n');
            }
        };
        walk_blocks(&self.blocks, &mut push);
        s
    }
}

/// Split the body into `##` sections, respecting fenced code blocks.
/// Returns (preamble, [(title, markdown)]).
pub fn split_sections(body: &str) -> (String, Vec<(String, String)>) {
    let mut preamble = String::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    let mut fence: Option<String> = None;

    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if is_fence {
            let marker = trimmed
                .chars()
                .take_while(|c| *c == '`' || *c == '~')
                .collect::<String>();
            match &fence {
                Some(open) if marker.starts_with(open) || open.starts_with(&marker) => fence = None,
                None => fence = Some(marker),
                _ => {}
            }
        }

        let is_h2 = fence.is_none() && line.starts_with("## ");
        if is_h2 {
            if let Some((title, text)) = current.take() {
                sections.push((title, trim_section(&text)));
            }
            let title = line.trim_start_matches("## ").trim().to_string();
            current = Some((title, String::new()));
            continue;
        }

        match current.as_mut() {
            Some((_, text)) => text.push_str(line),
            None => preamble.push_str(line),
        }
    }

    if let Some((title, text)) = current.take() {
        sections.push((title, trim_section(&text)));
    }

    (preamble, sections)
}

fn trim_section(text: &str) -> String {
    text.trim_matches('\n').to_string()
}

/// Rebuild a body from a preamble and ordered sections.
pub fn join_sections(preamble: &str, sections: &[(String, String)]) -> String {
    let mut out = String::new();
    let pre = preamble.trim();
    if !pre.is_empty() {
        out.push_str(pre);
        out.push_str("\n\n");
    }
    for (title, markdown) in sections {
        out.push_str("## ");
        out.push_str(title.trim());
        out.push('\n');
        let body = markdown.trim_matches('\n');
        if !body.is_empty() {
            out.push('\n');
            out.push_str(body);
            out.push('\n');
        }
        out.push('\n');
    }
    out.trim_end().to_string() + "\n"
}

/// Parse the body's `##` sections into preview-ready structures.
pub fn sections_from_body(body: &str) -> Vec<CardSection> {
    let (preamble, raw_sections) = split_sections(body);
    let mut out = Vec::new();
    if !preamble.trim().is_empty() {
        out.push(CardSection {
            title: "正文".into(),
            blocks: crate::markdown::parse_blocks(&preamble),
            markdown: preamble.trim().to_string(),
        });
    }
    for (title, markdown) in raw_sections {
        out.push(CardSection {
            blocks: crate::markdown::parse_blocks(&markdown),
            markdown,
            title,
        });
    }
    out
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Monday of the week containing `date`, as `YYYY-MM-DD`.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    let days = date.weekday().num_days_from_monday() as i64;
    date - chrono::Duration::days(days)
}

/// Template for a fresh card of the given kind.
pub fn new_card_markdown(
    id: &str,
    kind: &CardKind,
    date: &str,
    project: Option<&str>,
    title: Option<&str>,
) -> String {
    let fm = Frontmatter {
        present: true,
        id: Some(id.to_string()),
        kind: Some(kind.label().to_string()),
        date: Some(date.to_string()),
        project: project.map(|p| p.to_string()),
        status: Some("todo".into()),
        tags: Vec::new(),
        title: title.map(|t| t.to_string()),
        extra: Vec::new(),
    };
    let mut body = String::new();
    for section in kind.sections() {
        body.push_str(&format!("## {section}\n\n"));
    }
    format!("---\n{}---\n\n{}", fm.to_yaml(), body)
}

/// Rewrite the frontmatter of an existing markdown file, keeping the body.
pub fn with_frontmatter(raw: &str, fm: &Frontmatter) -> String {
    let (_, body) = crate::frontmatter::parse(raw);
    format!("---\n{}---\n{}", fm.to_yaml(), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn doc_becomes_card() {
        let raw = "---\nid: exp-001\nkind: 实验卡\ndate: 2026-11-25\nstatus: doing\ntags: [x]\n---\n\n## 问题\n\n为什么\n\n## 结论\n\n好\n";
        let doc = Document::parse(raw, None);
        let card = Card::from_document(&doc, "cards/a.md").unwrap();
        assert_eq!(card.id, "exp-001");
        assert_eq!(card.kind, CardKind::Experiment);
        assert_eq!(card.status, CardStatus::Doing);
        assert_eq!(card.sections.len(), 2);
        assert_eq!(card.sections[0].title, "问题");
    }

    #[test]
    fn new_card_has_all_sections() {
        let md = new_card_markdown(
            "exp-002",
            &CardKind::Experiment,
            "2026-11-25",
            None,
            Some("新实验"),
        );
        let doc = Document::parse(&md, None);
        let card = Card::from_document(&doc, "a.md").unwrap();
        assert_eq!(card.sections.len(), 6);
        assert!(md.contains("关键命令"));
    }

    #[test]
    fn frontmatter_roundtrip_keeps_title() {
        let md = new_card_markdown(
            "exp-003",
            &CardKind::Idea,
            "2026-11-25",
            Some("课题"),
            Some("灵感"),
        );
        let doc = Document::parse(&md, None);
        let card = Card::from_document(&doc, "a.md").unwrap();
        assert_eq!(card.title, "灵感");
        let again = Document::parse(&card.to_markdown(), None);
        assert_eq!(again.title, "灵感");
    }

    #[test]
    fn week_start_is_monday() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        assert_eq!(
            week_start(d),
            NaiveDate::from_ymd_opt(2026, 11, 23).unwrap()
        );
    }
}
