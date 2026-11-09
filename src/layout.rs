//! A layout: the letterhead a document is set in. Its structured settings (colours,
//! fonts, page, running header and footer) become both a Typst preamble for the
//! PDF and CSS variables for the web page, so the two agree without anyone writing
//! either language; each output keeps an override for what the settings cannot say.

use serde::{Deserialize, Serialize};

use crate::markdown::{escape, string};

/// The letterhead a document renders with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Layout {
    /// The accent: heading rules, links, the masthead line. `#rrggbb`.
    pub accent: String,
    /// The body text colour. `#rrggbb`.
    pub text: String,
    /// The body font family, falling back to the bundled serif.
    pub body_font: Option<String>,
    /// The heading font family, falling back to the body font.
    pub heading_font: Option<String>,
    /// The body size in points.
    pub size: f32,
    /// The line spacing, as Typst leading in em.
    pub leading: f32,
    /// `a4`, `us-letter` or any Typst paper name.
    pub paper: String,
    /// Landscape instead of portrait.
    pub landscape: bool,
    /// Page margins in millimetres: top, right, bottom, left.
    pub margins: [f32; 4],
    /// A logo for the masthead: the path of a file handed to the render.
    pub logo: Option<String>,
    /// The masthead's height for the logo, in millimetres.
    pub logo_height: f32,
    /// A background for every page: the path of a file handed to the render.
    pub background: Option<String>,
    /// The running header, with `{title}`, `{version}`, `{effective}`, `{url}`,
    /// `{page}` and `{pages}` filled in. Empty for none.
    pub header: String,
    /// The running footer, with the same placeholders.
    pub footer: String,
    /// Typst appended after the generated preamble, for what the settings cannot say.
    pub typst_override: String,
    /// CSS appended after the generated variables, for the web page.
    pub css_override: String,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            accent: "#bc4b2d".into(),
            text: "#1f1f1f".into(),
            body_font: None,
            heading_font: None,
            size: 10.5,
            leading: 0.7,
            paper: "a4".into(),
            landscape: false,
            margins: [24.0, 22.0, 24.0, 22.0],
            logo: None,
            logo_height: 12.0,
            background: None,
            header: String::new(),
            footer: "{title} · {version} · Page {page} of {pages}".into(),
            typst_override: String::new(),
            css_override: String::new(),
        }
    }
}

/// What a rendered document says about itself: shown in the masthead and the
/// running parts, and written into the PDF's metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DocumentMeta {
    /// The title, also the PDF's title.
    pub title: String,
    /// A version label (`2.0`, `2026-11-09`), or empty.
    pub version: String,
    /// When it takes effect, as written for readers (`9 November 2026`), or empty.
    pub effective: String,
    /// Where the document lives online, or empty.
    pub url: String,
    /// The PDF's author.
    pub author: String,
    /// The language, as a two-letter code (`en`).
    pub language: String,
    /// What the document is about: the PDF's description (subject).
    pub description: String,
    /// Search keywords written into the PDF.
    pub keywords: Vec<String>,
}

impl DocumentMeta {
    /// The Typst `#set document(..)` rule carrying this metadata.
    pub fn document_rule(&self) -> String {
        let mut args = vec![
            format!("title: {}", string(&self.title)),
            format!("author: {}", string(&self.author)),
        ];
        if !self.description.trim().is_empty() {
            args.push(format!("description: {}", string(&self.description)));
        }
        let keywords: Vec<String> = self
            .keywords
            .iter()
            .map(|k| k.trim())
            .filter(|k| !k.is_empty())
            .map(string)
            .collect();
        if !keywords.is_empty() {
            args.push(format!("keywords: ({},)", keywords.join(", ")));
        }
        format!("#set document({})", args.join(", "))
    }
}

impl Layout {
    /// The Typst preamble that sets the page, the type, the running parts and the
    /// masthead for a document described by `meta`.
    pub fn preamble(&self, meta: &DocumentMeta) -> String {
        let accent = colour(&self.accent, "#bc4b2d");
        let text = colour(&self.text, "#1f1f1f");
        let body = fonts(self.body_font.as_deref());
        let heading = fonts(self.heading_font.as_deref().or(self.body_font.as_deref()));
        let [top, right, bottom, left] = self.margins;
        let lang = if meta.language.len() == 2 {
            meta.language.to_lowercase()
        } else {
            "en".into()
        };
        let mut p = String::new();
        p.push_str(&meta.document_rule());
        p.push('\n');
        p.push_str(&format!(
            "#set page(paper: {}, flipped: {}, margin: (top: {top}mm, right: {right}mm, bottom: {bottom}mm, left: {left}mm)",
            string(&self.paper),
            self.landscape
        ));
        if !self.header.trim().is_empty() {
            p.push_str(&format!(
                ", header: context text(size: 0.8em, fill: rgb(\"#6b6b6b\"))[{}]",
                running(&self.header, meta)
            ));
        }
        if !self.footer.trim().is_empty() {
            p.push_str(&format!(
                ", footer: context text(size: 0.8em, fill: rgb(\"#6b6b6b\"))[{}]",
                running(&self.footer, meta)
            ));
        }
        if let Some(bg) = &self.background {
            p.push_str(&format!(
                ", background: image({}, width: 100%, height: 100%)",
                string(bg)
            ));
        }
        p.push_str(")\n");
        p.push_str(&format!(
            "#set text(font: {body}, size: {}pt, fill: rgb({text}), lang: {})\n",
            self.size,
            string(&lang)
        ));
        p.push_str(&format!(
            "#set par(leading: {}em, spacing: 1.1em)\n",
            self.leading
        ));
        p.push_str(&format!(
            "#show heading: it => block(above: 1.6em, below: 0.8em)[#line(length: 2.6em, stroke: 1.6pt + rgb({accent})) #v(0.3em) #text(font: {heading}, fill: rgb({text}))[#it.body]]\n"
        ));
        p.push_str(&format!("#show link: set text(fill: rgb({accent}))\n"));
        p.push_str(&format!(
            "#set table(stroke: 0.5pt + rgb(\"#d6d6d6\"), inset: 6pt, fill: (x, y) => if y == 0 {{ rgb({accent}).lighten(85%) }})\n"
        ));
        p.push_str("#show table.cell.where(y: 0): set text(weight: \"bold\")\n");
        p.push_str("#set quote(block: true)\n");
        p.push_str(&format!(
            "#show quote: it => block(inset: (left: 1em), stroke: (left: 2pt + rgb({accent})))[#it.body]\n"
        ));
        p.push_str(&self.masthead(meta, &accent, &heading));
        if !self.typst_override.trim().is_empty() {
            p.push_str(&self.typst_override);
            p.push('\n');
        }
        p
    }

    /// The masthead on page one: the logo, the title, the version and date line.
    fn masthead(&self, meta: &DocumentMeta, accent: &str, heading: &str) -> String {
        let mut m = String::from("#block(below: 2em)[\n");
        if let Some(logo) = &self.logo {
            m.push_str(&format!(
                "#image({}, height: {}mm)\n#v(0.6em)\n",
                string(logo),
                self.logo_height
            ));
        }
        m.push_str(&format!(
            "#line(length: 100%, stroke: 2pt + rgb({accent}))\n#v(0.4em)\n"
        ));
        if !meta.title.is_empty() {
            m.push_str(&format!(
                "#text(font: {heading}, size: 2em, weight: \"bold\")[{}]\n",
                escape(&meta.title)
            ));
        }
        let line: Vec<String> = [
            (!meta.version.is_empty()).then(|| format!("Version {}", meta.version)),
            (!meta.effective.is_empty()).then(|| format!("Effective {}", meta.effective)),
            (!meta.url.is_empty()).then(|| meta.url.clone()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !line.is_empty() {
            m.push_str(&format!(
                "#v(0.2em)\n#text(size: 0.85em, fill: rgb(\"#6b6b6b\"))[{}]\n",
                escape(&line.join(" · "))
            ));
        }
        m.push_str("]\n");
        m
    }

    /// CSS custom properties for the web rendering of a document in this layout,
    /// followed by the layout's CSS override.
    pub fn css(&self) -> String {
        let mut css = String::from(":root {\n");
        css.push_str(&format!("  --doc-accent: {};\n", css_value(&self.accent)));
        css.push_str(&format!("  --doc-text: {};\n", css_value(&self.text)));
        if let Some(f) = &self.body_font {
            css.push_str(&format!("  --doc-font-body: \"{}\";\n", css_value(f)));
        }
        if let Some(f) = self.heading_font.as_ref().or(self.body_font.as_ref()) {
            css.push_str(&format!("  --doc-font-heading: \"{}\";\n", css_value(f)));
        }
        css.push_str(&format!("  --doc-size: {}pt;\n", self.size));
        css.push_str("}\n");
        if !self.css_override.trim().is_empty() {
            css.push_str(&self.css_override);
            css.push('\n');
        }
        css
    }
}

/// A `#rrggbb` colour as a Typst string, or `fallback` when it is not one.
fn colour(value: &str, fallback: &str) -> String {
    let ok = value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit());
    string(if ok { value } else { fallback })
}

/// A Typst font list: the family, then the bundled serif as a fallback.
fn fonts(family: Option<&str>) -> String {
    match family {
        Some(f) if !f.trim().is_empty() => {
            format!("({}, \"Libertinus Serif\")", string(f.trim()))
        }
        _ => "(\"Libertinus Serif\",)".into(),
    }
}

/// A value safe inside a CSS declaration: no characters that could end it.
fn css_value(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, ';' | '{' | '}' | '<' | '>' | '"' | '\\'))
        .collect()
}

/// A running header or footer with its placeholders filled in: text escaped,
/// the page counters as Typst calls (the part is set inside `context`).
fn running(template: &str, meta: &DocumentMeta) -> String {
    // A segment that is only an empty placeholder is dropped with its separator,
    // so a document with no version does not print "Title ·  · Page 1".
    let empty = |segment: &str| match segment.trim() {
        "{version}" => meta.version.is_empty(),
        "{effective}" => meta.effective.is_empty(),
        "{url}" => meta.url.is_empty(),
        "{title}" => meta.title.is_empty(),
        _ => false,
    };
    let kept: Vec<&str> = template.split(" · ").filter(|s| !empty(s)).collect();
    let template = kept.join(" · ");
    let mut out = String::new();
    let mut rest = template.as_str();
    while let Some(start) = rest.find('{') {
        out.push_str(&escape(&rest[..start]));
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let value = match &after[..end] {
                    "title" => escape(&meta.title),
                    "version" => escape(&meta.version),
                    "effective" => escape(&meta.effective),
                    "url" => escape(&meta.url),
                    "page" => "#counter(page).display()".into(),
                    "pages" => "#counter(page).final().first()".into(),
                    other => escape(&format!("{{{other}}}")),
                };
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&escape(&rest[start..]));
                rest = "";
            }
        }
    }
    out.push_str(&escape(rest));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_placeholder_takes_its_separator_with_it() {
        let meta = DocumentMeta {
            title: "Terms".into(),
            ..DocumentMeta::default()
        };
        let out = running("{title} · {version} · Page {page}", &meta);
        assert_eq!(out, "Terms · Page #counter(page).display()");
    }
}
