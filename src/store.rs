//! Reading and writing layouts and templates: the boot sync of registered ones,
//! loading for a render, reset to default and duplicate.

use laterite_core::query::{bind_values, build, insert_returning_id};
use laterite_core::sea_query;
use laterite_core::serde_json;
use laterite_core::sqlx;
use laterite_core::{AnyRowExt, Db};
use sea_query::{Alias, Expr, Query};
use serde_json::Value;

use crate::registry::{Format, PdfLayoutReg, PdfTemplateReg};
use crate::Layout;

pub(crate) const LAYOUTS: &str = "pdf_layouts";
pub(crate) const TEMPLATES: &str = "pdf_templates";
pub(crate) const TRANSLATIONS: &str = "pdf_template_translations";

/// A stored layout: its settings, and what lives beside them.
#[derive(Debug, Clone, Default)]
pub struct StoredLayout {
    /// The settings; `logo` and `background` are filled in at render.
    pub layout: Layout,
    /// The author written into the PDF, or empty.
    pub author: String,
    /// The media record of the masthead logo.
    pub logo_id: Option<i64>,
    /// The media record of the page background.
    pub background_id: Option<i64>,
    /// The media records of the layout's font files.
    pub font_ids: Vec<i64>,
}

/// A stored template, as a render needs it.
#[derive(Debug, Clone)]
pub struct Template {
    /// The row id.
    pub id: i64,
    /// The template's code.
    pub code: String,
    /// The title, also the PDF's.
    pub title: String,
    /// How the body is written.
    pub format: Format,
    /// The body.
    pub body: String,
    /// The layout it is set in, when it names one.
    pub layout_id: Option<i64>,
    /// Overrides the layout's paper when set.
    pub paper: Option<String>,
    /// Overrides the layout's orientation when set.
    pub landscape: bool,
    /// The preview data.
    pub sample_data: Value,
    /// What it is for: the PDF's description.
    pub description: String,
    /// Search keywords written into the PDF.
    pub keywords: Vec<String>,
}

/// Comma-separated keywords as a list.
fn keywords(text: &str) -> Vec<String> {
    text.split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn margins_text(m: [f32; 4]) -> String {
    m.iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The layout a row holds, defaults standing in for empty columns.
fn stored_from_row(row: &sqlx::any::AnyRow) -> StoredLayout {
    let layout = layout_from_row(row);
    StoredLayout {
        author: row
            .get_text_opt("author")
            .ok()
            .flatten()
            .unwrap_or_default(),
        logo_id: row.get_int_opt("logo_id").ok().flatten(),
        background_id: row.get_int_opt("background_id").ok().flatten(),
        font_ids: row
            .get_text_opt("fonts")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        layout,
    }
}

/// The layout settings a row holds, defaults standing in for empty columns.
fn layout_from_row(row: &sqlx::any::AnyRow) -> Layout {
    let d = Layout::default();
    let text = |c: &str| {
        row.get_text_opt(c)
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
    };
    let num = |c: &str, fallback: f32| {
        text(c)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(fallback)
    };
    let margins = text("margins")
        .map(|s| {
            let v: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
            match v.as_slice() {
                [all] => [*all; 4],
                [y, x] => [*y, *x, *y, *x],
                [t, r, b, l] => [*t, *r, *b, *l],
                _ => d.margins,
            }
        })
        .unwrap_or(d.margins);
    Layout {
        accent: text("accent").unwrap_or(d.accent),
        text: text("text_colour").unwrap_or(d.text),
        body_font: text("body_font"),
        heading_font: text("heading_font"),
        title_font: text("title_font"),
        size: num("size", d.size),
        leading: num("leading", d.leading),
        paper: text("paper").unwrap_or(d.paper),
        landscape: row.get_bool("landscape").unwrap_or(false),
        margins,
        header: text("header").unwrap_or_default(),
        footer: row
            .get_text_opt("footer")
            .ok()
            .flatten()
            .unwrap_or(d.footer),
        typst_override: text("typst_override").unwrap_or_default(),
        css_override: text("css_override").unwrap_or_default(),
        logo_height: num("logo_height", d.logo_height),
        ..d
    }
}

const LAYOUT_COLUMNS: [&str; 18] = [
    "code",
    "name",
    "accent",
    "text_colour",
    "body_font",
    "heading_font",
    "title_font",
    "size",
    "leading",
    "paper",
    "landscape",
    "margins",
    "header",
    "footer",
    "typst_override",
    "css_override",
    "logo_height",
    "author",
];

fn layout_values(reg: &PdfLayoutReg) -> Vec<sea_query::SimpleExpr> {
    let l = &reg.layout;
    vec![
        reg.code.clone().into(),
        reg.name.clone().into(),
        l.accent.clone().into(),
        l.text.clone().into(),
        l.body_font.clone().into(),
        l.heading_font.clone().into(),
        l.title_font.clone().into(),
        l.size.to_string().into(),
        l.leading.to_string().into(),
        l.paper.clone().into(),
        i32::from(l.landscape).into(),
        margins_text(l.margins).into(),
        l.header.clone().into(),
        l.footer.clone().into(),
        l.typst_override.clone().into(),
        l.css_override.clone().into(),
        l.logo_height.to_string().into(),
        reg.author.clone().into(),
    ]
}

const TEMPLATE_COLUMNS: [&str; 7] = [
    "code",
    "title",
    "description",
    "format",
    "body",
    "sample_data",
    "keywords",
];

fn template_values(reg: &PdfTemplateReg) -> Vec<sea_query::SimpleExpr> {
    vec![
        reg.code.clone().into(),
        reg.title.clone().into(),
        reg.description.clone().into(),
        reg.format.as_str().into(),
        reg.body.clone().into(),
        serde_json::to_string_pretty(&reg.sample_data)
            .unwrap_or_default()
            .into(),
        reg.keywords.join(", ").into(),
    ]
}

/// The id and customised flag of the row with `code`, when there is one.
async fn find(db: &Db, table: &str, code: &str) -> Result<Option<(i64, bool)>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns([Alias::new("id"), Alias::new("customized")])
            .from(Alias::new(table))
            .and_where(Expr::col(Alias::new("code")).eq(code))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.map(|r| {
        (
            r.get_int("id").unwrap_or_default(),
            r.get_bool("customized").unwrap_or(false),
        )
    }))
}

/// Writes registered layouts and templates into the database: a missing one is
/// inserted, one not customised in the admin is brought up to date with its code,
/// and a customised one is left alone.
pub async fn sync(
    db: &Db,
    layouts: &[&PdfLayoutReg],
    templates: &[&PdfTemplateReg],
) -> Result<(), sqlx::Error> {
    for reg in layouts {
        match find(db, LAYOUTS, &reg.code).await? {
            None => {
                insert(db, LAYOUTS, &LAYOUT_COLUMNS, layout_values(reg), None).await?;
            }
            Some((id, false)) => {
                update(db, LAYOUTS, id, &LAYOUT_COLUMNS, layout_values(reg), None).await?
            }
            Some((_, true)) => {}
        }
    }
    for reg in templates {
        let layout_id = match &reg.layout {
            Some(code) => find(db, LAYOUTS, code).await?.map(|(id, _)| id),
            None => None,
        };
        let id = match find(db, TEMPLATES, &reg.code).await? {
            None => {
                insert(
                    db,
                    TEMPLATES,
                    &TEMPLATE_COLUMNS,
                    template_values(reg),
                    Some(layout_id),
                )
                .await?
            }
            Some((id, false)) => {
                update(
                    db,
                    TEMPLATES,
                    id,
                    &TEMPLATE_COLUMNS,
                    template_values(reg),
                    Some(layout_id),
                )
                .await?;
                id
            }
            Some((_, true)) => continue,
        };
        sync_translations(db, id, reg).await?;
    }
    Ok(())
}

/// Writes a registered template's translations under the row `template_id`.
async fn sync_translations(
    db: &Db,
    template_id: i64,
    reg: &PdfTemplateReg,
) -> Result<(), sqlx::Error> {
    for (locale, title, body) in &reg.translations {
        let existing = {
            let (sql, values) = build(
                db.backend,
                Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new(TRANSLATIONS))
                    .and_where(Expr::col(Alias::new("template_id")).eq(template_id))
                    .and_where(Expr::col(Alias::new("locale")).eq(locale.as_str()))
                    .to_owned(),
            );
            bind_values(sqlx::query(&sql), values)
                .fetch_optional(&db.pool)
                .await?
                .and_then(|r| r.get_int("id").ok())
        };
        let (sql, values) = match existing {
            Some(id) => build(
                db.backend,
                Query::update()
                    .table(Alias::new(TRANSLATIONS))
                    .value(Alias::new("title"), title.as_str())
                    .value(Alias::new("body"), body.as_str())
                    .value(Alias::new("updated_at"), now())
                    .and_where(Expr::col(Alias::new("id")).eq(id))
                    .to_owned(),
            ),
            None => build(
                db.backend,
                Query::insert()
                    .into_table(Alias::new(TRANSLATIONS))
                    .columns(
                        [
                            "template_id",
                            "locale",
                            "title",
                            "body",
                            "created_at",
                            "updated_at",
                        ]
                        .map(Alias::new),
                    )
                    .values_panic([
                        template_id.into(),
                        locale.as_str().into(),
                        title.as_str().into(),
                        body.as_str().into(),
                        now().into(),
                        now().into(),
                    ])
                    .to_owned(),
            ),
        };
        bind_values(sqlx::query(&sql), values)
            .execute(&db.pool)
            .await?;
    }
    Ok(())
}

/// The title and body of template `template_id` in the first of `locales` it
/// has; an empty title or body keeps the template's own.
pub async fn translation(
    db: &Db,
    template_id: i64,
    locales: &[String],
) -> Result<Option<(Option<String>, Option<String>)>, sqlx::Error> {
    if locales.is_empty() {
        return Ok(None);
    }
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(["locale", "title", "body"].map(Alias::new))
            .from(Alias::new(TRANSLATIONS))
            .and_where(Expr::col(Alias::new("template_id")).eq(template_id))
            .and_where(Expr::col(Alias::new("locale")).is_in(locales.iter().map(String::as_str)))
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&db.pool)
        .await?;
    let nonempty = |r: &sqlx::any::AnyRow, c: &str| {
        r.get_text_opt(c)
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
    };
    Ok(locales.iter().find_map(|wanted| {
        rows.iter()
            .find(|r| {
                r.get_text("locale")
                    .is_ok_and(|l| l.eq_ignore_ascii_case(wanted))
            })
            .map(|r| (nonempty(r, "title"), nonempty(r, "body")))
    }))
}

/// Inserts a row and returns its id. Renders the statement before anything is
/// awaited, so the future holds no query builder and stays `Send`.
fn insert<'a>(
    db: &'a Db,
    table: &str,
    columns: &[&str],
    mut values: Vec<sea_query::SimpleExpr>,
    layout_id: Option<Option<i64>>,
) -> impl std::future::Future<Output = Result<i64, sqlx::Error>> + Send + 'a {
    let stamp = now();
    let mut cols: Vec<Alias> = columns.iter().map(|c| Alias::new(*c)).collect();
    cols.extend([Alias::new("created_at"), Alias::new("updated_at")]);
    values.extend([stamp.clone().into(), stamp.into()]);
    if let Some(layout) = layout_id {
        cols.push(Alias::new("layout_id"));
        values.push(layout.into());
    }
    let stmt = Query::insert()
        .into_table(Alias::new(table))
        .columns(cols)
        .values_panic(values)
        .to_owned();
    insert_returning_id(db, stmt, Alias::new("id"))
}

/// Updates a row with `values`, clearing its customised flag. Rendered before
/// anything is awaited, like [`insert`].
fn update<'a>(
    db: &'a Db,
    table: &str,
    id: i64,
    columns: &[&str],
    values: Vec<sea_query::SimpleExpr>,
    layout_id: Option<Option<i64>>,
) -> impl std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a {
    let mut stmt = Query::update();
    stmt.table(Alias::new(table));
    for (c, v) in columns.iter().zip(values) {
        stmt.value(Alias::new(*c), v);
    }
    if let Some(layout) = layout_id {
        stmt.value(Alias::new("layout_id"), layout);
    }
    stmt.value(Alias::new("customized"), 0i32)
        .value(Alias::new("updated_at"), now())
        .and_where(Expr::col(Alias::new("id")).eq(id));
    let (sql, values) = build(db.backend, stmt.to_owned());
    async move {
        bind_values(sqlx::query(&sql), values)
            .execute(&db.pool)
            .await?;
        Ok(())
    }
}

/// The layout with `id`, or the default layout when there is none.
pub async fn layout(db: &Db, id: Option<i64>) -> Result<StoredLayout, sqlx::Error> {
    let Some(id) = id else {
        return Ok(StoredLayout::default());
    };
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(LAYOUT_COLUMNS.map(Alias::new))
            .columns(["logo_id", "background_id", "fonts"].map(Alias::new))
            .from(Alias::new(LAYOUTS))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.as_ref().map(stored_from_row).unwrap_or_default())
}

/// The template with `code`, or with `id` when `code` is `None`.
pub async fn template(
    db: &Db,
    code: Option<&str>,
    id: Option<i64>,
) -> Result<Option<Template>, sqlx::Error> {
    // The builder lives only in this block, so nothing un-`Send` crosses the await.
    let (sql, values) = {
        let mut select = Query::select();
        select
            .columns(
                [
                    "id",
                    "code",
                    "title",
                    "format",
                    "body",
                    "layout_id",
                    "paper",
                    "landscape",
                    "sample_data",
                    "description",
                    "keywords",
                ]
                .map(Alias::new),
            )
            .from(Alias::new(TEMPLATES));
        match (code, id) {
            (Some(c), _) => select.and_where(Expr::col(Alias::new("code")).eq(c)),
            (None, Some(i)) => select.and_where(Expr::col(Alias::new("id")).eq(i)),
            (None, None) => return Ok(None),
        };
        build(db.backend, select.to_owned())
    };
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.map(|r| Template {
        id: r.get_int("id").unwrap_or_default(),
        code: r.get_text("code").unwrap_or_default(),
        title: r.get_text("title").unwrap_or_default(),
        format: Format::parse(&r.get_text("format").unwrap_or_default()),
        body: r.get_text_opt("body").ok().flatten().unwrap_or_default(),
        layout_id: r.get_int_opt("layout_id").ok().flatten(),
        paper: r
            .get_text_opt("paper")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty()),
        landscape: r.get_bool("landscape").unwrap_or(false),
        sample_data: r
            .get_text_opt("sample_data")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Object(Default::default())),
        description: r
            .get_text_opt("description")
            .ok()
            .flatten()
            .unwrap_or_default(),
        keywords: keywords(
            &r.get_text_opt("keywords")
                .ok()
                .flatten()
                .unwrap_or_default(),
        ),
    }))
}

/// The code of the row `id` in `table`.
async fn code_of(db: &Db, table: &str, id: i64) -> Result<Option<String>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .column(Alias::new("code"))
            .from(Alias::new(table))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.and_then(|r| r.get_text("code").ok()))
}

/// Why a reset or duplicate did nothing.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// No row has that id.
    #[error("not found")]
    NotFound,
    /// The row was made in the admin: no code ships it, so there is no default.
    #[error("not registered by any module, so it has no default")]
    NotRegistered,
    /// The database refused.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Restores the registered layout `id` from its code.
pub async fn reset_layout(db: &Db, id: i64, regs: &[&PdfLayoutReg]) -> Result<(), StoreError> {
    let code = code_of(db, LAYOUTS, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let reg = regs
        .iter()
        .find(|r| r.code == code)
        .ok_or(StoreError::NotRegistered)?;
    update(db, LAYOUTS, id, &LAYOUT_COLUMNS, layout_values(reg), None).await?;
    Ok(())
}

/// Restores the registered template `id` from its code.
pub async fn reset_template(db: &Db, id: i64, regs: &[&PdfTemplateReg]) -> Result<(), StoreError> {
    let code = code_of(db, TEMPLATES, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let reg = regs
        .iter()
        .find(|r| r.code == code)
        .ok_or(StoreError::NotRegistered)?;
    let layout_id = match &reg.layout {
        Some(c) => find(db, LAYOUTS, c).await?.map(|(i, _)| i),
        None => None,
    };
    update(
        db,
        TEMPLATES,
        id,
        &TEMPLATE_COLUMNS,
        template_values(reg),
        Some(layout_id),
    )
    .await?;
    Ok(())
}

/// A free code for a copy of `code`: `code_copy`, then `code_copy2` and on.
async fn copy_code(db: &Db, table: &str, code: &str) -> Result<String, sqlx::Error> {
    let base = format!("{code}_copy");
    if find(db, table, &base).await?.is_none() {
        return Ok(base);
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}{n}");
        if find(db, table, &candidate).await?.is_none() {
            return Ok(candidate);
        }
        n += 1;
    }
}

/// Copies the row `id` in `table` under a free code, marked customised, and
/// returns the copy's id.
pub async fn duplicate(db: &Db, table: &str, id: i64) -> Result<i64, StoreError> {
    let code = code_of(db, table, id).await?.ok_or(StoreError::NotFound)?;
    let new_code = copy_code(db, table, &code).await?;
    let columns: Vec<&str> = if table == LAYOUTS {
        let mut c = LAYOUT_COLUMNS.to_vec();
        c.extend(["logo_id", "background_id", "fonts"]);
        c
    } else {
        [
            "code",
            "title",
            "description",
            "format",
            "body",
            "layout_id",
            "paper",
            "landscape",
            "sample_data",
            "keywords",
        ]
        .to_vec()
    };
    // Copy column by column through the row, so the copy is portable across backends.
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(columns.iter().map(|c| Alias::new(*c)))
            .from(Alias::new(table))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_one(&db.pool)
        .await?;
    let pending = {
        let mut values: Vec<sea_query::SimpleExpr> = Vec::new();
        for c in &columns {
            let v: sea_query::SimpleExpr = match *c {
                "code" => new_code.clone().into(),
                "landscape" => i32::from(row.get_bool(c).unwrap_or(false)).into(),
                "layout_id" | "logo_id" | "background_id" => {
                    row.get_int_opt(c).ok().flatten().into()
                }
                _ => row.get_text_opt(c).ok().flatten().into(),
            };
            values.push(v);
        }
        let mut cols: Vec<&str> = columns.clone();
        cols.push("customized");
        values.push(1i32.into());
        insert(db, table, &cols, values, None)
    };
    let copy = pending.await?;
    if table == TEMPLATES {
        copy_translations(db, id, copy).await?;
    }
    Ok(copy)
}

/// Copies template `from`'s translations onto template `to`.
async fn copy_translations(db: &Db, from: i64, to: i64) -> Result<(), sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(["locale", "title", "body"].map(Alias::new))
            .from(Alias::new(TRANSLATIONS))
            .and_where(Expr::col(Alias::new("template_id")).eq(from))
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&db.pool)
        .await?;
    for r in rows {
        let text = |c: &str| r.get_text_opt(c).ok().flatten();
        let (sql, values) = build(
            db.backend,
            Query::insert()
                .into_table(Alias::new(TRANSLATIONS))
                .columns(
                    [
                        "template_id",
                        "locale",
                        "title",
                        "body",
                        "created_at",
                        "updated_at",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    to.into(),
                    text("locale").into(),
                    text("title").into(),
                    text("body").into(),
                    now().into(),
                    now().into(),
                ])
                .to_owned(),
        );
        bind_values(sqlx::query(&sql), values)
            .execute(&db.pool)
            .await?;
    }
    Ok(())
}

/// How many templates are set in the layout `id`.
pub async fn templates_using(db: &Db, layout_id: i64) -> Result<i64, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .expr(Expr::col(Alias::new("id")).count())
            .from(Alias::new(TEMPLATES))
            .and_where(Expr::col(Alias::new("layout_id")).eq(layout_id))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_one(&db.pool)
        .await?;
    Ok(sqlx::Row::try_get::<i64, _>(&row, 0).unwrap_or_default())
}

/// Every layout, as a module would register it: code, name, author and
/// settings. Media (logo, background) stays behind: its ids belong to this
/// database.
pub async fn all_layouts(db: &Db) -> Result<Vec<PdfLayoutReg>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(LAYOUT_COLUMNS.map(Alias::new))
            .from(Alias::new(LAYOUTS))
            .order_by(Alias::new("code"), sea_query::Order::Asc)
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&db.pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let stored = stored_from_row(r);
            PdfLayoutReg::new(
                r.get_text("code").unwrap_or_default(),
                r.get_text("name").unwrap_or_default(),
                stored.layout,
            )
            .author(stored.author)
        })
        .collect())
}

/// Every template, as a module would register it, translations included.
pub async fn all_templates(db: &Db) -> Result<Vec<PdfTemplateReg>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .expr_as(
                Expr::col((Alias::new(TEMPLATES), Alias::new("id"))),
                Alias::new("id"),
            )
            .columns(
                [
                    "code",
                    "title",
                    "description",
                    "format",
                    "body",
                    "sample_data",
                    "keywords",
                ]
                .map(|c| (Alias::new(TEMPLATES), Alias::new(c))),
            )
            .expr_as(
                Expr::col((Alias::new(LAYOUTS), Alias::new("code"))),
                Alias::new("layout_code"),
            )
            .from(Alias::new(TEMPLATES))
            .left_join(
                Alias::new(LAYOUTS),
                Expr::col((Alias::new(LAYOUTS), Alias::new("id")))
                    .equals((Alias::new(TEMPLATES), Alias::new("layout_id"))),
            )
            .order_by(
                (Alias::new(TEMPLATES), Alias::new("code")),
                sea_query::Order::Asc,
            )
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&db.pool)
        .await?;
    let mut out = Vec::new();
    for r in &rows {
        let text = |c: &str| r.get_text_opt(c).ok().flatten().unwrap_or_default();
        let mut reg = PdfTemplateReg::new(
            text("code"),
            text("title"),
            Format::parse(&text("format")),
            text("body"),
        )
        .description(text("description"))
        .keywords(keywords(&text("keywords")))
        .sample(
            serde_json::from_str(&text("sample_data")).unwrap_or(Value::Object(Default::default())),
        );
        if let Some(layout) = r.get_text_opt("layout_code").ok().flatten() {
            reg = reg.layout(layout);
        }
        let id = r.get_int("id").unwrap_or_default();
        let (sql, values) = build(
            db.backend,
            Query::select()
                .columns(["locale", "title", "body"].map(Alias::new))
                .from(Alias::new(TRANSLATIONS))
                .and_where(Expr::col(Alias::new("template_id")).eq(id))
                .order_by(Alias::new("locale"), sea_query::Order::Asc)
                .to_owned(),
        );
        for t in bind_values(sqlx::query(&sql), values)
            .fetch_all(&db.pool)
            .await?
        {
            let text = |c: &str| t.get_text_opt(c).ok().flatten().unwrap_or_default();
            reg = reg.translation(text("locale"), text("title"), text("body"));
        }
        out.push(reg);
    }
    Ok(out)
}

/// Writes `layouts` and `templates` into the database by code, over whatever
/// is there, each marked customised so the boot sync leaves it as imported.
/// Returns how many of each it wrote.
pub async fn import(
    db: &Db,
    layouts: &[PdfLayoutReg],
    templates: &[PdfTemplateReg],
) -> Result<(usize, usize), sqlx::Error> {
    for reg in layouts {
        let id = match find(db, LAYOUTS, &reg.code).await? {
            Some((id, _)) => {
                update(db, LAYOUTS, id, &LAYOUT_COLUMNS, layout_values(reg), None).await?;
                id
            }
            None => insert(db, LAYOUTS, &LAYOUT_COLUMNS, layout_values(reg), None).await?,
        };
        mark_customized(db, LAYOUTS, id).await?;
    }
    for reg in templates {
        let layout_id = match &reg.layout {
            Some(code) => find(db, LAYOUTS, code).await?.map(|(id, _)| id),
            None => None,
        };
        let id = match find(db, TEMPLATES, &reg.code).await? {
            Some((id, _)) => {
                update(
                    db,
                    TEMPLATES,
                    id,
                    &TEMPLATE_COLUMNS,
                    template_values(reg),
                    Some(layout_id),
                )
                .await?;
                id
            }
            None => {
                insert(
                    db,
                    TEMPLATES,
                    &TEMPLATE_COLUMNS,
                    template_values(reg),
                    Some(layout_id),
                )
                .await?
            }
        };
        sync_translations(db, id, reg).await?;
        mark_customized(db, TEMPLATES, id).await?;
    }
    Ok((layouts.len(), templates.len()))
}

async fn mark_customized(db: &Db, table: &str, id: i64) -> Result<(), sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::update()
            .table(Alias::new(table))
            .value(Alias::new("customized"), 1i32)
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned(),
    );
    bind_values(sqlx::query(&sql), values)
        .execute(&db.pool)
        .await?;
    Ok(())
}
