//! Renders a sample policy to the path given as the first argument:
//! `cargo run -p rainmill-pdf --example sample -- out.pdf`.

use rainmill_pdf::{render_markdown, Assets, DocumentMeta, Layout, Output};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample.pdf".into());
    let markdown = r#"## Who we are

Acme is a product of Acme Holdings. Acme Holdings decides why and how your information is used.

## What we collect

| What | When | Why |
| --- | --- | --- |
| Mobile number | Always (required) | To tell you when Acme launches |
| Email address | If you give one | A second way to reach you |

We rely on your consent for **all of it**, and you can withdraw it at any time.[^1]

## Your rights

1. Ask what we hold.
2. Have it corrected.
3. Have it deleted.

> Write to us from the number you signed up with.

[^1]: Withdrawing stops future messages; it does not undo what happened before.
"#;
    let meta = DocumentMeta {
        title: "Privacy Policy".into(),
        version: "1.0".into(),
        effective: "9 November 2026".into(),
        url: "https://acme.com/legal/privacy".into(),
        author: "Acme".into(),
        language: "en".into(),
        ..DocumentMeta::default()
    };
    let pdf = render_markdown(
        markdown,
        &Layout::default(),
        &meta,
        &Assets::default(),
        &Output::archival(),
    )
    .expect("renders");
    std::fs::write(&out, pdf).expect("writes");
    println!("wrote {out}");
}
