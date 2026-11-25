//! All Tauri commands.  The UI never touches the filesystem directly.

use std::path::{Path, PathBuf};

use aur_core::card::{
    join_sections, new_card_markdown, split_sections, Card, CardKind, CardStatus,
};
use aur_core::{frontmatter, word_count, Document};
use aur_export::{CardbookOptions, ExportOptions, ExportReport};
use aur_vault::Storage;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::state::{settings_path, AppState, VaultInfo};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDto {
    path: String,
    name: String,
    is_dir: bool,
    depth: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionDto {
    title: String,
    markdown: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardDto {
    id: String,
    path: String,
    title: String,
    kind: String,
    date: String,
    project: Option<String>,
    status: String,
    status_label: String,
    tags: Vec<String>,
    sections: Vec<SectionDto>,
    accent: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocDto {
    path: String,
    content: String,
    title: String,
    words: usize,
    card: Option<CardDto>,
}

fn card_dto(card: &Card) -> CardDto {
    CardDto {
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
            .map(|s| SectionDto {
                title: s.title.clone(),
                markdown: s.markdown.clone(),
            })
            .collect(),
        accent: card.kind.accent().to_string(),
    }
}

fn doc_dto(path: &str, content: &str) -> DocDto {
    let doc = Document::parse(content, path.rsplit('/').next());
    let words = word_count(&doc.raw_body);
    let card = Card::from_document(&doc, path).map(|c| card_dto(&c));
    DocDto {
        path: path.to_string(),
        content: content.to_string(),
        title: doc.title.clone(),
        words,
        card,
    }
}

fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

fn absolute(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

// ---------------------------------------------------------------------------
// Vault
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn open_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<VaultInfo, String> {
    state.open(&app, &path)
}

#[tauri::command]
pub fn vault_info(state: State<'_, AppState>) -> Result<Option<VaultInfo>, String> {
    let Some(vault) = state.current() else {
        return Ok(None);
    };
    let cards = vault
        .index
        .lock()
        .map(|i| i.cards().map(|c| c.len()).unwrap_or(0))
        .unwrap_or(0);
    let docs = vault
        .index
        .lock()
        .map(|i| i.count().unwrap_or(0))
        .unwrap_or(0);
    Ok(Some(VaultInfo {
        root: vault.root.to_string_lossy().to_string(),
        name: vault
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Aureate".into()),
        docs,
        cards,
        index_mode: vault.index_mode.clone(),
    }))
}

#[tauri::command]
pub fn list_entries(state: State<'_, AppState>) -> Result<Vec<EntryDto>, String> {
    let vault = state.vault()?;
    let entries = vault.storage.list().map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .map(|e| EntryDto {
            path: e.path,
            name: e.name,
            is_dir: e.is_dir,
            depth: e.depth,
        })
        .collect())
}

#[tauri::command]
pub fn sync_index(state: State<'_, AppState>) -> Result<VaultInfo, String> {
    let vault = state.vault()?;
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    let docs = index.rebuild(&vault.root).map_err(|e| e.to_string())?;
    let cards = index.cards().map(|c| c.len()).unwrap_or(0);
    Ok(VaultInfo {
        root: vault.root.to_string_lossy().to_string(),
        name: vault
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Aureate".into()),
        docs,
        cards,
        index_mode: vault.index_mode.clone(),
    })
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn read_doc(state: State<'_, AppState>, path: String) -> Result<DocDto, String> {
    let vault = state.vault()?;
    let content = vault.storage.read(&path).map_err(|e| e.to_string())?;
    Ok(doc_dto(&path, &content))
}

/// Open one file directly: its folder becomes the working root, so the user
/// never has to think about "vaults" first.
#[tauri::command]
pub fn open_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<DocDto, String> {
    let p = Path::new(&path);
    if !p.is_file() {
        return Err("不是文件".into());
    }
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".md") || lower.ends_with(".markdown")) {
        return Err("只能打开 Markdown 文件".into());
    }
    let parent = p.parent().ok_or("无法定位文件所在目录")?;
    state.open(&app, &parent.to_string_lossy())?;
    let vault = state.vault()?;
    let rel = vault.rel_path(&path).ok_or("文件不在该目录下")?;
    let content = vault.storage.read(&rel).map_err(|e| e.to_string())?;
    Ok(doc_dto(&rel, &content))
}

#[tauri::command]
pub fn save_doc(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<DocDto, String> {
    let vault = state.vault()?;
    vault
        .storage
        .write_atomic(&path, &content)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(&path, &content, now_ms());
    }
    Ok(doc_dto(&path, &content))
}

#[tauri::command]
pub fn create_doc(
    state: State<'_, AppState>,
    dir: Option<String>,
    name: Option<String>,
    content: Option<String>,
) -> Result<DocDto, String> {
    let vault = state.vault()?;
    let dir = dir
        .unwrap_or_else(|| "docs".into())
        .trim_matches('/')
        .to_string();
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
    let body = content.unwrap_or_else(|| format!("# {base}\n\n"));
    vault.storage.create_dir(&dir).map_err(|e| e.to_string())?;
    vault
        .storage
        .write_atomic(&rel, &body)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(&rel, &body, now_ms());
    }
    Ok(doc_dto(&rel, &body))
}

#[tauri::command]
pub fn delete_entry(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let vault = state.vault()?;
    vault.storage.remove(&path).map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.remove(&path);
    }
    Ok(())
}

#[tauri::command]
pub fn rename_entry(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<DocDto, String> {
    let vault = state.vault()?;
    let src = vault.storage.resolve(&from).map_err(|e| e.to_string())?;
    let dst = vault.storage.resolve(&to).map_err(|e| e.to_string())?;
    if dst.exists() {
        return Err("目标文件已存在".into());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&src, &dst).map_err(|e| e.to_string())?;
    let content = std::fs::read_to_string(&dst).map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.remove(&from);
        let _ = index.upsert(&to, &content, now_ms());
    }
    Ok(doc_dto(&to, &content))
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardMetaInput {
    title: String,
    kind: String,
    date: String,
    project: Option<String>,
    status: String,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SectionInput {
    title: String,
    markdown: String,
}

#[tauri::command]
pub fn create_card(
    state: State<'_, AppState>,
    kind: String,
    title: Option<String>,
    project: Option<String>,
    body: Option<String>,
) -> Result<DocDto, String> {
    let vault = state.vault()?;
    create_card_impl(&vault, kind, title, project, body)
}

fn create_card_impl(
    vault: &crate::state::VaultCtx,
    kind: String,
    title: Option<String>,
    project: Option<String>,
    body: Option<String>,
) -> Result<DocDto, String> {
    let kind = CardKind::parse_kind(&kind);
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
    let mut markdown = new_card_markdown(&id, &kind, &today, project.as_deref(), Some(&title));
    if let Some(body) = body.filter(|b| !b.trim().is_empty()) {
        let (preamble, mut sections) = split_sections(aur_core::frontmatter::parse(&markdown).1);
        if sections.is_empty() {
            sections.push((kind.sections()[0].to_string(), body));
        } else {
            let first = sections[0].1.clone();
            sections[0].1 = if first.trim().is_empty() {
                body
            } else {
                format!("{first}\n\n{body}")
            };
        }
        let new_body = join_sections(&preamble, &sections);
        markdown = format!(
            "---\n{}---\n\n{}",
            aur_core::frontmatter::parse(&markdown).0.to_yaml(),
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
    Ok(doc_dto(&path, &markdown))
}

#[tauri::command]
pub fn save_card(
    state: State<'_, AppState>,
    path: String,
    meta: CardMetaInput,
    sections: Vec<SectionInput>,
) -> Result<DocDto, String> {
    let vault = state.vault()?;
    let existing = vault.storage.read(&path).map_err(|e| e.to_string())?;
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
        if s.title == "正文" {
            preamble = s.markdown;
        } else {
            next.push((s.title, s.markdown));
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
        .write_atomic(&path, &raw)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(&path, &raw, now_ms());
    }
    Ok(doc_dto(&path, &raw))
}

#[tauri::command]
pub fn update_card_status(
    state: State<'_, AppState>,
    id: String,
    status: String,
) -> Result<DocDto, String> {
    let vault = state.vault()?;
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
    fm.status = Some(CardStatus::parse_status(&status).as_str().to_string());
    let raw = format!("---\n{}---\n{}", fm.to_yaml(), body);
    vault
        .storage
        .write_atomic(&path, &raw)
        .map_err(|e| e.to_string())?;
    if let Ok(index) = vault.index.lock() {
        let _ = index.upsert(&path, &raw, now_ms());
    }
    Ok(doc_dto(&path, &raw))
}

#[tauri::command]
pub fn cards(state: State<'_, AppState>) -> Result<Vec<aur_index::CardRow>, String> {
    let vault = state.vault()?;
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    index.cards().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_card(state: State<'_, AppState>, id: String) -> Result<DocDto, String> {
    let vault = state.vault()?;
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
    let content = vault.storage.read(&path).map_err(|e| e.to_string())?;
    Ok(doc_dto(&path, &content))
}

#[tauri::command]
pub fn backlinks(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<aur_index::Backlink>, String> {
    let vault = state.vault()?;
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    index.backlinks(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ledger(state: State<'_, AppState>) -> Result<LedgerDto, String> {
    let vault = state.vault()?;
    let today = chrono::Local::now().date_naive();
    let start = aur_core::card::week_start(today);
    let end = start + chrono::Duration::days(6);
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    let data = index
        .ledger(&start.to_string(), &end.to_string())
        .map_err(|e| e.to_string())?;
    Ok(LedgerDto {
        week_start: start.to_string(),
        week_end: end.to_string(),
        data,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerDto {
    week_start: String,
    week_end: String,
    data: aur_index::Ledger,
}

#[tauri::command]
pub fn search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<aur_index::SearchHit>, String> {
    let vault = state.vault()?;
    let index = vault.index.lock().map_err(|e| e.to_string())?;
    index
        .search(&query, limit.unwrap_or(30))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_script(
    state: State<'_, AppState>,
    card_id: String,
    index: usize,
    code: String,
) -> Result<String, String> {
    let vault = state.vault()?;
    let safe: String = card_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = format!("scripts/{safe}-{index}.sh");
    let mut body = String::from("#!/usr/bin/env bash\n# saved by Aureate\n\n");
    body.push_str(&code);
    if !body.ends_with('\n') {
        body.push('\n');
    }
    vault
        .storage
        .create_dir("scripts")
        .map_err(|e| e.to_string())?;
    vault
        .storage
        .write_atomic(&path, &body)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(abs) = vault.storage.resolve(&path) {
            let _ = std::fs::set_permissions(&abs, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// Preview & export
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn preview(
    state: State<'_, AppState>,
    path: Option<String>,
    content: String,
    dark: bool,
) -> Result<String, String> {
    let base_dir = path.as_ref().map(|p| {
        let vault = state.current();
        match vault {
            Some(v) => absolute(&v.root, p)
                .parent()
                .map(|d| d.to_path_buf())
                .unwrap_or_else(|| v.root.clone()),
            None => PathBuf::from("."),
        }
    });
    let doc = Document::parse(&content, path.as_deref());
    Ok(aur_render::render_document(
        &doc,
        &aur_render::RenderOptions { dark, base_dir },
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResultDto {
    output: String,
    pages: usize,
    bytes: usize,
    warnings: Vec<String>,
}

impl From<ExportReport> for ExportResultDto {
    fn from(r: ExportReport) -> Self {
        ExportResultDto {
            output: r.output.to_string_lossy().to_string(),
            pages: r.pages,
            bytes: r.bytes,
            warnings: r.warnings,
        }
    }
}

#[tauri::command]
pub async fn export_pdf(
    state: State<'_, AppState>,
    path: Option<String>,
    content: String,
    out: String,
    options: ExportOptions,
) -> Result<ExportResultDto, String> {
    let fonts = state.fonts();
    let doc_path = path.map(PathBuf::from);
    let out_path = PathBuf::from(out);
    let report = tauri::async_runtime::spawn_blocking(move || {
        aur_export::export_markdown(&content, doc_path.as_deref(), &out_path, &options, &fonts)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    state.remember_export(&report.output);
    Ok(report.into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    input: String,
    output: String,
    pages: usize,
    error: Option<String>,
    index: usize,
}

#[tauri::command]
pub async fn export_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    dir: String,
    out_dir: Option<String>,
    options: ExportOptions,
) -> Result<Vec<ProgressDto>, String> {
    let vault = state.vault()?;
    let fonts = state.fonts();
    let input = absolute(&vault.root, &dir);
    let out = out_dir.map(|d| absolute(&vault.root, &d));
    let app_handle = app.clone();

    let report = tauri::async_runtime::spawn_blocking(move || {
        let mut i = 0usize;
        aur_export::export_folder(&input, out.as_deref(), &options, &fonts, |item| {
            let _ = app_handle.emit(
                "export-progress",
                ProgressDto {
                    input: item.input.to_string_lossy().to_string(),
                    output: item.output.to_string_lossy().to_string(),
                    pages: item.pages,
                    error: item.error.clone(),
                    index: i,
                },
            );
            i += 1;
        })
    })
    .await
    .map_err(|e| e.to_string())?;

    for item in &report.items {
        if item.error.is_none() {
            state.remember_export(&item.output);
        }
    }

    Ok(report
        .items
        .into_iter()
        .map(|i| ProgressDto {
            input: i.input.to_string_lossy().to_string(),
            output: i.output.to_string_lossy().to_string(),
            pages: i.pages,
            error: i.error,
            index: 0,
        })
        .collect())
}

#[tauri::command]
pub async fn export_cardbook(
    state: State<'_, AppState>,
    paths: Vec<String>,
    out: String,
    options: CardbookOptions,
) -> Result<ExportResultDto, String> {
    let vault = state.vault()?;
    let fonts = state.fonts();
    let mut cards = Vec::new();
    for p in &paths {
        let raw = vault.storage.read(p).map_err(|e| e.to_string())?;
        let doc = Document::parse(&raw, p.rsplit('/').next());
        if let Some(card) = Card::from_document(&doc, p) {
            cards.push(card);
        }
    }
    if cards.is_empty() {
        return Err("没有可导出的卡片".into());
    }
    let root = vault.root.clone();
    let out_path = PathBuf::from(out);
    let report = tauri::async_runtime::spawn_blocking(move || {
        aur_export::export_cardbook(&cards, &root, &out_path, &options, &fonts)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    state.remember_export(&report.output);
    Ok(report.into())
}

#[tauri::command]
pub fn typst_source(
    state: State<'_, AppState>,
    path: Option<String>,
    content: String,
    options: ExportOptions,
) -> Result<String, String> {
    let fonts = state.fonts();
    Ok(aur_export::generate_source(
        &content,
        path.as_deref().map(Path::new),
        &options,
        &fonts,
    ))
}

// ---------------------------------------------------------------------------
// Shell helpers & diagnostics
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn read_image(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
    if meta.len() > 24 * 1024 * 1024 {
        return Err("图片超过 24 MB，未加载".into());
    }
    let data = std::fs::read(p).map_err(|e| e.to_string())?;
    let mime = match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        _ => "image/png",
    };
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    Ok(format!("data:{mime};base64,{b64}"))
}

#[tauri::command]
pub fn reveal(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let vault = state.vault()?;
    let p = confined(&state, &vault.root, &path)?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(p)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = p.parent().unwrap_or(&p);
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Only paths inside the working folder may be opened or revealed — the app
/// must never surface its own package directory to the user.
fn confined(state: &AppState, root: &Path, raw: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(raw);
    let abs = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let real = abs.canonicalize().map_err(|_| "文件不存在".to_string())?;
    // Allowed: anything in the working folder, plus files this session just
    // exported.  The app's own package is never on that list.
    if real.starts_with(root) || state.is_export(&real) {
        return Ok(real);
    }
    Err("只能打开当前工作目录内的文件".into())
}

#[tauri::command]
pub fn open_external(state: State<'_, AppState>, path: String) -> Result<(), String> {
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("mailto:") {
        return open_with_system(&path);
    }
    let vault = state.vault()?;
    let real = confined(&state, &vault.root, &path)?;
    open_with_system(&real.to_string_lossy())
}

fn open_with_system(path: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontReport {
    serif: String,
    sans: String,
    mono: String,
    has_cjk: bool,
    warnings: Vec<String>,
    families: Vec<String>,
}

#[tauri::command]
pub fn font_report(state: State<'_, AppState>) -> FontReport {
    let set = aur_export::load_fonts(&state.fonts());
    let mut families: Vec<String> = set.book.families().map(|(f, _)| f.to_string()).collect();
    families.sort();
    families.truncate(60);
    FontReport {
        serif: set.serif.clone(),
        sans: set.sans.clone(),
        mono: set.mono.clone(),
        has_cjk: set.has_cjk,
        warnings: set.warnings.clone(),
        families,
    }
}

/// Render one formula to inline SVG with the same Typst engine that prints
/// the PDF, so preview and paper can never disagree.
#[tauri::command]
pub fn preview_math(
    tex: String,
    display: Option<bool>,
    size: Option<f64>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let size = size.unwrap_or(15.0);
    aur_export::render_math(&tex, display.unwrap_or(false), size, &state.fonts())
        .map(|m| m.svg)
        .map_err(|e| format!("公式渲染失败：{e}"))
}

/// UI preferences (theme, paper, zoom …) live in the app package next to the
/// index, so opening the app never writes to the user's home folder.
#[tauri::command]
pub fn load_prefs(app: AppHandle) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(settings_path(&app)).ok()?;
    serde_json::from_str(&raw).ok()
}

#[tauri::command]
pub fn save_prefs(app: AppHandle, prefs: serde_json::Value) -> Result<(), String> {
    let path = settings_path(&app);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
