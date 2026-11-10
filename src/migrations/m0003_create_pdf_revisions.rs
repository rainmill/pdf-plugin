//! pdf_revisions: what a template or layout held after each save in the admin,
//! the last twenty kept per record, for the History page and its Restore.

use laterite_core::sea_query;
use laterite_core::strata::*;
use sea_query::Alias;

pub struct Migration;

#[async_trait(?Send)]
impl laterite_core::Migration for Migration {
    fn name(&self) -> &str {
        "0003_create_pdf_revisions"
    }
    async fn up(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        s.exec(
            Table::create()
                .table(Alias::new("pdf_revisions"))
                .col(
                    ColumnDef::new(Alias::new("id"))
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                // pdf_templates or pdf_layouts.
                .col(key_col(Alias::new("entity")).not_null())
                .col(
                    ColumnDef::new(Alias::new("record_id"))
                        .big_integer()
                        .not_null(),
                )
                // A JSON object of the record's columns.
                .col(ColumnDef::new(Alias::new("snapshot")).text().not_null())
                // Who saved it: a username, or the process's name.
                .col(ColumnDef::new(Alias::new("author")).text().null())
                .col(ColumnDef::new(Alias::new("created_at")).text().not_null())
                .to_owned(),
        )
        .await?;
        s.exec(
            Index::create()
                .name("pdf_revisions_record")
                .table(Alias::new("pdf_revisions"))
                .col(Alias::new("entity"))
                .col(Alias::new("record_id"))
                .to_owned(),
        )
        .await
    }
}
