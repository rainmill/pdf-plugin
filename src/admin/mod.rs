//! The admin: layout and template screens as descriptors, a picker over layouts,
//! and the routes behind the editor's live preview and the form actions (download,
//! duplicate, reset to default).

use laterite_core::serde_json;
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

use crate::document::RenderError;
use crate::pdf::Parts;
use crate::registry::{Format, PdfLayoutReg, PdfTemplateReg};
use crate::store::{self, StoreError, LAYOUTS, TEMPLATES};
use crate::{Pdf, RenderOptions, Rendered};

/// The settings item holding templates and layouts as tabs.
pub const SECTION: &str = "rainmill.pdf";

/// The permission for every PDF screen: opening templates and layouts.
pub const MANAGE: &str = "rainmill.pdf.manage";
/// Previewing and downloading a template or layout as a PDF.
pub const PREVIEW: &str = "rainmill.pdf.preview";
/// Creating a template, duplicates included.
pub const TEMPLATES_CREATE: &str = "rainmill.pdf.templates.create";
/// Saving a template, resetting it, and its translations.
pub const TEMPLATES_UPDATE: &str = "rainmill.pdf.templates.update";
/// Deleting a template.
pub const TEMPLATES_DELETE: &str = "rainmill.pdf.templates.delete";
/// Creating a layout, duplicates included.
pub const LAYOUTS_CREATE: &str = "rainmill.pdf.layouts.create";
/// Saving a layout and resetting it.
pub const LAYOUTS_UPDATE: &str = "rainmill.pdf.layouts.update";
/// Deleting a layout.
pub const LAYOUTS_DELETE: &str = "rainmill.pdf.layouts.delete";
/// The picker source over layouts, for a template's layout field.
pub const LAYOUT_SOURCE: &str = "rainmill.pdf.layouts";

pub(crate) fn permissions() -> Vec<Permission> {
    let group = || t!("PDF");
    vec![
        Permission::new(MANAGE, t!("Open PDF templates and layouts"), group()),
        Permission::new(PREVIEW, t!("Preview and download PDFs"), group()),
        Permission::new(TEMPLATES_CREATE, t!("Create PDF templates"), group()),
        Permission::new(TEMPLATES_UPDATE, t!("Edit PDF templates"), group()),
        Permission::new(TEMPLATES_DELETE, t!("Delete PDF templates"), group()),
        Permission::new(LAYOUTS_CREATE, t!("Create PDF layouts"), group()),
        Permission::new(LAYOUTS_UPDATE, t!("Edit PDF layouts"), group()),
        Permission::new(LAYOUTS_DELETE, t!("Delete PDF layouts"), group()),
    ]
}

pub(crate) fn section() -> laterite_admin::nav::NavItem {
    laterite_admin::nav::NavItem::section(SECTION, t!("PDF Templates"))
        .icon("file-text")
        .description(t!("Edit the templates and layouts PDFs are made from."))
        .slot(laterite_core::arrange::slot::SYSTEM)
        .order(16)
        .in_settings(t!("Documents"))
}

/// The History page's stylesheet, by its asset key.
pub const HISTORY_CSS: &str = "rainmill-pdf-history.css";

pub(crate) fn history_css() -> laterite_admin::AdminAsset {
    laterite_admin::AdminAsset::new("text/css", include_bytes!("../../assets/history.css"))
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
    pdf: Pdf,
    admin_path: String,
    templates: Arc<Vec<PdfTemplateReg>>,
    layouts: Arc<Vec<PdfLayoutReg>>,
}

impl Cx {
    /// Where the tool routes mount.
    fn tools(&self) -> String {
        format!(
            "{}/rainmill/pdf/tools",
            self.admin_path.trim_end_matches('/')
        )
    }

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
            pdf: Pdf::new(ctx.db().clone(), ctx.plugin_defined())
                .library(ctx.plugin_defined(), ctx.storage_dir()),
            admin_path: ctx.admin_path().to_string(),
            templates: Arc::new(templates),
            layouts: Arc::new(layouts),
        };
        let guarded = |code: &'static str| {
            laterite_admin::axum::middleware::from_fn(
                move |Extension(shell): Extension<laterite_admin::Shell>,
                      req: laterite_admin::axum::extract::Request,
                      next: laterite_admin::axum::middleware::Next| async move {
                    if shell.allows(code) {
                        next.run(req).await
                    } else {
                        StatusCode::FORBIDDEN.into_response()
                    }
                },
            )
        };
        let c = |cx: &Cx| cx.clone();
        let (p, tp, tr, td, lp, lr, ld) = (c(&cx), c(&cx), c(&cx), c(&cx), c(&cx), c(&cx), c(&cx));
        let preview_routes = Router::new()
            .route(
                "/preview",
                post(move |Form(form): Form<HashMap<String, String>>| preview(p.clone(), form)),
            )
            .route(
                "/templates/{id}/pdf",
                get(move |Path(id): Path<i64>| template_pdf(tp.clone(), id)),
            )
            .route(
                "/layouts/{id}/pdf",
                get(move |Path(id): Path<i64>| layout_pdf(lp.clone(), id)),
            )
            .route_layer(guarded(PREVIEW));
        let write = |kind: &'static str, action: &'static str, cx: Cx, code: &'static str| {
            let table = if kind == "templates" {
                TEMPLATES
            } else {
                LAYOUTS
            };
            Router::new()
                .route(
                    &format!("/{kind}/{{id}}/{action}"),
                    post(
                        move |Path(id): Path<i64>,
                              Extension(s): Extension<SessionHandle>,
                              h: HeaderMap| {
                            let cx = cx.clone();
                            async move {
                                if action == "reset" {
                                    reset(cx, table, id, s, h).await
                                } else {
                                    duplicate(cx, table, id, s, h).await
                                }
                            }
                        },
                    ),
                )
                .route_layer(guarded(code))
        };
        let history = |kind: &'static str, cx: Cx, restore_code: &'static str| {
            let restore_cx = cx.clone();
            Router::new()
                .route(
                    &format!("/{kind}/{{id}}/history"),
                    get(
                        move |Path(id): Path<i64>, Extension(shell): Extension<laterite_admin::Shell>| {
                            let cx = cx.clone();
                            async move {
                                let may_restore = shell.allows(restore_code);
                                history_page(cx, kind, id, shell, may_restore).await
                            }
                        },
                    ),
                )
                .merge(
                    Router::new()
                        .route(
                            &format!("/{kind}/{{id}}/history/{{revision}}/restore"),
                            post(
                                move |Path((id, revision)): Path<(i64, i64)>,
                                      Extension(shell): Extension<laterite_admin::Shell>,
                                      Extension(s): Extension<SessionHandle>,
                                      h: HeaderMap| {
                                    let cx = restore_cx.clone();
                                    async move {
                                        restore(cx, kind, id, revision, shell.username(), s, h)
                                            .await
                                    }
                                },
                            ),
                        )
                        .route_layer(guarded(restore_code)),
                )
        };
        let (th, lh) = (c(&cx), c(&cx));
        preview_routes
            .merge(history("templates", th, TEMPLATES_UPDATE))
            .merge(history("layouts", lh, LAYOUTS_UPDATE))
            .merge(write("templates", "reset", tr, TEMPLATES_UPDATE))
            .merge(write("templates", "duplicate", td, TEMPLATES_CREATE))
            .merge(write("layouts", "reset", lr, LAYOUTS_UPDATE))
            .merge(write("layouts", "duplicate", ld, LAYOUTS_CREATE))
    }
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
    let parts = Parts {
        code: field("code").trim().to_string(),
        title: field("title").to_string(),
        format: Format::parse(field("format")),
        body: field("text").to_string(),
        layout_id: field("layout_id").trim().parse().ok(),
        paper: Some(field("paper").trim().to_string()).filter(|p| !p.is_empty()),
        landscape: matches!(field("landscape"), "1" | "on" | "true"),
        description: field("description").to_string(),
        templated: true,
        ..Parts::default()
    };
    match cx
        .pdf
        .render_parts(parts, data, &RenderOptions::default())
        .await
    {
        Ok(pdf) => pdf.inline(),
        Err(e) => pane_message(&e.to_string()),
    }
}

/// A saved template rendered with its sample data.
async fn template_pdf(cx: Cx, id: i64) -> Response {
    let Ok(Some(template)) = store::template(&cx.db, None, Some(id)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let data = template.sample_data.clone();
    respond(
        cx.pdf
            .render_parts(Parts::from(&template), data, &RenderOptions::default())
            .await,
    )
}

/// A layout shown with a sample page of text, to judge the letterhead.
async fn layout_pdf(cx: Cx, id: i64) -> Response {
    let stored = match store::layout(&cx.db, Some(id)).await {
        Ok(l) => l,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let parts = Parts {
        code: "layout".into(),
        title: "Sample document".into(),
        body: "## A heading\n\nBody text in the layout's font and colour, with a [link](https://acme.com) and **strong** words.\n\n| Column | Column |\n| --- | --- |\n| Cell | Cell |\n\n> A quoted line.\n".into(),
        ..Parts::default()
    };
    respond(
        cx.pdf
            .render_in(parts, stored, Value::Null, &RenderOptions::default())
            .await,
    )
}

fn respond(result: Result<Rendered, RenderError>) -> Response {
    match result {
        Ok(pdf) => pdf.inline(),
        Err(RenderError::Db(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
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

/// The versions of a template or layout, newest first, each with what changed
/// from the one before and a Restore.
async fn history_page(
    cx: Cx,
    kind: &'static str,
    id: i64,
    shell: laterite_admin::Shell,
    may_restore: bool,
) -> Response {
    use crate::history::{self, escape};
    let table = history::table_of(kind);
    let revisions = match history::list(&cx.db, table, id).await {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let name = revisions
        .first()
        .and_then(|r| {
            r.snapshot
                .get(if kind == "templates" { "title" } else { "name" })
        })
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let mut body = String::from("<div class=\"rm-history\">");
    if revisions.is_empty() {
        body.push_str(&format!(
            "<p class=\"lat-note\">{}</p>",
            escape(&shell.t("Nothing saved here yet. Each save in the admin adds a version."))
        ));
    }
    for (i, rev) in revisions.iter().enumerate() {
        body.push_str(
            "<section class=\"lat-card rm-history__item\"><div class=\"rm-history__head\">",
        );
        body.push_str(&format!(
            "<span class=\"rm-history__when\">{}</span>",
            escape(&shell.datetime(&rev.created_at))
        ));
        if !rev.author.is_empty() {
            body.push_str(&format!(
                "<span class=\"rm-history__who\">{}</span>",
                escape(&rev.author)
            ));
        }
        if i == 0 {
            body.push_str(
                &laterite_admin::ui::Badge::new(t!("Current"))
                    .render(&shell)
                    .into_string(),
            );
        } else if may_restore {
            let href = format!("{}/{kind}/{id}/history/{}/restore", cx.tools(), rev.id);
            body.push_str(
                &laterite_admin::ui::Button::post(t!("Restore"), href)
                    .small()
                    .confirm(t!(
                        "Restore this version? The current one stays in the history."
                    ))
                    .render(&shell)
                    .into_string(),
            );
        }
        body.push_str("</div>");
        match revisions.get(i + 1) {
            Some(before) => {
                let changes = history::changes(table, &before.snapshot, &rev.snapshot);
                if changes.is_empty() {
                    body.push_str(&format!(
                        "<p class=\"rm-history__note\">{}</p>",
                        escape(&shell.t("Saved with no changes."))
                    ));
                } else {
                    body.push_str("<dl class=\"rm-history__changes\">");
                    for change in changes {
                        body.push_str(&format!(
                            "<dt>{}</dt><dd>{}</dd>",
                            escape(&shell.t(change.label)),
                            change.html
                        ));
                    }
                    body.push_str("</dl>");
                }
            }
            None => body.push_str(&format!(
                "<p class=\"rm-history__note\">{}</p>",
                escape(&shell.t("The earliest version kept."))
            )),
        }
        body.push_str("</section>");
    }
    body.push_str("</div>");
    let title = if name.is_empty() {
        t!("History")
    } else {
        laterite_core::Text::dynamic(format!("{}: {name}", shell.t("History")))
    };
    shell.page_with(
        laterite_admin::PageHead::new(title)
            .back(t!("Back"), cx.edit(kind, id))
            .assets(&[HISTORY_CSS]),
        laterite_admin::html::Markup::raw_unchecked(body),
    )
}

/// Puts a saved version back.
async fn restore(
    cx: Cx,
    kind: &'static str,
    id: i64,
    revision: i64,
    author: &str,
    session: SessionHandle,
    headers: HeaderMap,
) -> Response {
    let table = crate::history::table_of(kind);
    match crate::history::restore(&cx.db, table, id, revision, author).await {
        Ok(true) => {
            session.push_flash(
                FlashLevel::Success,
                t!("Restored. The version you replaced stays in the history."),
            );
            redirect(cx.edit(kind, id), &headers)
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
