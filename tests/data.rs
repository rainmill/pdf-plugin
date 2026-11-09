//! Data and output: QR codes and barcodes, image sizes, metadata, password
//! protection, and the `Pdf` handle's variables, render hooks, locales and
//! output helpers.

use std::sync::Arc;

use laterite_core::{Db, Module, ModuleId, Registry};
use laterite_media::{LocalDisk, Owner};
use rainmill_pdf::registry::Format;
use rainmill_pdf::{
    markdown_source, render_typst, store, Assets, DocumentMeta, Layout, Output, Pdf, PdfError,
    PdfRenderHookReg, PdfTemplateReg, PdfVariableReg, RenderError, RenderHook, RenderOptions,
};
use serde_json::{json, Value};

fn meta() -> DocumentMeta {
    DocumentMeta {
        title: "Ticket".into(),
        ..DocumentMeta::default()
    }
}

/// The text a PDF shows, read back through lopdf.
fn text_of(pdf: &[u8]) -> String {
    let doc = lopdf::Document::load_mem(pdf).expect("a readable PDF");
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    doc.extract_text(&pages).unwrap_or_default()
}

#[test]
fn typst_templates_draw_qr_codes_and_barcodes() {
    let source = "#qr(\"https://acme.com/t/42\", width: 3cm)\n#barcode(\"750103131130\", kind: \"ean13\", width: 5cm)\n#barcode(sys.inputs.ref)";
    let mut assets = Assets::default();
    assets.inputs.insert("ref".into(), json!("ACME-42"));
    let pdf = render_typst(source, &meta(), &assets, &Output::default()).expect("renders");
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn a_value_a_barcode_cannot_hold_is_a_compile_error() {
    let err = render_typst(
        "Fine.\n#barcode(\"letters\", kind: \"ean13\")",
        &meta(),
        &Assets::default(),
        &Output::default(),
    )
    .expect_err("an EAN-13 holds digits");
    match err {
        PdfError::Compile(messages) => assert!(
            messages
                .iter()
                .any(|m| m.contains("cannot hold") && m.starts_with("line 2:")),
            "{messages:?}"
        ),
        other => panic!("expected a compile error, got {other}"),
    }
}

#[test]
fn markdown_codes_and_image_titles_become_sized_images() {
    let filled = rainmill_pdf::document::fill(
        "{{ qr(url, width=\"25mm\") }}\n\n{{ barcode(sku) }}\n\n![Logo](/logo.png \"width=40%, height=2cm\")",
        &json!({ "url": "https://acme.com", "sku": "A-1" }),
    )
    .expect("fills");
    let mut assets = Assets::default();
    assets.files.push(("/logo.png".into(), Vec::new()));
    let source = markdown_source(&filled, &Layout::default(), &meta(), &assets);
    assert!(
        source.contains("/_pdf/qr/") && source.contains("width: 25mm"),
        "{source}"
    );
    assert!(source.contains("/_pdf/barcode/code128/") && source.contains("width: 60mm"));
    assert!(source.contains("#image(\"/logo.png\", width: 40%, height: 2cm)"));
}

#[test]
fn metadata_is_written_into_the_pdf() {
    let meta = DocumentMeta {
        title: "Invoice 2026/1".into(),
        author: "Acme".into(),
        description: "An invoice for one order".into(),
        keywords: vec!["invoice".into(), "acme-order".into()],
        ..DocumentMeta::default()
    };
    let pdf =
        render_typst("Hello.", &meta, &Assets::default(), &Output::archival()).expect("renders");
    let raw = String::from_utf8_lossy(&pdf);
    assert!(
        raw.contains("acme-order"),
        "the keywords are in the XMP packet"
    );
    assert!(raw.contains("An invoice for one order"));
}

#[test]
fn a_protected_pdf_opens_only_with_its_password() {
    let pdf = render_typst(
        "Private words.",
        &meta(),
        &Assets::default(),
        &Output::protected("s3cret"),
    )
    .expect("renders");
    assert!(String::from_utf8_lossy(&pdf).contains("/Encrypt"));
    assert!(
        lopdf::Document::load_mem_with_options(&pdf, lopdf::LoadOptions::with_password("wrong"))
            .is_err_and(|_| true)
            || lopdf::Document::load_mem_with_options(
                &pdf,
                lopdf::LoadOptions::with_password("wrong")
            )
            .is_ok_and(|d| d.is_encrypted())
    );
    let opened =
        lopdf::Document::load_mem_with_options(&pdf, lopdf::LoadOptions::with_password("s3cret"))
            .expect("the password opens it");
    assert!(!opened.is_encrypted());
    let pages: Vec<u32> = opened.get_pages().keys().copied().collect();
    assert!(opened
        .extract_text(&pages)
        .unwrap_or_default()
        .contains("Private"));
}

#[test]
fn archival_and_a_password_cannot_be_combined() {
    let output = Output {
        archival: true,
        password: Some("x".into()),
    };
    let err = render_typst("Hi.", &meta(), &Assets::default(), &output).expect_err("refused");
    assert!(matches!(err, PdfError::Protect(_)), "{err}");
}

struct Stamp;

impl RenderHook for Stamp {
    fn data(&self, _code: &str, mut data: Value) -> Value {
        data["stamp"] = json!("DRAFT");
        data
    }

    fn source(&self, _code: &str, source: String) -> String {
        format!("{source}\n\nHooked footer.")
    }
}

async fn setup(
    extra: impl FnOnce(&mut Registry),
) -> (Db, laterite_core::testing::TestGuard, Registry) {
    let mut migrations = laterite_admin::builtin_migrations();
    migrations.push(rainmill_pdf::migrations::migrations());
    let (db, guard) = laterite_core::testing::connect_test(&migrations).await;
    let mut registry = Registry::new();
    registry.set_owner(rainmill_pdf::PdfModule.id());
    rainmill_pdf::PdfModule.register(&mut registry);
    registry.set_owner(ModuleId::new("acme"));
    registry.add(
        PdfTemplateReg::new(
            "acme.letter",
            "Letter",
            Format::Markdown,
            "Dear {{ name }} of {{ company.name }}, {{ stamp }}.",
        )
        .description("A letter to a customer")
        .keywords(["letter"])
        .translation("pt", "Carta", "Caro {{ name }}, {{ stamp }}.")
        .translation("pt-BR", "", "Prezado {{ name }}, {{ stamp }}."),
    );
    registry.add(PdfVariableReg::new("company", json!({ "name": "Acme" })));
    registry.add(PdfVariableReg::new("name", json!("Nobody")));
    extra(&mut registry);
    let layouts = registry.items::<rainmill_pdf::PdfLayoutReg>();
    let templates = registry.items::<PdfTemplateReg>();
    store::sync(&db, &layouts, &templates).await.unwrap();
    (db, guard, registry)
}

#[tokio::test]
async fn variables_fill_in_and_the_call_data_wins() {
    let (db, _guard, registry) = setup(|_| {}).await;
    let pdf = Pdf::new(db, &registry);
    let out = pdf
        .render("acme.letter", json!({ "name": "Ada" }))
        .await
        .unwrap();
    let text = text_of(&out.bytes);
    assert!(text.contains("Ada") && text.contains("Acme"), "{text}");
    assert!(!text.contains("Nobody"));
    assert_eq!(out.filename, "letter.pdf");
}

#[tokio::test]
async fn hooks_change_the_data_and_the_source_of_their_template() {
    let (db, _guard, registry) = setup(|r| {
        r.add(PdfRenderHookReg::new(Arc::new(Stamp)).only("acme.letter"));
        r.add(PdfRenderHookReg::new(Arc::new(Stamp)).only("acme.other"));
    })
    .await;
    let out = Pdf::new(db, &registry)
        .render("acme.letter", json!({ "name": "Ada" }))
        .await
        .unwrap();
    let text = text_of(&out.bytes);
    assert!(
        text.contains("DRAFT") && text.contains("Hooked footer"),
        "{text}"
    );
    assert_eq!(
        text.matches("Hooked footer").count(),
        1,
        "the other code's hook did not run"
    );
}

#[tokio::test]
async fn a_locale_falls_back_through_its_language_to_the_template() {
    let (db, _guard, registry) = setup(|_| {}).await;
    let pdf = Pdf::new(db, &registry);
    let br = pdf
        .render_with(
            "acme.letter",
            json!({ "name": "Ada" }),
            &RenderOptions::locale("pt-BR"),
        )
        .await
        .unwrap();
    assert!(text_of(&br.bytes).contains("Prezado"));
    // An empty translated title keeps the template's own.
    assert_eq!(br.filename, "letter.pdf");
    let pt = pdf
        .render_with(
            "acme.letter",
            json!({ "name": "Ada" }),
            &RenderOptions::locale("pt-PT"),
        )
        .await
        .unwrap();
    assert!(text_of(&pt.bytes).contains("Caro"));
    assert_eq!(pt.filename, "carta.pdf");
    let de = pdf
        .render_with(
            "acme.letter",
            json!({ "name": "Ada" }),
            &RenderOptions::locale("de"),
        )
        .await
        .unwrap();
    assert!(text_of(&de.bytes).contains("Dear"));
}

#[tokio::test]
async fn an_unknown_code_is_not_found() {
    let (db, _guard, registry) = setup(|_| {}).await;
    let err = Pdf::new(db, &registry)
        .render("acme.none", Value::Null)
        .await
        .unwrap_err();
    assert!(matches!(err, RenderError::NotFound(_)));
}

#[tokio::test]
async fn a_rendered_pdf_is_stored_attached_and_served() {
    let (db, _guard, registry) = setup(|_| {}).await;
    let dir = std::env::temp_dir().join(format!("rainmill-pdf-{}", std::process::id()));
    let pdf = Pdf::new(db.clone(), &registry).disk("public", Arc::new(LocalDisk::new(dir.clone())));
    let out = pdf
        .render("acme.letter", json!({ "name": "Ada" }))
        .await
        .unwrap();

    let attachment = out.attachment();
    assert_eq!(attachment.content_type, "application/pdf");
    assert_eq!(attachment.filename, "letter.pdf");

    let record = out
        .store(
            &pdf,
            "public",
            Some(Owner {
                owner_type: "acme_orders",
                owner_id: "7",
                field: "invoice",
            }),
        )
        .await
        .unwrap();
    assert_eq!(record.content_type, "application/pdf");
    assert!(
        !record.in_library,
        "an attached PDF stays out of the library"
    );
    assert_eq!(pdf.read(&record).await.unwrap(), out.bytes);
    let links = laterite_media::link::of_owner(
        &db,
        Owner {
            owner_type: "acme_orders",
            owner_id: "7",
            field: "invoice",
        },
    )
    .await
    .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].media_id, record.id);

    let response = out.download();
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    assert_eq!(disposition, "attachment; filename=\"letter.pdf\"");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_layout_logo_comes_from_the_media_library() {
    use laterite_media::{ingest, record, stream_of, NewMedia};
    let (db, _guard, registry) = setup(|r| {
        r.add(rainmill_pdf::PdfLayoutReg::new(
            "acme.letterhead",
            "Letterhead",
            Layout::default(),
        ));
        r.add(
            PdfTemplateReg::new("acme.logo", "With logo", Format::Markdown, "Body.")
                .layout("acme.letterhead"),
        );
    })
    .await;
    let dir = std::env::temp_dir().join(format!("rainmill-pdf-logo-{}", std::process::id()));
    let disk = Arc::new(LocalDisk::new(dir.clone()));
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="10"><rect width="40" height="10" fill="#e5bf02"/></svg>"##;
    let stored = ingest(disk.as_ref(), stream_of(svg.to_vec()), 1 << 20)
        .await
        .unwrap();
    let id = record::create(
        &db,
        NewMedia {
            disk: "public".into(),
            hash: stored.hash,
            size: stored.size as i64,
            content_type: stored.content_type,
            original_name: "logo.svg".into(),
            in_library: true,
            created_by: None,
        },
    )
    .await
    .unwrap();
    let (sql, values) = laterite_core::query::build(
        db.backend,
        sea_query::Query::update()
            .table(sea_query::Alias::new("pdf_layouts"))
            .value(sea_query::Alias::new("logo_id"), id)
            .and_where(sea_query::Expr::col(sea_query::Alias::new("code")).eq("acme.letterhead"))
            .to_owned(),
    );
    laterite_core::query::bind_values(sqlx::query(&sql), values)
        .execute(&db.pool)
        .await
        .unwrap();

    let pdf = Pdf::new(db, &registry).disk("public", disk);
    let with = pdf.render("acme.logo", Value::Null).await.unwrap();
    // The logo is drawn: the PDF carries a filled path the plain layout lacks.
    let plain = rainmill_pdf::render_body(
        Format::Markdown,
        "Body.",
        &Layout::default(),
        &DocumentMeta {
            title: "With logo".into(),
            ..DocumentMeta::default()
        },
        &Value::Null,
        &Output::default(),
    )
    .unwrap();
    assert_ne!(
        with.bytes.len(),
        plain.len(),
        "the logo changes the document"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_document_renders_verbatim_with_its_version_in_the_masthead() {
    let (db, _guard, registry) = setup(|_| {}).await;
    let meta = DocumentMeta {
        title: "Privacy Policy".into(),
        version: "2.1".into(),
        effective: "9 November 2026".into(),
        ..DocumentMeta::default()
    };
    let out = Pdf::new(db, &registry)
        .render_document("Write {{ to }} us.", None, meta, &RenderOptions::default())
        .await
        .unwrap();
    let text = text_of(&out.bytes);
    assert!(
        text.contains("{{ to }}") || text.contains("{{to}}"),
        "taken verbatim: {text}"
    );
    assert!(
        text.contains("2.1"),
        "the version is in the masthead: {text}"
    );
    assert_eq!(out.filename, "privacy-policy.pdf");
}
