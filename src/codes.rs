//! QR codes and barcodes, drawn as SVG when the document asks for them.
//!
//! A document names a code by a path: `/_pdf/qr/<hex>.svg` or
//! `/_pdf/barcode/<kind>/<hex>.svg`, where `<hex>` is the value's UTF-8 bytes in
//! hex. The world draws the file the first time the path is read, so a Typst
//! template can make a code from a value it only learns while compiling, and a
//! markdown template's `{{ qr(value) }}` writes an image at the same path.

/// Where the generated files live, under the document's root.
pub(crate) const ROOT: &str = "/_pdf/";

/// The Typst library every document imports: `qr` and `barcode`.
pub(crate) const LIBRARY: &str = r#"#let _hex(value) = array(bytes(str(value))).map(b => (if b < 16 { "0" } else { "" }) + str(b, base: 16)).sum(default: "")
#let qr(value, ..args) = image("/_pdf/qr/" + _hex(value) + ".svg", ..args)
#let barcode(value, kind: "code128", ..args) = image("/_pdf/barcode/" + kind + "/" + _hex(value) + ".svg", ..args)
"#;

/// The barcode symbologies `barcode(..)` draws, by the name a template uses.
pub const BARCODE_KINDS: [&str; 7] = [
    "code128", "code39", "code93", "codabar", "ean13", "ean8", "itf",
];

/// The path of a QR code for `value`.
pub fn qr_path(value: &str) -> String {
    format!("{ROOT}qr/{}.svg", hex(value))
}

/// The path of a `kind` barcode for `value`.
pub fn barcode_path(kind: &str, value: &str) -> String {
    format!("{ROOT}barcode/{kind}/{}.svg", hex(value))
}

fn hex(value: &str) -> String {
    value.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<String> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// The SVG a generated path stands for: `None` when the path is not one of
/// ours, `Some(Err(..))` when it is but the value cannot be drawn.
pub(crate) fn draw(path: &str) -> Option<Result<String, String>> {
    let rest = path.strip_prefix(ROOT)?;
    let rest = rest.strip_suffix(".svg")?;
    if let Some(encoded) = rest.strip_prefix("qr/") {
        let value = unhex(encoded)?;
        return Some(qr_svg(&value));
    }
    let (kind, encoded) = rest.strip_prefix("barcode/")?.split_once('/')?;
    let value = unhex(encoded)?;
    Some(barcode_svg(kind, &value))
}

/// A QR code for `value`, with its quiet zone.
pub fn qr_svg(value: &str) -> Result<String, String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(value.as_bytes())
        .map_err(|e| format!("a QR code cannot hold this value: {e}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .build())
}

/// A `kind` barcode (one of [`BARCODE_KINDS`]) for `value`.
pub fn barcode_svg(kind: &str, value: &str) -> Result<String, String> {
    use barcoders::sym::{codabar, code128, code39, code93, ean13, ean8, tf};
    let refused =
        |e: barcoders::error::Error| format!("a {kind} barcode cannot hold {value:?}: {e}");
    let bars = match kind {
        "code128" => {
            // Character set B (printable ASCII) unless the value names its own.
            let data = if value.starts_with(['\u{00C0}', '\u{0181}', '\u{0106}']) {
                value.to_string()
            } else {
                format!("\u{0181}{value}")
            };
            code128::Code128::new(data).map_err(refused)?.encode()
        }
        "code39" => code39::Code39::new(value).map_err(refused)?.encode(),
        "code93" => code93::Code93::new(value).map_err(refused)?.encode(),
        "codabar" => codabar::Codabar::new(value).map_err(refused)?.encode(),
        "ean13" => ean13::EAN13::new(value).map_err(refused)?.encode(),
        "ean8" => ean8::EAN8::new(value).map_err(refused)?.encode(),
        "itf" => tf::TF::interleaved(value).map_err(refused)?.encode(),
        other => {
            return Err(format!(
                "no barcode kind {other:?}; use one of {}",
                BARCODE_KINDS.join(", ")
            ))
        }
    };
    barcoders::generators::svg::SVG::new(60)
        .xmlns("http://www.w3.org/2000/svg".into())
        .generate(bars)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_round_trips_any_value() {
        let value = "INV/2026 · ₹1,200";
        let path = qr_path(value);
        assert!(path.starts_with("/_pdf/qr/"));
        assert_eq!(
            unhex(
                path.trim_start_matches("/_pdf/qr/")
                    .trim_end_matches(".svg")
            )
            .as_deref(),
            Some(value)
        );
        assert!(draw(&path).unwrap().unwrap().contains("<svg"));
    }

    #[test]
    fn barcodes_draw_and_refuse_by_kind() {
        assert!(barcode_svg("code128", "ACME-42").unwrap().contains("<rect"));
        assert!(barcode_svg("ean13", "750103131130").is_ok());
        assert!(barcode_svg("ean13", "letters").is_err());
        assert!(barcode_svg("upc", "1").unwrap_err().contains("code128"));
        assert!(draw("/_pdf/other.svg").is_none());
    }
}
