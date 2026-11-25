//! Business operations: documents, cards, search, export, shell helpers.
//! No framework types — the UI calls these directly.

use std::path::Path;

use aur_core::card::{
    join_sections, new_card_markdown, split_sections, Card, CardKind, CardStatus,
};
use aur_core::{frontmatter, Document};
use aur_index::{CardRow, Ledger, SearchHit};
use aur_vault::{Entry, Storage};

use crate::state::{AppState, VaultCtx};

fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SectionInfo {
    pub title: String,
    pub markdown: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CardInfo {
    pub id: String,
    pub path: String,
    pub title: String,
    pub kind: String,
    pub date: String,
    pub project: Option<String>,
    pub status: String,
    pub status_label: String,
    pub tags: Vec<String>,
    pub sections: Vec<SectionInfo>,
    pub accent: String,
}

#[derive(Debug, Clone)]
pub struct DocInfo {
    pub path: String,
    pub content: String,
    pub title: String,
    pub card: Option<CardInfo>,
}

fn card_info(card: &Card) -> CardInfo {
    CardInfo {
        id: card.id.clone(),
        path: card.path.clone(),
        title: card.title.clone(),
        kind: card.kind.label().to_string(),
        date: card.date.clone(),
        project: card.project.clone(),
        status: card.status.as_str().to_string(),
        status_label: card.status.label().to_string(),
        tags: card.tags.clone(),
        sections: card
            .sections
            .iter()
            .map(|s| SectionInfo {
                title: s.title.clone(),
                markdown: s.markdown.clone(),
            })
            .collect(),
        accent: card.kind.accent().to_string(),
    }
}

fn doc_info(path: &str, content: &str) -> DocInfo {
    let doc = Document::parse(content, path.rsplit('/').next());
    let card = Card::from_document(&doc, path).map(|c| card_info(&c));
    DocInfo {
        path: path.to_string(),
        content: content.to_string(),
        title: doc.title.clone(),
        card,
    }
}

pub fn read_doc(vault: &VaultCtx, path: &str) -> Result<DocInfo, String> {
    let content = vault.storage.read(path).map_err(|e| e.to_string())?;
    Ok(doc_info(path, &content))
}

/// Open one file directly: its folder becomes the working root, so the user
/// never has to think about "vaults" first.
pub fn open_file(
    app: &AppState,
    path: &str,
    on_change: Option<std::sync::mpsc::Sender<aur_vault::Change>>,
) -> Result<DocInfo, String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err("不是文件".into());
    }
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".md") || lower.ends_with(".markdown")) {
        return Err("只能打开 Markdown 文件".into());
    }
    let parent = p.parent().ok_or("无法定位文件所在目录")?;
    let vault = match app.current() {
        Some(v) if v.root == parent => v,
        _ => {
            app.open(&parent.to_string_lossy(), on_change)?;
            app.vault()?
        }
    };
    let rel = vault.rel_path(path).ok_or("文件不在该目录下")?;
    read_doc(&vault, &rel)
}

pub fn save_doc(vault: &VaultCtx, path: &str, content: &str) -> Result<DocInfo, String> {
    vault
        .storage
        .write_atomic(path, content)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(path, content, now_ms());
    }
    Ok(doc_info(path, content))
}

pub fn create_doc(
    vault: &VaultCtx,
    dir: Option<&str>,
    name: Option<&str>,
) -> Result<DocInfo, String> {
    let dir = dir.unwrap_or("docs").trim_matches('/').to_string();
    let base = name
        .map(|n| n.trim().trim_end_matches(".md").to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "未命名文档".into());
    let mut rel = format!("{dir}/{base}.md");
    let mut n = 2;
    while vault.storage.exists(&rel) {
        rel = format!("{dir}/{base}-{n}.md");
        n += 1;
    }
    let body = format!("# {base}\n\n");
    vault.storage.create_dir(&dir).map_err(|e| e.to_string())?;
    vault
        .storage
        .write_atomic(&rel, &body)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(&rel, &body, now_ms());
    }
    Ok(doc_info(&rel, &body))
}

pub fn delete_entry(vault: &VaultCtx, path: &str) -> Result<(), String> {
    vault.storage.trash(path).map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.remove(path);
    }
    Ok(())
}

pub fn rename_entry(vault: &VaultCtx, from: &str, to: &str) -> Result<DocInfo, String> {
    let src = vault.storage.resolve(from).map_err(|e| e.to_string())?;
    let dst = vault.storage.resolve(to).map_err(|e| e.to_string())?;
    if dst.exists() {
        return Err("目标文件已存在".into());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&src, &dst).map_err(|e| e.to_string())?;
    let content = std::fs::read_to_string(&dst).map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.remove(from);
        let _ = index.upsert(to, &content, now_ms());
    }
    Ok(doc_info(to, &content))
}

pub fn list_entries(vault: &VaultCtx) -> Result<Vec<Entry>, String> {
    vault.storage.list().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CardMetaInput {
    pub title: String,
    pub kind: String,
    pub date: String,
    pub project: Option<String>,
    pub status: String,
    pub tags: Vec<String>,
}

pub fn create_card(
    vault: &VaultCtx,
    kind: &str,
    title: Option<&str>,
    project: Option<&str>,
    body: Option<&str>,
) -> Result<DocInfo, String> {
    let kind = CardKind::parse_kind(kind);
    let today = aur_core::card::today();
    let base_id = format!(
        "{}-{}",
        kind.slug(),
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let mut id = base_id.clone();
    let mut n = 2;
    while vault.storage.exists(&format!("cards/{id}.md")) {
        id = format!("{base_id}-{n}");
        n += 1;
    }
    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "未命名卡片".to_string());
    let mut markdown = new_card_markdown(&id, &kind, &today, project, Some(&title));
    if let Some(body) = body.filter(|b| !b.trim().is_empty()) {
        let (preamble, mut sections) = split_sections(frontmatter::parse(&markdown).1);
        if sections.is_empty() {
            sections.push((kind.sections()[0].to_string(), body.to_string()));
        } else {
            let first = sections[0].1.clone();
            sections[0].1 = if first.trim().is_empty() {
                body.to_string()
            } else {
                format!("{first}\n\n{body}")
            };
        }
        let new_body = join_sections(&preamble, &sections);
        markdown = format!(
            "---\n{}---\n\n{}",
            frontmatter::parse(&markdown).0.to_yaml(),
            new_body
        );
    }

    vault
        .storage
        .create_dir("cards")
        .map_err(|e| e.to_string())?;
    let path = format!("cards/{id}.md");
    vault
        .storage
        .write_atomic(&path, &markdown)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(&path, &markdown, now_ms());
    }
    Ok(doc_info(&path, &markdown))
}

pub fn save_card(
    vault: &VaultCtx,
    path: &str,
    meta: &CardMetaInput,
    sections: &[(String, String)],
) -> Result<DocInfo, String> {
    let existing = vault.storage.read(path).map_err(|e| e.to_string())?;
    let (mut fm, body) = frontmatter::parse(&existing);
    let body = body.to_string();

    fm.present = true;
    if fm.id.is_none() {
        fm.id = Some(format!(
            "card-{}",
            chrono::Local::now().format("%Y%m%d%H%M%S")
        ));
    }
    fm.kind = Some(meta.kind.clone());
    fm.date = Some(meta.date.clone());
    fm.project = meta.project.clone().filter(|p| !p.trim().is_empty());
    fm.status = Some(meta.status.clone());
    fm.tags = meta.tags.clone();
    fm.title = Some(meta.title.clone());

    let (mut preamble, _) = split_sections(&body);
    let mut next: Vec<(String, String)> = Vec::new();
    for s in sections {
        if s.0 == "正文" {
            preamble = s.1.clone();
        } else {
            next.push((s.0.clone(), s.1.clone()));
        }
    }
    if next.is_empty() {
        let kind = CardKind::parse_kind(&meta.kind);
        next = kind
            .sections()
            .iter()
            .map(|s| (s.to_string(), String::new()))
            .collect();
    }

    let new_body = join_sections(&preamble, &next);
    let raw = format!("---\n{}---\n\n{}", fm.to_yaml(), new_body);
    vault
        .storage
        .write_atomic(path, &raw)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(path, &raw, now_ms());
    }
    Ok(doc_info(path, &raw))
}

pub fn update_card_status(vault: &VaultCtx, id: &str, status: &str) -> Result<DocInfo, String> {
    let path = {
        let index = vault.index.lock().map_err(|e| e.to_string())?;
        index
            .cards()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.path)
            .ok_or_else(|| format!("找不到卡片 {id}"))?
    };
    let raw = vault.storage.read(&path).map_err(|e| e.to_string())?;
    let (mut fm, body) = frontmatter::parse(&raw);
    fm.present = true;
    fm.status = Some(CardStatus::parse_status(status).as_str().to_string());
    let raw = format!("---\n{}---\n{}", fm.to_yaml(), body);
    vault
        .storage
        .write_atomic(&path, &raw)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(&path, &raw, now_ms());
    }
    Ok(doc_info(&path, &raw))
}

pub fn cards(vault: &VaultCtx) -> Result<Vec<CardRow>, String> {
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    index.cards().map_err(|e| e.to_string())
}

pub fn card_path(vault: &VaultCtx, id: &str) -> Result<String, String> {
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    index
        .cards()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|c| c.id == id)
        .map(|c| c.path)
        .ok_or_else(|| format!("找不到卡片 {id}"))
}

#[derive(Debug, Clone)]
pub struct LedgerInfo {
    pub week_start: String,
    pub week_end: String,
    pub data: Ledger,
}

pub fn ledger(vault: &VaultCtx) -> Result<LedgerInfo, String> {
    let today = chrono::Local::now().date_naive();
    let start = aur_core::card::week_start(today);
    let end = start + chrono::Duration::days(6);
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    let data = index
        .ledger(&start.to_string(), &end.to_string())
        .map_err(|e| e.to_string())?;
    Ok(LedgerInfo {
        week_start: start.to_string(),
        week_end: end.to_string(),
        data,
    })
}

pub fn search(vault: &VaultCtx, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    index.search(query, limit).map_err(|e| e.to_string())
}
