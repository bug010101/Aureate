//! Application state: the open vault, its index, and font lookup paths.
//! Pure Rust — no webview, no shell bindings.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};

use aur_index::Index;
use aur_vault::{Change, FsStorage, WatchHandle};

#[derive(Debug, Clone)]
pub struct VaultInfo {
    pub name: String,
    pub docs: usize,
    pub cards: usize,
    /// `"file"` when the on-disk index is live, `"memory"` when it had to fall
    /// back (read-only folder, permission denied, corrupt database …).
    pub index_mode: String,
}

pub struct VaultCtx {
    pub root: PathBuf,
    pub storage: FsStorage,
    pub index: Mutex<Index>,
    #[allow(dead_code)]
    pub watch: Mutex<Option<WatchHandle>>,
    pub index_mode: String,
}

impl VaultCtx {
    pub fn rel_path(&self, abs: &str) -> Option<String> {
        let p = Path::new(abs);
        p.strip_prefix(&self.root)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
    }

    pub fn info(&self) -> VaultInfo {
        let index = self.index.lock().ok();
        VaultInfo {
            name: self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Aureate".into()),
            docs: index.as_ref().map(|i| i.count().unwrap_or(0)).unwrap_or(0),
            cards: index
                .as_ref()
                .map(|i| i.cards().map(|c| c.len()).unwrap_or(0))
                .unwrap_or(0),
            index_mode: self.index_mode.clone(),
        }
    }
}

pub struct AppState {
    vault: RwLock<Option<Arc<VaultCtx>>>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            vault: RwLock::new(None),
        }
    }
}

impl AppState {
    pub fn vault(&self) -> Result<Arc<VaultCtx>, String> {
        self.vault
            .read()
            .ok()
            .and_then(|v| v.clone())
            .ok_or_else(|| "还没有打开仓库".to_string())
    }

    pub fn current(&self) -> Option<Arc<VaultCtx>> {
        self.vault.read().ok().and_then(|v| v.clone())
    }

    /// Open a folder as the working vault.  File-system changes are forwarded
    /// to `on_change` (the UI polls a channel) when given.
    pub fn open(
        &self,
        path: &str,
        on_change: Option<mpsc::Sender<Change>>,
    ) -> Result<VaultInfo, String> {
        let storage = FsStorage::new(path)
            .map_err(|e| e.to_string())?
            .with_trash_dir(trash_dir(Path::new(path)));
        storage.ensure_layout().map_err(|e| e.to_string())?;

        // The index is a disposable cache, so it lives in the app's own data
        // directory — never inside the user's folder of notes.
        let (index, index_mode) = match open_index(storage.root()) {
            Ok(index) => (index, "file"),
            Err(_) => (
                Index::open_in_memory().map_err(|e| e.to_string())?,
                "memory",
            ),
        };
        let ctx = Arc::new(VaultCtx {
            root: storage.root().to_path_buf(),
            storage,
            index: Mutex::new(index),
            watch: Mutex::new(None),
            index_mode: index_mode.to_string(),
        });
        let docs = ctx
            .index
            .lock()
            .map_err(|e| e.to_string())?
            .rebuild(&ctx.root)
            .map_err(|e| e.to_string())?;
        let cards = ctx
            .index
            .lock()
            .map(|i| i.cards().map(|c| c.len()).unwrap_or(0))
            .unwrap_or(0);

        if let Some(tx) = on_change {
            if let Ok(watcher) = ctx.storage.watch(move |_change| {
                let _ = tx.send(Change {
                    path: String::new(),
                    removed: false,
                });
            }) {
                if let Ok(mut slot) = ctx.watch.lock() {
                    *slot = Some(watcher);
                }
            }
        }

        let info = VaultInfo {
            docs,
            cards,
            ..ctx.info()
        };

        if let Ok(mut slot) = self.vault.write() {
            *slot = Some(ctx);
        }
        Ok(info)
    }
}

/// Everything Aureate writes at runtime lives here: a folder inside the
/// application package (`Aureate.app/Contents/Resources/data`), next to the
/// executable in development, or `$HOME/.aureate/data` as the last resort.
/// Nothing is left in Documents, `Application Support`, or any other corner.
pub fn data_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("AUREATE_DATA_DIR") {
        let dir = PathBuf::from(dir);
        if is_writable(&dir) {
            return dir;
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // .app bundle layout: Contents/MacOS/bin → Contents/Resources/data
            candidates.push(parent.join("../../Resources/data"));
            candidates.push(parent.join("../Resources/data"));
            candidates.push(parent.join("data"));
        }
    }
    for dir in candidates {
        if is_writable(&dir) {
            return dir;
        }
    }

    let fallback = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".aureate")
        .join("data");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// Writable in the "can I actually create a file here" sense, not the
/// permission-bit sense.
pub fn is_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-probe");
    match std::fs::write(&probe, b"1") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Short, stable name for a vault path — used for cache files that must not
/// collide between two folders with the same name.
fn vault_tag(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Deleted files go to the app's own trash, never into the user's folder.
pub fn trash_dir(root: &Path) -> PathBuf {
    data_root().join("trash").join(vault_tag(root))
}

/// Open the on-disk index, healing a database a crash left unusable.  The
/// caller falls back to an in-memory index when this fails.
fn open_index(root: &Path) -> Result<Index, String> {
    let path = data_root()
        .join("index")
        .join(format!("{}.db", vault_tag(root)));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match Index::open(&path) {
        Ok(index) => Ok(index),
        Err(_) => {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut p = path.clone().into_os_string();
                p.push(suffix);
                let _ = std::fs::remove_file(PathBuf::from(p));
            }
            Index::open(&path).map_err(|e| e.to_string())
        }
    }
}

pub fn settings_path() -> PathBuf {
    data_root().join("settings.json")
}

/// Font search order: bundled resources, dev checkout, executable sidecar,
/// optional user directory, then system fallbacks.
pub fn resolve_font_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(extra) = std::env::var_os("AUREATE_FONT_DIR") {
        dirs.push(PathBuf::from(extra));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("../../Resources/assets/fonts"));
            dirs.push(parent.join("../Resources/assets/fonts"));
            dirs.push(parent.join("assets/fonts"));
            dirs.push(parent.join("fonts"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/fonts"));
    dirs.extend(aur_export::fonts::system_font_candidates());
    dirs.retain(|p| p.exists());
    dirs
}
