//! `aur-export` — the only export pipeline.
//!
//! Markdown AST → Typst markup → Typst compiler → PDF.  No platform printing
//! APIs, no second engine: every platform produces the same bytes.

pub mod fonts;
pub mod math;
pub mod options;
pub mod pdf;
pub mod raster;
pub mod svg;
pub mod typst_gen;
pub mod world;

pub use fonts::{load as load_fonts, load_embedded, FontSet};
pub use options::{CardPage, CardbookOptions, ExportOptions, Margin, Paper, Theme};
pub use pdf::{
    collect_markdown, export_cardbook, export_folder, export_markdown, generate_source, BatchItem,
    BatchReport, ExportError, ExportReport,
};
pub use raster::{render_pages, PageImage, RenderError};
pub use svg::{render_math, MathSvg};
