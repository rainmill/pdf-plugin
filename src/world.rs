//! The world a document compiles in: the standard library, the bundled fonts plus
//! any the caller adds, one main source, and the files the caller hands over by
//! path. Nothing is read from disk or fetched from the network, so a template can
//! reach nothing the caller did not give it.

use std::collections::HashMap;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Dict, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

/// The path the main source is compiled from.
const MAIN: &str = "/main.typ";

/// A self-contained compilation: the source, its files and its fonts.
pub(crate) struct DocWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: Source,
    files: HashMap<FileId, Bytes>,
}

impl DocWorld {
    /// A world over `source`, with `files` reachable at their paths (`/logo.png`),
    /// `extra_fonts` beside the bundled ones, and `inputs` under `sys.inputs`.
    pub(crate) fn new(
        source: String,
        files: &[(String, Vec<u8>)],
        extra_fonts: &[Vec<u8>],
        inputs: Dict,
    ) -> Self {
        let mut fonts: Vec<Font> = typst_assets::fonts()
            .flat_map(|data| Font::iter(Bytes::new(data)))
            .collect();
        for data in extra_fonts {
            fonts.extend(Font::iter(Bytes::new(data.clone())));
        }
        let book = FontBook::from_fonts(&fonts);
        let main = Source::new(file_id(MAIN), source);
        let files = files
            .iter()
            .filter_map(|(path, bytes)| {
                let path = if path.starts_with('/') {
                    path.clone()
                } else {
                    format!("/{path}")
                };
                VirtualPath::new(&path)
                    .ok()
                    .map(|_| (file_id(&path), Bytes::new(bytes.clone())))
            })
            .collect();
        Self {
            library: LazyHash::new(Library::builder().with_inputs(inputs).build()),
            book: LazyHash::new(book),
            fonts,
            main,
            files,
        }
    }
}

/// The id of a file at `path` in the project root.
fn file_id(path: &str) -> FileId {
    let vpath = VirtualPath::new(path)
        .unwrap_or_else(|_| VirtualPath::new("/invalid").expect("a literal path is valid"));
    FileId::new(RootedPath::new(VirtualRoot::Project, vpath))
}

impl World for DocWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::NotSource)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main.id() {
            return Ok(Bytes::from_string(self.main.clone()));
        }
        self.files
            .get(&id)
            .cloned()
            .ok_or_else(|| FileError::NotFound(Default::default()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        let now = chrono::Utc::now();
        let now = match offset {
            Some(hours) => now + chrono::Duration::seconds(hours.seconds() as i64),
            None => now,
        };
        let date = now.date_naive();
        use chrono::Datelike;
        Datetime::from_ymd(date.year(), date.month() as u8, date.day() as u8)
    }
}
