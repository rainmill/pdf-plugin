//! A layout's font files from the media library, and the font of its masthead
//! title.

use laterite_core::sea_query;
use laterite_core::strata::*;
use sea_query::Alias;

pub struct Migration;

#[async_trait(?Send)]
impl laterite_core::Migration for Migration {
    fn name(&self) -> &str {
        "0004_add_layout_fonts"
    }
    async fn up(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        let add = |name: &str| {
            Table::alter()
                .table(Alias::new("pdf_layouts"))
                .add_column(ColumnDef::new(Alias::new(name)).text().null().to_owned())
                .to_owned()
        };
        // A JSON list of media record ids: TTF, OTF or TTC files.
        s.exec(add("fonts")).await?;
        s.exec(add("title_font")).await?;
        Ok(())
    }
    async fn down(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        for name in ["fonts", "title_font"] {
            s.exec(
                Table::alter()
                    .table(Alias::new("pdf_layouts"))
                    .drop_column(Alias::new(name))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
}
