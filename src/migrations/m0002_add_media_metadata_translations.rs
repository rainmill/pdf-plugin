//! A layout's logo, background and author from the media library; a template's
//! keywords; and pdf_template_translations, a template's words in other locales.

use laterite_core::strata::*;
use sea_query::Alias;

pub struct Migration;

#[async_trait(?Send)]
impl laterite_core::Migration for Migration {
    fn name(&self) -> &str {
        "0002_add_media_metadata_translations"
    }
    async fn up(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        let add = |table: &str, col: ColumnDef| {
            Table::alter()
                .table(Alias::new(table))
                .add_column(col)
                .to_owned()
        };
        let text = |name: &str| ColumnDef::new(Alias::new(name)).text().null().to_owned();
        let id = |name: &str| {
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .null()
                .to_owned()
        };
        // Media record ids: the masthead logo and a full-page background.
        s.exec(add("pdf_layouts", id("logo_id"))).await?;
        s.exec(add("pdf_layouts", text("logo_height"))).await?;
        s.exec(add("pdf_layouts", id("background_id"))).await?;
        // Written into every PDF set in the layout.
        s.exec(add("pdf_layouts", text("author"))).await?;
        // Comma separated, written into the PDF.
        s.exec(add("pdf_templates", text("keywords"))).await?;
        s.exec(
            Table::create()
                .table(Alias::new("pdf_template_translations"))
                .col(
                    ColumnDef::new(Alias::new("id"))
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("template_id"))
                        .big_integer()
                        .not_null(),
                )
                // BCP 47: `de`, `pt-BR`.
                .col(key_col(Alias::new("locale")).not_null())
                // Empty keeps the template's own.
                .col(text("title"))
                .col(text("body"))
                .col(text("created_at"))
                .col(text("updated_at"))
                .to_owned(),
        )
        .await?;
        s.exec(
            Index::create()
                .name("pdf_template_translations_locale")
                .table(Alias::new("pdf_template_translations"))
                .col(Alias::new("template_id"))
                .col(Alias::new("locale"))
                .unique()
                .to_owned(),
        )
        .await
    }
}
