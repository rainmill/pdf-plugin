//! `lat pdf`, run through its registration as the application binary runs it:
//! check, render, and an export read back by import.

use laterite_core::{CommandCx, CommandReg, Module, ModuleId, Registry};
use rainmill_pdf::registry::Format;
use rainmill_pdf::{store, Layout, PdfLayoutReg, PdfModule, PdfTemplateReg};

async fn setup(
    body: &str,
) -> (
    laterite_core::Db,
    laterite_core::testing::TestGuard,
    Registry,
) {
    let mut migrations = laterite_admin::builtin_migrations();
    migrations.push(rainmill_pdf::migrations::migrations());
    let (db, guard) = laterite_core::testing::connect_test(&migrations).await;
    let mut registry = Registry::new();
    registry.set_owner(PdfModule.id());
    PdfModule.register(&mut registry);
    registry.set_owner(ModuleId::new("acme"));
    registry
        .add(PdfLayoutReg::new("acme.letterhead", "Letterhead", Layout::default()).author("Acme"));
    registry.add(
        PdfTemplateReg::new("acme.letter", "Letter", Format::Markdown, body)
            .layout("acme.letterhead")
            .keywords(["letter"])
            .sample(serde_json::json!({ "name": "Ada" }))
            .translation("de", "Brief", "Liebe {{ name }}."),
    );
    let layouts = registry.items::<PdfLayoutReg>();
    let templates = registry.items::<PdfTemplateReg>();
    store::sync(&db, &layouts, &templates).await.unwrap();
    (db, guard, registry)
}

async fn pdf(db: &laterite_core::Db, registry: &Registry, args: &[&str]) -> Result<(), String> {
    let reg = registry
        .items::<CommandReg>()
        .into_iter()
        .find(|r| r.name == "pdf")
        .expect("the plugin registers `pdf`");
    let dir = std::env::temp_dir();
    let cx = CommandCx::new(db, registry, dir.to_str().unwrap());
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    reg.command.run(&cx, &args).await.map_err(|e| e.to_string())
}

#[tokio::test]
async fn check_reports_a_template_that_does_not_render() {
    let (db, _guard, registry) = setup("Dear {{ name }}.").await;
    pdf(&db, &registry, &["check"]).await.expect("renders");
    let (db, _guard, registry) = setup("Dear {{ name ").await;
    let err = pdf(&db, &registry, &["check"]).await.unwrap_err();
    assert!(err.contains("1 templates did not render"), "{err}");
}

#[tokio::test]
async fn render_writes_the_pdf_where_asked() {
    let (db, _guard, registry) = setup("Dear {{ name }}.").await;
    let out = std::env::temp_dir().join(format!("rainmill-pdf-cmd-{}.pdf", std::process::id()));
    pdf(
        &db,
        &registry,
        &[
            "render",
            "acme.letter",
            "--locale",
            "de",
            "--out",
            out.to_str().unwrap(),
        ],
    )
    .await
    .unwrap();
    assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF"));
    let _ = std::fs::remove_file(out);
    assert!(pdf(&db, &registry, &["render"])
        .await
        .unwrap_err()
        .contains("usage"));
}

#[tokio::test]
async fn an_export_imports_back_over_later_changes() {
    let (db, _guard, registry) = setup("Dear {{ name }}.").await;
    let dir = std::env::temp_dir().join(format!("rainmill-pdf-export-{}", std::process::id()));
    pdf(&db, &registry, &["export", dir.to_str().unwrap()])
        .await
        .unwrap();
    let text = std::fs::read_to_string(dir.join("templates/acme.letter.md")).unwrap();
    assert!(
        text.starts_with("---\n") && text.contains("layout: acme.letterhead"),
        "{text}"
    );
    assert!(text.trim_end().ends_with("Dear {{ name }}."));
    assert!(dir.join("templates/acme.letter/de.md").is_file());
    assert!(dir.join("layouts/acme.letterhead.yaml").is_file());

    // Change the template, then import the export over it.
    let changed = PdfTemplateReg::new("acme.letter", "Letter", Format::Markdown, "Changed.");
    store::import(&db, &[], &[changed]).await.unwrap();
    let before = store::template(&db, Some("acme.letter"), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.body, "Changed.");
    pdf(&db, &registry, &["import", dir.to_str().unwrap()])
        .await
        .unwrap();
    let after = store::template(&db, Some("acme.letter"), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.body, "Dear {{ name }}.");
    assert_eq!(after.keywords, ["letter"]);
    assert!(after.layout_id.is_some(), "its layout found again by code");
    let _ = std::fs::remove_dir_all(dir);
}
