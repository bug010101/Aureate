//! PDF production: the only export path in the app.
//!
//! Markdown → AST (`aur-core`) → Typst markup → Typst compiler → PDF bytes.
//! The same code runs on every platform, so output is byte-for-byte identical
//! regardless of the OS.

use std::path::{Path, PathBuf};

use aur_core::ast::Block;
use aur_core::{walk_blocks, Card, Document};
use typst_layout::PagedDocument;

use crate::fonts::{self, FontSet};
use crate::options::{CardbookOptions, ExportOptions};
use crate::typst_gen;
use crate::world::TypstWorld;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ExportError {
    #[error("读写文件失败：{0}")]
    Io(String),
    #[error("Typst 编译失败：{0}")]
    Compile(String),
    #[error("PDF 生成失败：{0}")]
    Pdf(String),
}

#[derive(Debug, Clone)]
pub struct ExportReport {
    pub output: PathBuf,
    pub pages: usize,
    pub bytes: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BatchItem {
    pub input: PathBuf,
    pub output: PathBuf,
    pub pages: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BatchReport {
    pub items: Vec<BatchItem>,
    pub succeeded: usize,
    pub failed: usize,
}

/// Export one markdown document.
#[allow(clippy::too_many_arguments)]
pub fn export_markdown(
    markdown: &str,
    doc_path: Option<&Path>,
    output: &Path,
    opts: &ExportOptions,
    font_paths: &[PathBuf],
    embedded_fonts: &[(&'static str, Vec<u8>)],
) -> Result<ExportReport, ExportError> {
    let title_hint = doc_path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string());
    let doc = Document::parse(markdown, title_hint.as_deref());
    let base_dir = doc_path
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let font_set = fonts::load_embedded(font_paths, embedded_fonts);
    let has_math = contains_math(&doc);

    let mut attempts = 0;
    let mut use_text_math = opts.math_as_text;
    loop {
        attempts += 1;
        let mut effective = opts.clone();
        effective.math_as_text = use_text_math;

        let gen = typst_gen::generate(&doc, &effective, &font_set, &base_dir);
        let mut warnings = font_set.warnings.clone();
        warnings.extend(gen.warnings.clone());

        match compile(&gen.source, font_set.clone(), gen.assets, output) {
            Ok((pages, bytes)) => {
                return Ok(ExportReport {
                    output: output.to_path_buf(),
                    pages,
                    bytes,
                    warnings,
                });
            }
            Err(err) => {
                // Second chance: a single unmappable formula must not cost the
                // whole document.  Re-render math as literal TeX.
                if attempts == 1 && has_math && !use_text_math {
                    use_text_math = true;
                    continue;
                }
                return Err(err);
            }
        }
    }
}

fn compile(
    source: &str,
    fonts: std::sync::Arc<FontSet>,
    assets: typst_gen::Assets,
    output: &Path,
) -> Result<(usize, usize), ExportError> {
    let world = TypstWorld::new(source.to_string(), fonts, assets.map);
    let warned = typst::compile::<PagedDocument>(&world);
    let document = warned.output.map_err(|errors| {
        let mut msg = String::new();
        for (i, err) in errors.iter().take(4).enumerate() {
            if i > 0 {
                msg.push('；');
            }
            msg.push_str(&err.message);
            for hint in &err.hints {
                msg.push_str(&format!("（{}）", hint.v));
            }
        }
        if errors.len() > 4 {
            msg.push_str(&format!(" …共 {} 个错误", errors.len()));
        }
        ExportError::Compile(msg)
    })?;

    let pages = document.pages().len();
    let bytes = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
        .map_err(|e| ExportError::Pdf(format!("{:?}", e)))?;
    write_atomic(output, &bytes)?;
    Ok((pages, bytes.len()))
}

fn contains_math(doc: &Document) -> bool {
    let mut found = false;
    let mut visit = |b: &Block| {
        if matches!(b, Block::Math { .. }) {
            found = true;
        }
    };
    walk_blocks(&doc.blocks, &mut visit);
    found
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ExportError::Io(e.to_string()))?;
    }
    let tmp = path.with_extension(format!(
        "{}.aureate-tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("pdf")
    ));
    std::fs::write(&tmp, bytes).map_err(|e| ExportError::Io(e.to_string()))?;
    #[cfg(windows)]
    {
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ExportError::Io(e.to_string())
    })?;
    Ok(())
}

/// Recursively collect markdown files, deepest 5 levels, skipping dot folders.
pub fn collect_markdown(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .max_depth(6)
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
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md") | Some("markdown")
        ) {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    files
}

/// Export a whole folder, continuing after failures and reporting each item.
pub fn export_folder(
    input: &Path,
    out_dir: Option<&Path>,
    opts: &ExportOptions,
    font_paths: &[PathBuf],
    embedded_fonts: &[(&'static str, Vec<u8>)],
    mut on_progress: impl FnMut(&BatchItem),
) -> BatchReport {
    let files = collect_markdown(input);
    let total = files.len();
    let mut report = BatchReport::default();

    for (index, file) in files.iter().enumerate() {
        let relative = file.strip_prefix(input).unwrap_or(file);
        let target = match out_dir {
            Some(dir) => dir.join(relative).with_extension("pdf"),
            None => file.with_extension("pdf"),
        };

        let item = if target == *file {
            BatchItem {
                input: file.clone(),
                output: target,
                pages: 0,
                error: Some("输出会覆盖源文件".into()),
            }
        } else {
            match std::fs::read_to_string(file) {
                Ok(markdown) => {
                    match export_markdown(
                        &markdown,
                        Some(file),
                        &target,
                        opts,
                        font_paths,
                        embedded_fonts,
                    ) {
                        Ok(rep) => BatchItem {
                            input: file.clone(),
                            output: target,
                            pages: rep.pages,
                            error: None,
                        },
                        Err(e) => BatchItem {
                            input: file.clone(),
                            output: target,
                            pages: 0,
                            error: Some(e.to_string()),
                        },
                    }
                }
                Err(e) => BatchItem {
                    input: file.clone(),
                    output: target,
                    pages: 0,
                    error: Some(format!("读取失败：{e}")),
                },
            }
        };

        if item.error.is_none() {
            report.succeeded += 1;
        } else {
            report.failed += 1;
        }
        let _ = (index, total);
        on_progress(&item);
        report.items.push(item);
    }

    report
}

/// Export cards as a cardbook: one card per page.
pub fn export_cardbook(
    cards: &[Card],
    vault_root: &Path,
    output: &Path,
    opts: &CardbookOptions,
    font_paths: &[PathBuf],
    embedded_fonts: &[(&'static str, Vec<u8>)],
) -> Result<ExportReport, ExportError> {
    let font_set = fonts::load_embedded(font_paths, embedded_fonts);
    let gen = typst_gen::generate_cardbook(cards, vault_root, opts, &font_set);
    let mut warnings = font_set.warnings.clone();
    warnings.extend(gen.warnings.clone());

    let (pages, bytes) = compile(&gen.source, font_set, gen.assets, output)?;
    Ok(ExportReport {
        output: output.to_path_buf(),
        pages,
        bytes,
        warnings,
    })
}

/// Render the generated Typst source without compiling — used by tests and
/// by the "copy Typst source" debugging action.
pub fn generate_source(
    markdown: &str,
    doc_path: Option<&Path>,
    opts: &ExportOptions,
    font_paths: &[PathBuf],
    embedded_fonts: &[(&'static str, Vec<u8>)],
) -> String {
    let title_hint = doc_path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string());
    let doc = Document::parse(markdown, title_hint.as_deref());
    let base_dir = doc_path
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let font_set = fonts::load_embedded(font_paths, embedded_fonts);
    typst_gen::generate(&doc, opts, &font_set, &base_dir).source
}
