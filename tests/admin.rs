//! The admin through the real router: registered templates synced at boot, the
//! customised flag, reset and duplicate, the live preview, and the guards.

use std::sync::Arc;

use laterite_admin::axum::body::Body;
use laterite_admin::axum::http::{Request, StatusCode};
use laterite_admin::axum::Router;
use laterite_admin::{router, AdminConfig, Contributions};
use laterite_auth::{AuthConfig, AuthService, NewOperator, RequestContext};
use laterite_core::{CatalogStore, Db, Module, ModuleId, Registry};
use rainmill_pdf::registry::Format;
use rainmill_pdf::{store, Layout, PdfLayoutReg, PdfModule, PdfTemplateReg};
use tower::ServiceExt;

const BASE: &str = "/rainmill/pdf";

fn shipped() -> (PdfLayoutReg, PdfTemplateReg) {
    (
        PdfLayoutReg::new(
            "acme.letterhead",
            "Acme letterhead",
            Layout {
                accent: "#e5bf02".into(),
                ..Layout::default()
            },
        ),
        PdfTemplateReg::new(
            "acme.invoice",
            "Invoice",
            Format::Markdown,
            "# Invoice {{ number }}\n\nThank you.",
        )
        .layout("acme.letterhead")
        .sample(serde_json::json!({ "number": "2026/1" })),
    )
}

async fn setup() -> (Db, laterite_core::testing::TestGuard, Registry) {
    let mut migrations = laterite_admin::builtin_migrations();
    migrations.push(rainmill_pdf::migrations::migrations());
    let (db, guard) = laterite_core::testing::connect_test(&migrations).await;
    let mut registry = Registry::new();
    registry.set_owner(PdfModule.id());
    PdfModule.register(&mut registry);
    registry.set_owner(ModuleId::new("acme"));
    let (layout, template) = shipped();
    registry.add(layout);
    registry.add(template);
    let layouts = registry.items::<PdfLayoutReg>();
    let templates = registry.items::<PdfTemplateReg>();
    store::sync(&db, &layouts, &templates).await.unwrap();
    (db, guard, registry)
}

fn app(db: &Db, mut registry: Registry) -> Router {
    use laterite_admin::picker::PickerSourceReg;
    use laterite_admin::routes::ScreenReg;
    use laterite_admin::{Permission, Resource};
    let mut resources = registry.take::<Resource>();
    for r in &mut resources {
        r.base_path = format!("{BASE}{}", r.base_path);
        if let Some(form) = r.form.as_mut() {
            form.base_path = r.base_path.clone();
        }
    }
    let mut screens = registry.take::<ScreenReg>();
    for s in &mut screens {
        s.base_path = format!("{BASE}{}", s.base_path);
    }
    let permissions = registry.take::<Permission>();
    let picker_sources = registry.take::<PickerSourceReg>();
    let listeners = registry.take::<laterite_core::ModelListenerReg>();
    router(
        AuthService::new(db.clone(), AuthConfig::default()),
        db.clone(),
        Contributions {
            resources,
            screens,
            permissions,
            picker_sources,
            listeners,
            plugin_defined: Arc::new(registry),
            ..Default::default()
        },
        AdminConfig::default(),
        Arc::new(CatalogStore::default()),
    )
}

async fn superuser(db: &Db) -> String {
    let svc = AuthService::new(db.clone(), AuthConfig::default());
    svc.create_superuser(NewOperator {
        username: "root",
        email: "root@acme.test",
        first_name: "Root",
        last_name: None,
        password: "rootpw12345",
        timezone: None,
    })
    .await
    .unwrap();
    svc.authenticate("root", "rootpw12345", &RequestContext::default())
        .await
        .unwrap()
        .token
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Vec<(String, String)>, Vec<u8>) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = laterite_admin::axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, body)
}

fn get(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header("cookie", format!("laterite_session={token}"))
        .body(Body::empty())
        .unwrap()
}

fn post(path: &str, token: &str, csrf: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("cookie", format!("laterite_session={token}"))
        .header("sec-fetch-site", "same-origin")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("_csrf={csrf}&{body}")))
        .unwrap()
}

fn csrf_of(html: &str) -> String {
    html.split("name=\"_csrf\" value=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap()
        .to_string()
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> &'a str {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

#[tokio::test]
async fn a_registered_template_is_synced_editable_and_resettable() {
    let (db, _guard, registry) = setup().await;
    let token = superuser(&db).await;
    let template = store::template(&db, Some("acme.invoice"), None)
        .await
        .unwrap()
        .expect("synced");
    assert!(
        template.layout_id.is_some(),
        "its layout is resolved by code"
    );
    let app = app(&db, registry);

    let (status, _, html) = send(
        app.clone(),
        get(
            &format!("/admin{BASE}/templates/{}/edit", template.id),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(html).unwrap();
    assert!(
        html.contains("data-layout=\"side\"")
            && html.contains("data-preview=\"/admin/rainmill/pdf/tools/preview\""),
        "the split editor previews through the plugin"
    );
    assert!(
        html.contains("Reset to default")
            && html.contains("Duplicate")
            && html.contains("Download PDF"),
        "the actions show"
    );
    let csrf = csrf_of(&html);

    // An admin edit marks it customised; the boot sync then leaves it alone.
    let (status, _, _) = send(
        app.clone(),
        post(
            &format!("/admin{BASE}/templates/{}/edit", template.id),
            &token,
            &csrf,
            &format!("title=Our+invoice&code=acme.invoice&format=markdown&layout_id={}&body=Edited&sample_data=%7B%7D", template.layout_id.unwrap()),
        ),
    )
    .await;
    assert!(
        status.is_redirection() || status == StatusCode::NO_CONTENT || status == StatusCode::OK,
        "{status}"
    );
    let after = store::template(&db, Some("acme.invoice"), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.body, "Edited");
    let (layout, shipped_template) = shipped();
    store::sync(&db, &[&layout], &[&shipped_template])
        .await
        .unwrap();
    assert_eq!(
        store::template(&db, Some("acme.invoice"), None)
            .await
            .unwrap()
            .unwrap()
            .body,
        "Edited",
        "customised survives a boot"
    );

    // Reset brings back what the module ships.
    let (status, headers, _) = send(
        app.clone(),
        post(
            &format!("/admin{BASE}/tools/templates/{}/reset", template.id),
            &token,
            &csrf,
            "",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(header(&headers, "location").ends_with(&format!("/templates/{}/edit", template.id)));
    let reset = store::template(&db, Some("acme.invoice"), None)
        .await
        .unwrap()
        .unwrap();
    assert!(reset.body.starts_with("# Invoice"), "{}", reset.body);
}

#[tokio::test]
async fn the_code_never_changes_and_a_copy_gets_a_free_one() {
    let (db, _guard, registry) = setup().await;
    let token = superuser(&db).await;
    let template = store::template(&db, Some("acme.invoice"), None)
        .await
        .unwrap()
        .unwrap();
    let app = app(&db, registry);
    let (_, _, html) = send(
        app.clone(),
        get(
            &format!("/admin{BASE}/templates/{}/edit", template.id),
            &token,
        ),
    )
    .await;
    let csrf = csrf_of(&String::from_utf8(html).unwrap());

    let (status, _, body) = send(
        app.clone(),
        post(
            &format!("/admin{BASE}/templates/{}/edit", template.id),
            &token,
            &csrf,
            "title=Invoice&code=acme.renamed&format=markdown&body=x",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8(body)
        .unwrap()
        .contains("The code cannot change"));

    let (status, headers, _) = send(
        app.clone(),
        post(
            &format!("/admin{BASE}/tools/templates/{}/duplicate", template.id),
            &token,
            &csrf,
            "",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let copy = store::template(&db, Some("acme.invoice_copy"), None)
        .await
        .unwrap()
        .expect("a copy");
    assert!(header(&headers, "location").ends_with(&format!("/templates/{}/edit", copy.id)));
    // A copy has no default: resetting it says so rather than doing anything.
    let (status, _, _) = send(
        app.clone(),
        post(
            &format!("/admin{BASE}/tools/templates/{}/reset", copy.id),
            &token,
            &csrf,
            "",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn the_preview_renders_the_unsaved_form_and_explains_a_failure() {
    let (db, _guard, registry) = setup().await;
    let token = superuser(&db).await;
    let template = store::template(&db, Some("acme.invoice"), None)
        .await
        .unwrap()
        .unwrap();
    let app = app(&db, registry);
    let (_, _, html) = send(
        app.clone(),
        get(
            &format!("/admin{BASE}/templates/{}/edit", template.id),
            &token,
        ),
    )
    .await;
    let csrf = csrf_of(&String::from_utf8(html).unwrap());

    let (status, headers, body) = send(
        app.clone(),
        post(&format!("/admin{BASE}/tools/preview"), &token, &csrf, "format=markdown&title=Invoice&sample_data=%7B%22number%22%3A%227%22%7D&text=%23+Invoice+%7B%7B+number+%7D%7D"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
    assert!(body.starts_with(b"%PDF-"));

    let (_, headers, body) = send(
        app.clone(),
        post(
            &format!("/admin{BASE}/tools/preview"),
            &token,
            &csrf,
            "format=markdown&sample_data=%7Bnot+json&text=x",
        ),
    )
    .await;
    assert!(header(&headers, "content-type").starts_with("text/html"));
    assert!(String::from_utf8(body).unwrap().contains("not valid JSON"));

    let (status, headers, _) = send(
        app.clone(),
        get(
            &format!("/admin{BASE}/tools/templates/{}/pdf", template.id),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
}

#[tokio::test]
async fn a_layout_in_use_cannot_be_deleted() {
    let (db, _guard, registry) = setup().await;
    let token = superuser(&db).await;
    let template = store::template(&db, Some("acme.invoice"), None)
        .await
        .unwrap()
        .unwrap();
    let layout_id = template.layout_id.unwrap();
    assert_eq!(store::templates_using(&db, layout_id).await.unwrap(), 1);
    let app = app(&db, registry);
    let (_, _, html) = send(
        app.clone(),
        get(&format!("/admin{BASE}/layouts/{layout_id}/edit"), &token),
    )
    .await;
    let csrf = csrf_of(&String::from_utf8(html).unwrap());
    let (status, _, _) = send(
        app.clone(),
        post(
            &format!("/admin{BASE}/layouts/delete"),
            &token,
            &csrf,
            &format!("id={layout_id}"),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::NOT_FOUND, "the delete route exists");
    let rows: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM pdf_layouts WHERE id = {layout_id}"
    ))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows, 1, "the layout a template is set in is still there");
    let (status, headers, _) = send(
        app.clone(),
        get(
            &format!("/admin{BASE}/tools/layouts/{layout_id}/pdf"),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
}
