//! The Typst `World`: source, fonts and image assets for one compilation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt};

use crate::fonts::FontSet;

pub struct TypstWorld {
    lib: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: FileId,
    source: Source,
    /// Virtual path (without leading slash) → real file on disk.
    assets: HashMap<String, PathBuf>,
    cache: Mutex<HashMap<FileId, Bytes>>,
}

impl TypstWorld {
    pub fn new(source: String, fonts: Arc<FontSet>, assets: HashMap<String, PathBuf>) -> Self {
        let vpath = VirtualPath::new("/main.typ").expect("valid virtual path");
        let main = FileId::new(RootedPath::new(VirtualRoot::Project, vpath));
        let source = Source::new(main, source);
        TypstWorld {
            lib: LazyHash::new(Library::default()),
            book: LazyHash::new(fonts.book.clone()),
            fonts: fonts.fonts.clone(),
            main,
            source,
            assets,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn source_text(&self) -> &str {
        self.source.text()
    }
}

impl typst::World for TypstWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.lib
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(
                id.vpath().get_with_slash().to_string().into(),
            ))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if let Ok(cache) = self.cache.lock() {
            if let Some(bytes) = cache.get(&id) {
                return Ok(bytes.clone());
            }
        }

        let vpath = id.vpath().get_without_slash().to_string();
        let Some(real) = self.assets.get(&vpath) else {
            return Err(FileError::NotFound(vpath.into()));
        };
        let data = std::fs::read(real).map_err(|e| FileError::from_io(e, real))?;
        let bytes = Bytes::new(data);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(id, bytes.clone());
        }
        Ok(bytes)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        let now = chrono::Local::now();
        use chrono::Datelike;
        Datetime::from_ymd(now.year(), now.month() as u8, now.day() as u8)
    }
}
