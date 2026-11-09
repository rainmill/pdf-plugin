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

mod layout;
pub mod markdown;
mod render;
mod world;

pub use layout::{DocumentMeta, Layout};
pub use render::{markdown_source, render_markdown, render_typst, Assets, Output, PdfError};
