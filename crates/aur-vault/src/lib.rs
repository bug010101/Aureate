//! `aur-vault` — storage abstraction over a plain folder.
//!
//! The vault is just a directory of `.md` files.  Everything here is written
//! with that constraint in mind: readers and writers never lock the data in,
//! and deletion moves files to an in-vault trash instead of destroying them.

use std::path::{Component, Path, PathBuf};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

#[derive(Debug, Clone)]
pub struct Entry {
    /// Vault-relative path with forward slashes.
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub depth: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("{0}")]
    Io(String),
    #[error("路径不在仓库内：{0}")]
    Outside(String),
    #[error("仓库未初始化：{0}")]
    Invalid(String),
}

impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        VaultError::Io(e.to_string())
    }
}

pub trait Storage {
    fn read(&self, id: &str) -> Result<String, VaultError>;
    fn write(&self, id: &str, content: &str) -> Result<(), VaultError>;
    fn list(&self) -> Result<Vec<Entry>, VaultError>;
    fn remove(&self, id: &str) -> Result<(), VaultError>;
}

pub struct FsStorage {
    root: PathBuf,
    /// Where deleted files go.  `None` means "a hidden folder inside the
    /// vault"; the app points this at its own package so a user folder never
    /// gains `.aureate/`.
    trash: Option<PathBuf>,
}

impl FsStorage {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, VaultError> {
        let root = root.into();
        if !root.is_dir() {
            return Err(VaultError::Invalid(root.display().to_string()));
        }
        Ok(FsStorage {
            root: root
                .canonicalize()
                .map_err(|e| VaultError::Io(e.to_string()))?,
            trash: None,
        })
    }

    pub fn with_trash_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.trash = Some(dir.into());
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Prepare an opened folder for use.
    ///
    /// Only the root itself is created. The vault is the user's own folder of
    /// Markdown files — Aureate does not scatter `docs/`, `cards/` or hidden
    /// index folders into it. The trash folder appears on the first delete.
    pub fn ensure_layout(&self) -> Result<(), VaultError> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    pub fn resolve(&self, rel: &str) -> Result<PathBuf, VaultError> {
        let rel = rel.replace('\\', "/");
        // Absolute paths are never interpreted as "inside the vault".
        if rel.starts_with('/') {
            return Err(VaultError::Outside(rel.clone()));
        }
        let rel = rel.trim_start_matches('/');
        let candidate = Path::new(rel);
        if candidate.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(VaultError::Outside(rel.to_string()));
        }
        Ok(self.root.join(candidate))
    }

    pub fn create_dir(&self, rel: &str) -> Result<(), VaultError> {
        let path = self.resolve(rel)?;
        std::fs::create_dir_all(path)?;
        Ok(())
    }

    /// Atomic write: temp file in the destination folder, then rename.
    pub fn write_atomic(&self, rel: &str, content: &str) -> Result<(), VaultError> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!(
            "{}.tmp",
            path.extension().and_then(|e| e.to_str()).unwrap_or("md")
        ));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(content.as_bytes())?;
            f.sync_all()?;
        }
        #[cfg(windows)]
        {
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
        }
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            VaultError::Io(e.to_string())
        })?;
        Ok(())
    }

    /// Move to `.aureate/trash/<timestamp>-<name>` instead of deleting.
    pub fn trash(&self, rel: &str) -> Result<(), VaultError> {
        let src = self.resolve(rel)?;
        if !src.exists() {
            return Ok(());
        }
        let trash_dir = self
            .trash
            .clone()
            .unwrap_or_else(|| self.root.join(".aureate/trash"));
        std::fs::create_dir_all(&trash_dir)?;
        let stamp = chrono_stamp();
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        let target = trash_dir.join(format!("{stamp}-{name}"));

        // The app keeps its trash inside the package, which may sit on another
        // volume than the vault — rename then fails with EXDEV, so fall back to
        // copy-then-remove rather than reporting a bogus error.
        if std::fs::rename(&src, &target).is_err() {
            if src.is_dir() {
                copy_tree(&src, &target)?;
                std::fs::remove_dir_all(&src)?;
            } else {
                std::fs::copy(&src, &target)?;
                std::fs::remove_file(&src)?;
            }
        }
        Ok(())
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.resolve(rel).map(|p| p.exists()).unwrap_or(false)
    }

    pub fn watch(
        &self,
        mut on_change: impl FnMut(Change) + Send + 'static,
    ) -> Result<WatchHandle, VaultError> {
        let root = self.root.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if event.kind.is_access() {
                return;
            }
            for path in &event.paths {
                let changed = Change {
                    path: path
                        .strip_prefix(&root)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| path.to_string_lossy().to_string()),
                    removed: matches!(
                        event.kind,
                        notify::EventKind::Remove(_)
                            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                    ),
                };
                if changed.path.contains(".aureate") || changed.path.contains(".git/") {
                    continue;
                }
                on_change(changed);
            }
        })
        .map_err(|e| VaultError::Io(e.to_string()))?;

        watcher
            .watch(&self.root, RecursiveMode::Recursive)
            .map_err(|e| VaultError::Io(e.to_string()))?;

        Ok(WatchHandle { _watcher: watcher })
    }
}

pub struct WatchHandle {
    _watcher: RecommendedWatcher,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub removed: bool,
}

impl Storage for FsStorage {
    fn read(&self, id: &str) -> Result<String, VaultError> {
        let path = self.resolve(id)?;
        Ok(std::fs::read_to_string(path)?)
    }

    fn write(&self, id: &str, content: &str) -> Result<(), VaultError> {
        self.write_atomic(id, content)
    }

    fn list(&self) -> Result<Vec<Entry>, VaultError> {
        let mut entries = Vec::new();
        let walker = walkdir::WalkDir::new(&self.root)
            .max_depth(8)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if e.depth() == 0 {
                    return true;
                }
                !(name.starts_with('.')
                    || name == "node_modules"
                    || name == "target"
                    || name == "dist")
            });

        for entry in walker.flatten() {
            if entry.depth() == 0 {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&self.root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().is_dir();
            if !is_dir
                && !matches!(
                    entry.path().extension().and_then(|e| e.to_str()),
                    Some("md") | Some("markdown") | Some("png") | Some("jpg") | Some("jpeg")
                )
            {
                continue;
            }
            let meta = entry.metadata().ok();
            entries.push(Entry {
                path: rel,
                name,
                is_dir,
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                depth: entry.depth() - 1,
            });
        }
        Ok(entries)
    }

    fn remove(&self, id: &str) -> Result<(), VaultError> {
        self.trash(id)
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), VaultError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn chrono_stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("../../target/test-vaults");
        p.push(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn write_is_atomic_and_roundtrips() {
        let root = temp_vault("write");
        let store = FsStorage::new(&root).unwrap();
        store.write_atomic("docs/a.md", "# 你好\n").unwrap();
        assert_eq!(store.read("docs/a.md").unwrap(), "# 你好\n");
        assert!(!root.join("docs/a.md.tmp").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_paths_outside_vault() {
        let root = temp_vault("outside");
        let store = FsStorage::new(&root).unwrap();
        assert!(store.resolve("../etc/passwd").is_err());
        assert!(store.resolve("/etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_moves_to_trash() {
        let root = temp_vault("trash");
        let store = FsStorage::new(&root).unwrap();
        store.write_atomic("a.md", "x").unwrap();
        store.remove("a.md").unwrap();
        assert!(!root.join("a.md").exists());
        let trash: Vec<_> = std::fs::read_dir(root.join(".aureate/trash"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(trash.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
