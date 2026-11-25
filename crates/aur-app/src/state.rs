//! Application state: the open vault, its index, and the font paths.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use aur_index::Index;
use aur_vault::{FsStorage, WatchHandle};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInfo {
    pub root: String,
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
    pub watch: Mutex<Option<WatchHandle>>,
    /// `"file"` or `"memory"` — see `VaultInfo::index_mode`.
    pub index_mode: String,
}

impl VaultCtx {
    pub fn rel_path(&self, abs: &str) -> Option<String> {
        let p = Path::new(abs);
        p.strip_prefix(&self.root)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
    }
}

pub struct AppState {
    vault: RwLock<Option<Arc<VaultCtx>>>,
    fonts: Mutex<Vec<PathBuf>>,
    /// Files this session produced (exported PDFs).  They may live outside the
    /// working folder, and they are the only outside paths we will open.
    exports: Mutex<Vec<PathBuf>>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            vault: RwLock::new(None),
            fonts: Mutex::new(Vec::new()),
            exports: Mutex::new(Vec::new()),
        }
    }
}

impl AppState {
    pub fn set_fonts(&self, fonts: Vec<PathBuf>) {
        if let Ok(mut slot) = self.fonts.lock() {
            *slot = fonts;
        }
    }

    pub fn remember_export(&self, path: &Path) {
        let Ok(real) = path.canonicalize() else {
            return;
        };
        if let Ok(mut list) = self.exports.lock() {
            if list.len() > 32 {
                list.remove(0);
            }
            if !list.contains(&real) {
                list.push(real);
            }
        }
    }

    pub fn is_export(&self, path: &Path) -> bool {
        self.exports
            .lock()
            .map(|l| l.iter().any(|p| p == path))
            .unwrap_or(false)
    }

    pub fn fonts(&self) -> Vec<PathBuf> {
        self.fonts.lock().map(|f| f.clone()).unwrap_or_default()
    }

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

    pub fn open(&self, app: &AppHandle, path: &str) -> Result<VaultInfo, String> {
        let storage = FsStorage::new(path)
            .map_err(|e| e.to_string())?
            .with_trash_dir(trash_dir(app, Path::new(path)));
        storage.ensure_layout().map_err(|e| e.to_string())?;

        // The index is a disposable cache, so it lives in the app's own data
        // directory — never inside the user's folder of notes, and never in a
        // location macOS/Windows may guard behind a permission prompt.
        let (index, index_mode) = match open_index(app, storage.root()) {
            Ok(index) => (index, "file"),
            Err(_) => (
                Index::open_in_memory().map_err(|e| e.to_string())?,
                "memory",
            ),
        };
        let docs = index.rebuild(storage.root()).unwrap_or(0);
        let cards = index.cards().map(|c| c.len()).unwrap_or(0);

        let ctx = Arc::new(VaultCtx {
            root: storage.root().to_path_buf(),
            storage,
            index: Mutex::new(index),
            watch: Mutex::new(None),
            index_mode: index_mode.to_string(),
        });

        let handle = app.clone();
        if let Ok(watcher) = ctx.storage.watch(move |change| {
            let _ = handle.emit("vault-changed", change);
        }) {
            if let Ok(mut slot) = ctx.watch.lock() {
                *slot = Some(watcher);
            }
        }

        let info = VaultInfo {
            root: ctx.root.to_string_lossy().to_string(),
            name: ctx
                .root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Aureate".into()),
            docs,
            cards,
            index_mode: index_mode.to_string(),
        };

        if let Ok(mut slot) = self.vault.write() {
            *slot = Some(ctx);
        }
        Ok(info)
    }
}

/// Everything Aureate writes at runtime lives here: a folder inside the
/// application package (`Aureate.app/Contents/Resources/data` on macOS, next to
/// the executable elsewhere).  Nothing is left in Documents, `Application
/// Support`, or any other corner of the machine.
///
/// A read-only install (`/Applications`, `Program Files`) is the one case that
/// cannot honour this, so we degrade instead of failing: the OS user-data
/// folder is the last resort and the UI says so.
pub fn data_root(app: &AppHandle) -> PathBuf {
    if let Some(dir) = std::env::var_os("AUREATE_DATA_DIR") {
        let dir = PathBuf::from(dir);
        if is_writable(&dir) {
            return dir;
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("data"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("data"));
        }
    }
    for dir in candidates {
        if is_writable(&dir) {
            return dir;
        }
    }

    let fallback = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("data");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// Writable in the "can I actually create a file here" sense, not the
/// permission-bit sense — a signed bundle may look writable and not be.
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

fn index_dir(app: &AppHandle) -> PathBuf {
    data_root(app).join("index")
}

fn index_file(app: &AppHandle, root: &Path) -> PathBuf {
    index_file_in(&index_dir(app), root)
}

fn index_file_in(base: &Path, root: &Path) -> PathBuf {
    base.join(format!("{}.db", vault_tag(root)))
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
pub fn trash_dir(app: &AppHandle, root: &Path) -> PathBuf {
    data_root(app).join("trash").join(vault_tag(root))
}

/// Open the on-disk index, healing a database a crash left unusable.  The
/// caller falls back to an in-memory index when this fails.
fn open_index(app: &AppHandle, root: &Path) -> Result<Index, String> {
    let path = index_file(app, root);
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

pub fn settings_path(app: &AppHandle) -> PathBuf {
    data_root(app).join("settings.json")
}

/// Font search order: bundled resources, dev checkout, executable sidecar,
/// optional user directory, then system fallbacks.
pub fn resolve_font_paths(app: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(res) = app.path().resource_dir() {
        dirs.push(res.join("assets/fonts"));
        dirs.push(res.join("fonts"));
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/fonts"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("assets/fonts"));
            dirs.push(parent.join("fonts"));
        }
    }
    if let Some(extra) = std::env::var_os("AUREATE_FONT_DIR") {
        dirs.push(PathBuf::from(extra));
    }
    if let Ok(data) = app.path().app_data_dir() {
        dirs.push(data.join("fonts"));
    }
    dirs.extend(aur_export::fonts::system_font_candidates());
    dirs.retain(|p| p.exists());
    dirs
}
