//! egui UI preferences, stored inside the app's own data directory.

use std::path::Path;

use aur_export::{CardbookOptions, ExportOptions};
use serde::{Deserialize, Serialize};

use crate::state::settings_path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Prefs {
    pub options: Option<ExportOptions>,
    pub cardbook: Option<CardbookOptions>,
    pub dark: bool,
    pub zoom: f32,
}

pub fn load() -> Prefs {
    let path = settings_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(prefs: &Prefs) {
    let path = settings_path();
    let Some(parent) = Path::new(&path).parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(prefs) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}
