//! Bundled font locations.  The four Noto CJK fonts live in the app
//! bundle's `Contents/Resources/fonts` — one copy shared by both
//! architectures of the universal binary, keeping it small.

use std::path::PathBuf;

/// Candidate directories that may contain the bundled fonts, best first.
pub fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(extra) = std::env::var_os("AUREATE_FONT_DIR") {
        dirs.push(PathBuf::from(extra));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Aureate.app/Contents/MacOS/<exe> → Contents/Resources/fonts
            dirs.push(parent.join("../Resources/fonts"));
            // Dev runs with the assets dir copied next to the binary.
            dirs.push(parent.join("assets/fonts"));
        }
    }
    // Dev checkout (`cargo run`): fonts still live in the source tree.
    dirs.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/fonts"
    )));
    dirs.retain(|p| p.is_dir());
    dirs
}

/// Locate a bundled font by file name on disk.
pub fn find_font(name: &str) -> Option<PathBuf> {
    font_dirs()
        .iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}
