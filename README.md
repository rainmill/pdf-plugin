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
    &Assets::default(), Output { archival: true })?;
```

## Rendering

Function | Takes | Returns
--- | --- | ---
`render_markdown(md, layout, meta, assets, output)` | markdown set in a layout | PDF bytes
`render_typst(source, assets, output)` | Typst source; data under `sys.inputs` | PDF bytes
`markdown_source(md, layout, meta, assets)` | the same as `render_markdown` | the Typst source it compiles
`markdown::to_typst(md, images)` | markdown | Typst markup, every text run escaped

`Assets` | Holds
--- | ---
`files` | files the source reads by path (`/logo.png`): images, a background
`fonts` | font files beside the bundled ones
`inputs` | JSON values read as `sys.inputs.<key>`

`Output { archival: true }` writes PDF/A-2b. A compile error is `PdfError::Compile`, one message per error with its line.

## Layouts

Key | Default | Description
--- | --- | ---
`accent` | `#bc4b2d` | Heading rules, links, the masthead line, the table header tint.
`text` | `#1f1f1f` | Body text colour.
`body_font`, `heading_font` | the bundled serif | Font families; a missing one falls back to Libertinus Serif.
`size`, `leading` | `10.5`, `0.7` | Body size in points, line spacing in em.
`paper`, `landscape` | `a4`, `false` | Any Typst paper name.
`margins` | `[24, 22, 24, 22]` | Millimetres: top, right, bottom, left.
`logo`, `logo_height` | none, `12` | A file path for the masthead logo; its height in millimetres.
`background` | none | A file path drawn behind every page.
`header`, `footer` | none, `{title} · {version} · Page {page} of {pages}` | Running parts; `{title}`, `{version}`, `{effective}`, `{url}`, `{page}`, `{pages}`.
`typst_override` | none | Typst appended after the generated preamble.
`css_override` | none | CSS appended after `Layout::css()`'s variables.

`Layout::css()` gives the same settings as CSS custom properties (`--doc-accent`, `--doc-text`, `--doc-font-body`, `--doc-font-heading`, `--doc-size`) for the web rendering of a document.

## Bundled fonts

The bundled fonts come from `typst-assets` (Libertinus Serif, New Computer Modern, DejaVu Sans Mono and others) under their own licences, listed in that crate's `NOTICE`.

## Licence

MIT or Apache-2.0, at your option.
