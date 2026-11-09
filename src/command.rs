//! `lat pdf`: sync, check, render, export and import templates and layouts
//! from the command line.

use std::path::{Path, PathBuf};

use laterite_core::command::{Command, CommandCx, CommandError};
use laterite_core::source::{parse, render, Format as FileFormat};
use serde_json::{Map, Value};

use crate::registry::{Format, PdfLayoutReg, PdfTemplateReg};
use crate::{store, Layout, Output, Pdf, RenderOptions};

/// The usage line each mistake prints.
const USAGE: &str = "usage: pdf sync | check | render <code> [--data FILE] [--locale L] [--out FILE] [--archival] | export [DIR] | import DIR";

pub(crate) struct PdfCommand;

/// The words after a verb: positionals, and `--name value` or `--flag` options.
struct Words<'a> {
    positional: Vec<&'a str>,
    options: Vec<(&'a str, Option<&'a str>)>,
}

impl<'a> Words<'a> {
    fn read(args: &'a [String], flags: &[&str]) -> Self {
        let (mut positional, mut options) = (Vec::new(), Vec::new());
        let mut it = args.iter().map(String::as_str);
        while let Some(word) = it.next() {
            match word.strip_prefix("--") {
                Some(name) if flags.contains(&name) => options.push((name, None)),
                Some(name) => options.push((name, it.next())),
                None => positional.push(word),
            }
        }
        Self {
            positional,
            options,
        }
    }

    fn option(&self, name: &str) -> Option<&'a str> {
        self.options
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, v)| *v)
    }

    fn flag(&self, name: &str) -> bool {
        self.options.iter().any(|(n, _)| *n == name)
    }
}

#[laterite_core::strata::async_trait]
impl Command for PdfCommand {
    async fn run(&self, cx: &CommandCx<'_>, args: &[String]) -> Result<(), CommandError> {
        let (verb, rest) = args.split_first().ok_or(USAGE)?;
        match verb.as_str() {
            "sync" => sync(cx).await,
            "check" => check(cx).await,
            "render" => render_one(cx, rest).await,
            "export" => export(cx, rest).await,
            "import" => import(cx, rest).await,
            _ => Err(USAGE.into()),
        }
    }
}

/// Writes the modules' templates and layouts into the database, as the boot does.
async fn sync(cx: &CommandCx<'_>) -> Result<(), CommandError> {
    let layouts = cx.registry().items::<PdfLayoutReg>();
    let templates = cx.registry().items::<PdfTemplateReg>();
    store::sync(cx.db(), &layouts, &templates).await?;
    println!(
        "Synced {} layouts and {} templates the modules ship; customised ones were left alone.",
        layouts.len(),
        templates.len()
    );
    Ok(())
}

/// Renders every template with its sample data and reports each one.
async fn check(cx: &CommandCx<'_>) -> Result<(), CommandError> {
    let pdf = pdf(cx);
    let templates = store::all_templates(cx.db()).await?;
    let mut failed = 0;
    for t in &templates {
        match pdf.render(&t.code, t.sample_data.clone()).await {
            Ok(_) => println!("ok    {}", t.code),
            Err(e) => {
                failed += 1;
                println!("FAIL  {}: {e}", t.code);
            }
        }
    }
    println!("{} templates, {failed} failed.", templates.len());
    if failed > 0 {
        return Err(format!("{failed} templates did not render").into());
    }
    Ok(())
}

/// Renders one template to a file.
async fn render_one(cx: &CommandCx<'_>, args: &[String]) -> Result<(), CommandError> {
    let words = Words::read(args, &["archival"]);
    let code = *words.positional.first().ok_or(USAGE)?;
    let data: Value = match words.option("data") {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => store::template(cx.db(), Some(code), None)
            .await?
            .map(|t| t.sample_data)
            .unwrap_or(Value::Null),
    };
    let options = RenderOptions {
        locale: words.option("locale").map(str::to_string),
        output: if words.flag("archival") {
            Output::archival()
        } else {
            Output::default()
        },
        ..RenderOptions::default()
    };
    let rendered = pdf(cx).render_with(code, data, &options).await?;
    let out = words
        .option("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&rendered.filename));
    std::fs::write(&out, &rendered.bytes)?;
    println!("Wrote {} ({} bytes).", out.display(), rendered.bytes.len());
    Ok(())
}

/// Writes every template and layout as files: `layouts/<code>.yaml`,
/// `templates/<code>.md` (front matter, then the body) and each translation
/// at `templates/<code>/<locale>.md`.
async fn export(cx: &CommandCx<'_>, args: &[String]) -> Result<(), CommandError> {
    let words = Words::read(args, &[]);
    let dir = PathBuf::from(words.positional.first().copied().unwrap_or("pdf-export"));
    let layouts = store::all_layouts(cx.db()).await?;
    let templates = store::all_templates(cx.db()).await?;
    std::fs::create_dir_all(dir.join("layouts"))?;
    std::fs::create_dir_all(dir.join("templates"))?;
    for l in &layouts {
        let mut attrs = match serde_json::to_value(&l.layout)? {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        // Media ids belong to one database.
        attrs.remove("logo");
        attrs.remove("background");
        attrs.insert("name".into(), l.name.clone().into());
        attrs.insert("author".into(), l.author.clone().into());
        let path = dir.join("layouts").join(format!("{}.yaml", l.code));
        std::fs::write(path, render(FileFormat::Yaml, &attrs, None))?;
    }
    for t in &templates {
        let mut attrs = Map::new();
        attrs.insert("title".into(), t.title.clone().into());
        attrs.insert("description".into(), t.description.clone().into());
        attrs.insert("format".into(), t.format.as_str().into());
        if let Some(layout) = &t.layout {
            attrs.insert("layout".into(), layout.clone().into());
        }
        attrs.insert("keywords".into(), t.keywords.clone().into());
        attrs.insert("sample_data".into(), t.sample_data.clone());
        let path = dir.join("templates").join(format!("{}.md", t.code));
        std::fs::write(path, render(FileFormat::Markdown, &attrs, Some(&t.body)))?;
        for (locale, title, body) in &t.translations {
            let folder = dir.join("templates").join(&t.code);
            std::fs::create_dir_all(&folder)?;
            let mut attrs = Map::new();
            attrs.insert("title".into(), title.clone().into());
            std::fs::write(
                folder.join(format!("{locale}.md")),
                render(FileFormat::Markdown, &attrs, Some(body)),
            )?;
        }
    }
    println!(
        "Exported {} layouts and {} templates to {}.",
        layouts.len(),
        templates.len(),
        dir.display()
    );
    Ok(())
}

/// Reads an export back in, writing each template and layout over the one with
/// its code and marking it customised.
async fn import(cx: &CommandCx<'_>, args: &[String]) -> Result<(), CommandError> {
    let words = Words::read(args, &[]);
    let dir = PathBuf::from(*words.positional.first().ok_or(USAGE)?);
    let layouts = read_layouts(&dir.join("layouts"))?;
    let templates = read_templates(&dir.join("templates"))?;
    let (l, t) = store::import(cx.db(), &layouts, &templates).await?;
    println!(
        "Imported {l} layouts and {t} templates from {}.",
        dir.display()
    );
    Ok(())
}

fn files(dir: &Path, extension: &str) -> Result<Vec<(String, String)>, CommandError> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some(extension) {
            continue;
        }
        let code = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        out.push((code, std::fs::read_to_string(&path)?));
    }
    out.sort();
    Ok(out)
}

fn read_layouts(dir: &Path) -> Result<Vec<PdfLayoutReg>, CommandError> {
    let mut out = Vec::new();
    for (code, text) in files(dir, "yaml")? {
        let (mut attrs, _) = parse(FileFormat::Yaml, &text).map_err(|e| format!("{code}: {e}"))?;
        let text_of = |attrs: &mut Map<String, Value>, k: &str| match attrs.remove(k) {
            Some(Value::String(s)) => s,
            _ => String::new(),
        };
        let name = text_of(&mut attrs, "name");
        let author = text_of(&mut attrs, "author");
        let layout: Layout = serde_json::from_value(Value::Object(attrs))
            .map_err(|e| format!("layout {code}: {e}"))?;
        out.push(PdfLayoutReg::new(code, name, layout).author(author));
    }
    Ok(out)
}

fn read_templates(dir: &Path) -> Result<Vec<PdfTemplateReg>, CommandError> {
    let mut out = Vec::new();
    for (code, text) in files(dir, "md")? {
        let (attrs, body) =
            parse(FileFormat::Markdown, &text).map_err(|e| format!("{code}: {e}"))?;
        let get = |k: &str| attrs.get(k).and_then(Value::as_str).unwrap_or_default();
        let mut reg = PdfTemplateReg::new(
            code.clone(),
            get("title"),
            Format::parse(get("format")),
            body.unwrap_or_default(),
        )
        .description(get("description"))
        .keywords(
            attrs
                .get("keywords")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        )
        .sample(
            attrs
                .get("sample_data")
                .cloned()
                .unwrap_or(Value::Object(Map::new())),
        );
        if !get("layout").is_empty() {
            reg = reg.layout(get("layout"));
        }
        for (locale, text) in files(&dir.join(&code), "md")? {
            let (attrs, body) =
                parse(FileFormat::Markdown, &text).map_err(|e| format!("{code}/{locale}: {e}"))?;
            let title = attrs
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            reg = reg.translation(locale, title, body.unwrap_or_default());
        }
        out.push(reg);
    }
    Ok(out)
}

fn pdf(cx: &CommandCx<'_>) -> Pdf {
    Pdf::new(cx.db().clone(), cx.registry()).library(cx.registry(), cx.storage_dir())
}

/// The command's registration.
pub(crate) fn registration() -> laterite_core::CommandReg {
    laterite_core::CommandReg::new(
        "pdf",
        laterite_core::t!("Sync, check, render, export and import PDF templates"),
        std::sync::Arc::new(PdfCommand),
    )
}
