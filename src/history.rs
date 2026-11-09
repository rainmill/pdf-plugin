//! History: a snapshot of a template or layout after each save in the admin,
//! the last [`KEEP`] kept, shown as word-level differences and restorable.

use laterite_core::query::{bind_values, build};
use laterite_core::{AnyRowExt, Db, DbBackend};
use sea_query::{Alias, Expr, Order, Query};
use serde_json::{Map, Value};
use sqlx::AnyConnection;

use crate::store::{LAYOUTS, TEMPLATES};

/// How many saves a record keeps.
pub const KEEP: usize = 20;
pub(crate) const REVISIONS: &str = "pdf_revisions";

/// How a column is read and written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    /// Prose or code, diffed word by word.
    Long,
    Int,
    Bool,
}

/// The columns a snapshot holds, with their labels, per table.
fn columns(table: &str) -> &'static [(&'static str, &'static str, Kind)] {
    use Kind::*;
    if table == TEMPLATES {
        &[
            ("title", "Title", Text),
            ("description", "Description", Long),
            ("format", "Format", Text),
            ("layout_id", "Layout", Int),
            ("paper", "Paper", Text),
            ("landscape", "Landscape", Bool),
            ("keywords", "Keywords", Text),
            ("body", "Body", Long),
            ("sample_data", "Sample data", Long),
        ]
    } else {
        &[
            ("name", "Name", Text),
            ("logo_id", "Logo", Int),
            ("logo_height", "Logo height", Text),
            ("background_id", "Page background", Int),
            ("author", "Author", Text),
            ("accent", "Accent", Text),
            ("text_colour", "Text colour", Text),
            ("body_font", "Body font", Text),
            ("heading_font", "Heading font", Text),
            ("size", "Size", Text),
            ("leading", "Line spacing", Text),
            ("paper", "Paper", Text),
            ("margins", "Margins", Text),
            ("landscape", "Landscape", Bool),
            ("header", "Header", Long),
            ("footer", "Footer", Long),
            ("typst_override", "Typst override", Long),
            ("css_override", "CSS override", Long),
        ]
    }
}

/// One saved version.
#[derive(Debug, Clone)]
pub struct Revision {
    /// The revision's id.
    pub id: i64,
    /// What the record held, by column.
    pub snapshot: Map<String, Value>,
    /// Who saved it.
    pub author: String,
    /// When, as RFC 3339 UTC.
    pub created_at: String,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The record `id` in `table` as a snapshot, or `None` when it is gone.
async fn snapshot(
    conn: &mut AnyConnection,
    backend: DbBackend,
    table: &str,
    id: i64,
) -> Result<Option<Map<String, Value>>, sqlx::Error> {
    let cols = columns(table);
    let (sql, values) = build(
        backend,
        Query::select()
            .columns(cols.iter().map(|(c, _, _)| Alias::new(*c)))
            .from(Alias::new(table))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned(),
    );
    let Some(row) = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&mut *conn)
        .await?
    else {
        return Ok(None);
    };
    let mut map = Map::new();
    for (c, _, kind) in cols {
        let value = match kind {
            Kind::Int => row
                .get_int_opt(c)
                .ok()
                .flatten()
                .map(Value::from)
                .unwrap_or(Value::Null),
            Kind::Bool => Value::Bool(row.get_bool(c).unwrap_or(false)),
            Kind::Text | Kind::Long => row
                .get_text_opt(c)
                .ok()
                .flatten()
                .map(Value::from)
                .unwrap_or(Value::Null),
        };
        map.insert(c.to_string(), value);
    }
    Ok(Some(map))
}

/// How many versions the record has.
async fn count(
    conn: &mut AnyConnection,
    backend: DbBackend,
    table: &str,
    id: i64,
) -> Result<i64, sqlx::Error> {
    let (sql, values) = build(
        backend,
        Query::select()
            .expr(Expr::col(Alias::new("id")).count())
            .from(Alias::new(REVISIONS))
            .and_where(Expr::col(Alias::new("entity")).eq(table))
            .and_where(Expr::col(Alias::new("record_id")).eq(id))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_one(&mut *conn)
        .await?;
    Ok(sqlx::Row::try_get::<i64, _>(&row, 0).unwrap_or_default())
}

/// Records what `id` holds now as a version by `author`, then drops all but
/// the newest [`KEEP`]. With `baseline`, records only when it has none yet:
/// the state before the first tracked save.
pub(crate) async fn record(db: &Db, table: &str, id: i64, author: &str) -> Result<(), sqlx::Error> {
    let mut conn = db.pool.acquire().await?;
    record_on(&mut conn, db.backend, table, id, author, false).await
}

/// [`record`] on a connection the caller holds, such as a save's transaction.
pub(crate) async fn record_on(
    conn: &mut AnyConnection,
    backend: DbBackend,
    table: &str,
    id: i64,
    author: &str,
    baseline: bool,
) -> Result<(), sqlx::Error> {
    if baseline && count(conn, backend, table, id).await? > 0 {
        return Ok(());
    }
    let Some(snapshot) = snapshot(conn, backend, table, id).await? else {
        return Ok(());
    };
    let (sql, values) = build(
        backend,
        Query::insert()
            .into_table(Alias::new(REVISIONS))
            .columns(["entity", "record_id", "snapshot", "author", "created_at"].map(Alias::new))
            .values_panic([
                table.into(),
                id.into(),
                Value::Object(snapshot).to_string().into(),
                author.into(),
                now().into(),
            ])
            .to_owned(),
    );
    bind_values(sqlx::query(&sql), values)
        .execute(&mut *conn)
        .await?;
    prune(conn, backend, table, id).await
}

/// Drops all but the newest [`KEEP`] versions of a record.
async fn prune(
    conn: &mut AnyConnection,
    backend: DbBackend,
    table: &str,
    id: i64,
) -> Result<(), sqlx::Error> {
    let old: Vec<i64> = list_on(conn, backend, table, id)
        .await?
        .into_iter()
        .skip(KEEP)
        .map(|r| r.id)
        .collect();
    if old.is_empty() {
        return Ok(());
    }
    let (sql, values) = build(
        backend,
        Query::delete()
            .from_table(Alias::new(REVISIONS))
            .and_where(Expr::col(Alias::new("id")).is_in(old))
            .to_owned(),
    );
    bind_values(sqlx::query(&sql), values)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The versions of record `id` in `table`, newest first.
pub async fn list(db: &Db, table: &str, id: i64) -> Result<Vec<Revision>, sqlx::Error> {
    let mut conn = db.pool.acquire().await?;
    list_on(&mut conn, db.backend, table, id).await
}

async fn list_on(
    conn: &mut AnyConnection,
    backend: DbBackend,
    table: &str,
    id: i64,
) -> Result<Vec<Revision>, sqlx::Error> {
    let (sql, values) = build(
        backend,
        Query::select()
            .columns(["id", "snapshot", "author", "created_at"].map(Alias::new))
            .from(Alias::new(REVISIONS))
            .and_where(Expr::col(Alias::new("entity")).eq(table))
            .and_where(Expr::col(Alias::new("record_id")).eq(id))
            .order_by(Alias::new("id"), Order::Desc)
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows
        .iter()
        .map(|r| Revision {
            id: r.get_int("id").unwrap_or_default(),
            snapshot: r
                .get_text("snapshot")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            author: r.get_text_opt("author").ok().flatten().unwrap_or_default(),
            created_at: r.get_text("created_at").unwrap_or_default(),
        })
        .collect())
}

/// Puts version `revision` of record `id` back, marked customised, and records
/// the restore as a version of its own. `false` when no such version exists.
pub async fn restore(
    db: &Db,
    table: &str,
    id: i64,
    revision: i64,
    author: &str,
) -> Result<bool, sqlx::Error> {
    let Some(rev) = list(db, table, id)
        .await?
        .into_iter()
        .find(|r| r.id == revision)
    else {
        return Ok(false);
    };
    let (sql, values) = {
        let mut stmt = Query::update();
        stmt.table(Alias::new(table));
        for (c, _, kind) in columns(table) {
            let v = rev.snapshot.get(*c).cloned().unwrap_or(Value::Null);
            let expr: sea_query::SimpleExpr = match kind {
                Kind::Int => v.as_i64().into(),
                Kind::Bool => i32::from(v.as_bool().unwrap_or(false)).into(),
                Kind::Text | Kind::Long => v.as_str().map(str::to_string).into(),
            };
            stmt.value(Alias::new(*c), expr);
        }
        stmt.value(Alias::new("customized"), 1i32)
            .value(Alias::new("updated_at"), now())
            .and_where(Expr::col(Alias::new("id")).eq(id));
        build(db.backend, stmt.to_owned())
    };
    bind_values(sqlx::query(&sql), values)
        .execute(&db.pool)
        .await?;
    record(db, table, id, author).await?;
    Ok(true)
}

/// Drops every version of a record, with the record.
pub(crate) fn drop_sql(backend: DbBackend, table: &str, id: i64) -> (String, sea_query::Values) {
    build(
        backend,
        Query::delete()
            .from_table(Alias::new(REVISIONS))
            .and_where(Expr::col(Alias::new("entity")).eq(table))
            .and_where(Expr::col(Alias::new("record_id")).eq(id))
            .to_owned(),
    )
}

/// One field that differs between two versions: its label and the change as
/// HTML, words removed in `<del>` and added in `<ins>`.
#[derive(Debug, Clone)]
pub struct Change {
    /// The field's label.
    pub label: &'static str,
    /// The difference, escaped HTML.
    pub html: String,
}

/// What changed from `before` to `after` in `table`'s columns.
pub fn changes(
    table: &str,
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> Vec<Change> {
    let text = |m: &Map<String, Value>, c: &str| match m.get(c) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    columns(table)
        .iter()
        .filter_map(|(c, label, kind)| {
            let (old, new) = (text(before, c), text(after, c));
            if old == new {
                return None;
            }
            let html = if *kind == Kind::Long {
                word_diff(&old, &new)
            } else {
                format!(
                    "<del>{}</del> <ins>{}</ins>",
                    escape(if old.is_empty() { "(empty)" } else { &old }),
                    escape(if new.is_empty() { "(empty)" } else { &new })
                )
            };
            Some(Change { label, html })
        })
        .collect()
}

/// `old` to `new` word by word, each run of changes shown whole: what went,
/// then what replaced it.
fn word_diff(old: &str, new: &str) -> String {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_words(old, new);
    let tokens: Vec<(ChangeTag, &str)> = diff
        .iter_all_changes()
        .map(|c| (c.tag(), c.value()))
        .collect();
    let (mut out, mut gone, mut added) = (String::new(), String::new(), String::new());
    let flush = |out: &mut String, gone: &mut String, added: &mut String| {
        if !gone.is_empty() {
            out.push_str(&format!("<del>{}</del>", escape(gone.trim())));
        }
        if !gone.is_empty() && !added.is_empty() {
            out.push(' ');
        }
        if !added.is_empty() {
            out.push_str(&format!("<ins>{}</ins>", escape(added.trim())));
        }
        gone.clear();
        added.clear();
    };
    for (i, (tag, word)) in tokens.iter().enumerate() {
        match tag {
            ChangeTag::Delete => gone.push_str(word),
            ChangeTag::Insert => added.push_str(word),
            ChangeTag::Equal => {
                let inside = word.trim().is_empty()
                    && (!gone.is_empty() || !added.is_empty())
                    && tokens
                        .get(i + 1)
                        .is_some_and(|(t, _)| *t != ChangeTag::Equal);
                if inside {
                    // A space between two changes belongs to both runs.
                    gone.push_str(word);
                    added.push_str(word);
                } else {
                    flush(&mut out, &mut gone, &mut added);
                    out.push_str(&escape(word));
                }
            }
        }
    }
    flush(&mut out, &mut gone, &mut added);
    out
}

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The table a kind of record lives in: `templates` or `layouts`.
pub(crate) fn table_of(kind: &str) -> &'static str {
    if kind == "templates" {
        TEMPLATES
    } else {
        LAYOUTS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_field_differs_word_by_word_and_escapes() {
        let before = serde_json::json!({ "body": "Acme runs <this> site." });
        let after = serde_json::json!({ "body": "Acme runs that site." });
        let c = changes(
            TEMPLATES,
            before.as_object().unwrap(),
            after.as_object().unwrap(),
        );
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].label, "Body");
        assert_eq!(
            c[0].html,
            "Acme runs <del>&lt;this&gt;</del> <ins>that</ins> site."
        );
    }

    #[test]
    fn a_run_of_changes_reads_as_what_went_then_what_came() {
        assert_eq!(
            word_diff(
                "thank you for your order.",
                "thank you for shopping with Acme."
            ),
            "thank you for <del>your order.</del> <ins>shopping with Acme.</ins>"
        );
    }
}
