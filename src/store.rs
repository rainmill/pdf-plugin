//! Reading and writing layouts and templates: the boot sync of registered ones,
//! loading for a render, reset to default and duplicate.

use laterite_core::query::{bind_values, build, insert_returning_id};
use laterite_core::{AnyRowExt, Db};
use sea_query::{Alias, Expr, Query};
use serde_json::Value;

use crate::registry::{Format, PdfLayoutReg, PdfTemplateReg};
use crate::Layout;

pub(crate) const LAYOUTS: &str = "pdf_layouts";
pub(crate) const TEMPLATES: &str = "pdf_templates";

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
        ..d
    }
}

const LAYOUT_COLUMNS: [&str; 15] = [
    "code",
    "name",
    "accent",
    "text_colour",
    "body_font",
    "heading_font",
    "size",
    "leading",
    "paper",
    "landscape",
    "margins",
    "header",
    "footer",
    "typst_override",
    "css_override",
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
        l.size.to_string().into(),
        l.leading.to_string().into(),
        l.paper.clone().into(),
        i32::from(l.landscape).into(),
        margins_text(l.margins).into(),
        l.header.clone().into(),
        l.footer.clone().into(),
        l.typst_override.clone().into(),
        l.css_override.clone().into(),
    ]
}

const TEMPLATE_COLUMNS: [&str; 6] = [
    "code",
    "title",
    "description",
    "format",
    "body",
    "sample_data",
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
        match find(db, TEMPLATES, &reg.code).await? {
            None => {
                insert(
                    db,
                    TEMPLATES,
                    &TEMPLATE_COLUMNS,
                    template_values(reg),
                    Some(layout_id),
                )
                .await?;
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
                .await?
            }
            Some((_, true)) => {}
        }
    }
    Ok(())
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
pub async fn layout(db: &Db, id: Option<i64>) -> Result<Layout, sqlx::Error> {
    let Some(id) = id else {
        return Ok(Layout::default());
    };
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(LAYOUT_COLUMNS.map(Alias::new))
            .from(Alias::new(LAYOUTS))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.as_ref().map(layout_from_row).unwrap_or_default())
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
        LAYOUT_COLUMNS.to_vec()
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
                "layout_id" => row.get_int_opt(c).ok().flatten().into(),
                _ => row.get_text_opt(c).ok().flatten().into(),
            };
            values.push(v);
        }
        let mut cols: Vec<&str> = columns.clone();
        cols.push("customized");
        values.push(1i32.into());
        insert(db, table, &cols, values, None)
    };
    Ok(pending.await?)
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
