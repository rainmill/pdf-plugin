//! Rainmill PDF: documents rendered to PDF with [Typst](https://typst.app), for
//! Laterite applications.
//!
//! Write the document as markdown and set it in a [`Layout`] (the letterhead:
//! colours, fonts, page, running header and footer, logo), or write Typst
//! directly for full control. Rendering runs in a sealed world: the bundled
//! fonts, the files and fonts you hand over, and nothing else, so a template
//! reaches no disk and no network.
//!
//! ```no_run
//! use rainmill_pdf::{render_markdown, Assets, DocumentMeta, Layout, Output};
//!
//! let meta = DocumentMeta {
//!     title: "Terms of Use".into(),
//!     version: "1.0".into(),
//!     effective: "9 November 2026".into(),
//!     ..DocumentMeta::default()
//! };
//! let pdf = render_markdown("# Who we are\n\nAcme runs this site.", &Layout::default(),
//!     &meta, &Assets::default(), &Output::archival())?;
//! # Ok::<(), rainmill_pdf::PdfError>(())
//! ```
//!
//! Stored templates render through [`Pdf`], with the registered variables,
//! render hooks and media disks; [`site`] puts them on the site, [`history`]
//! keeps their saves, and [`testing::Recorder`] stands in for rendering in an
//! application's tests.
#![warn(missing_docs)]

pub mod admin;
pub mod codes;
mod command;
pub mod document;
pub mod history;
mod layout;
mod listener;
pub mod markdown;
pub mod migrations;
mod pdf;
mod protect;
pub mod registry;
mod render;
pub mod site;
pub mod store;
pub mod testing;
mod world;

use std::sync::Arc;

use laterite_core::strata::async_trait;
use laterite_core::{
    BootCx, CoreResult, MigrationSet, ModelListenerReg, Module, ModuleId, Registry,
};

pub use document::{render_body, RenderError};
pub use layout::{DocumentMeta, Layout};
pub use pdf::{Pdf, RenderOptions, Rendered};
pub use registry::{
    Format, PdfFontReg, PdfLayoutReg, PdfRenderHookReg, PdfTemplateReg, PdfVariableReg, RenderHook,
};
pub use render::{markdown_source, render_markdown, render_typst, Assets, Output, PdfError};

/// This plugin's entry point, collected by the generated plugins manifest.
pub fn module() -> Box<dyn Module> {
    Box::new(PdfModule)
}

/// The Rainmill PDF plugin: layouts and templates in the admin, and rendering.
///
/// A module ships its own with `registry.add(PdfTemplateReg::new(..))` and
/// `registry.add(PdfLayoutReg::new(..))`; they are written into the database
/// at boot.
pub struct PdfModule;

#[async_trait]
impl Module for PdfModule {
    fn id(&self) -> ModuleId {
        ModuleId::new(migrations::MODULE_ID)
    }

    fn migrations(&self) -> MigrationSet {
        migrations::migrations()
    }

    fn register(&self, registry: &mut Registry) {
        use laterite_admin::AdminRegistry;
        for permission in admin::permissions() {
            registry.add_permission(permission);
        }
        registry.add_nav(admin::section());
        for resource in admin::resources() {
            registry.add_resource(resource);
        }
        registry.add_picker_source(admin::layout_source());
        registry.add_screen(admin::screen());
        registry.add_asset(admin::HISTORY_CSS, admin::history_css());
        registry.add(command::registration());
        let signer = site::Signer::default();
        registry.add(laterite_admin::routes::PublicRouteReg::new(
            site::PATH,
            Arc::new(site::PdfRoute(signer.clone())),
        ));
        registry.add(laterite_web::theme::markup::MarkupReg::new(
            "rainmill.pdf",
            Arc::new(site::PdfMarkup(signer.clone())),
        ));
        registry.add(laterite_web::theme::component::ComponentReg::new(
            "rainmill.pdf",
            "PDF download",
            Arc::new(site::PdfComponent(signer)),
        ));
        for entity in [store::TEMPLATES, store::LAYOUTS] {
            registry.add(ModelListenerReg::for_entity(
                entity,
                Arc::new(listener::Customise),
            ));
        }
        registry.add(ModelListenerReg::for_entity(
            store::LAYOUTS,
            Arc::new(listener::LayoutInUse),
        ));
        registry.add(ModelListenerReg::for_entity(
            store::TEMPLATES,
            Arc::new(listener::DropTranslations),
        ));
        for entity in [store::TEMPLATES, store::LAYOUTS] {
            registry.add(ModelListenerReg::for_entity(
                entity,
                Arc::new(listener::History(entity)),
            ));
        }
    }

    async fn boot(&self, cx: &BootCx<'_>) -> CoreResult<()> {
        let layouts = cx.registry.items::<PdfLayoutReg>();
        let templates = cx.registry.items::<PdfTemplateReg>();
        store::sync(cx.db, &layouts, &templates)
            .await
            .map_err(|e| laterite_core::CoreError::Config(format!("syncing PDF templates: {e}")))
    }
}
