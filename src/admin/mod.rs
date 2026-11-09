//! The admin: layout and template screens as descriptors, a picker over layouts,
//! and the routes behind the editor's live preview and the form actions (download,
//! duplicate, reset to default).

use std::collections::HashMap;
use std::sync::Arc;

use laterite_admin::axum::extract::{Form, Path};
use laterite_admin::axum::http::{header, HeaderMap, StatusCode};
use laterite_admin::axum::response::{Html, IntoResponse, Response};
use laterite_admin::axum::routing::{get, post};
use laterite_admin::axum::{Extension, Router};
use laterite_admin::picker::{PickerSourceReg, TableSource};
use laterite_admin::routes::{RouteCtx, Screen, ScreenReg};
use laterite_admin::{FlashLevel, Permission, Resource, SessionHandle};
use laterite_core::{t, Db, Registry};
use serde_json::Value;

use crate::document::{render_body, RenderError};
use crate::registry::{Format, PdfLayoutReg, PdfTemplateReg};
use crate::store::{self, StoreError, LAYOUTS, TEMPLATES};
use crate::{DocumentMeta, Output};

/// The permission for every PDF screen.
pub const MANAGE: &str = "rainmill.pdf.manage";
/// The picker source over layouts, for a template's layout field.
pub const LAYOUT_SOURCE: &str = "rainmill.pdf.layouts";

pub(crate) fn permission() -> Permission {
    Permission::new(MANAGE, t!("Manage PDF templates and layouts"), t!("PDF"))
}

pub(crate) fn resources() -> Vec<Resource> {
    vec![
        laterite_admin::resource!("admin/templates.yaml"),
        laterite_admin::resource!("admin/layouts.yaml"),
    ]
}

pub(crate) fn layout_source() -> PickerSourceReg {
    PickerSourceReg::new(
        LAYOUT_SOURCE,
        Arc::new(TableSource::new(LAYOUTS, "id", "name").with_hint("code")),
    )
    .require(MANAGE)
}

pub(crate) fn screen() -> ScreenReg {
    ScreenReg::new("/tools", MANAGE, Arc::new(Tools))
}

/// The routes under `/admin/rainmill/pdf/tools`.
struct Tools;

#[derive(Clone)]
struct Cx {
    db: Db,
    admin_path: String,
    templates: Arc<Vec<PdfTemplateReg>>,
    layouts: Arc<Vec<PdfLayoutReg>>,
}

impl Cx {
    fn edit(&self, kind: &str, id: i64) -> String {
        format!(
            "{}/rainmill/pdf/{kind}/{id}/edit",
            self.admin_path.trim_end_matches('/')
        )
    }
}

fn registered(registry: &Registry) -> (Vec<PdfTemplateReg>, Vec<PdfLayoutReg>) {
    (
        registry
            .items::<PdfTemplateReg>()
            .into_iter()
            .cloned()
            .collect(),
        registry
            .items::<PdfLayoutReg>()
            .into_iter()
            .cloned()
            .collect(),
    )
}

impl Screen for Tools {
    fn mount(&self, ctx: &RouteCtx) -> Router {
        let (templates, layouts) = registered(ctx.plugin_defined());
        let cx = Cx {
            db: ctx.db().clone(),
            admin_path: ctx.admin_path().to_string(),
            templates: Arc::new(templates),
            layouts: Arc::new(layouts),
        };
        let c = |cx: &Cx| cx.clone();
        let (p, tp, tr, td, lp, lr, ld) = (c(&cx), c(&cx), c(&cx), c(&cx), c(&cx), c(&cx), c(&cx));
        Router::new()
            .route(
                "/preview",
                post(move |Form(form): Form<HashMap<String, String>>| preview(p.clone(), form)),
            )
            .route(
                "/templates/{id}/pdf",
                get(move |Path(id): Path<i64>| template_pdf(tp.clone(), id)),
            )
            .route(
                "/templates/{id}/reset",
                post(
                    move |Path(id): Path<i64>,
                          Extension(s): Extension<SessionHandle>,
                          h: HeaderMap| {
                        reset(tr.clone(), TEMPLATES, id, s, h)
                    },
                ),
            )
            .route(
                "/templates/{id}/duplicate",
                post(
                    move |Path(id): Path<i64>,
                          Extension(s): Extension<SessionHandle>,
                          h: HeaderMap| {
                        duplicate(td.clone(), TEMPLATES, id, s, h)
                    },
                ),
            )
            .route(
                "/layouts/{id}/pdf",
                get(move |Path(id): Path<i64>| layout_pdf(lp.clone(), id)),
            )
            .route(
                "/layouts/{id}/reset",
                post(
                    move |Path(id): Path<i64>,
                          Extension(s): Extension<SessionHandle>,
                          h: HeaderMap| {
                        reset(lr.clone(), LAYOUTS, id, s, h)
                    },
                ),
            )
            .route(
                "/layouts/{id}/duplicate",
                post(
                    move |Path(id): Path<i64>,
                          Extension(s): Extension<SessionHandle>,
                          h: HeaderMap| {
                        duplicate(ld.clone(), LAYOUTS, id, s, h)
                    },
                ),
            )
    }
}

fn pdf_response(bytes: Vec<u8>, name: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{name}.pdf\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    )
        .into_response()
}

/// A message in the preview pane, in place of the document.
fn pane_message(message: &str) -> Response {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Html(format!("<p class=\"lat-markdown__empty\">{escaped}</p>")).into_response()
}

fn redirect(to: String, headers: &HeaderMap) -> Response {
    if headers.contains_key("hx-request") {
        ([("HX-Redirect", to)], StatusCode::NO_CONTENT).into_response()
    } else {
        (StatusCode::SEE_OTHER, [(header::LOCATION, to)]).into_response()
    }
}

/// The editor's live preview: the body being edited, rendered with the form's
/// format, layout and sample data as they stand, unsaved.
async fn preview(cx: Cx, form: HashMap<String, String>) -> Response {
    let field = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
    let data: Value = match field("sample_data").trim() {
        "" => Value::Object(Default::default()),
        raw => match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return pane_message(&format!("The sample data is not valid JSON: {e}")),
        },
    };
    let layout_id = field("layout_id").trim().parse().ok();
    let mut layout = match store::layout(&cx.db, layout_id).await {
        Ok(l) => l,
        Err(e) => return pane_message(&e.to_string()),
    };
    if !field("paper").trim().is_empty() {
        layout.paper = field("paper").trim().to_string();
    }
    if matches!(field("landscape"), "1" | "on" | "true") {
        layout.landscape = true;
    }
    let meta = DocumentMeta {
        title: field("title").to_string(),
        ..DocumentMeta::default()
    };
    match render_body(
        Format::parse(field("format")),
        field("text"),
        &layout,
        &meta,
        &data,
        Output::default(),
    ) {
        Ok(pdf) => pdf_response(pdf, "preview"),
        Err(e) => pane_message(&e.to_string()),
    }
}

/// A saved template rendered with its sample data.
async fn template_pdf(cx: Cx, id: i64) -> Response {
    let Ok(Some(template)) = store::template(&cx.db, None, Some(id)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = async {
        let mut layout = store::layout(&cx.db, template.layout_id).await?;
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
            &template.sample_data,
            Output::default(),
        )
    }
    .await;
    match result {
        Ok(pdf) => pdf_response(pdf, &template.code),
        Err(RenderError::Db(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
    }
}

/// A layout shown with a sample page of text, to judge the letterhead.
async fn layout_pdf(cx: Cx, id: i64) -> Response {
    let layout = match store::layout(&cx.db, Some(id)).await {
        Ok(l) => l,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let meta = DocumentMeta {
        title: "Sample document".into(),
        version: "1.0".into(),
        effective: chrono::Utc::now().format("%-d %B %Y").to_string(),
        ..DocumentMeta::default()
    };
    let sample = "## A heading\n\nBody text in the layout's font and colour, with a [link](https://acme.com) and **strong** words.\n\n| Column | Column |\n| --- | --- |\n| Cell | Cell |\n\n> A quoted line.\n";
    match render_body(
        Format::Markdown,
        sample,
        &layout,
        &meta,
        &Value::Null,
        Output::default(),
    ) {
        Ok(pdf) => pdf_response(pdf, "layout"),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
    }
}

async fn reset(
    cx: Cx,
    table: &'static str,
    id: i64,
    session: SessionHandle,
    headers: HeaderMap,
) -> Response {
    let outcome = if table == TEMPLATES {
        let regs: Vec<&PdfTemplateReg> = cx.templates.iter().collect();
        store::reset_template(&cx.db, id, &regs).await
    } else {
        let regs: Vec<&PdfLayoutReg> = cx.layouts.iter().collect();
        store::reset_layout(&cx.db, id, &regs).await
    };
    match outcome {
        Ok(()) => session.push_flash(
            FlashLevel::Success,
            t!("Reset to the version its module ships."),
        ),
        Err(StoreError::NotRegistered) => session.push_flash(
            FlashLevel::Error,
            t!("This was made in the admin, so it has no default to reset to."),
        ),
        Err(StoreError::NotFound) => return StatusCode::NOT_FOUND.into_response(),
        Err(StoreError::Db(e)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
    let kind = if table == TEMPLATES {
        "templates"
    } else {
        "layouts"
    };
    redirect(cx.edit(kind, id), &headers)
}

async fn duplicate(
    cx: Cx,
    table: &'static str,
    id: i64,
    session: SessionHandle,
    headers: HeaderMap,
) -> Response {
    let kind = if table == TEMPLATES {
        "templates"
    } else {
        "layouts"
    };
    match store::duplicate(&cx.db, table, id).await {
        Ok(copy) => {
            session.push_flash(FlashLevel::Success, t!("Duplicated. This is the copy."));
            redirect(cx.edit(kind, copy), &headers)
        }
        Err(StoreError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
