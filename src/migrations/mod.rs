//! This plugin's migrations, in apply order.

laterite_core::migration_set! {
    module_id: "rainmill.pdf",
    m0001_create_pdf,
    m0002_add_media_metadata_translations,
    m0003_create_pdf_revisions,
}
