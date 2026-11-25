//! Rasterised preview pages: the same Typst compilation the PDF uses,
//! rendered to RGBA pixels.  What you see here is what the PDF will be.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::fonts::{self, FontSet};
use crate::options::ExportOptions;
use crate::typst_gen;
use crate::world::TypstWorld;
use aur_core::Document;
use typst_layout::PagedDocument;

/// One rendered page, straight RGBA (unpremultiplied), 8 bits per channel.
#[derive(Debug, Clone)]
pub struct PageImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("Typst 编译失败：{0}")]
    Compile(String),
}

/// Compile the document and rasterise every page at `pixels_per_pt`.
pub fn render_pages(
    markdown: &str,
    doc_path: Option<&Path>,
    opts: &ExportOptions,
    font_paths: &[PathBuf],
    embedded_fonts: &[(&'static str, Vec<u8>)],
    pixels_per_pt: f32,
) -> Result<(Vec<PageImage>, Vec<String>), RenderError> {
    let title_hint = doc_path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string());
    let doc = Document::parse(markdown, title_hint.as_deref());
    let base_dir = doc_path
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let font_set = fonts::load_embedded(font_paths, embedded_fonts);

    let mut attempts = 0;
    let mut use_text_math = opts.math_as_text;
    loop {
        attempts += 1;
        let mut effective = opts.clone();
        effective.math_as_text = use_text_math;

        let gen = typst_gen::generate(&doc, &effective, &font_set, &base_dir);
        let warnings = font_set.warnings.clone();
        let warnings: Vec<String> = warnings
            .into_iter()
            .chain(gen.warnings.iter().cloned())
            .collect();

        match compile_pages(&gen.source, font_set.clone(), gen.assets, pixels_per_pt) {
            Ok(pages) => return Ok((pages, warnings)),
            Err(_) => {
                // Same rescue as the PDF path: one bad formula must not
                // cost the whole preview.
                if attempts == 1 && !use_text_math {
                    use_text_math = true;
                    continue;
                }
                return Err(RenderError::Compile(
                    "预览编译失败，已尝试按纯文本渲染公式".to_string(),
                ));
            }
        }
    }
}

fn compile_pages(
    source: &str,
    fonts: Arc<FontSet>,
    assets: typst_gen::Assets,
    pixels_per_pt: f32,
) -> Result<Vec<PageImage>, RenderError> {
    let world = TypstWorld::new(source.to_string(), fonts, assets.map);
    let warned = typst::compile::<PagedDocument>(&world);
    let document = warned
        .output
        .map_err(|_| RenderError::Compile("编译错误".into()))?;

    let mut pages = Vec::new();
    for page in document.pages() {
        let opts = typst_render::RenderOptions {
            pixel_per_pt: typst::utils::Scalar::new(f64::from(pixels_per_pt)),
            render_bleed: false,
        };
        let pixmap = typst_render::render(page, &opts);
        let width = pixmap.width();
        let height = pixmap.height();
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for p in pixmap.pixels() {
            let c = p.demultiply();
            rgba.push(c.red());
            rgba.push(c.green());
            rgba.push(c.blue());
            rgba.push(c.alpha());
        }
        pages.push(PageImage {
            width,
            height,
            rgba,
        });
    }
    Ok(pages)
}
