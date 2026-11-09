//! Rendering a stored template with data: a markdown body has its `{{ variables }}`
//! filled first and is then set in its layout; a Typst body reads the data from
//! `sys.inputs`.

use laterite_core::Db;
use serde_json::Value;

use crate::registry::Format;
use crate::{render_markdown, render_typst, store, Assets, DocumentMeta, Layout, Output, PdfError};

/// Why a stored template did not render.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// No template has that code.
    #[error("no PDF template has the code {0}")]
    NotFound(String),
    /// A markdown body's `{{ variables }}` did not render.
    #[error("the template's variables did not render: {0}")]
    Variables(String),
    /// The document did not compile or could not be written.
    #[error(transparent)]
    Pdf(#[from] PdfError),
    /// The database refused.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// The PDF of the template `code` rendered with `data`.
pub async fn render(
    db: &Db,
    code: &str,
    data: Value,
    output: Output,
) -> Result<Vec<u8>, RenderError> {
    let template = store::template(db, Some(code), None)
        .await?
        .ok_or_else(|| RenderError::NotFound(code.to_string()))?;
    let mut layout = store::layout(db, template.layout_id).await?;
    if let Some(paper) = &template.paper {
        layout.paper = paper.clone();
    }
    layout.landscape = layout.landscape || template.landscape;
    let meta = DocumentMeta {
        title: template.title.clone(),
        ..DocumentMeta::default()
    };
    render_body(
        template.format,
        &template.body,
        &layout,
        &meta,
        &data,
        output,
    )
}

/// A body in `format` rendered with `data`, set in `layout` when it is markdown.
pub fn render_body(
    format: Format,
    body: &str,
    layout: &Layout,
    meta: &DocumentMeta,
    data: &Value,
    output: Output,
) -> Result<Vec<u8>, RenderError> {
    let mut assets = Assets::default();
    if let Value::Object(map) = data {
        assets.inputs = map.clone();
    }
    match format {
        Format::Markdown => {
            let filled = fill(body, data)?;
            Ok(render_markdown(&filled, layout, meta, &assets, output)?)
        }
        Format::Typst => Ok(render_typst(body, &assets, output)?),
    }
}

/// A markdown body with its `{{ variables }}` filled from `data`. The result is
/// still markdown: the converter escapes it for Typst, so a value can change the
/// formatting it sits in but never become code.
pub fn fill(body: &str, data: &Value) -> Result<String, RenderError> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Chainable);
    env.render_str(body, minijinja::Value::from(minijinja::value::Serde(data)))
        .map_err(|e| RenderError::Variables(e.to_string()))
}
