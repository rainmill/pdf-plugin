//! The handle an application renders templates through: it carries the
//! registered variables and render hooks, and the disks media files live on.

use std::sync::Arc;

use laterite_admin::axum::http::header;
use laterite_admin::axum::response::{IntoResponse, Response};
use laterite_core::{Db, Registry};
use laterite_media::{
    blob_path, ingest, link, record, stream_of, DiskReg, MediaRecord, NewMedia, Owner,
    StorageDriver,
};
use serde_json::Value;

use crate::document::{Job, RenderError};
use crate::registry::{Format, PdfFontReg, PdfRenderHookReg, PdfVariableReg};
use crate::store::{self, StoredLayout, Template};
use crate::{DocumentMeta, Output};

/// Renders stored templates with the registered variables and hooks.
///
/// ```no_run
/// # async fn f(db: laterite_core::Db, registry: &laterite_core::Registry) -> Result<(), rainmill_pdf::RenderError> {
/// use rainmill_pdf::{Pdf, RenderOptions};
/// use serde_json::json;
///
/// let pdf = Pdf::new(db, registry);
/// let invoice = pdf
///     .render_with("acme.shop.invoice", json!({ "number": "2026/1" }), &RenderOptions::locale("de"))
///     .await?;
/// let attachment = invoice.attachment();
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct Pdf {
    db: Db,
    variables: Arc<Vec<PdfVariableReg>>,
    hooks: Arc<Vec<PdfRenderHookReg>>,
    fonts: Arc<Vec<PdfFontReg>>,
    disks: Vec<(String, Arc<dyn StorageDriver>)>,
    recorder: Option<crate::testing::Recorder>,
}

/// How one render is made.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// The locale to render in (`pt-BR`): its translation, then its language's
    /// (`pt`), then the template itself.
    pub locale: Option<String>,
    /// PDF/A or a password.
    pub output: Output,
    /// The PDF's author, over the layout's.
    pub author: Option<String>,
}

impl RenderOptions {
    /// Rendered in `locale`.
    pub fn locale(locale: impl Into<String>) -> Self {
        Self {
            locale: Some(locale.into()),
            ..Self::default()
        }
    }
}

/// A rendered PDF and the name it is offered under.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// The PDF.
    pub bytes: Vec<u8>,
    /// The file name, from the title (`terms-of-use.pdf`).
    pub filename: String,
}

/// The parts of a template a render needs, saved or still in the editor.
#[derive(Debug, Clone, Default)]
pub(crate) struct Parts {
    pub code: String,
    pub title: String,
    pub format: Format,
    pub body: String,
    pub layout_id: Option<i64>,
    pub paper: Option<String>,
    pub landscape: bool,
    pub description: String,
    pub keywords: Vec<String>,
    pub version: String,
    pub effective: String,
    pub url: String,
    pub author: String,
    /// Fill `{{ variables }}` in a markdown body.
    pub templated: bool,
}

impl From<&Template> for Parts {
    fn from(t: &Template) -> Self {
        Self {
            code: t.code.clone(),
            title: t.title.clone(),
            format: t.format,
            body: t.body.clone(),
            layout_id: t.layout_id,
            paper: t.paper.clone(),
            landscape: t.landscape,
            description: t.description.clone(),
            keywords: t.keywords.clone(),
            templated: true,
            ..Self::default()
        }
    }
}

impl Pdf {
    /// A handle over `db` with the variables, hooks and disks in `registry`.
    pub fn new(db: Db, registry: &Registry) -> Self {
        Self {
            db,
            variables: Arc::new(
                registry
                    .items::<PdfVariableReg>()
                    .into_iter()
                    .cloned()
                    .collect(),
            ),
            hooks: Arc::new(
                registry
                    .items::<PdfRenderHookReg>()
                    .into_iter()
                    .cloned()
                    .collect(),
            ),
            fonts: Arc::new(
                registry
                    .items::<PdfFontReg>()
                    .into_iter()
                    .cloned()
                    .collect(),
            ),
            disks: registry
                .contributions::<DiskReg>()
                .into_iter()
                .map(|d| (d.item.name.clone(), Arc::clone(&d.item.driver)))
                .collect(),
            recorder: registry
                .items::<crate::testing::Recorder>()
                .first()
                .map(|r| (*r).clone()),
        }
    }

    /// Adds the disk `name`, or replaces it. [`Pdf::library`] adds the media
    /// library's two.
    pub fn disk(mut self, name: impl Into<String>, driver: Arc<dyn StorageDriver>) -> Self {
        let name = name.into();
        self.disks.retain(|(n, _)| *n != name);
        self.disks.push((name, driver));
        self
    }

    /// Adds the media library's disks, public and private, so layouts may use
    /// any library file: the application's registered `public` and `private`
    /// disks, else those under `storage_dir`.
    pub fn library(self, registry: &Registry, storage_dir: &str) -> Self {
        self.disk(
            laterite_admin::medialib::DISK,
            laterite_admin::medialib::library_driver(registry, storage_dir),
        )
        .disk(
            laterite_admin::medialib::PRIVATE_DISK,
            laterite_admin::medialib::private_driver(registry, storage_dir),
        )
    }

    /// The database it reads templates from.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// The template `code` rendered with `data`.
    pub async fn render(&self, code: &str, data: Value) -> Result<Rendered, RenderError> {
        self.render_with(code, data, &RenderOptions::default())
            .await
    }

    /// The template `code` rendered with `data` as `options` say.
    pub async fn render_with(
        &self,
        code: &str,
        data: Value,
        options: &RenderOptions,
    ) -> Result<Rendered, RenderError> {
        let template = store::template(&self.db, Some(code), None)
            .await?
            .ok_or_else(|| RenderError::NotFound(code.to_string()))?;
        let mut parts = Parts::from(&template);
        if let Some(locale) = &options.locale {
            if let Some((title, body)) =
                store::translation(&self.db, template.id, &locale_chain(locale)).await?
            {
                parts.title = title.unwrap_or(parts.title);
                parts.body = body.unwrap_or(parts.body);
            }
        }
        self.render_parts(parts, data, options).await
    }

    /// `body` (markdown, taken verbatim) set in the layout `layout_id`, or the
    /// default layout when `None`, described by `meta`: its title, version,
    /// effective date and URL fill the masthead and running parts. For a
    /// document a module keeps itself, such as a versioned policy.
    pub async fn render_document(
        &self,
        body: &str,
        layout_id: Option<i64>,
        meta: DocumentMeta,
        options: &RenderOptions,
    ) -> Result<Rendered, RenderError> {
        let parts = Parts {
            code: meta.title.clone(),
            title: meta.title,
            body: body.to_string(),
            layout_id,
            description: meta.description,
            keywords: meta.keywords,
            version: meta.version,
            effective: meta.effective,
            url: meta.url,
            author: meta.author,
            templated: false,
            ..Parts::default()
        };
        self.render_parts(parts, Value::Null, options).await
    }

    /// `parts` rendered with `data`: the layout and its media loaded, the
    /// variables merged under the data, compiled off the async runtime.
    pub(crate) async fn render_parts(
        &self,
        parts: Parts,
        data: Value,
        options: &RenderOptions,
    ) -> Result<Rendered, RenderError> {
        let stored = store::layout(&self.db, parts.layout_id).await?;
        self.render_in(parts, stored, data, options).await
    }

    /// `parts` set in `stored`.
    pub(crate) async fn render_in(
        &self,
        parts: Parts,
        stored: StoredLayout,
        data: Value,
        options: &RenderOptions,
    ) -> Result<Rendered, RenderError> {
        let mut layout = stored.layout.clone();
        if let Some(paper) = &parts.paper {
            layout.paper = paper.clone();
        }
        layout.landscape = layout.landscape || parts.landscape;
        let mut files = Vec::new();
        if let Some((path, bytes)) = self.media_file(stored.logo_id, "logo").await? {
            layout.logo = Some(path.clone());
            files.push((path, bytes));
        }
        if let Some((path, bytes)) = self.media_file(stored.background_id, "background").await? {
            layout.background = Some(path.clone());
            files.push((path, bytes));
        }
        let mut fonts: Vec<Vec<u8>> = self.fonts.iter().map(|f| f.bytes.to_vec()).collect();
        for id in &stored.font_ids {
            let found = record::find(&self.db, *id)
                .await
                .map_err(|e| RenderError::Media(e.to_string()))?;
            if let Some(rec) = found {
                fonts.push(self.read(&rec).await?);
            }
        }
        let meta = DocumentMeta {
            title: parts.title.clone(),
            author: options
                .author
                .clone()
                .unwrap_or(if parts.author.is_empty() {
                    stored.author
                } else {
                    parts.author.clone()
                }),
            description: parts.description,
            keywords: parts.keywords,
            version: parts.version,
            effective: parts.effective,
            url: parts.url,
            language: options
                .locale
                .as_deref()
                .map(|l| l.split(['-', '_']).next().unwrap_or(l).to_string())
                .unwrap_or_default(),
        };
        let stem = slug(&parts.title);
        let filename = if stem.is_empty() {
            slug(&parts.code)
        } else {
            stem
        };
        let filename = format!(
            "{}.pdf",
            if filename.is_empty() {
                "document"
            } else {
                &filename
            }
        );
        if let Some(recorder) = &self.recorder {
            recorder.record(crate::testing::Recorded {
                code: parts.code.clone(),
                data: self.with_variables(data),
                locale: options.locale.clone(),
            });
            return Ok(Rendered {
                bytes: crate::testing::STUB.to_vec(),
                filename,
            });
        }
        let job = Job {
            code: parts.code.clone(),
            format: parts.format,
            body: parts.body,
            layout,
            meta,
            data: self.with_variables(data),
            files,
            fonts,
            hooks: self.hooks.to_vec(),
            output: options.output.clone(),
            templated: parts.templated,
        };
        let bytes = tokio::task::spawn_blocking(move || job.run())
            .await
            .map_err(|e| RenderError::Media(format!("the render stopped: {e}")))??;
        let stem = slug(&parts.title);
        let filename = if stem.is_empty() {
            slug(&parts.code)
        } else {
            stem
        };
        Ok(Rendered {
            bytes,
            filename: format!(
                "{}.pdf",
                if filename.is_empty() {
                    "document"
                } else {
                    &filename
                }
            ),
        })
    }

    /// `data` with every registered variable it does not already carry.
    fn with_variables(&self, data: Value) -> Value {
        let mut data = match data {
            Value::Null => Value::Object(Default::default()),
            other => other,
        };
        if let Value::Object(map) = &mut data {
            for v in self.variables.iter() {
                map.entry(v.name.clone()).or_insert_with(|| v.value.clone());
            }
        }
        data
    }

    /// The image the media record `id` holds, at `/media/<name>.<ext>`. A record
    /// that is gone, or is not an image, renders without it.
    async fn media_file(
        &self,
        id: Option<i64>,
        name: &str,
    ) -> Result<Option<(String, Vec<u8>)>, RenderError> {
        let Some(id) = id else { return Ok(None) };
        let Some(rec) = record::find(&self.db, id)
            .await
            .map_err(|e| RenderError::Media(e.to_string()))?
        else {
            return Ok(None);
        };
        let Some(ext) = image_extension(&rec.content_type) else {
            return Ok(None);
        };
        let bytes = self.read(&rec).await?;
        Ok(Some((format!("/media/{name}.{ext}"), bytes)))
    }

    /// The bytes of the media record `rec`.
    pub async fn read(&self, rec: &MediaRecord) -> Result<Vec<u8>, RenderError> {
        use futures::TryStreamExt;
        let driver = self.driver(&rec.disk)?;
        let stream = driver
            .get(&blob_path(&rec.hash))
            .await
            .map_err(|e| RenderError::Media(e.to_string()))?;
        let chunks: Vec<_> = stream
            .try_collect()
            .await
            .map_err(|e| RenderError::Media(e.to_string()))?;
        Ok(chunks.concat())
    }

    fn driver(&self, disk: &str) -> Result<&Arc<dyn StorageDriver>, RenderError> {
        self.disks
            .iter()
            .find(|(n, _)| n == disk)
            .map(|(_, d)| d)
            .ok_or_else(|| RenderError::Media(format!("no disk named {disk:?} is registered")))
    }
}

impl Rendered {
    /// The PDF shown in the browser.
    pub fn inline(self) -> Response {
        self.respond("inline")
    }

    /// The PDF saved as a download.
    pub fn download(self) -> Response {
        self.respond("attachment")
    }

    fn respond(self, disposition: &str) -> Response {
        (
            [
                (header::CONTENT_TYPE, "application/pdf".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("{disposition}; filename=\"{}\"", self.filename),
                ),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            self.bytes,
        )
            .into_response()
    }

    /// The PDF as a mail attachment: `Message::template(..).attach(..)` takes its
    /// parts, or push it onto `message.attachments`.
    pub fn attachment(&self) -> laterite_core::mail::Attachment {
        laterite_core::mail::Attachment {
            filename: self.filename.clone(),
            content_type: "application/pdf".into(),
            bytes: self.bytes.clone(),
        }
    }

    /// Stores the PDF on `disk` as a media record and, given an `owner`, links it
    /// to that record's field (replacing what the field held). A PDF with no
    /// owner is placed in the media library, where it can be found.
    pub async fn store(
        &self,
        pdf: &Pdf,
        disk: &str,
        owner: Option<Owner<'_>>,
    ) -> Result<MediaRecord, RenderError> {
        let media = |e: &dyn std::fmt::Display| RenderError::Media(e.to_string());
        let driver = pdf.driver(disk)?;
        let stored = ingest(driver.as_ref(), stream_of(self.bytes.clone()), u64::MAX)
            .await
            .map_err(|e| media(&e))?;
        let id = record::create(
            &pdf.db,
            NewMedia {
                disk: disk.to_string(),
                hash: stored.hash,
                size: stored.size as i64,
                content_type: stored.content_type,
                original_name: self.filename.clone(),
                in_library: owner.is_none(),
                created_by: None,
            },
        )
        .await
        .map_err(|e| media(&e))?;
        if let Some(owner) = owner {
            link::set(&pdf.db, owner, &[id])
                .await
                .map_err(|e| media(&e))?;
        }
        record::find(&pdf.db, id)
            .await
            .map_err(|e| media(&e))?
            .ok_or_else(|| RenderError::Media("the stored record vanished".into()))
    }
}

/// `pt-BR`, then `pt`.
fn locale_chain(locale: &str) -> Vec<String> {
    let mut chain = vec![locale.to_string()];
    if let Some((language, _)) = locale.split_once(['-', '_']) {
        chain.push(language.to_string());
    }
    chain
}

/// The extension Typst reads an image of `content_type` by.
fn image_extension(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => return None,
    })
}

/// A file name stem: lowercase letters and digits, hyphen separated.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}
