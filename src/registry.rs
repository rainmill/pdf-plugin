//! Templates and layouts a module ships in code. They are written into the
//! database at boot, where the admin can edit them; an edited one is marked
//! customised and no longer follows the code, until it is reset to default.

use laterite_core::serde_json;
use std::sync::Arc;

use serde_json::Value;

use crate::Layout;

/// How a template's body is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Markdown with `{{ variables }}`, set in the template's layout.
    #[default]
    Markdown,
    /// Typst source, reading its data from `sys.inputs`.
    Typst,
}

impl Format {
    /// The stored name: `markdown` or `typst`.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Markdown => "markdown",
            Format::Typst => "typst",
        }
    }

    /// The format a stored name stands for; anything unknown is markdown.
    pub fn parse(name: &str) -> Self {
        match name {
            "typst" => Format::Typst,
            _ => Format::Markdown,
        }
    }
}

/// A font file a module hands to the renderer (TTF, OTF or TTC), named by its
/// family in any layout's `body_font` or `heading_font`:
/// `registry.add(PdfFontReg::new(include_bytes!("fonts/AcmeSans.ttf")))`.
#[derive(Debug, Clone)]
pub struct PdfFontReg {
    /// The font file.
    pub bytes: Arc<[u8]>,
}

impl PdfFontReg {
    /// The font in `bytes`.
    pub fn new(bytes: impl AsRef<[u8]>) -> Self {
        Self {
            bytes: Arc::from(bytes.as_ref()),
        }
    }
}

/// A layout a module ships.
#[derive(Debug, Clone)]
pub struct PdfLayoutReg {
    /// Unique and permanent, dotted like every id (`acme.letterhead`).
    pub code: String,
    /// The name the admin shows.
    pub name: String,
    /// The settings.
    pub layout: Layout,
    /// The author written into every PDF set in it: the issuing organisation.
    pub author: String,
}

impl PdfLayoutReg {
    /// A layout registered as `code`, shown as `name`.
    pub fn new(code: impl Into<String>, name: impl Into<String>, layout: Layout) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            layout,
            author: String::new(),
        }
    }

    /// Writes `author` into every PDF set in it.
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }
}

/// A template a module ships.
#[derive(Debug, Clone)]
pub struct PdfTemplateReg {
    /// Unique and permanent, dotted like every id (`acme.shop.invoice`).
    pub code: String,
    /// The title the admin shows and the PDF carries.
    pub title: String,
    /// What the template is for, shown in the admin.
    pub description: String,
    /// How the body is written.
    pub format: Format,
    /// The body.
    pub body: String,
    /// The code of the layout it is set in; the default layout when `None`.
    pub layout: Option<String>,
    /// The data the admin previews it with.
    pub sample_data: Value,
    /// Search keywords written into the PDF.
    pub keywords: Vec<String>,
    /// The title and body in other locales: `(locale, title, body)`.
    pub translations: Vec<(String, String, String)>,
}

impl PdfTemplateReg {
    /// A template registered as `code`, titled `title`, with `body` in `format`.
    pub fn new(
        code: impl Into<String>,
        title: impl Into<String>,
        format: Format,
        body: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            title: title.into(),
            description: String::new(),
            format,
            body: body.into(),
            layout: None,
            sample_data: Value::Object(Default::default()),
            keywords: Vec::new(),
            translations: Vec::new(),
        }
    }

    /// Writes `keywords` into the PDF.
    pub fn keywords<I: IntoIterator<Item = S>, S: Into<String>>(mut self, keywords: I) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    /// The title and body for `locale` (`de`, `pt-BR`), in the same format.
    pub fn translation(
        mut self,
        locale: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        self.translations
            .push((locale.into(), title.into(), body.into()));
        self
    }

    /// Says what it is for.
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = text.into();
        self
    }

    /// Sets it in the layout registered as `code`.
    pub fn layout(mut self, code: impl Into<String>) -> Self {
        self.layout = Some(code.into());
        self
    }

    /// Previews it with `data`.
    pub fn sample(mut self, data: Value) -> Self {
        self.sample_data = data;
        self
    }
}

/// A value every template can read, registered by a module:
/// `registry.add(PdfVariableReg::new("company", json!({ "name": "Acme" })))`.
/// The data a render is called with wins over a variable of the same name.
#[derive(Debug, Clone)]
pub struct PdfVariableReg {
    /// The top-level name templates read it under.
    pub name: String,
    /// The value.
    pub value: Value,
}

impl PdfVariableReg {
    /// A variable `name` holding `value`.
    pub fn new(name: impl Into<String>, value: Value) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

/// Changes a render on its way through: the data before the template sees it,
/// and the Typst source before it compiles.
pub trait RenderHook: Send + Sync {
    /// The data template `code` renders with, changed or as it came.
    fn data(&self, code: &str, data: Value) -> Value {
        let _ = code;
        data
    }

    /// The Typst source template `code` compiles, changed or as it came. A
    /// markdown template's source is its layout's preamble and the converted body.
    fn source(&self, code: &str, source: String) -> String {
        let _ = code;
        source
    }
}

/// A [`RenderHook`] a module registers, for every template or for one code:
/// `registry.add(PdfRenderHookReg::new(Arc::new(StampDraft)).only("acme.shop.invoice"))`.
#[derive(Clone)]
pub struct PdfRenderHookReg {
    /// The hook.
    pub hook: Arc<dyn RenderHook>,
    /// The template it runs for; every template when `None`.
    pub code: Option<String>,
}

impl PdfRenderHookReg {
    /// `hook`, run for every template.
    pub fn new(hook: Arc<dyn RenderHook>) -> Self {
        Self { hook, code: None }
    }

    /// Runs it for template `code` alone.
    pub fn only(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Whether it runs for template `code`.
    pub fn applies(&self, code: &str) -> bool {
        self.code.as_deref().is_none_or(|c| c == code)
    }
}

impl std::fmt::Debug for PdfRenderHookReg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfRenderHookReg")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}
