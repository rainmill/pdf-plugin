//! pdf_layouts and pdf_templates: the letterheads and the documents set in them.

use laterite_core::strata::*;
use sea_query::Alias;

pub struct Migration;

#[async_trait(?Send)]
impl laterite_core::Migration for Migration {
    fn name(&self) -> &str {
        "0001_create_pdf"
    }
    async fn up(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        let id = || {
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .auto_increment()
                .primary_key()
                .to_owned()
        };
        let text = |name: &str| ColumnDef::new(Alias::new(name)).text().null().to_owned();
        s.exec(
            Table::create()
                .table(Alias::new("pdf_layouts"))
                .col(id())
                // Unique and never changed once created: templates and code name it.
                .col(key_col(Alias::new("code")).not_null().unique_key())
                .col(ColumnDef::new(Alias::new("name")).text().not_null())
                .col(text("accent"))
                .col(text("text_colour"))
                .col(text("body_font"))
                .col(text("heading_font"))
                // Numbers kept as written ("10.5"), read into the layout at render.
                .col(text("size"))
                .col(text("leading"))
                .col(text("paper"))
                .col(bool_col(Alias::new("landscape")).not_null().default(0))
                // Millimetres: "top,right,bottom,left".
                .col(text("margins"))
                .col(text("header"))
                .col(text("footer"))
                .col(text("typst_override"))
                .col(text("css_override"))
                // Edited in the admin: a registered layout no longer follows its code.
                .col(bool_col(Alias::new("customized")).not_null().default(0))
                .col(text("created_at"))
                .col(text("updated_at"))
                .to_owned(),
        )
        .await?;
        s.exec(
            Table::create()
                .table(Alias::new("pdf_templates"))
                .col(id())
                .col(key_col(Alias::new("code")).not_null().unique_key())
                .col(ColumnDef::new(Alias::new("title")).text().not_null())
                .col(text("description"))
                // markdown or typst.
                .col(key_col(Alias::new("format")).not_null().default("markdown"))
                .col(text("body"))
                .col(ColumnDef::new(Alias::new("layout_id")).big_integer().null())
                // Overrides the layout's paper and orientation when set.
                .col(text("paper"))
                .col(bool_col(Alias::new("landscape")).not_null().default(0))
                // A JSON object the previews render with.
                .col(text("sample_data"))
                .col(bool_col(Alias::new("customized")).not_null().default(0))
                .col(text("created_at"))
                .col(text("updated_at"))
                .to_owned(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn the_tables_build_on_the_test_backend() {
        let (_db, _guard) = laterite_core::testing::connect_test(&[super::super::migrations()]).await;
    }
}
