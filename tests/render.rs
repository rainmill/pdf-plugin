//! Rendering through the public API: markdown in a layout, Typst with inputs,
//! archival output, and the failures a caller sees.

use laterite_core::serde_json;
use rainmill_pdf::markdown::to_typst;
use rainmill_pdf::{
    markdown_source, render_markdown, render_typst, Assets, DocumentMeta, Layout, Output, PdfError,
};

fn none(_: &str) -> Option<String> {
    None
}

fn meta() -> DocumentMeta {
    DocumentMeta {
        title: "Terms of Use".into(),
        version: "2.0".into(),
        effective: "9 November 2026".into(),
        url: "https://acme.com/legal/terms".into(),
        author: "Acme".into(),
        language: "en".into(),
        ..DocumentMeta::default()
    }
}

const SAMPLE: &str = r##"# Who we are

Acme runs **this site**, *on purpose*, with ~~no~~ some `code` and a [link](https://acme.com).

1. First
2. Second, with a footnote[^a]

- One
  - Nested
- Two

> A quoted line.

| Plan | Price |
| --- | ---: |
| Free | 0 |
| Pro  | 10 |

```
fn main() { println!("#not typst"); }
```

---

![Logo](/logo.png) and ![Missing](/nope.png)

[^a]: The footnote's text.
"##;

#[test]
fn every_markdown_construct_becomes_typst_that_compiles() {
    let typst = to_typst(SAMPLE, &none);
    for expected in [
        "= Who we are",
        "#strong[this site]",
        "#emph[on purpose]",
        "#strike[no]",
        "#raw(\"code\")",
        "#link(\"https://acme.com\")[link]",
        "#enum(start: 1,",
        "#list(",
        "#quote(block: true)[A quoted line.]",
        "#table(columns: 2, align: (left, right,)",
        "table.header([Plan], [Price])",
        "#raw(block: true, \"fn main() { println!(\\\"#not typst\\\"); }\\n\")",
        "#line(length: 100%)",
        "#footnote[The footnote's text.]",
        "#emph[Logo]",
    ] {
        assert!(
            typst.contains(expected),
            "missing {expected:?} in:\n{typst}"
        );
    }
    let pdf = render_markdown(
        SAMPLE,
        &Layout::default(),
        &meta(),
        &Assets::default(),
        &Output::default(),
    )
    .expect("the sample renders");
    assert!(pdf.starts_with(b"%PDF-"), "a PDF");
}

#[test]
fn text_that_looks_like_typst_stays_text() {
    let typst = to_typst(
        "#set page(paper: \"a0\") and $x$ and @label <tag> 2. not a list // comment",
        &none,
    );
    assert!(typst.contains("\\#set page(paper: \"a0\")"), "{typst}");
    assert!(typst.contains("\\$x\\$"), "{typst}");
    assert!(typst.contains("\\@label \\<tag\\>"), "{typst}");
    assert!(typst.contains("2\\. not a list"), "{typst}");
    assert!(typst.contains("\\/\\/ comment"), "{typst}");
    render_typst(&typst, &meta(), &Assets::default(), &Output::default()).expect("still compiles");
}

#[test]
fn an_image_renders_when_its_file_is_handed_over() {
    // A 1x1 PNG.
    let png: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0, 0x05, 0x00, 0x01, 0xFF,
        0x89, 0x99, 0x3D, 0x1D, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let assets = Assets {
        files: vec![("/logo.png".into(), png)],
        ..Assets::default()
    };
    let layout = Layout {
        logo: Some("/logo.png".into()),
        ..Layout::default()
    };
    let source = markdown_source("![Logo](/logo.png)", &layout, &meta(), &assets);
    assert!(source.contains("#image(\"/logo.png\")"), "{source}");
    render_markdown(
        "![Logo](/logo.png)",
        &layout,
        &meta(),
        &assets,
        &Output::default(),
    )
    .expect("renders with the image and the masthead logo");
}

#[test]
fn the_layout_sets_the_page_type_and_running_parts() {
    let layout = Layout {
        accent: "#e5bf02".into(),
        body_font: Some("DM Sans".into()),
        paper: "us-letter".into(),
        header: "{title}".into(),
        footer: "Page {page} of {pages}".into(),
        ..Layout::default()
    };
    let preamble = layout.preamble(&meta());
    assert!(preamble.contains("paper: \"us-letter\""), "{preamble}");
    assert!(
        preamble.contains("font: (\"DM Sans\", \"Libertinus Serif\")"),
        "{preamble}"
    );
    assert!(
        preamble.contains("#counter(page).final().first()"),
        "{preamble}"
    );
    assert!(
        preamble.contains("Version 2\\.0 · Effective 9 November 2026"),
        "{preamble}"
    );
    let css = layout.css();
    assert!(
        css.contains("--doc-accent: #e5bf02;") && css.contains("--doc-font-body: \"DM Sans\";"),
        "{css}"
    );
    // A colour that is not #rrggbb falls back rather than reaching Typst.
    let odd = Layout {
        accent: "red\"); #panic(".into(),
        ..Layout::default()
    }
    .preamble(&meta());
    assert!(!odd.contains("#panic"), "{odd}");
    render_markdown(
        "Body.",
        &layout,
        &meta(),
        &Assets::default(),
        &Output::default(),
    )
    .expect("a missing font falls back to the bundled serif");
}

#[test]
fn typst_reads_its_inputs() {
    let mut inputs = serde_json::Map::new();
    inputs.insert("number".into(), serde_json::json!("2026/1"));
    inputs.insert(
        "lines".into(),
        serde_json::json!([{ "item": "Tea", "qty": 2 }]),
    );
    let assets = Assets {
        inputs,
        ..Assets::default()
    };
    let source = "#let d = sys.inputs\nInvoice #d.number: #for l in d.lines [#l.item × #l.qty]";
    render_typst(source, &meta(), &assets, &Output::default()).expect("renders with inputs");
}

#[test]
fn a_compile_error_names_its_line() {
    let err = render_typst(
        "Fine.\n#undefined-function()",
        &meta(),
        &Assets::default(),
        &Output::default(),
    )
    .expect_err("does not compile");
    match err {
        PdfError::Compile(messages) => {
            assert!(
                messages.iter().any(|m| m.starts_with("line 2:")),
                "{messages:?}"
            )
        }
        other => panic!("expected a compile error, got {other}"),
    }
}

#[test]
fn archival_output_is_pdf_a() {
    let pdf = render_markdown(
        SAMPLE,
        &Layout::default(),
        &meta(),
        &Assets::default(),
        &Output::archival(),
    )
    .expect("renders as PDF/A");
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains("pdfaid:part") || text.contains("pdfaid"),
        "the PDF/A identification is written"
    );
}
