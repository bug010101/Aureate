//! The Aureate interface — pure Rust with egui.  No HTML, no JavaScript.
//!
//! Layout: left sidebar (files / cards) · central editor + Typst preview ·
//! status bar.  The preview compiles with the exact Typst engine that writes
//! the PDF and rasterises the same pages, so preview and PDF cannot disagree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aur_index::{CardRow, SearchHit};
use aur_vault::{Change, Entry, Storage};
use eframe::egui;
use egui::{Color32, FontData, FontDefinitions, TextureHandle, Widget};

use crate::ops::{self, CardMetaInput, DocInfo};
use crate::prefs::{self, Prefs};
use crate::preview::{spawn_worker, PreviewJob};
use crate::state::{AppState, VaultInfo};

#[derive(Debug, Clone, Copy, PartialEq)]
enum SideTab {
    Files,
    Cards,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum WriteMode {
    Edit,
    Split,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CardsView {
    Form,
    Timeline,
    Board,
    Ledger,
}

enum ExportDone {
    Pdf(aur_export::ExportReport),
    Batch(aur_export::BatchReport),
}

#[derive(Debug, Clone, Default)]
struct CardForm {
    active: bool,
    path: String,
    kind: String,
    title: String,
    date: String,
    project: String,
    status: String,
    tags: String,
    sections: Vec<(String, String)>,
}

impl CardForm {
    fn from_info(info: &ops::CardInfo) -> Self {
        CardForm {
            active: true,
            path: info.path.clone(),
            kind: info.kind.clone(),
            title: info.title.clone(),
            date: info.date.clone(),
            project: info.project.clone().unwrap_or_default(),
            status: info.status.clone(),
            tags: info.tags.join(", "),
            sections: info
                .sections
                .iter()
                .map(|s| (s.title.clone(), s.markdown.clone()))
                .collect(),
        }
    }
}

#[derive(Clone)]
struct PageTex {
    tex: TextureHandle,
    width_px: f32,
    height_px: f32,
}

pub struct AureateApp {
    state: Arc<AppState>,
    font_paths: Vec<PathBuf>,
    watch_tx: Sender<Change>,
    watch_rx: Receiver<Change>,
    watch_dirty: Option<Instant>,

    info: Option<VaultInfo>,
    entries: Vec<Entry>,
    collapsed: HashSet<String>,
    cards: Vec<CardRow>,

    side_tab: SideTab,
    write_mode: WriteMode,
    dark: bool,

    doc: Option<DocInfo>,
    editor: String,
    dirty: bool,
    last_edit: Instant,

    options: aur_export::ExportOptions,
    cardbook: aur_export::CardbookOptions,
    prefs_save_at: Option<Instant>,

    preview_tx: Sender<PreviewJob>,
    preview_rx: Receiver<crate::preview::PreviewResult>,
    preview_gen: u64,
    preview_stale: bool,
    last_schedule: Instant,
    pages: Vec<PageTex>,
    preview_error: Option<String>,
    preview_warning: Option<String>,
    zoom: f32,

    cards_view: CardsView,
    card_filter: String,
    card_form: CardForm,
    ledger: Option<ops::LedgerInfo>,

    search_open: bool,
    search_query: String,
    search_hits: Option<Vec<SearchHit>>,

    toast: Option<(String, Instant)>,
    settings_open: bool,
    rename_target: Option<String>,
    rename_buffer: String,
    new_card_kind: Option<usize>,

    export_rx: Option<Receiver<Result<ExportDone, String>>>,
}

impl AureateApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let prefs: Prefs = prefs::load();
        setup_fonts(&cc.egui_ctx);
        let dark = prefs.dark;
        cc.egui_ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });

        let (watch_tx, watch_rx) = mpsc::channel();
        let (preview_tx, preview_rx) = spawn_worker();
        let font_paths = crate::state::resolve_font_paths();

        AureateApp {
            state: Arc::new(AppState::default()),
            font_paths,
            watch_tx,
            watch_rx,
            watch_dirty: None,
            info: None,
            entries: Vec::new(),
            collapsed: HashSet::new(),
            cards: Vec::new(),
            side_tab: SideTab::Files,
            write_mode: WriteMode::Split,
            dark,
            doc: None,
            editor: String::new(),
            dirty: false,
            last_edit: Instant::now(),
            options: prefs.options.clone().unwrap_or_default(),
            cardbook: prefs.cardbook.clone().unwrap_or_default(),
            prefs_save_at: None,
            preview_tx,
            preview_rx,
            preview_gen: 0,
            preview_stale: true,
            last_schedule: Instant::now(),
            pages: Vec::new(),
            preview_error: None,
            preview_warning: None,
            zoom: if prefs.zoom > 0.0 { prefs.zoom } else { 1.0 },
            cards_view: CardsView::Form,
            card_filter: String::new(),
            card_form: CardForm::default(),
            ledger: None,
            search_open: false,
            search_query: String::new(),
            search_hits: None,
            toast: None,
            settings_open: false,
            rename_target: None,
            rename_buffer: String::new(),
            new_card_kind: None,
            export_rx: None,
        }
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn prefs_changed(&mut self) {
        self.prefs_save_at = Some(Instant::now());
    }

    fn open_vault(&mut self, path: &Path) {
        match self
            .state
            .open(&path.to_string_lossy(), Some(self.watch_tx.clone()))
        {
            Ok(info) => {
                self.info = Some(info);
                self.collapsed.clear();
                self.refresh();
                self.toast(format!("已打开 {}", path.display()));
            }
            Err(e) => self.toast(format!("打开失败：{e}")),
        }
    }

    fn refresh(&mut self) {
        let Ok(vault) = self.state.vault() else {
            return;
        };
        self.info = Some(vault.info());
        self.entries = ops::list_entries(&vault).unwrap_or_default();
        self.cards = ops::cards(&vault).unwrap_or_default();
        if self.cards_view == CardsView::Ledger {
            self.ledger = ops::ledger(&vault).ok();
        }
    }

    fn open_doc_path(&mut self, rel: &str) {
        if self.dirty {
            self.save_doc(false);
        }
        let Ok(vault) = self.state.vault() else {
            return;
        };
        match ops::read_doc(&vault, rel) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.dirty = false;
                self.preview_stale = true;
                self.last_schedule = Instant::now();
                self.preview_error = None;
                self.pages.clear();
                if let Some(card) = self.doc.as_ref().and_then(|d| d.card.clone()) {
                    self.card_form = CardForm::from_info(&card);
                    self.side_tab = SideTab::Cards;
                    self.cards_view = CardsView::Form;
                } else {
                    self.card_form = CardForm::default();
                }
            }
            Err(e) => self.toast(format!("读取失败：{e}")),
        }
    }

    fn open_card_id(&mut self, id: &str) {
        let Ok(vault) = self.state.vault() else {
            return;
        };
        match ops::card_path(&vault, id).and_then(|p| ops::read_doc(&vault, &p)) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.dirty = false;
                self.preview_stale = true;
                self.last_schedule = Instant::now();
                self.pages.clear();
                if let Some(card) = self.doc.as_ref().and_then(|d| d.card.clone()) {
                    self.card_form = CardForm::from_info(&card);
                } else {
                    self.card_form = CardForm::default();
                }
            }
            Err(e) => self.toast(format!("打开卡片失败：{e}")),
        }
    }

    fn save_doc(&mut self, quiet: bool) {
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let Ok(vault) = self.state.vault() else {
            return;
        };
        match ops::save_doc(&vault, &doc.path, &self.editor) {
            Ok(saved) => {
                self.doc = Some(saved);
                self.dirty = false;
                self.refresh();
                if !quiet {
                    self.toast("已保存");
                }
            }
            Err(e) => self.toast(format!("保存失败：{e}")),
        }
    }

    fn save_card_form(&mut self) {
        let path = self.card_form.path.clone();
        if path.is_empty() {
            return;
        }
        let Ok(vault) = self.state.vault() else {
            return;
        };
        let meta = CardMetaInput {
            title: self.card_form.title.clone(),
            kind: self.card_form.kind.clone(),
            date: self.card_form.date.clone(),
            project: Some(self.card_form.project.clone()).filter(|p| !p.trim().is_empty()),
            status: self.card_form.status.clone(),
            tags: self
                .card_form
                .tags
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
        };
        let sections = self.card_form.sections.clone();
        match ops::save_card(&vault, &path, &meta, &sections) {
            Ok(saved) => {
                self.doc = Some(saved);
                self.dirty = false;
                self.refresh();
                self.preview_stale = true;
                self.last_schedule = Instant::now();
                self.toast("卡片已保存");
            }
            Err(e) => self.toast(format!("保存卡片失败：{e}")),
        }
    }

    fn send_preview_job(&mut self) {
        self.preview_gen += 1;
        let doc_path = self.doc.as_ref().map(|d| {
            self.state
                .vault()
                .ok()
                .map(|v| v.root.join(&d.path))
                .unwrap_or_else(|| PathBuf::from(&d.path))
        });
        let job = PreviewJob {
            gen: self.preview_gen,
            markdown: self.editor.clone(),
            doc_path,
            options: self.options.clone(),
            font_paths: self.font_paths.clone(),
        };
        self.preview_stale = false;
        let _ = self.preview_tx.send(job);
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let mut fresh: Option<crate::preview::PreviewResult> = None;
        while let Ok(result) = self.preview_rx.try_recv() {
            fresh = Some(result);
        }
        if let Some(result) = fresh {
            if result.gen == self.preview_gen {
                if let Some(err) = result.error {
                    self.preview_error = Some(err);
                    self.pages.clear();
                } else {
                    self.preview_error = None;
                    self.preview_warning = result.warnings.first().cloned();
                    let mut texs = Vec::new();
                    for (i, page) in result.pages.iter().enumerate() {
                        let mut colors = Vec::with_capacity(page.rgba.len() / 4);
                        for px in page.rgba.as_chunks::<4>().0 {
                            colors
                                .push(Color32::from_rgba_unmultiplied(px[0], px[1], px[2], px[3]));
                        }
                        let image = egui::ColorImage::new(
                            [page.width as usize, page.height as usize],
                            colors,
                        );
                        let tex = ctx.load_texture(format!("page-{i}"), image, Default::default());
                        texs.push(PageTex {
                            tex,
                            width_px: page.width as f32,
                            height_px: page.height as f32,
                        });
                    }
                    self.pages = texs;
                }
            }
        }
    }

    fn export_pdf(&mut self) {
        if self.dirty {
            self.save_doc(true);
        }
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let Ok(vault) = self.state.vault() else {
            return;
        };
        let src = vault.root.join(&doc.path);
        let default_name = if doc.title.trim().is_empty() {
            "document.pdf".to_string()
        } else {
            format!("{}.pdf", doc.title)
        };
        let Some(out) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("PDF", &["pdf"])
            .save_file()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let fonts = self.font_paths.clone();
        let options = self.options.clone();
        let _ = std::thread::Builder::new()
            .name("aureate-export".into())
            .spawn(move || {
                let result = aur_export::export_markdown(
                    &doc.content,
                    Some(&src),
                    &out.clone(),
                    &options,
                    &fonts,
                    &[],
                )
                .map_err(|e| e.to_string());
                let _ = tx.send(result.map(ExportDone::Pdf));
            });
        self.export_rx = Some(rx);
        self.toast("导出已开始");
    }

    fn export_cardbook(&mut self) {
        let Ok(vault) = self.state.vault() else {
            return;
        };
        let mut cards = Vec::new();
        for row in self.cards.clone() {
            if let Ok(raw) = vault.storage.read(&row.path) {
                let doc = aur_core::Document::parse(&raw, Some(&row.title));
                if let Some(card) = aur_core::Card::from_document(&doc, &row.path) {
                    cards.push(card);
                }
            }
        }
        if cards.is_empty() {
            self.toast("没有可导出的卡片");
            return;
        }
        let Some(out) = rfd::FileDialog::new()
            .set_file_name("cardbook.pdf")
            .add_filter("PDF", &["pdf"])
            .save_file()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let fonts = self.font_paths.clone();
        let options = self.cardbook.clone();
        let root = vault.root.clone();
        let _ = std::thread::Builder::new()
            .name("aureate-cardbook".into())
            .spawn(move || {
                let result =
                    aur_export::export_cardbook(&cards, &root, &out.clone(), &options, &fonts, &[])
                        .map_err(|e| e.to_string());
                let _ = tx.send(result.map(ExportDone::Pdf));
            });
        self.export_rx = Some(rx);
        self.toast("卡册导出已开始");
    }

    fn export_folder_ui(&mut self) {
        let Ok(vault) = self.state.vault() else {
            return;
        };
        let Some(dir) = rfd::FileDialog::new()
            .set_directory(&vault.root)
            .pick_folder()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let fonts = self.font_paths.clone();
        let options = self.options.clone();
        let _ = std::thread::Builder::new()
            .name("aureate-batch".into())
            .spawn(move || {
                let report =
                    aur_export::export_folder(&dir.clone(), None, &options, &fonts, &[], |_| {});
                let _ = tx.send(Ok(ExportDone::Batch(report)));
            });
        self.export_rx = Some(rx);
        self.toast("批量导出已开始");
    }
}

impl eframe::App for AureateApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.poll_background(&ctx);

        self.top_bar(ui);
        self.sidebar(ui);
        self.status_bar(ui);

        if self.info.is_none() {
            self.onboard(ui);
        } else if self.side_tab == SideTab::Cards {
            self.cards_body(ui);
        } else {
            self.write_body(ui);
        }

        self.settings_window(&ctx);
        self.dialog_windows(&ctx);
        self.expire_toast();
    }
}

impl AureateApp {
    fn expire_toast(&mut self) {
        if let Some((_, at)) = self.toast {
            if at.elapsed() > Duration::from_secs(5) {
                self.toast = None;
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (mut save, mut export, mut find, mut newdoc) = (false, false, false, false);
        ctx.input(|i| {
            let cmd = i.modifiers.command;
            save = cmd && i.key_pressed(egui::Key::S);
            export = cmd && i.key_pressed(egui::Key::E);
            find = cmd && i.key_pressed(egui::Key::F);
            newdoc = cmd && i.key_pressed(egui::Key::N);
        });
        if save && self.doc.is_some() {
            self.save_doc(false);
        }
        if export && self.doc.is_some() {
            self.export_pdf();
        }
        if find && self.info.is_some() {
            self.search_open = true;
        }
        if newdoc && self.info.is_some() {
            if let Ok(vault) = self.state.vault() {
                if let Ok(doc) = ops::create_doc(&vault, None, None) {
                    self.refresh();
                    self.open_doc_path(&doc.path);
                }
            }
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        self.poll_preview(ctx);

        if let Some(rx) = &self.export_rx {
            if let Ok(result) = rx.try_recv() {
                self.export_rx = None;
                match result {
                    Ok(ExportDone::Pdf(report)) => self.toast(format!(
                        "导出成功：{}（{} 页）",
                        report.output.display(),
                        report.pages
                    )),
                    Ok(ExportDone::Batch(report)) => self.toast(format!(
                        "批量导出完成：成功 {}，失败 {}",
                        report.succeeded, report.failed
                    )),
                    Err(e) => self.toast(format!("导出失败：{e}")),
                }
            }
        }

        // Preview debouncing: send a fresh job 600 ms after the last change.
        if self.preview_stale && self.last_schedule.elapsed() > Duration::from_millis(600) {
            self.send_preview_job();
        }

        if let Some(at) = self.prefs_save_at {
            if at.elapsed() > Duration::from_secs(1) {
                self.prefs_save_at = None;
                prefs::save(&Prefs {
                    options: Some(self.options.clone()),
                    cardbook: Some(self.cardbook.clone()),
                    dark: self.dark,
                    zoom: self.zoom,
                });
            }
        }

        // Watcher events: debounce, then resync.
        let mut changed = false;
        while self.watch_rx.try_recv().is_ok() {
            changed = true;
        }
        if changed {
            self.watch_dirty = Some(Instant::now());
        }
        if let Some(at) = self.watch_dirty {
            if at.elapsed() > Duration::from_millis(500) {
                self.watch_dirty = None;
                self.refresh();
            }
        }

        // Keep repainting while a preview job is in flight.
        if self.preview_stale || self.export_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("topbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if egui::Button::new("写作")
                    .selected(self.side_tab == SideTab::Files)
                    .ui(ui)
                    .clicked()
                {
                    self.side_tab = SideTab::Files;
                }
                if egui::Button::new("卡片")
                    .selected(self.side_tab == SideTab::Cards)
                    .ui(ui)
                    .clicked()
                {
                    self.side_tab = SideTab::Cards;
                }
                ui.separator();
                if egui::Button::new("编辑")
                    .selected(self.write_mode == WriteMode::Edit)
                    .small()
                    .ui(ui)
                    .clicked()
                {
                    self.write_mode = WriteMode::Edit;
                }
                if egui::Button::new("分屏")
                    .selected(self.write_mode == WriteMode::Split)
                    .small()
                    .ui(ui)
                    .clicked()
                {
                    self.write_mode = WriteMode::Split;
                }
                if egui::Button::new("预览")
                    .selected(self.write_mode == WriteMode::Preview)
                    .small()
                    .ui(ui)
                    .clicked()
                {
                    self.write_mode = WriteMode::Preview;
                }
                ui.separator();
                let title = self
                    .doc
                    .as_ref()
                    .map(|d| d.title.clone())
                    .unwrap_or_else(|| "Aureate".into());
                ui.strong(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("设置").clicked() {
                        self.settings_open = !self.settings_open;
                    }
                    if ui.button("批量导出").clicked() && self.info.is_some() {
                        self.export_folder_ui();
                    }
                    if ui.button("导出 PDF").clicked() && self.doc.is_some() {
                        self.export_pdf();
                    }
                    if ui.button("保存").clicked() && self.doc.is_some() {
                        self.save_doc(false);
                    }
                    let label = if self.dark {
                        "☀ 浅色"
                    } else {
                        "🌙 深色"
                    };
                    if ui.button(label).clicked() {
                        self.dark = !self.dark;
                        ui.ctx().set_visuals(if self.dark {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        });
                        self.prefs_changed();
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let words = aur_core::word_count(&self.editor);
                let state = if self.dirty { "未保存" } else { "已保存" };
                ui.label(format!("{words} 字 · {state}"));
                if let Some(info) = &self.info {
                    ui.separator();
                    ui.label(format!(
                        "{} · {} 文档 · {} 卡片 · 索引{}",
                        info.name,
                        info.docs,
                        info.cards,
                        if info.index_mode == "file" {
                            "正常"
                        } else {
                            "内存"
                        }
                    ));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((msg, _)) = &self.toast {
                        ui.label(egui::RichText::new(msg).weak());
                    }
                });
            });
        });
    }

    fn onboard(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.25);
                ui.heading("Aureate");
                ui.label("只导出 PDF 的本地 Markdown 写作器 · 纯 Rust");
                ui.add_space(24.0);
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new("选择文件夹").size(16.0),
                    ))
                    .clicked()
                {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.open_vault(&dir);
                    }
                }
                ui.add_space(8.0);
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new("打开单个文件").size(16.0),
                    ))
                    .clicked()
                {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("Markdown", &["md", "markdown"])
                        .pick_file()
                    {
                        match ops::open_file(
                            &self.state,
                            &file.to_string_lossy(),
                            Some(self.watch_tx.clone()),
                        ) {
                            Ok(doc) => {
                                self.refresh();
                                self.open_doc_path(&doc.path);
                                self.toast(format!("已打开 {}", file.display()));
                            }
                            Err(e) => self.toast(format!("打开失败：{e}")),
                        }
                    }
                }
                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new(
                        "每次启动由你手动选择；运行数据全部留在应用内部，不写用户目录",
                    )
                    .weak()
                    .small(),
                );
            });
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        if self.info.is_none() {
            return;
        }
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.small_button("＋ 文档").clicked() {
                        if let Ok(vault) = self.state.vault() {
                            if let Ok(doc) = ops::create_doc(&vault, None, None) {
                                self.refresh();
                                self.open_doc_path(&doc.path);
                            }
                        }
                    }
                    if ui.small_button("打开文件").clicked() {
                        if let Some(file) = rfd::FileDialog::new()
                            .add_filter("Markdown", &["md", "markdown"])
                            .pick_file()
                        {
                            match ops::open_file(
                                &self.state,
                                &file.to_string_lossy(),
                                Some(self.watch_tx.clone()),
                            ) {
                                Ok(doc) => {
                                    self.refresh();
                                    self.open_doc_path(&doc.path);
                                }
                                Err(e) => self.toast(format!("打开失败：{e}")),
                            }
                        }
                    }
                    if ui.small_button("搜索").clicked() {
                        self.search_open = true;
                    }
                });
                ui.separator();

                if self.side_tab == SideTab::Files {
                    self.file_tree(ui);
                } else {
                    self.cards_sidebar(ui);
                }
            });
    }

    fn file_tree(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let entries = self.entries.clone();
                let collapsed = self.collapsed.clone();
                // Ancestors of the current entry, in traversal order.
                let mut ancestors: Vec<String> = Vec::new();
                for entry in entries {
                    ancestors.truncate(entry.depth);
                    if ancestors.iter().any(|a| collapsed.contains(a)) {
                        if entry.is_dir {
                            ancestors.push(entry.path.clone());
                        }
                        continue;
                    }
                    let indent = entry.depth as f32 * 14.0;
                    let selected = self
                        .doc
                        .as_ref()
                        .map(|d| d.path == entry.path)
                        .unwrap_or(false);
                    let (icon, label) = if entry.is_dir {
                        let open = !collapsed.contains(&entry.path);
                        (if open { "▾" } else { "▸" }, entry.name.clone())
                    } else {
                        ("📄", entry.name.clone())
                    };
                    let resp = ui
                        .indent(egui::IdSalt::new(("tree", &entry.path)), |ui| {
                            ui.add_space(indent);
                            ui.button(format!("{icon} {label}"))
                        })
                        .inner;
                    if selected {
                        ui.painter().rect_stroke(
                            resp.rect.expand2(egui::vec2(4.0, 1.0)),
                            3.0,
                            egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if entry.is_dir {
                        if resp.clicked() {
                            if collapsed.contains(&entry.path) {
                                self.collapsed.remove(&entry.path);
                            } else {
                                self.collapsed.insert(entry.path.clone());
                            }
                        }
                        resp.context_menu(|ui| {
                            if ui.button("新建文档").clicked() {
                                if let Ok(vault) = self.state.vault() {
                                    if let Ok(doc) =
                                        ops::create_doc(&vault, Some(&entry.path), None)
                                    {
                                        self.refresh();
                                        self.open_doc_path(&doc.path);
                                    }
                                }
                                ui.close();
                            }
                            if ui.button("删除文件夹").clicked() {
                                self.delete_entry(&entry.path.clone());
                                ui.close();
                            }
                        });
                        ancestors.push(entry.path.clone());
                    } else {
                        if resp.clicked() {
                            self.open_doc_path(&entry.path.clone());
                        }
                        resp.context_menu(|ui| {
                            if ui.button("重命名").clicked() {
                                self.rename_target = Some(entry.path.clone());
                                self.rename_buffer = entry.name.clone();
                                ui.close();
                            }
                            if ui.button("删除").clicked() {
                                self.delete_entry(&entry.path.clone());
                                ui.close();
                            }
                        });
                    }
                }
            });
    }

    fn delete_entry(&mut self, path: &str) {
        let Ok(vault) = self.state.vault() else {
            return;
        };
        match ops::delete_entry(&vault, path) {
            Ok(()) => {
                if self.doc.as_ref().map(|d| d.path == path).unwrap_or(false) {
                    self.doc = None;
                    self.editor.clear();
                    self.dirty = false;
                    self.pages.clear();
                    self.preview_stale = true;
                }
                self.refresh();
                self.toast("已移入应用内回收站");
            }
            Err(e) => self.toast(format!("删除失败：{e}")),
        }
    }

    fn cards_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.small_button("＋ 卡片").clicked() {
                self.new_card_kind = Some(0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let current = self.card_filter.clone();
                egui::ComboBox::from_id_salt("card-filter")
                    .selected_text(if current.is_empty() {
                        "全部类型"
                    } else {
                        current.as_str()
                    })
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.card_filter, String::new(), "全部类型");
                        for k in ["实验卡", "论文卡", "想法卡", "会议卡", "周卡"] {
                            ui.selectable_value(&mut self.card_filter, k.to_string(), k);
                        }
                    });
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let cards: Vec<CardRow> = self
                    .cards
                    .iter()
                    .filter(|c| self.card_filter.is_empty() || c.kind == self.card_filter)
                    .cloned()
                    .collect();
                for card in cards {
                    if ui
                        .button(format!("{} · {}", card.date, card.title))
                        .clicked()
                    {
                        self.open_card_id(&card.id);
                    }
                }
            });
    }

    fn write_body(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.doc.clone() else {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.35);
                    ui.label(egui::RichText::new("从左侧选择一个文档，或新建一个").weak());
                });
            });
            return;
        };

        match self.write_mode {
            WriteMode::Edit => {
                egui::CentralPanel::default().show(ui, |ui| self.editor_pane(ui));
            }
            WriteMode::Preview => {
                egui::CentralPanel::default().show(ui, |ui| self.preview_pane(ui));
            }
            WriteMode::Split => {
                egui::Panel::right("preview")
                    .resizable(true)
                    .default_size(460.0)
                    .show(ui, |ui| self.preview_pane(ui));
                egui::CentralPanel::default().show(ui, |ui| self.editor_pane(ui));
            }
        }
        let _ = doc;
    }

    fn editor_pane(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let before = self.editor.clone();
                egui::TextEdit::multiline(&mut self.editor)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width())
                    .code_editor()
                    .show(ui);
                if self.editor != before {
                    self.dirty = true;
                    self.last_edit = Instant::now();
                    self.preview_stale = true;
                    self.last_schedule = Instant::now();
                }
            });
    }

    fn preview_pane(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("预览 · 与 PDF 一致").weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("－").clicked() {
                    self.zoom = (self.zoom - 0.1).max(0.3);
                    self.prefs_changed();
                }
                ui.label(format!("{:.0}%", self.zoom * 100.0));
                if ui.small_button("＋").clicked() {
                    self.zoom = (self.zoom + 0.1).min(2.0);
                    self.prefs_changed();
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(err) = &self.preview_error {
                    ui.colored_label(Color32::from_rgb(200, 60, 60), err);
                } else if self.pages.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("正在编译预览……").weak());
                    });
                } else {
                    let available = ui.available_width();
                    for page in &self.pages {
                        ui.vertical_centered(|ui| {
                            ui.add_space(6.0);
                            let width_pt = page.width_px / 2.0;
                            let display_w = (width_pt * self.zoom).min(available);
                            let display_h = display_w * (page.height_px / page.width_px);
                            ui.add(
                                egui::Image::new(&page.tex)
                                    .fit_to_exact_size(egui::vec2(display_w, display_h)),
                            );
                            ui.add_space(6.0);
                        });
                    }
                    if let Some(w) = &self.preview_warning {
                        ui.label(egui::RichText::new(w).weak().small());
                    }
                }
            });
    }

    fn cards_body(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("cards-toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (view, label) in [
                    (CardsView::Form, "表单"),
                    (CardsView::Timeline, "时间线"),
                    (CardsView::Board, "看板"),
                    (CardsView::Ledger, "周历"),
                ] {
                    if egui::Button::new(label)
                        .selected(self.cards_view == view)
                        .ui(ui)
                        .clicked()
                    {
                        if view == CardsView::Ledger {
                            self.ledger =
                                self.state.vault().ok().and_then(|v| ops::ledger(&v).ok());
                        }
                        self.cards_view = view;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("导出卡册").clicked() {
                        self.export_cardbook();
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| match self.cards_view {
            CardsView::Form => self.card_form_view(ui),
            CardsView::Timeline => self.card_timeline(ui),
            CardsView::Board => self.card_board(ui),
            CardsView::Ledger => self.card_ledger(ui),
        });
    }

    fn card_form_view(&mut self, ui: &mut egui::Ui) {
        if !self.card_form.active {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.label(
                    egui::RichText::new("当前文档不是卡片。从左侧卡片列表打开一张，或新建一张。")
                        .weak(),
                );
            });
            return;
        }
        ui.add_space(4.0);
        egui::Grid::new("card-meta")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("标题");
                ui.add(egui::TextEdit::singleline(&mut self.card_form.title).desired_width(320.0));
                ui.end_row();

                ui.label("类型");
                let kind = self.card_form.kind.clone();
                egui::ComboBox::from_id_salt("card-kind")
                    .selected_text(kind)
                    .show_ui(ui, |ui| {
                        for k in ["实验卡", "论文卡", "想法卡", "会议卡", "周卡"] {
                            ui.selectable_value(&mut self.card_form.kind, k.to_string(), k);
                        }
                    });
                ui.end_row();

                ui.label("日期");
                ui.add(egui::TextEdit::singleline(&mut self.card_form.date).desired_width(140.0));
                ui.end_row();

                ui.label("项目");
                ui.add(
                    egui::TextEdit::singleline(&mut self.card_form.project).desired_width(220.0),
                );
                ui.end_row();

                ui.label("状态");
                let status = self.card_form.status.clone();
                egui::ComboBox::from_id_salt("card-status")
                    .selected_text(status_label(&status))
                    .show_ui(ui, |ui| {
                        for (s, label) in [
                            ("todo", "待办"),
                            ("doing", "进行中"),
                            ("done", "已完成"),
                            ("blocked", "受阻"),
                        ] {
                            ui.selectable_value(&mut self.card_form.status, s.to_string(), label);
                        }
                    });
                ui.end_row();

                ui.label("标签");
                ui.add(egui::TextEdit::singleline(&mut self.card_form.tags).desired_width(320.0));
                ui.end_row();
            });

        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let sections = self.card_form.sections.clone();
                for (i, (title, _)) in sections.iter().enumerate() {
                    ui.label(egui::RichText::new(format!("## {title}")).strong());
                    let mut body = sections[i].1.clone();
                    let resp = egui::TextEdit::multiline(&mut body)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width())
                        .desired_rows(4)
                        .show(ui);
                    if resp.response.changed() {
                        self.card_form.sections[i].1 = body;
                    }
                    ui.add_space(6.0);
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("保存卡片").clicked() {
                self.save_card_form();
            }
            if ui.button("在编辑器中打开").clicked() {
                self.side_tab = SideTab::Files;
                self.open_doc_path(&self.card_form.path.clone());
            }
        });
    }

    fn card_timeline(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut cards = self.cards.clone();
                cards.sort_by(|a, b| b.date.cmp(&a.date));
                for card in cards {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{}  ", card.date));
                        ui.label(egui::RichText::new(card.kind.clone()).weak());
                        if ui.link(&card.title).clicked() {
                            self.open_card_id(&card.id);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut status = card.status.clone();
                            egui::ComboBox::from_id_salt(format!("tl-{}", card.id))
                                .selected_text(status_label(&status))
                                .width(70.0)
                                .show_ui(ui, |ui| {
                                    for (s, label) in [
                                        ("todo", "待办"),
                                        ("doing", "进行中"),
                                        ("done", "已完成"),
                                        ("blocked", "受阻"),
                                    ] {
                                        ui.selectable_value(&mut status, s.to_string(), label);
                                    }
                                });
                            if status != card.status {
                                self.set_card_status(&card.id, &status);
                            }
                        });
                    });
                    ui.separator();
                }
            });
    }

    fn set_card_status(&mut self, id: &str, status: &str) {
        if let Ok(vault) = self.state.vault() {
            if ops::update_card_status(&vault, id, status).is_ok() {
                self.refresh();
            }
        }
    }

    fn card_board(&mut self, ui: &mut egui::Ui) {
        let columns = [
            ("todo", "待办"),
            ("doing", "进行中"),
            ("done", "已完成"),
            ("blocked", "受阻"),
        ];
        let width = ui.available_width() / columns.len() as f32;
        ui.horizontal(|ui| {
            for (status, label) in columns {
                ui.allocate_ui(egui::vec2(width, 200.0), |ui| {
                    ui.heading(label);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let cards: Vec<CardRow> = self
                                .cards
                                .iter()
                                .filter(|c| c.status == status)
                                .cloned()
                                .collect();
                            for card in cards {
                                let resp = ui.button(card.title.as_str());
                                if resp.clicked() {
                                    self.open_card_id(&card.id);
                                }
                                resp.context_menu(|ui| {
                                    for (s, l) in columns {
                                        if ui.button(format!("移到 {l}")).clicked() {
                                            self.set_card_status(&card.id, s);
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        });
                });
            }
        });
    }

    fn card_ledger(&mut self, ui: &mut egui::Ui) {
        let Some(ledger) = self.ledger.clone() else {
            ui.label(egui::RichText::new("加载中……").weak());
            return;
        };
        let d = &ledger.data;
        ui.add_space(4.0);
        ui.heading(format!("本周 {} ~ {}", ledger.week_start, ledger.week_end));
        ui.add_space(4.0);
        ui.columns(4, |cols| {
            let stats = [
                (d.total.to_string(), "全部卡片"),
                (d.doing.to_string(), "进行中"),
                (d.done.to_string(), "已完成"),
                (d.active_days.to_string(), "活跃天数"),
            ];
            for (col, (num, label)) in cols.iter_mut().zip(stats) {
                col.vertical(|ui| {
                    ui.label(egui::RichText::new(num).size(24.0).strong());
                    ui.label(label);
                });
            }
        });
        ui.separator();
        ui.label("按类型");
        for (kind, n) in &d.by_kind {
            ui.horizontal(|ui| {
                ui.label(kind.clone());
                ui.add(egui::ProgressBar::new(*n as f32 / d.total.max(1) as f32).show_percentage());
            });
        }
        ui.add_space(4.0);
        ui.label("按项目");
        for (project, n) in &d.by_project {
            ui.label(format!("{project}: {n}"));
        }
        ui.add_space(4.0);
        ui.label("本周卡片");
        let week_cards: Vec<CardRow> = self
            .cards
            .iter()
            .filter(|c| {
                c.date.as_str() >= ledger.week_start.as_str()
                    && c.date.as_str() <= ledger.week_end.as_str()
            })
            .cloned()
            .collect();
        for card in week_cards {
            if ui.link(format!("{} · {}", card.date, card.title)).clicked() {
                self.open_card_id(&card.id);
            }
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        if !open {
            return;
        }
        egui::Window::new("排版设置（预览与 PDF 一致）")
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                egui::Grid::new("settings")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("主题");
                        let theme = self.options.theme;
                        egui::ComboBox::from_id_salt("theme")
                            .selected_text(theme_label(theme))
                            .show_ui(ui, |ui| {
                                for (t, label) in [
                                    (aur_export::Theme::Github, "GitHub"),
                                    (aur_export::Theme::Academic, "学术论文"),
                                    (aur_export::Theme::Business, "商务"),
                                    (aur_export::Theme::Minimal, "极简"),
                                ] {
                                    ui.selectable_value(&mut self.options.theme, t, label);
                                }
                            });
                        ui.end_row();

                        ui.label("纸张");
                        let paper = self.options.paper;
                        egui::ComboBox::from_id_salt("paper")
                            .selected_text(paper_label(paper))
                            .show_ui(ui, |ui| {
                                for (p, label) in [
                                    (aur_export::Paper::A4, "A4"),
                                    (aur_export::Paper::A5, "A5"),
                                    (aur_export::Paper::Letter, "Letter"),
                                    (aur_export::Paper::Legal, "Legal"),
                                ] {
                                    ui.selectable_value(&mut self.options.paper, p, label);
                                }
                            });
                        ui.end_row();

                        ui.label("页边距");
                        let margin = self.options.margin;
                        egui::ComboBox::from_id_salt("margin")
                            .selected_text(margin_label(margin))
                            .show_ui(ui, |ui| {
                                for (m, label) in [
                                    (aur_export::Margin::Narrow, "窄"),
                                    (aur_export::Margin::Normal, "标准"),
                                    (aur_export::Margin::Wide, "宽"),
                                ] {
                                    ui.selectable_value(&mut self.options.margin, m, label);
                                }
                            });
                        ui.end_row();

                        ui.label("字号");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.options.font_size)
                                    .speed(0.5)
                                    .range(8.0..=16.0)
                                    .suffix(" pt"),
                            )
                            .changed()
                        {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("行距");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.options.line_height)
                                    .speed(0.02)
                                    .range(0.5..=1.2),
                            )
                            .changed()
                        {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("横排纸张");
                        if ui.checkbox(&mut self.options.landscape, "").changed() {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("页码");
                        if ui
                            .checkbox(&mut self.options.footer_page_numbers, "")
                            .changed()
                        {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("目录");
                        if ui.checkbox(&mut self.options.toc, "").changed() {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("H1 分页");
                        if ui.checkbox(&mut self.options.h1_page_break, "").changed() {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("首行缩进");
                        if ui
                            .checkbox(&mut self.options.first_line_indent, "")
                            .changed()
                        {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("文档标题");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.options.title)
                                    .desired_width(220.0),
                            )
                            .changed()
                        {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();

                        ui.label("页眉");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.options.header)
                                    .desired_width(220.0),
                            )
                            .changed()
                        {
                            self.prefs_changed();
                            self.preview_stale = true;
                            self.last_schedule = Instant::now();
                        }
                        ui.end_row();
                    });
                ui.separator();
                ui.label(
                    egui::RichText::new(
                        "页眉支持 {title} 与 {date} 占位符；设置改动即时刷新预览。",
                    )
                    .weak()
                    .small(),
                );
            });
        if !open && self.settings_open {
            self.settings_open = false;
            self.prefs_changed();
        }
        self.settings_open = open;
    }

    fn dialog_windows(&mut self, ctx: &egui::Context) {
        // Rename
        let mut rename_open = self.rename_target.is_some();
        if rename_open {
            let target = self.rename_target.clone().unwrap_or_default();
            let mut close = false;
            egui::Window::new("重命名")
                .open(&mut rename_open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.rename_buffer)
                            .desired_width(280.0)
                            .lock_focus(true),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("确定").clicked() {
                            if let Ok(vault) = self.state.vault() {
                                let name = self.rename_buffer.trim().trim_end_matches(".md");
                                if !name.is_empty() {
                                    let parent = Path::new(&target)
                                        .parent()
                                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                                        .unwrap_or_default();
                                    let to = if parent.is_empty() {
                                        format!("{name}.md")
                                    } else {
                                        format!("{parent}/{name}.md")
                                    };
                                    match ops::rename_entry(&vault, &target, &to) {
                                        Ok(_) => {
                                            self.refresh();
                                            if self
                                                .doc
                                                .as_ref()
                                                .map(|d| d.path == target)
                                                .unwrap_or(false)
                                            {
                                                self.open_doc_path(&to);
                                            }
                                        }
                                        Err(e) => self.toast(format!("重命名失败：{e}")),
                                    }
                                }
                            }
                            close = true;
                        }
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                rename_open = false;
            }
        }
        self.rename_target = if rename_open {
            self.rename_target.clone()
        } else {
            None
        };

        // New card
        if let Some(kind_idx) = self.new_card_kind {
            let kinds = ["实验卡", "论文卡", "想法卡", "会议卡", "周卡"];
            let mut open = true;
            egui::Window::new("新建卡片")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (i, k) in kinds.iter().enumerate() {
                            if egui::Button::new(*k)
                                .selected(self.new_card_kind == Some(i))
                                .ui(ui)
                                .clicked()
                            {
                                self.new_card_kind = Some(i);
                            }
                        }
                    });
                    if ui.button("创建").clicked() {
                        if let Ok(vault) = self.state.vault() {
                            match ops::create_card(&vault, kinds[kind_idx], None, None, None) {
                                Ok(doc) => {
                                    self.refresh();
                                    self.open_doc_path(&doc.path);
                                    self.side_tab = SideTab::Cards;
                                    self.cards_view = CardsView::Form;
                                    self.new_card_kind = None;
                                }
                                Err(e) => self.toast(format!("创建失败：{e}")),
                            }
                        }
                    }
                });
            if !open {
                self.new_card_kind = None;
            }
        }

        // Search overlay
        let mut search_open = self.search_open;
        if search_open {
            let mut close = false;
            egui::Window::new("搜索")
                .open(&mut search_open)
                .collapsible(false)
                .default_width(520.0)
                .show(ctx, |ui| {
                    let submitted = egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("搜索标题与正文，Enter 确认")
                        .desired_width(440.0)
                        .lock_focus(true)
                        .ui(ui)
                        .lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if submitted && !self.search_query.trim().is_empty() {
                        if let Ok(vault) = self.state.vault() {
                            self.search_hits = ops::search(&vault, &self.search_query, 30).ok();
                        }
                    }
                    let hits = self.search_hits.clone();
                    if let Some(hits) = hits {
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                if hits.is_empty() {
                                    ui.label(egui::RichText::new("没有结果").weak());
                                }
                                for hit in hits {
                                    let path = hit.path.clone();
                                    let snippet = hit.snippet.clone();
                                    if ui.link(&hit.title).clicked() {
                                        self.open_doc_path(&path);
                                        close = true;
                                    }
                                    ui.label(
                                        egui::RichText::new(
                                            snippet.replace(['\u{1}', '\u{2}'], ""),
                                        )
                                        .weak()
                                        .small(),
                                    );
                                    ui.separator();
                                }
                            });
                    }
                });
            if close {
                search_open = false;
            }
        }
        self.search_open = search_open;
    }
}

fn status_label(status: &str) -> &'static str {
    match status {
        "doing" => "进行中",
        "done" => "已完成",
        "blocked" => "受阻",
        _ => "待办",
    }
}

fn theme_label(theme: aur_export::Theme) -> &'static str {
    match theme {
        aur_export::Theme::Github => "GitHub",
        aur_export::Theme::Academic => "学术论文",
        aur_export::Theme::Business => "商务",
        aur_export::Theme::Minimal => "极简",
    }
}

fn paper_label(paper: aur_export::Paper) -> &'static str {
    match paper {
        aur_export::Paper::A4 => "A4",
        aur_export::Paper::A5 => "A5",
        aur_export::Paper::A6 => "A6",
        aur_export::Paper::Letter => "Letter",
        aur_export::Paper::Legal => "Legal",
    }
}

fn margin_label(margin: aur_export::Margin) -> &'static str {
    match margin {
        aur_export::Margin::Narrow => "窄",
        aur_export::Margin::Normal => "标准",
        aur_export::Margin::Wide => "宽",
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let loaded = crate::fonts::find_font("NotoSansSC-Regular.otf")
        .and_then(|p| std::fs::read(p).ok())
        .map(|bytes| {
            defs.font_data
                .insert("noto-sc".into(), FontData::from_owned(bytes).into());
            true
        })
        .unwrap_or(false);
    if !loaded {
        return;
    }
    if let Some(family) = defs.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "noto-sc".into());
    }
    if let Some(family) = defs.families.get_mut(&egui::FontFamily::Monospace) {
        family.push("noto-sc".into());
    }
    ctx.set_fonts(defs);
}
