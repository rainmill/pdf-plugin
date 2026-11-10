# Rainmill PDF

Documents rendered to PDF with [Typst](https://typst.app), for
[Laterite](https://laterite.rs) applications. Module id `rainmill.pdf`.

```rust
use rainmill_pdf::{render_markdown, Assets, DocumentMeta, Layout, Output};

let meta = DocumentMeta {
    title: "Terms of Use".into(),
    version: "1.0".into(),
    effective: "9 November 2026".into(),
    ..DocumentMeta::default()
};
let layout = Layout { accent: "#bc4b2d".into(), ..Layout::default() };
let pdf = render_markdown("## Who we are\n\nAcme runs this site.", &layout, &meta,
    &Assets::default(), &Output::archival())?;
```

## Templates in the admin

Register the module (`rainmill_pdf::module()`) and Settings gains **PDF Templates** under System, with **Templates** and **Layouts** tabs (`/admin/rainmill/pdf/templates`, `/admin/rainmill/pdf/layouts`). A layout's logo and page background are chosen from the media library. A template's body is edited beside a live PDF of it, rendered with the form's layout and sample data as they stand.

Ship templates and layouts from code; they are written into the database at boot:

```rust
use rainmill_pdf::{Format, Layout, PdfLayoutReg, PdfTemplateReg};

registry.add(PdfLayoutReg::new("acme.letterhead", "Acme letterhead", Layout::default()));
registry.add(
    PdfTemplateReg::new("acme.shop.invoice", "Invoice", Format::Markdown, include_str!("invoice.md"))
        .layout("acme.letterhead")
        .sample(laterite_core::serde_json::json!({ "number": "2026/1" })),
);
```

Render one with data through a `Pdf` handle, which carries the registered variables, render hooks and media disks:

```rust
use rainmill_pdf::{Pdf, RenderOptions};

let pdf = Pdf::new(db.clone(), &registry).library(&registry, "storage"); // public and private library files
let invoice = pdf
    .render_with("acme.shop.invoice", json!({ "number": "2026/7" }), &RenderOptions::locale("de"))
    .await?;

invoice.clone().download();                     // an axum response, saved by the browser
invoice.clone().inline();                       // shown in the browser
let mail = Message::template("acme.shop.receipt", vars).to(&email);
let mail = Message { attachments: vec![invoice.attachment()], ..mail };
let record = invoice.store(&pdf, "public", Some(Owner { owner_type: "acme_orders", owner_id: "7", field: "invoice" })).await?;
```

Behaviour | Result
--- | ---
Editing a registered template or layout in the admin | Marked customised; later boots leave it alone.
Reset to default | Restores what the module ships and clears the customised flag.
Duplicate | A customised copy under `<code>_copy` (`_copy2`, ...).
Changing a code | Refused: code and other templates refer to it.
Deleting a layout a template is set in | Refused, naming how many templates use it.
A markdown body | `{{ variables }}` filled from the data, then set in its layout.
A Typst body | Reads the data as `sys.inputs`.
Deleting a template | Deletes its translations.

Permission | Allows
--- | ---
`rainmill.pdf.manage` | Opening templates and layouts, read-only without the grants below, and their history
`rainmill.pdf.preview` | The live preview, Download PDF and Preview PDF
`rainmill.pdf.templates.create`, `.update`, `.delete` | Creating (and duplicating), saving (resetting, restoring, translations) and deleting templates
`rainmill.pdf.layouts.create`, `.update`, `.delete` | The same for layouts

### History

Every save in the admin keeps a version: the last 20 per template or layout, with who saved it. **History** on the form lists them newest first, each with what changed from the one before (word by word for the body, the sample data and the overrides), and **Restore** puts one back; the version it replaces stays in the history. Deleting a record deletes its history.

```rust
let versions = rainmill_pdf::history::list(&db, "pdf_templates", id).await?;
rainmill_pdf::history::restore(&db, "pdf_templates", id, versions[1].id, "ada").await?;
```

### Variables and render hooks

```rust
use rainmill_pdf::{PdfRenderHookReg, PdfVariableReg, RenderHook};

registry.add(PdfVariableReg::new("company", json!({ "name": "Acme", "vat": "GB123" })));

struct Draft;
impl RenderHook for Draft {
    fn data(&self, _code: &str, mut data: Value) -> Value {
        data["watermark"] = json!("DRAFT");
        data
    }
    fn source(&self, _code: &str, source: String) -> String {
        format!("#set page(background: rotate(-30deg, text(60pt, gray)[DRAFT]))\n{source}")
    }
}
registry.add(PdfRenderHookReg::new(Arc::new(Draft)).only("acme.shop.invoice"));
```

Item | Behaviour
--- | ---
`PdfVariableReg` | A top-level value every template reads; the call's data wins on the same name.
`RenderHook::data` | Changes the data before the template sees it.
`RenderHook::source` | Changes the Typst source before it compiles; for markdown, the preamble and converted body.
`PdfRenderHookReg::only(code)` | Runs the hook for one template.

### Locales

A template has translations: a title and body per locale, in the template's format. Add them in the admin under the template's **Translations**, or in code:

```rust
PdfTemplateReg::new("acme.shop.invoice", "Invoice", Format::Markdown, body)
    .translation("de", "Rechnung", body_de)
    .translation("pt-BR", "", body_pt_br);
```

`RenderOptions::locale("pt-BR")` | Uses
--- | ---
a `pt-BR` translation | its title and body; an empty one keeps the template's
else a `pt` translation | the same
else | the template itself

The locale's language is also the document's language (hyphenation, quotes).

### Output

`Rendered` | Gives
--- | ---
`bytes`, `filename` | The PDF and a name from its title (`invoice.pdf`).
`inline()` | A response shown in the browser.
`download()` | A response saved as a download.
`attachment()` | A `laterite_core::mail::Attachment`.
`store(&pdf, disk, owner)` | A media record on `disk`, linked to `owner`'s field when given; with no owner it is placed in the media library.

`RenderOptions` | Default | Description
--- | --- | ---
`locale` | none | The translation to render.
`output` | `Output::default()` | `Output::archival()` for PDF/A-2b, `Output::protected(password)` for AES-256 encryption.
`author` | the layout's | The PDF's author.

## On the site

```twig
<a href="{{ pdf_url('acme.shop.invoice', data={'number': order.number}) }}">Invoice</a>
<a href="{{ pdf_url('acme.legal.terms', locale='de', download=true) }}">AGB</a>
```

```ini
[pdf]
template = "acme.shop.ticket"
download = true
label = "Download your ticket"
```

```twig
{{ component("pdf") }}
```

Item | Gives
--- | ---
`pdf_url(code, data={}, locale=none, download=false)` | A link that renders the template with that data, at `/rainmill/pdf/<token>`.
The `rainmill.pdf` component | `url` and `label` for its `template` rendered with the page's URL parameters as the data; its markup is a Download link.

Link | Answers
--- | ---
Followed within a day | The PDF, shown or as a download.
Followed later | `410`, asking the reader to reload the page it came from.
Altered, or naming no template | `404`.
A template that fails to render | `500` with a plain message; `lat pdf check` names the reason.

## Testing

```rust
use rainmill_pdf::testing::Recorder;

let recorder = Recorder::new();
registry.add(recorder.clone());
// ... run the code that renders through Pdf::new(db, &registry) ...
recorder.assert_rendered_with("acme.shop.invoice", |data| data["number"] == "2026/7");
recorder.assert_nothing_rendered();
```

A registered `Recorder` makes every render record its code, data (variables merged) and locale, and return `testing::STUB` instead of compiling.

## Command line

```bash
lat pdf sync
lat pdf check
lat pdf render acme.shop.invoice --data invoice.json --locale de --out invoice.pdf
lat pdf export pdf-export
lat pdf import pdf-export
```

Verb | Does
--- | ---
`sync` | Writes the modules' templates and layouts into the database, as each start does; customised ones are left alone.
`check` | Renders every template with its sample data, one line each; exits non-zero when any fails.
`render <code>` | Writes one PDF: `--data` a JSON file (the sample data otherwise), `--locale`, `--out` (the title's file name otherwise), `--archival` for PDF/A.
`export [DIR]` | Writes `layouts/<code>.yaml`, `templates/<code>.md` (front matter, then the body) and `templates/<code>/<locale>.md` per translation; `pdf-export` by default. Logos and backgrounds stay behind.
`import DIR` | Reads an export back, writing each over the one with its code, marked customised.

## Rendering

Function | Takes | Returns
--- | --- | ---
`render_markdown(md, layout, meta, assets, output)` | markdown set in a layout | PDF bytes
`render_typst(source, meta, assets, output)` | Typst source; data under `sys.inputs` | PDF bytes
`markdown_source(md, layout, meta, assets)` | the same as `render_markdown` | the Typst source it compiles
`markdown::to_typst(md, images)` | markdown | Typst markup, every text run escaped

`Assets` | Holds
--- | ---
`files` | files the source reads by path (`/logo.png`): images, a background
`fonts` | font files beside the bundled ones
`inputs` | JSON values read as `sys.inputs.<key>`

`Output::archival()` writes PDF/A-2b; `Output::protected("password")` encrypts with AES-256 so the PDF opens only with that password (printing and copying allowed). PDF/A forbids encryption, so the two together are `PdfError::Protect`. A compile error is `PdfError::Compile`, one message per error with its line in your source.

### QR codes and barcodes

```markdown
{{ qr(order.url) }}
{{ qr(order.url, width="25mm") }}
{{ barcode(order.number, kind="ean13", width="50mm") }}
```

```typst
#qr(sys.inputs.url, width: 3cm)
#barcode(sys.inputs.number, kind: "code39", height: 1.2cm)
```

Kind | Holds
--- | ---
`code128` (default) | printable ASCII
`code39`, `code93` | upper-case letters, digits, a few symbols
`codabar` | digits and `-$:/.+`, with start and stop letters
`ean13`, `ean8` | 12 or 7 digits (the check digit is added)
`itf` | an even number of digits

A value a barcode cannot hold is a compile error naming the line.

### Metadata

`DocumentMeta` | Written as
--- | ---
`title` | the PDF title
`author` | the PDF author
`description` | the PDF description (subject)
`keywords` | the PDF keywords
`language` | the document language

A stored template takes its description and keywords from the admin; the author from its layout.

### Images

An image's markdown title sizes it: `![Logo](/logo.png "width=30mm")`, `"width=40%, height=2cm"`. Units: `mm`, `cm`, `in`, `pt`, `em`, `%`.

## Layouts

Key | Default | Description
--- | --- | ---
`accent` | `#bc4b2d` | Heading rules, links, the masthead line, the table header tint.
`text` | `#1f1f1f` | Body text colour.
`body_font`, `heading_font` | the bundled serif | Font families; a missing one falls back to Libertinus Serif.
`title_font` | the heading font | The masthead title's font family.
`fonts` | none | Font files for the families above. In the admin, chosen from the media library's Fonts.
`size`, `leading` | `10.5`, `0.7` | Body size in points, line spacing in em.
`paper`, `landscape` | `a4`, `false` | Any Typst paper name.
`margins` | `[24, 22, 24, 22]` | Millimetres: top, right, bottom, left.
`logo`, `logo_height` | none, `12` | A file path for the masthead logo; its height in millimetres. In the admin, chosen from the media library.
`background` | none | A file path drawn behind every page. In the admin, chosen from the media library.
`header`, `footer` | none, `{title} · {version} · Page {page} of {pages}` | Running parts; `{title}`, `{version}`, `{effective}`, `{url}`, `{page}`, `{pages}`.
`typst_override` | none | Typst run after the generated rules and before the masthead; it may redefine `doc-title(title)` and `doc-meta(line)`, which draw the masthead's title and version line.
`css_override` | none | CSS appended after `Layout::css()`'s variables.

`Layout::css()` gives the same settings as CSS custom properties (`--doc-accent`, `--doc-text`, `--doc-font-body`, `--doc-font-heading`, `--doc-size`) for the web rendering of a document.

## Fonts

```rust
// in a module's register()
registry.add(PdfFontReg::new(include_bytes!("fonts/AcmeSans-Regular.ttf")));
registry.add(PdfFontReg::new(include_bytes!("fonts/AcmeSans-Bold.ttf")));
registry.add(PdfLayoutReg::new("acme.letterhead", "Letterhead", Layout {
    body_font: Some("Acme Sans".into()),
    ..Layout::default()
}));
```

Source | Fonts
--- | ---
Bundled | `typst-assets`: Libertinus Serif, New Computer Modern, DejaVu Sans Mono and others, under the licences in that crate's `NOTICE`
A layout's `fonts` | Files from the media library, for PDFs set in that layout
`PdfFontReg` | TTF, OTF or TTC files a module registers, named by family in any layout, in code or in the admin

## Licence

MIT or Apache-2.0, at your option.
