//! The site side: a theme's `pdf_url` link renders the template it names, a
//! tampered link renders nothing, the component offers a Download link, and a
//! registered recorder captures renders without compiling.

use laterite_core::minijinja;
use laterite_core::serde_json;
use std::sync::Arc;

use laterite_admin::axum::body::Body;
use laterite_admin::axum::http::{Request, StatusCode};
use laterite_admin::routes::{PublicRouteReg, RouteCtx};
use laterite_core::{Module, ModuleId, Registry};
use laterite_web::theme::component::{ComponentContext, ComponentReg};
use laterite_web::theme::markup::MarkupReg;
use rainmill_pdf::registry::Format;
use rainmill_pdf::testing::{Recorder, STUB};
use rainmill_pdf::{store, Pdf, PdfModule, PdfTemplateReg};
use serde_json::json;
use tower::ServiceExt;

async fn setup(
    extra: impl FnOnce(&mut Registry),
) -> (
    laterite_core::Db,
    laterite_core::testing::TestGuard,
    Arc<Registry>,
) {
    let mut migrations = laterite_admin::builtin_migrations();
    migrations.push(rainmill_pdf::migrations::migrations());
    let (db, guard) = laterite_core::testing::connect_test(&migrations).await;
    let mut registry = Registry::new();
    registry.set_owner(PdfModule.id());
    PdfModule.register(&mut registry);
    registry.set_owner(ModuleId::new("acme"));
    registry.add(PdfTemplateReg::new(
        "acme.ticket",
        "Ticket",
        Format::Markdown,
        "Ticket for {{ name }}.",
    ));
    extra(&mut registry);
    let templates = registry.items::<PdfTemplateReg>();
    store::sync(&db, &[], &templates).await.unwrap();
    (db, guard, Arc::new(registry))
}

/// The site route mounted as the application mounts it, and the theme engine
/// with the plugin's markup installed.
fn site(
    db: &laterite_core::Db,
    registry: &Arc<Registry>,
) -> (
    laterite_admin::axum::Router,
    minijinja::Environment<'static>,
) {
    let reg = registry
        .items::<PublicRouteReg>()
        .into_iter()
        .find(|r| r.path == "/rainmill/pdf")
        .expect("the plugin registers its route");
    let ctx = RouteCtx::builder(db.clone())
        .base_path("/rainmill/pdf")
        .contributions(registry.clone())
        .build();
    let router =
        laterite_admin::axum::Router::new().nest_service("/rainmill/pdf", reg.route.mount(&ctx));
    let mut env = minijinja::Environment::new();
    for m in registry.items::<MarkupReg>() {
        m.markup.install(&mut env);
    }
    (router, env)
}

async fn get(router: &laterite_admin::axum::Router, path: &str) -> (StatusCode, String, Vec<u8>) {
    let resp = router
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let kind = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let body = laterite_admin::axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, kind, body)
}

#[tokio::test]
async fn a_theme_link_renders_its_template_and_a_tampered_one_nothing() {
    let (db, _guard, registry) = setup(|_| {}).await;
    let (router, env) = site(&db, &registry);
    let url = env
        .render_str("{{ pdf_url('acme.ticket', data={'name': 'Ada'}) }}", ())
        .unwrap();
    assert!(url.starts_with("/rainmill/pdf/"), "{url}");
    let (status, kind, body) = get(&router, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind, "application/pdf");
    assert!(body.starts_with(b"%PDF"));

    let mut tampered = url.clone();
    tampered.push('x');
    assert_eq!(get(&router, &tampered).await.0, StatusCode::NOT_FOUND);
    let other = env.render_str("{{ pdf_url('acme.none') }}", ()).unwrap();
    assert_eq!(
        get(&router, &other).await.0,
        StatusCode::NOT_FOUND,
        "an unknown code"
    );
}

#[tokio::test]
async fn the_component_links_the_template_with_the_page_parameters() {
    let (db, _guard, registry) = setup(|_| {}).await;
    let (router, _env) = site(&db, &registry);
    let component = registry
        .items::<ComponentReg>()
        .into_iter()
        .find(|c| c.code == "rainmill.pdf")
        .expect("the plugin registers its component")
        .component
        .clone();
    let properties = json!({ "template": "acme.ticket", "download": true })
        .as_object()
        .unwrap()
        .clone();
    let params = std::collections::BTreeMap::from([("name".to_string(), "Ada".to_string())]);
    let cx = ComponentContext {
        db: &db,
        properties: &properties,
        params: &params,
        path: "/tickets/ada",
        theme: "demo",
        preview: false,
    };
    let out = component.run(&cx).await.unwrap();
    let url = out.data["url"].as_str().unwrap().to_string();
    assert_eq!(out.data["download"], true);
    let resp = router
        .clone()
        .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()["content-disposition"].to_str().unwrap(),
        "attachment; filename=\"ticket.pdf\""
    );
}

#[tokio::test]
async fn a_recorder_captures_renders_without_compiling() {
    let recorder = Recorder::new();
    let held = recorder.clone();
    let (db, _guard, registry) = setup(move |r| r.add(held)).await;
    recorder.assert_nothing_rendered();
    let rendered = Pdf::new(db, &registry)
        .render("acme.ticket", json!({ "name": "Ada" }))
        .await
        .unwrap();
    assert_eq!(rendered.bytes, STUB);
    recorder.assert_rendered("acme.ticket");
    recorder.assert_rendered_with("acme.ticket", |data| data["name"] == "Ada");
    assert_eq!(recorder.renders().len(), 1);
}
