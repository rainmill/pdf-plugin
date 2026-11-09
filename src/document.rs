//! Rendering a template with data: render hooks change the data, a markdown body
//! has its `{{ variables }}` filled and is set in its layout, a Typst body reads
//! the data from `sys.inputs`, and render hooks change the source before it
//! compiles.

use serde_json::Value;

use crate::codes::{barcode_path, qr_path};
use crate::registry::{Format, PdfRenderHookReg};
use crate::render::{compile, IMPORT};
use crate::{markdown_source, Assets, DocumentMeta, Layout, Output, PdfError};

/// Why a template did not render.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// No template has that code.
    #[error("no PDF template has the code {0}")]
    NotFound(String),
    /// A markdown body's `{{ variables }}` did not render.
    #[error("the template's variables did not render: {0}")]
    Variables(String),
    /// The document did not compile, or could not be written or protected.
    #[error(transparent)]
    Pdf(#[from] PdfError),
    /// A media file could not be read or stored.
    #[error("media: {0}")]
    Media(String),
    /// The database refused.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Everything one render is made of, gathered before anything compiles.
pub(crate) struct Job {
    pub code: String,
    pub format: Format,
    pub body: String,
    pub layout: Layout,
    pub meta: DocumentMeta,
    pub data: Value,
    pub files: Vec<(String, Vec<u8>)>,
    pub fonts: Vec<Vec<u8>>,
    pub hooks: Vec<PdfRenderHookReg>,
    pub output: Output,
    /// A markdown body's `{{ variables }}` are filled; off for a document a
    /// module keeps verbatim, such as a legal text.
    pub templated: bool,
}

impl Job {
    /// The PDF.
    pub(crate) fn run(self) -> Result<Vec<u8>, RenderError> {
        let hooks: Vec<&PdfRenderHookReg> = self
            .hooks
            .iter()
            .filter(|h| h.applies(&self.code))
            .collect();
        let mut data = self.data;
        for h in &hooks {
            data = h.hook.data(&self.code, data);
        }
        let mut assets = Assets {
            files: self.files,
            fonts: self.fonts,
            ..Assets::default()
        };
        if let Value::Object(map) = &data {
            assets.inputs = map.clone();
        }
        let (prelude, source) = match self.format {
            Format::Markdown => {
                let filled = if self.templated {
                    fill(&self.body, &data)?
                } else {
                    self.body.clone()
                };
                (
                    IMPORT.to_string(),
                    markdown_source(&filled, &self.layout, &self.meta, &assets),
                )
            }
            Format::Typst => (
                format!("{IMPORT}{}\n", self.meta.document_rule()),
                self.body,
            ),
        };
        let source = hooks
            .iter()
            .fold(source, |s, h| h.hook.source(&self.code, s));
        Ok(compile(&prelude, &source, &assets, &self.output)?)
    }
}

/// A body in `format` rendered with `data`, set in `layout` when it is markdown.
/// No variables, hooks or media: [`crate::Pdf`] renders stored templates with all
/// three.
pub fn render_body(
    format: Format,
    body: &str,
    layout: &Layout,
    meta: &DocumentMeta,
    data: &Value,
    output: &Output,
) -> Result<Vec<u8>, RenderError> {
    Job {
        code: String::new(),
        format,
        body: body.to_string(),
        layout: layout.clone(),
        meta: meta.clone(),
        data: data.clone(),
        files: Vec::new(),
        fonts: Vec::new(),
        hooks: Vec::new(),
        output: output.clone(),
        templated: true,
    }
    .run()
}

/// A markdown body with its `{{ variables }}` filled from `data`. The result is
/// still markdown: the converter escapes it for Typst, so a value can change the
/// formatting it sits in but never become code. `{{ qr(value) }}` and
/// `{{ barcode(value, kind="ean13") }}` place codes, sized with `width="30mm"`.
pub fn fill(body: &str, data: &Value) -> Result<String, RenderError> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Chainable);
    env.add_function(
        "qr",
        |value: minijinja::Value, kwargs: minijinja::value::Kwargs| {
            let width = length(kwargs.get::<Option<String>>("width")?, "30mm");
            kwargs.assert_all_used()?;
            Ok::<_, minijinja::Error>(format!(
                "![QR code]({} \"width={width}\")",
                qr_path(&value.to_string())
            ))
        },
    );
    env.add_function(
        "barcode",
        |value: minijinja::Value, kwargs: minijinja::value::Kwargs| {
            let kind = kwargs
                .get::<Option<String>>("kind")?
                .unwrap_or_else(|| "code128".into());
            let width = length(kwargs.get::<Option<String>>("width")?, "60mm");
            kwargs.assert_all_used()?;
            let kind: String = kind.chars().filter(char::is_ascii_alphanumeric).collect();
            Ok::<_, minijinja::Error>(format!(
                "![Barcode]({} \"width={width}\")",
                barcode_path(&kind, &value.to_string())
            ))
        },
    );
    env.render_str(body, minijinja::Value::from(minijinja::value::Serde(data)))
        .map_err(|e| RenderError::Variables(e.to_string()))
}

/// A length a template asked for, kept to the characters a length uses.
fn length(asked: Option<String>, fallback: &str) -> String {
    asked
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '%'))
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}
