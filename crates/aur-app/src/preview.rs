//! Background preview compiler.  Sends a job per edit (debounced by the UI),
//! receives rendered page bitmaps.  The preview is the same Typst
//! compilation the PDF uses — what you see is literally the paper.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use aur_export::{render_pages, ExportOptions, PageImage};

const PIXELS_PER_PT: f32 = 2.0;

pub struct PreviewJob {
    pub gen: u64,
    pub markdown: String,
    pub doc_path: Option<PathBuf>,
    pub options: ExportOptions,
    pub font_paths: Vec<PathBuf>,
}

pub struct PreviewResult {
    pub gen: u64,
    pub pages: Vec<PageImage>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub fn spawn_worker() -> (Sender<PreviewJob>, Receiver<PreviewResult>) {
    let (job_tx, job_rx) = channel::<PreviewJob>();
    let (res_tx, res_rx) = channel::<PreviewResult>();

    let _ = std::thread::Builder::new()
        .name("aureate-preview".into())
        .spawn(move || {
            while let Ok(mut job) = job_rx.recv() {
                // Only the newest job matters — skip stale ones.
                while let Ok(newer) = job_rx.try_recv() {
                    job = newer;
                }
                let result = match render_pages(
                    &job.markdown,
                    job.doc_path.as_deref(),
                    &job.options,
                    &job.font_paths,
                    &[],
                    PIXELS_PER_PT,
                ) {
                    Ok((pages, warnings)) => PreviewResult {
                        gen: job.gen,
                        pages,
                        warnings,
                        error: None,
                    },
                    Err(e) => PreviewResult {
                        gen: job.gen,
                        pages: Vec::new(),
                        warnings: Vec::new(),
                        error: Some(e.to_string()),
                    },
                };
                let _ = res_tx.send(result);
            }
        });

    (job_tx, res_rx)
}
