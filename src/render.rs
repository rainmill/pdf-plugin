//! Rendering: Typst source (or markdown set in a layout) compiled in a sealed world
//! and written as a PDF, optionally to an archival standard.

use laterite_core::serde_json;
use typst::foundations::{Dict, IntoValue, Smart};
use typst_layout::PagedDocument;
use typst_pdf::{PdfOptions, PdfStandard, PdfStandards, Timestamp};

use crate::layout::{DocumentMeta, Layout};
use crate::markdown;
use crate::world::DocWorld;

/// What a render may use beyond its source.
#[derive(Debug, Clone, Default)]
pub struct Assets {
    /// Files the source may read, by path (`/logo.png`): images, data, a background.
    pub files: Vec<(String, Vec<u8>)>,
    /// Font files beside the bundled ones (TTF, OTF, TTC).
    pub fonts: Vec<Vec<u8>>,
    /// Values the source reads as `sys.inputs.<key>` (strings, numbers, nested JSON).
    pub inputs: serde_json::Map<String, serde_json::Value>,
}

/// How the PDF is written.
#[derive(Clone, Default)]
pub struct Output {
    /// PDF/A-2b: the archival standard, with every font embedded and no external
    /// dependencies. For records that must still open in decades (a published
    /// version of a policy).
    pub archival: bool,
    /// The password the PDF opens with (AES-256). PDF/A forbids encryption, so
    /// this and `archival` cannot be combined.
    pub password: Option<String>,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("archival", &self.archival)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

impl Output {
    /// PDF/A-2b output.
    pub fn archival() -> Self {
        Self {
            archival: true,
            ..Self::default()
        }
    }

    /// Output that opens only with `password`.
    pub fn protected(password: impl Into<String>) -> Self {
        Self {
            password: Some(password.into()),
            ..Self::default()
        }
    }
}

/// Why a render failed.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// The source did not compile; each entry is one message with its line.
    #[error("the document did not compile: {}", .0.join("; "))]
    Compile(Vec<String>),
    /// The compiled document could not be written as a PDF.
    #[error("the PDF could not be written: {0}")]
    Export(String),
    /// The PDF could not be password protected.
    #[error("the PDF could not be protected: {0}")]
    Protect(String),
}

/// The line every document starts with: the `qr` and `barcode` library.
pub(crate) const IMPORT: &str = "#import \"/_pdf/lib.typ\": *\n";

/// A PDF compiled from Typst `source` with `assets`, its metadata from `meta`
/// unless the source sets its own with `#set document(..)`. The source may call
/// `qr(value)` and `barcode(value, kind: "ean13")`.
pub fn render_typst(
    source: &str,
    meta: &DocumentMeta,
    assets: &Assets,
    output: &Output,
) -> Result<Vec<u8>, PdfError> {
    let prelude = format!("{IMPORT}{}\n", meta.document_rule());
    compile(&prelude, source, assets, output)
}

/// Compiles `prelude` then `source`; compile errors count lines from `source`.
pub(crate) fn compile(
    prelude: &str,
    source: &str,
    assets: &Assets,
    output: &Output,
) -> Result<Vec<u8>, PdfError> {
    if output.archival && output.password.is_some() {
        return Err(PdfError::Protect(
            "PDF/A forbids encryption: choose archival or a password".into(),
        ));
    }
    let offset = prelude.matches('\n').count();
    let full = format!("{prelude}{source}");
    let mut inputs = Dict::new();
    for (key, value) in &assets.inputs {
        inputs.insert(key.as_str().into(), json_to_value(value));
    }
    let world = DocWorld::new(full.clone(), &assets.files, &assets.fonts, inputs);
    let compiled = typst::compile::<PagedDocument>(&world);
    let document = compiled.output.map_err(|errors| {
        PdfError::Compile(
            errors
                .iter()
                .map(|e| {
                    // An error inside the library (a barcode that cannot hold
                    // its value) is reported at the template's call.
                    let line = world_line(&full, &world, e.span)
                        .or_else(|| {
                            e.trace
                                .iter()
                                .rev()
                                .find_map(|t| world_line(&full, &world, t.span))
                        })
                        .and_then(|l| l.checked_sub(offset))
                        .filter(|l| *l > 0);
                    match line {
                        Some(l) => format!("line {l}: {}", e.message),
                        None => e.message.to_string(),
                    }
                })
                .collect(),
        )
    })?;
    let standards = if output.archival {
        PdfStandards::new(&[PdfStandard::A_2b])
            .map_err(|e| PdfError::Export(e.message().to_string()))?
    } else {
        PdfStandards::default()
    };
    let options = PdfOptions {
        ident: Smart::Auto,
        // The render time: PDF/A requires a document date, and it is honest metadata.
        timestamp: now().map(Timestamp::new_utc),
        standards,
        ..PdfOptions::default()
    };
    let pdf = typst_pdf::pdf(&document, &options).map_err(|errors| {
        PdfError::Export(
            errors
                .iter()
                .map(|e| e.message.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    match &output.password {
        Some(password) => crate::protect::encrypt(&pdf, password).map_err(PdfError::Protect),
        None => Ok(pdf),
    }
}

/// A PDF of `markdown` set in `layout`, described by `meta`. Images in the
/// markdown are placed when `assets.files` holds a file at their URL's path, or
/// the path is a generated code ([`crate::codes::qr_path`]); otherwise their alt
/// text stands in. An image's title sizes it: `![Logo](/logo.png "width=30mm")`.
pub fn render_markdown(
    markdown_text: &str,
    layout: &Layout,
    meta: &DocumentMeta,
    assets: &Assets,
    output: &Output,
) -> Result<Vec<u8>, PdfError> {
    let source = markdown_source(markdown_text, layout, meta, assets);
    compile(IMPORT, &source, assets, output)
}

/// The Typst source `render_markdown` compiles: the layout's preamble, then the
/// converted markdown.
pub fn markdown_source(
    markdown_text: &str,
    layout: &Layout,
    meta: &DocumentMeta,
    assets: &Assets,
) -> String {
    let known: Vec<&str> = assets.files.iter().map(|(p, _)| p.as_str()).collect();
    let resolve = |url: &str| -> Option<String> {
        let path = if url.starts_with('/') {
            url.to_string()
        } else {
            format!("/{url}")
        };
        (path.starts_with(crate::codes::ROOT)
            || known
                .iter()
                .any(|k| *k == path || format!("/{}", k.trim_start_matches('/')) == path))
        .then_some(path)
    };
    let mut source = layout.preamble(meta);
    source.push('\n');
    source.push_str(&markdown::to_typst(markdown_text, &resolve));
    source
}

/// The current time in UTC as a Typst datetime.
fn now() -> Option<typst::foundations::Datetime> {
    use chrono::{Datelike, Timelike};
    let t = chrono::Utc::now();
    typst::foundations::Datetime::from_ymd_hms(
        t.year(),
        t.month() as u8,
        t.day() as u8,
        t.hour() as u8,
        t.minute() as u8,
        t.second() as u8,
    )
}

/// The 1-based line of `span` in the main source, when it points there.
fn world_line(
    source: &str,
    world: &DocWorld,
    span: impl Into<typst::syntax::DiagSpan>,
) -> Option<usize> {
    use typst::{World, WorldExt};
    let span = span.into();
    let id = span.id()?;
    if id != world.main() {
        return None;
    }
    let range = world.range(span)?;
    Some(
        source[..range.start.min(source.len())]
            .matches('\n')
            .count()
            + 1,
    )
}

/// A JSON value as the Typst value a source reads from `sys.inputs`.
fn json_to_value(value: &serde_json::Value) -> typst::foundations::Value {
    use typst::foundations::{Array, Value};
    match value {
        serde_json::Value::Null => Value::None,
        serde_json::Value::Bool(b) => b.into_value(),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => i.into_value(),
            None => n.as_f64().unwrap_or_default().into_value(),
        },
        serde_json::Value::String(s) => s.as_str().into_value(),
        serde_json::Value::Array(items) => {
            Value::Array(items.iter().map(json_to_value).collect::<Array>())
        }
        serde_json::Value::Object(map) => {
            let mut dict = Dict::new();
            for (k, v) in map {
                dict.insert(k.as_str().into(), json_to_value(v));
            }
            Value::Dict(dict)
        }
    }
}
