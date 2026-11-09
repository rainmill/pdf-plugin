//! PDFs on the site: `pdf_url(code, data)` in a theme makes a signed link that
//! renders the template when followed, and the `rainmill.pdf` component gives
//! a page a Download link for a template with the page's URL parameters.
//!
//! A link carries the code, the data and the locale in a Crypt token, so a
//! visitor can follow it but never render another template or other data. It
//! lasts a day; the page makes a fresh one each time it renders.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use laterite_admin::axum::extract::Path;
use laterite_admin::axum::http::StatusCode;
use laterite_admin::axum::response::{IntoResponse, Response};
use laterite_admin::axum::routing::get;
use laterite_admin::axum::Router;
use laterite_admin::routes::{PublicRoute, RouteCtx};
use laterite_web::theme::component::{
    Component, ComponentContext, ComponentOutput, PropertyDef, PropertyKind,
};
use laterite_web::theme::markup::Markup;
use laterite_web::theme::ThemeError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Pdf, RenderOptions};

/// Where the links point, namespaced so it never meets a page of the site.
pub const PATH: &str = "/rainmill/pdf";
/// The Crypt purpose a link's token is made and read under.
const PURPOSE: &str = "rainmill.pdf.link";
/// How long a link works.
const LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// What a link renders.
#[derive(Serialize, Deserialize)]
struct Link {
    /// The template's code.
    c: String,
    /// The data.
    #[serde(default)]
    d: Value,
    /// The locale, when one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    l: Option<String>,
    /// Offer it as a download instead of showing it.
    #[serde(default)]
    dl: bool,
}

/// The route context, set once the route mounts, so the theme function can
/// sign links with the application's key.
#[derive(Clone, Default)]
pub(crate) struct Signer(Arc<OnceLock<RouteCtx>>);

impl Signer {
    /// A link to `code` rendered with `data`, or `None` before the site mounts.
    fn url(
        &self,
        code: &str,
        data: Value,
        locale: Option<String>,
        download: bool,
    ) -> Option<String> {
        let ctx = self.0.get()?;
        let link = Link {
            c: code.to_string(),
            d: data,
            l: locale,
            dl: download,
        };
        let payload = serde_json::to_vec(&link).ok()?;
        let token = ctx.crypt().token(PURPOSE, &payload, LIFETIME);
        // A public route's base path is the path it mounts at.
        Some(format!("{}/{token}", ctx.base_path().trim_end_matches('/')))
    }
}

/// The public route that renders a link.
pub(crate) struct PdfRoute(pub Signer);

impl PublicRoute for PdfRoute {
    fn mount(&self, ctx: &RouteCtx) -> Router {
        let _ = self.0 .0.set(ctx.clone());
        let ctx = ctx.clone();
        Router::new().route(
            "/{token}",
            get(move |Path(token): Path<String>| {
                let ctx = ctx.clone();
                async move { follow(&ctx, &token).await }
            }),
        )
    }
}

async fn follow(ctx: &RouteCtx, token: &str) -> Response {
    let payload = match ctx.crypt().check_token(PURPOSE, token) {
        Ok(p) => p,
        Err(laterite_core::CryptError::Expired) => {
            return (
                StatusCode::GONE,
                "This link has expired. Reload the page it came from for a fresh one.",
            )
                .into_response()
        }
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let Ok(link) = serde_json::from_slice::<Link>(&payload) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let library = laterite_admin::medialib::library_driver(ctx.plugin_defined(), ctx.storage_dir());
    let pdf = Pdf::new(ctx.db().clone(), ctx.plugin_defined())
        .disk(laterite_admin::medialib::DISK, library);
    let options = RenderOptions {
        locale: link.l,
        ..RenderOptions::default()
    };
    match pdf.render_with(&link.c, link.d, &options).await {
        Ok(rendered) if link.dl => rendered.download(),
        Ok(rendered) => rendered.inline(),
        Err(crate::RenderError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        // The visitor sees that it failed; the reason is the site's to read in
        // `lat pdf check`, never a compile message on a public page.
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "This document could not be made right now. Please try again later.",
        )
            .into_response(),
    }
}

/// `pdf_url(code, data={}, locale=none, download=false)` for themes.
pub(crate) struct PdfMarkup(pub Signer);

impl Markup for PdfMarkup {
    fn install(&self, env: &mut minijinja::Environment<'static>) {
        let signer = self.0.clone();
        env.add_function(
            "pdf_url",
            move |code: String,
                  kwargs: minijinja::value::Kwargs|
                  -> Result<String, minijinja::Error> {
                let data: Option<minijinja::Value> = kwargs.get("data")?;
                let locale: Option<String> = kwargs.get("locale")?;
                let download: Option<bool> = kwargs.get("download")?;
                kwargs.assert_all_used()?;
                let data = match data {
                    Some(v) => serde_json::to_value(&v).unwrap_or(Value::Null),
                    None => Value::Object(Default::default()),
                };
                Ok(signer
                    .url(&code, data, locale, download.unwrap_or(false))
                    .unwrap_or_default())
            },
        );
    }
}

/// The `rainmill.pdf` component: `url` and `label` for a template rendered with
/// the page's URL parameters, and a Download link as its markup.
pub(crate) struct PdfComponent(pub Signer);

#[laterite_core::strata::async_trait]
impl Component for PdfComponent {
    fn properties(&self) -> Vec<PropertyDef> {
        let mut template = PropertyDef::new("template", "Template code", PropertyKind::Text);
        template.required = true;
        let mut label = PropertyDef::new("label", "Link text", PropertyKind::Text);
        label.default = Some(json!("Download PDF"));
        vec![
            template,
            PropertyDef::new("locale", "Locale", PropertyKind::Text),
            PropertyDef::new("download", "Offer as a download", PropertyKind::Bool),
            label,
        ]
    }

    fn markup(&self) -> Option<&'static str> {
        Some(
            r#"{% if component.url %}<a class="pdf-link" href="{{ component.url }}"{% if component.download %} download{% endif %}>{{ component.label }}</a>{% endif %}"#,
        )
    }

    async fn run(&self, cx: &ComponentContext<'_>) -> Result<ComponentOutput, ThemeError> {
        let text = |k: &str| {
            cx.properties
                .get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
        };
        let code = text("template").unwrap_or_default();
        let download = cx
            .properties
            .get("download")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let data = serde_json::to_value(cx.params).unwrap_or(Value::Null);
        let url = self
            .0
            .url(&code, data, text("locale"), download)
            .unwrap_or_default();
        Ok(ComponentOutput::data(json!({
            "url": url,
            "label": text("label").unwrap_or_else(|| "Download PDF".into()),
            "download": download,
        })))
    }
}
