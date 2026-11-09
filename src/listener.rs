//! What happens when the admin writes a layout or a template: the row is marked
//! customised so the boot sync leaves it alone, its code never changes once set,
//! and a layout a template is set in cannot be deleted.

use laterite_core::query::{bind_values, build};
use laterite_core::strata::async_trait;
use laterite_core::{t, ErrorBag, ModelListener, Op, Record, SaveCx, Text};
use sea_query::{Alias, Expr, Query};

use crate::store::{TEMPLATES, TRANSLATIONS};

/// Marks an admin save as customised and keeps the code fixed.
pub(crate) struct Customise;

#[async_trait]
impl ModelListener for Customise {
    async fn before_save(
        &self,
        _cx: &mut SaveCx<'_>,
        rec: &mut Record,
        op: Op,
    ) -> Result<(), ErrorBag> {
        if op == Op::Update && rec.changed("code") {
            let mut bag = ErrorBag::default();
            bag.add(
                "code",
                t!("The code cannot change once created: code and other templates refer to it."),
            );
            return Err(bag);
        }
        rec.set("customized", true);
        Ok(())
    }
}

/// Refuses to delete a layout a template is set in.
pub(crate) struct LayoutInUse;

#[async_trait]
impl ModelListener for LayoutInUse {
    async fn before_delete(&self, cx: &mut SaveCx<'_>, rec: &Record) -> Result<(), Text> {
        let Some(id) = rec.id() else { return Ok(()) };
        let (sql, values) = build(
            cx.backend(),
            Query::select()
                .expr(Expr::col(Alias::new("id")).count())
                .from(Alias::new(TEMPLATES))
                .and_where(Expr::col(Alias::new("layout_id")).eq(id))
                .to_owned(),
        );
        let row = bind_values(sqlx::query(&sql), values)
            .fetch_one(&mut *cx.conn())
            .await
            .map_err(|e| Text::from(e.to_string()))?;
        let used: i64 = sqlx::Row::try_get(&row, 0).unwrap_or_default();
        if used > 0 {
            return Err(laterite_core::tn!(
                "A template is set in this layout. Move it to another layout first.",
                "{n} templates are set in this layout. Move them to another layout first.",
                n = used
            ));
        }
        Ok(())
    }
}

/// Deletes a template's translations with it.
pub(crate) struct DropTranslations;

#[async_trait]
impl ModelListener for DropTranslations {
    async fn before_delete(&self, cx: &mut SaveCx<'_>, rec: &Record) -> Result<(), Text> {
        let Some(id) = rec.id() else { return Ok(()) };
        let (sql, values) = build(
            cx.backend(),
            Query::delete()
                .from_table(Alias::new(TRANSLATIONS))
                .and_where(Expr::col(Alias::new("template_id")).eq(id))
                .to_owned(),
        );
        bind_values(sqlx::query(&sql), values)
            .execute(&mut *cx.conn())
            .await
            .map_err(|e| Text::from(e.to_string()))?;
        Ok(())
    }
}
