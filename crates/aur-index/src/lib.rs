//! `aur-index` — a disposable SQLite cache over the vault.
//!
//! The `.md` files are the data.  Delete the database and everything is
//! rebuilt from disk, which is why this crate is allowed to be lossy.

use std::path::Path;

use aur_core::ast::wiki_links;
use aur_core::{Card, Document};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("索引数据库错误：{0}")]
    Sqlite(String),
    #[error("IO：{0}")]
    Io(String),
}

impl From<rusqlite::Error> for IndexError {
    fn from(e: rusqlite::Error) -> Self {
        IndexError::Sqlite(e.to_string())
    }
}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        IndexError::Io(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub title: String,
    pub kind: Option<String>,
    pub date: Option<String>,
    pub status: Option<String>,
    /// Marked-up excerpt: \u{1} starts a highlight, \u{2} ends it.
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardRow {
    pub id: String,
    pub path: String,
    pub title: String,
    pub kind: String,
    pub date: String,
    pub project: Option<String>,
    pub status: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Backlink {
    pub path: String,
    pub title: String,
    pub id: Option<String>,
    pub kind: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Ledger {
    pub total: usize,
    pub todo: usize,
    pub doing: usize,
    pub done: usize,
    pub blocked: usize,
    pub by_kind: Vec<(String, usize)>,
    pub by_project: Vec<(String, usize)>,
    pub active_days: usize,
}

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Index, IndexError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::prepare(conn)
    }

    pub fn open_in_memory() -> Result<Index, IndexError> {
        let conn = Connection::open_in_memory()?;
        Self::prepare(conn)
    }

    fn prepare(conn: Connection) -> Result<Index, IndexError> {
        conn.busy_timeout(std::time::Duration::from_millis(3000))?;
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS docs (
                path TEXT PRIMARY KEY,
                id TEXT,
                title TEXT NOT NULL,
                kind TEXT,
                date TEXT,
                project TEXT,
                status TEXT,
                tags TEXT,
                body TEXT,
                mtime INTEGER NOT NULL DEFAULT 0,
                is_card INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_docs_date ON docs(date);
            CREATE INDEX IF NOT EXISTS idx_docs_id ON docs(id);
            CREATE INDEX IF NOT EXISTS idx_docs_card ON docs(is_card);
            CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
                path UNINDEXED, title, body, tags, tokenize='trigram'
            );
            CREATE TABLE IF NOT EXISTS links (
                src TEXT NOT NULL,
                dst TEXT NOT NULL,
                PRIMARY KEY (src, dst)
            );
            "#,
        )?;
        Ok(Index { conn })
    }

    /// Insert or update one document.  `path` is vault-relative.
    pub fn upsert(&self, path: &str, raw: &str, mtime: i64) -> Result<(), IndexError> {
        let doc = Document::parse(raw, path.rsplit('/').next());
        let card = Card::from_document(&doc, path);

        let (id, kind, date, project, status, tags, is_card) = match &card {
            Some(c) => (
                Some(c.id.clone()),
                Some(c.kind.label().to_string()),
                Some(c.date.clone()),
                c.project.clone(),
                Some(c.status.as_str().to_string()),
                c.tags.join(", "),
                1i64,
            ),
            None => (None, None, None, None, None, String::new(), 0i64),
        };

        let body = strip_frontmatter(raw);
        let title = doc.title.clone();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO docs (path,id,title,kind,date,project,status,tags,body,mtime,is_card)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(path) DO UPDATE SET
               id=excluded.id, title=excluded.title, kind=excluded.kind, date=excluded.date,
               project=excluded.project, status=excluded.status, tags=excluded.tags,
               body=excluded.body, mtime=excluded.mtime, is_card=excluded.is_card",
            params![path, id, title, kind, date, project, status, tags, body, mtime, is_card],
        )?;
        tx.execute("DELETE FROM docs_fts WHERE path = ?1", params![path])?;
        tx.execute(
            "INSERT INTO docs_fts (path, title, body, tags) VALUES (?1, ?2, ?3, ?4)",
            params![path, doc.title, body, tags],
        )?;
        tx.execute("DELETE FROM links WHERE src = ?1", params![path])?;
        for target in wiki_links(&doc.blocks) {
            tx.execute(
                "INSERT OR IGNORE INTO links (src, dst) VALUES (?1, ?2)",
                params![path, target],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, path: &str) -> Result<(), IndexError> {
        self.conn
            .execute("DELETE FROM docs WHERE path = ?1", params![path])?;
        self.conn
            .execute("DELETE FROM docs_fts WHERE path = ?1", params![path])?;
        self.conn
            .execute("DELETE FROM links WHERE src = ?1", params![path])?;
        Ok(())
    }

    /// Full rebuild from a folder.  Returns the number of indexed documents.
    pub fn rebuild(&self, root: &Path) -> Result<usize, IndexError> {
        let mut seen = Vec::new();
        for file in crate::files::collect_markdown(root) {
            let Ok(raw) = std::fs::read_to_string(&file) else {
                continue;
            };
            let rel = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            let mtime = file
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            self.upsert(&rel, &raw, mtime)?;
            seen.push(rel);
        }

        // Drop documents that disappeared from disk.
        let rows: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT path FROM docs")?;
            let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
            iter.filter_map(|r| r.ok()).collect()
        };
        for path in rows {
            if !seen.contains(&path) {
                self.remove(&path)?;
            }
        }
        Ok(seen.len())
    }

    pub fn count(&self) -> Result<usize, IndexError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn cards(&self) -> Result<Vec<CardRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, title, kind, date, project, status, tags FROM docs
             WHERE is_card = 1 AND id IS NOT NULL ORDER BY date DESC, path ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CardRow {
                id: r.get(0)?,
                path: r.get(1)?,
                title: r.get(2)?,
                kind: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                date: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                project: r.get(5)?,
                status: r
                    .get::<_, Option<String>>(6)?
                    .unwrap_or_else(|| "todo".into()),
                tags: r
                    .get::<_, Option<String>>(7)?
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, IndexError> {
        let q = query.trim();
        if q.is_empty() {
            return self.recent(limit);
        }

        let chars = q.chars().count();
        if chars < 3 {
            return self.search_like(q, limit);
        }

        let tokens: Vec<String> = q
            .split_whitespace()
            .filter(|t| t.chars().count() >= 3)
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect();
        if tokens.is_empty() {
            return self.search_like(q, limit);
        }
        let matcher = tokens.join(" ");

        let mut stmt = self.conn.prepare(
            "SELECT d.path, d.title, d.kind, d.date, d.status,
                    snippet(docs_fts, 2, char(1), char(2), '…', 18)
             FROM docs_fts JOIN docs d ON d.path = docs_fts.path
             WHERE docs_fts MATCH ?1
             ORDER BY bm25(docs_fts, 0.0, 2.0, 1.0, 0.5)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![matcher, limit as i64], |r| {
            Ok(SearchHit {
                path: r.get(0)?,
                title: r.get(1)?,
                kind: r.get(2)?,
                date: r.get(3)?,
                status: r.get(4)?,
                snippet: r.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn search_like(&self, q: &str, limit: usize) -> Result<Vec<SearchHit>, IndexError> {
        let pattern = format!("%{}%", q);
        let mut stmt = self.conn.prepare(
            "SELECT path, title, kind, date, status, body FROM docs
             WHERE title LIKE ?1 OR body LIKE ?1 OR tags LIKE ?1
             ORDER BY mtime DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |r| {
            let body: String = r.get(5)?;
            Ok(SearchHit {
                path: r.get(0)?,
                title: r.get(1)?,
                kind: r.get(2)?,
                date: r.get(3)?,
                status: r.get(4)?,
                snippet: make_snippet(&body, q),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<SearchHit>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT path, title, kind, date, status, body FROM docs
             ORDER BY mtime DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let body: String = r.get(5)?;
            Ok(SearchHit {
                path: r.get(0)?,
                title: r.get(1)?,
                kind: r.get(2)?,
                date: r.get(3)?,
                status: r.get(4)?,
                snippet: preview(&body),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn backlinks(&self, id: &str) -> Result<Vec<Backlink>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT d.path, d.title, d.id, d.kind, d.date
             FROM links l JOIN docs d ON d.path = l.src
             WHERE l.dst = ?1 ORDER BY d.date DESC",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            Ok(Backlink {
                path: r.get(0)?,
                title: r.get(1)?,
                id: r.get(2)?,
                kind: r.get(3)?,
                date: r.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Cards whose `date` falls into the given inclusive range.
    pub fn ledger(&self, from: &str, to: &str) -> Result<Ledger, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, project, status, date FROM docs
             WHERE is_card = 1 AND date >= ?1 AND date <= ?2",
        )?;
        let rows = stmt.query_map(params![from, to], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?
                    .unwrap_or_else(|| "todo".into()),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        })?;

        let mut ledger = Ledger::default();
        let mut days = std::collections::HashSet::new();
        let mut kinds: std::collections::HashMap<String, usize> = Default::default();
        let mut projects: std::collections::HashMap<String, usize> = Default::default();

        for row in rows.flatten() {
            let (kind, project, status, date) = row;
            ledger.total += 1;
            match status.as_str() {
                "doing" => ledger.doing += 1,
                "done" => ledger.done += 1,
                "blocked" => ledger.blocked += 1,
                _ => ledger.todo += 1,
            }
            if !kind.is_empty() {
                *kinds.entry(kind).or_default() += 1;
            }
            if !project.is_empty() {
                *projects.entry(project).or_default() += 1;
            }
            if !date.is_empty() {
                days.insert(date);
            }
        }
        ledger.active_days = days.len();
        let mut k: Vec<_> = kinds.into_iter().collect();
        k.sort_by_key(|x| std::cmp::Reverse(x.1));
        let mut p: Vec<_> = projects.into_iter().collect();
        p.sort_by_key(|x| std::cmp::Reverse(x.1));
        ledger.by_kind = k;
        ledger.by_project = p;
        Ok(ledger)
    }

    /// Document titles by card id, used to resolve `[[id]]` hover labels.
    pub fn titles_for(&self, ids: &[String]) -> Result<Vec<(String, String)>, IndexError> {
        let mut out = Vec::new();
        for id in ids {
            let row = self.conn.query_row(
                "SELECT title FROM docs WHERE id = ?1 LIMIT 1",
                params![id],
                |r| r.get::<_, String>(0),
            );
            if let Ok(title) = row {
                out.push((id.clone(), title));
            }
        }
        Ok(out)
    }
}

fn strip_frontmatter(raw: &str) -> String {
    aur_core::frontmatter::parse(raw).1.to_string()
}

fn preview(body: &str) -> String {
    let text = body
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = text.trim();
    let cut: String = trimmed.chars().take(90).collect();
    if trimmed.chars().count() > 90 {
        format!("{cut}…")
    } else {
        cut
    }
}

fn make_snippet(body: &str, query: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(pos) = flat.find(query) else {
        return preview(body);
    };
    let start = flat[..pos]
        .char_indices()
        .rev()
        .nth(30)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end = (pos + query.len() + 60).min(flat.len());
    let slice = &flat[start..end];
    let marked = slice.replacen(query, &format!("\u{1}{}\u{2}", query), 1);
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        marked,
        if end < flat.len() { "…" } else { "" }
    )
}

mod files {
    use std::path::{Path, PathBuf};

    pub fn collect_markdown(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let walker = walkdir::WalkDir::new(root)
            .max_depth(7)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                if e.file_type().is_dir() {
                    !(name.starts_with('.') || name == "node_modules" || name == "target")
                } else {
                    true
                }
            });
        for entry in walker.flatten() {
            if entry.file_type().is_file()
                && matches!(
                    entry.path().extension().and_then(|e| e.to_str()),
                    Some("md") | Some("markdown")
                )
            {
                files.push(entry.path().to_path_buf());
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = "---\nid: exp-001\nkind: 实验卡\ndate: 2026-11-25\nstatus: doing\nproject: 课题一\ntags: [扩散]\n---\n\n## 问题\n\n为什么误差会累积？\n\n参见 [[paper-002]]\n";
    const PAPER: &str = "---\nid: paper-002\nkind: 论文卡\ndate: 2026-11-24\nstatus: done\n---\n\n## 结论\n\n扩散模型很强。\n";

    #[test]
    fn upsert_list_search_backlinks() {
        let idx = Index::open_in_memory().unwrap();
        idx.upsert("cards/a.md", CARD, 1).unwrap();
        idx.upsert("cards/b.md", PAPER, 2).unwrap();

        let cards = idx.cards().unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, "exp-001");

        let hits = idx.search("误差", 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].snippet.contains('\u{1}'), "{}", hits[0].snippet);

        let back = idx.backlinks("paper-002").unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id.as_deref(), Some("exp-001"));

        let ledger = idx.ledger("2026-11-23", "2026-11-29").unwrap();
        assert_eq!(ledger.total, 2);
        assert_eq!(ledger.doing, 1);
        assert_eq!(ledger.done, 1);
    }

    #[test]
    fn short_query_uses_like() {
        let idx = Index::open_in_memory().unwrap();
        idx.upsert("a.md", CARD, 1).unwrap();
        let hits = idx.search("误差", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn remove_cleans_fts() {
        let idx = Index::open_in_memory().unwrap();
        idx.upsert("a.md", CARD, 1).unwrap();
        idx.remove("a.md").unwrap();
        assert!(idx.search("误差", 10).unwrap().is_empty());
        assert_eq!(idx.count().unwrap(), 0);
    }
}
