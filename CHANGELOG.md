# Changelog

Notable changes to `rainmill-pdf`. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) as Cargo reads it: before 1.0 a minor
bump is breaking and a patch is additive.

## [Unreleased]

### Added

- A layout takes font files from the media library and a title font; `PdfFontReg` registers fonts from code.
- The Typst override runs before the masthead and can redefine `doc-title` and `doc-meta`.
- First release. PDF documents for Laterite with Typst: markdown or Typst templates, letterhead layouts, a live preview, QR codes and barcodes, PDF/A and password protection.
