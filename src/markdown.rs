//! Markdown to Typst markup: CommonMark with tables, strikethrough, task lists
//! and footnotes, parsed with `pulldown-cmark` and written as Typst function
//! calls (`#strong[..]`, `#list(..)`, `#table(..)`), so nesting never depends on
//! indentation or delimiter pairing. Every text run is escaped, so a document's
//! words can never become Typst code.

use std::collections::HashMap;

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// How an image the markdown names is placed: the file path the caller handed to
/// the render (a key of its files), or `None` to show the image's alt text.
pub type ImageResolver<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The Typst markup for `markdown`. Images go through `images`, which maps a
/// markdown image URL to a file path in the render's files.
pub fn to_typst(markdown: &str, images: ImageResolver<'_>) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut w = Writer {
        out: vec![String::new()],
        lists: Vec::new(),
        table: None,
        image: None,
        image_size: String::new(),
        code: false,
        links: Vec::new(),
        footnote: None,
        footnotes: HashMap::new(),
        images,
    };
    for event in Parser::new_ext(markdown, options) {
        w.event(event);
    }
    let body = w.out.pop().unwrap_or_default();
    // Footnote references were written as markers; definitions may come later
    // in the text, so they are filled in once everything is read.
    let mut out = String::with_capacity(body.len());
    let mut rest = body.as_str();
    while let Some(start) = rest.find(FOOTNOTE_OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + FOOTNOTE_OPEN.len_utf8()..];
        match after.find(FOOTNOTE_CLOSE) {
            Some(end) => {
                let label = &after[..end];
                let note = w.footnotes.get(label).map(|s| s.trim()).unwrap_or("");
                out.push_str("#footnote[");
                out.push_str(note);
                out.push(']');
                rest = &after[end + FOOTNOTE_CLOSE.len_utf8()..];
            }
            None => {
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.trim_end().to_string() + "\n"
}

const FOOTNOTE_OPEN: char = '\u{E000}';
const FOOTNOTE_CLOSE: char = '\u{E001}';

/// One list being written: its items so far and how it numbers.
struct List {
    start: Option<u64>,
    items: Vec<String>,
}

/// One table being written: its columns' alignment, the header and body cells.
struct Table {
    align: Vec<Alignment>,
    header: Vec<String>,
    cells: Vec<String>,
    in_head: bool,
}

struct Writer<'a> {
    /// A stack of buffers: a construct that wraps its content (emphasis, a list
    /// item, a cell) pushes one and folds it into the parent when it ends.
    out: Vec<String>,
    lists: Vec<List>,
    table: Option<Table>,
    /// An image being read: its URL; its alt text collects in the top buffer.
    image: Option<String>,
    /// The image's size arguments, from its title (`, width: 30mm`).
    image_size: String,
    /// Inside a code block: text is kept as written.
    code: bool,
    /// The URLs of the links being read, innermost last.
    links: Vec<String>,
    /// The label of the footnote definition being read.
    footnote: Option<String>,
    footnotes: HashMap<String, String>,
    images: ImageResolver<'a>,
}

impl Writer<'_> {
    fn push(&mut self, s: &str) {
        if let Some(top) = self.out.last_mut() {
            top.push_str(s);
        }
    }

    fn open(&mut self) {
        self.out.push(String::new());
    }

    fn close(&mut self) -> String {
        self.out.pop().unwrap_or_default()
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.code => self.push(&text),
            Event::Text(text) => {
                let escaped = escape(&text);
                self.push(&escaped);
            }
            Event::Code(code) => {
                let call = format!("#raw({})", string(&code));
                self.push(&call);
            }
            Event::SoftBreak => self.push(" "),
            Event::HardBreak => self.push(" \\\n"),
            Event::Rule => self.push("\n#line(length: 100%)\n\n"),
            Event::TaskListMarker(done) => self.push(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(label) => {
                let marker = format!("{FOOTNOTE_OPEN}{label}{FOOTNOTE_CLOSE}");
                self.push(&marker);
            }
            // Raw HTML is shown as the text it is, never interpreted.
            Event::Html(html) | Event::InlineHtml(html) => {
                let escaped = escape(&html);
                self.push(&escaped);
            }
            Event::InlineMath(m) | Event::DisplayMath(m) => {
                let escaped = escape(&m);
                self.push(&escaped);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::BlockQuote(_)
            | Tag::Item
            | Tag::TableCell
            | Tag::Superscript
            | Tag::Subscript => self.open(),
            Tag::CodeBlock(_) => {
                self.code = true;
                self.open();
            }
            Tag::Link { dest_url, .. } => {
                self.links.push(dest_url.to_string());
                self.open();
            }
            Tag::FootnoteDefinition(label) => {
                self.footnote = Some(label.to_string());
                self.open();
            }
            Tag::List(start) => self.lists.push(List {
                start,
                items: Vec::new(),
            }),
            Tag::Table(align) => {
                self.table = Some(Table {
                    align,
                    header: Vec::new(),
                    cells: Vec::new(),
                    in_head: false,
                })
            }
            Tag::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_head = true;
                }
            }
            Tag::TableRow => {}
            Tag::Image {
                dest_url, title, ..
            } => {
                self.image = Some(dest_url.to_string());
                self.image_size = size(&title);
                self.open();
            }
            Tag::HtmlBlock
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => self.open(),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let inner = self.close();
                self.push(&inner);
                self.push("\n\n");
            }
            TagEnd::Heading(level) => {
                let inner = self.close();
                let marks = "=".repeat(heading_level(level));
                let line = format!("{marks} {}\n\n", inner.trim());
                self.push(&line);
            }
            TagEnd::Emphasis => self.wrap("emph"),
            TagEnd::Strong => self.wrap("strong"),
            TagEnd::Strikethrough => self.wrap("strike"),
            TagEnd::Superscript => self.wrap("super"),
            TagEnd::Subscript => self.wrap("sub"),
            TagEnd::BlockQuote(_) => {
                let inner = self.close();
                let call = format!("#quote(block: true)[{}]\n\n", inner.trim());
                self.push(&call);
            }
            TagEnd::Link => {
                let inner = self.close();
                let url = self.links.pop().unwrap_or_default();
                let call = format!("#link({})[{inner}]", string(&url));
                self.push(&call);
            }
            TagEnd::CodeBlock => {
                self.code = false;
                let code = self.close();
                let call = format!("#raw(block: true, {})\n\n", string(&code));
                self.push(&call);
            }
            TagEnd::Item => {
                let inner = self.close();
                if let Some(list) = self.lists.last_mut() {
                    list.items.push(inner.trim().to_string());
                }
            }
            TagEnd::List(_) => {
                if let Some(list) = self.lists.pop() {
                    let items: Vec<String> = list.items.iter().map(|i| format!("[{i}]")).collect();
                    let call = match list.start {
                        Some(n) => format!("#enum(start: {n}, {})\n\n", items.join(", ")),
                        None => format!("#list({})\n\n", items.join(", ")),
                    };
                    self.push(&call);
                }
            }
            TagEnd::TableCell => {
                let inner = self.close();
                if let Some(t) = self.table.as_mut() {
                    let cell = format!("[{}]", inner.trim());
                    if t.in_head {
                        t.header.push(cell);
                    } else {
                        t.cells.push(cell);
                    }
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_head = false;
                }
            }
            TagEnd::TableRow => {}
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    let columns = t.align.len().max(1);
                    let align: Vec<&str> = t
                        .align
                        .iter()
                        .map(|a| match a {
                            Alignment::Center => "center",
                            Alignment::Right => "right",
                            _ => "left",
                        })
                        .collect();
                    let call = format!(
                        "#table(columns: {columns}, align: ({},), table.header({}), {})\n\n",
                        align.join(", "),
                        t.header.join(", "),
                        t.cells.join(", ")
                    );
                    self.push(&call);
                }
            }
            TagEnd::Image => {
                let alt = self.close();
                let url = self.image.take().unwrap_or_default();
                let placed = match (self.images)(&url) {
                    Some(path) => format!("#image({}{})", string(&path), self.image_size),
                    None => format!("#emph[{}]", alt.trim()),
                };
                self.push(&placed);
            }
            TagEnd::FootnoteDefinition => {
                let inner = self.close();
                let label = self.footnote.take().unwrap_or_default();
                self.footnotes.insert(label, inner);
            }
            TagEnd::HtmlBlock
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {
                let inner = self.close();
                self.push(&inner);
                self.push("\n\n");
            }
        }
    }

    /// Wraps the top buffer in a content function: `#strong[..]`.
    fn wrap(&mut self, function: &str) {
        let inner = self.close();
        let call = format!("#{function}[{inner}]");
        self.push(&call);
    }
}

fn heading_level(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Text escaped for Typst markup: every character with a meaning there gets a
/// backslash, and a full stop after a digit too, so `1.` never starts a list.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev_digit = false;
    for c in text.chars() {
        match c {
            '\\' | '#' | '*' | '_' | '`' | '$' | '<' | '>' | '@' | '[' | ']' | '~' | '=' | '-'
            | '+' | '/' => {
                out.push('\\');
                out.push(c);
            }
            '.' if prev_digit => out.push_str("\\."),
            _ => out.push(c),
        }
        prev_digit = c.is_ascii_digit();
    }
    out
}

/// A Typst string literal holding `s`.
pub(crate) fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Typst size arguments from an image title such as `width=30mm` or
/// `width=40%, height=2cm`; anything else in the title is ignored.
fn size(title: &str) -> String {
    let mut out = String::new();
    for part in title.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if !matches!(key, "width" | "height") {
            continue;
        }
        let number = value.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
        let unit = &value[number.len()..];
        if number.parse::<f32>().is_ok() && matches!(unit, "mm" | "cm" | "in" | "pt" | "em" | "%") {
            out.push_str(&format!(", {key}: {number}{unit}"));
        }
    }
    out
}
