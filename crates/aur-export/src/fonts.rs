//! Font loading for the Typst world.
//!
//! The PDF must not depend on whatever fonts the machine happens to have:
//! exported documents embed font subsets, so we always load a known set.
//! Bundled Noto CJK fonts come first; system fonts are only a fallback for
//! development machines that miss the bundled files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use typst::foundations::Bytes;
use typst::text::{Font, FontBook};

#[derive(Debug)]
pub struct FontSet {
    pub fonts: Vec<Font>,
    pub book: FontBook,
    pub serif: String,
    pub sans: String,
    pub mono: String,
    pub has_cjk: bool,
    pub warnings: Vec<String>,
}

const SERIF: &[&str] = &[
    "noto serif sc",
    "source han serif sc",
    "noto serif cjk sc",
    "source han serif cn",
    "songti sc",
    "stsong",
    "simsun",
    "noto serif cjk",
];

const SANS: &[&str] = &[
    "noto sans sc",
    "source han sans sc",
    "noto sans cjk sc",
    "pingfang sc",
    "microsoft yahei",
    "hiragino sans gb",
    "wenquanyi micro hei",
    "noto sans cjk",
];

const MONO: &[&str] = &[
    "dejavu sans mono",
    "jetbrains mono",
    "cascadia mono",
    "cascadia code",
    "consolas",
    "sf mono",
    "menlo",
    "liberation mono",
];

/// System files worth opening when the bundled fonts are absent.
pub fn system_font_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        [
            "/System/Library/Fonts/Supplemental/Songti.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
    #[cfg(target_os = "windows")]
    {
        [
            "C:/Windows/Fonts/msyh.ttc",
            "C:/Windows/Fonts/msyh.ttf",
            "C:/Windows/Fonts/simsun.ttc",
            "C:/Windows/Fonts/simhei.ttf",
            "C:/Windows/Fonts/consola.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        [
            "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
}

fn cache() -> &'static Mutex<Vec<(String, Arc<FontSet>)>> {
    type FontCache = Vec<(String, Arc<FontSet>)>;
    static CACHE: OnceLock<Mutex<FontCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Load fonts from a set of directories / files, with caching.
pub fn load(paths: &[PathBuf]) -> Arc<FontSet> {
    let key = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("|");

    if let Ok(cache) = cache().lock() {
        if let Some((_, set)) = cache.iter().find(|(k, _)| *k == key) {
            return set.clone();
        }
    }

    let set = Arc::new(load_uncached(paths));
    if let Ok(mut cache) = cache().lock() {
        cache.push((key, set.clone()));
    }
    set
}

fn font_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_file() {
            files.push(path.clone());
        } else if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if is_font(&p) {
                        files.push(p);
                    }
                }
            }
        }
    }
    files
}

fn is_font(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("otf" | "ttf" | "ttc" | "otc")
    )
}

fn load_uncached(paths: &[PathBuf]) -> FontSet {
    let mut fonts: Vec<Font> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut warnings = Vec::new();

    // Fonts shipped with the typst crate: math, latin serif, mono.
    for data in typst_assets::fonts() {
        for font in Font::iter(Bytes::new(data.to_vec())) {
            let id = font_id(&font);
            if seen.insert(id) {
                fonts.push(font);
            }
        }
    }

    for path in font_files(paths) {
        let Ok(data) = std::fs::read(&path) else {
            warnings.push(format!("无法读取字体 {}", path.display()));
            continue;
        };
        if data.len() > 64 * 1024 * 1024 {
            warnings.push(format!("跳过过大的字体 {}", path.display()));
            continue;
        }
        let mut any = false;
        for font in Font::iter(Bytes::new(data)) {
            any = true;
            let id = font_id(&font);
            if seen.insert(id) {
                fonts.push(font);
            }
        }
        if !any {
            warnings.push(format!("字体格式无法识别：{}", path.display()));
        }
    }

    let book = FontBook::from_fonts(fonts.iter());
    let families: Vec<String> = book.families().map(|(f, _)| f.to_lowercase()).collect();

    let pick = |candidates: &[&str]| -> Option<String> {
        for cand in candidates {
            for family in &families {
                if family == *cand || family.starts_with(&format!("{cand} ")) {
                    // Return the original casing from the book.
                    if let Some((orig, _)) =
                        book.families().find(|(f, _)| f.to_lowercase() == *family)
                    {
                        return Some(orig.to_string());
                    }
                }
            }
        }
        // Looser second pass: substring match.
        for cand in candidates {
            if let Some((orig, _)) = book
                .families()
                .find(|(f, _)| f.to_lowercase().contains(cand))
            {
                return Some(orig.to_string());
            }
        }
        None
    };

    let serif_found = pick(SERIF);
    let sans_found = pick(SANS);
    let mono_found = pick(MONO).unwrap_or_else(|| "DejaVu Sans Mono".to_string());
    let has_cjk = serif_found.is_some() || sans_found.is_some();

    if !has_cjk {
        warnings.push(
            "没有找到中文字体，PDF 中的中文可能显示为方块。请把 NotoSerifSC / NotoSansSC 放进字体目录。"
                .into(),
        );
    }

    let serif = serif_found
        .clone()
        .or_else(|| sans_found.clone())
        .unwrap_or_else(|| "Libertinus Serif".to_string());
    let sans = sans_found
        .clone()
        .or_else(|| serif_found.clone())
        .unwrap_or_else(|| "Libertinus Serif".to_string());

    FontSet {
        fonts,
        book,
        serif,
        sans,
        mono: mono_found,
        has_cjk,
        warnings,
    }
}

fn font_id(font: &Font) -> String {
    font.post_script_name()
        .unwrap_or_else(|| format!("{}-{:?}", font.info().family, font.info().variant))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fonts_always_load() {
        let set = load(&[]);
        assert!(!set.fonts.is_empty());
        assert!(set.book.families().count() > 0);
    }
}
