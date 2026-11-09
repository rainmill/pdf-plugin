//! Templates and layouts a module ships in code. They are written into the
//! database at boot, where the admin can edit them; an edited one is marked
//! customised and no longer follows the code, until it is reset to default.

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

/// A layout a module ships.
#[derive(Debug, Clone)]
pub struct PdfLayoutReg {
    /// Unique and permanent, dotted like every id (`acme.letterhead`).
    pub code: String,
    /// The name the admin shows.
    pub name: String,
    /// The settings.
    pub layout: Layout,
}

impl PdfLayoutReg {
    /// A layout registered as `code`, shown as `name`.
    pub fn new(code: impl Into<String>, name: impl Into<String>, layout: Layout) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            layout,
        }
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
        }
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
