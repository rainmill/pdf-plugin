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
//!     &meta, &Assets::default(), Output { archival: true })?;
//! # Ok::<(), rainmill_pdf::PdfError>(())
//! ```
#![warn(missing_docs)]

pub mod admin;
pub mod document;
mod layout;
mod listener;
pub mod markdown;
pub mod migrations;
pub mod registry;
mod render;
pub mod store;
mod world;

use std::sync::Arc;

use laterite_core::strata::async_trait;
use laterite_core::{
    BootCx, CoreResult, MigrationSet, ModelListenerReg, Module, ModuleId, Registry,
};

pub use document::{render, RenderError};
pub use layout::{DocumentMeta, Layout};
pub use registry::{Format, PdfLayoutReg, PdfTemplateReg};
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
        registry.add_permission(admin::permission());
        for resource in admin::resources() {
            registry.add_resource(resource);
        }
        registry.add_picker_source(admin::layout_source());
        registry.add_screen(admin::screen());
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
    }

    async fn boot(&self, cx: &BootCx<'_>) -> CoreResult<()> {
        let layouts = cx.registry.items::<PdfLayoutReg>();
        let templates = cx.registry.items::<PdfTemplateReg>();
        store::sync(cx.db, &layouts, &templates)
            .await
            .map_err(|e| laterite_core::CoreError::Config(format!("syncing PDF templates: {e}")))
    }
}
