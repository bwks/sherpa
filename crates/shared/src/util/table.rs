//! Terminal tables in the house style: `Style::modern()`, laid out by a [`TableLayout`].
//!
//! Callers map each item they show to one row of cells, `[String; N]`, and pass `N` headers, so
//! the compiler checks that every row has a cell for each header. Behind the `table` feature,
//! which is the only thing that builds `tabled`.

use tabled::{
    builder::Builder,
    settings::{
        Alignment, Modify, Panel, Style, Width,
        object::{Columns, Rows},
        peaker::PriorityMax,
        themes::BorderCorrection,
    },
};

/// How [`render_table`] lays a table out. Set the fields a table needs and take the rest from
/// [`TableLayout::default`]: no title, left-aligned columns, no width bound, and a header row.
/// The fields compose: any combination of them can be used together.
#[derive(Debug, Clone)]
pub struct TableLayout<'a> {
    pub title: Option<&'a str>,
    pub right: &'a [usize],
    pub wrap_width: Option<usize>,
    pub header_row: bool,
}

impl Default for TableLayout<'_> {
    fn default() -> Self {
        Self {
            title: None,
            right: &[],
            wrap_width: None,
            header_row: true,
        }
    }
}

/// Renders `rows` under `headers`, laid out by `layout`.
#[must_use]
pub fn render_table<const N: usize>(
    headers: [&str; N],
    rows: &[[String; N]],
    layout: &TableLayout,
) -> String {
    let mut builder = Builder::default();
    if layout.header_row {
        builder.push_record(headers);
    }
    for row in rows {
        builder.push_record(row.iter().map(String::as_str));
    }
    let mut table = builder.build();
    table.with(Style::modern());
    for column in layout.right {
        table.with(Modify::new(Columns::one(*column)).with(Alignment::right()));
    }
    if let Some(title) = layout.title {
        table
            .with(Panel::header(title))
            .with(Modify::new(Rows::first()).with(Alignment::center()))
            .with(BorderCorrection::span());
    }
    if let Some(width) = layout.wrap_width {
        table.with(
            Width::wrap(width)
                .keep_words(true)
                .priority(PriorityMax::right()),
        );
    }
    table.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fruit() -> Vec<[String; 2]> {
        vec![
            ["apple".to_owned(), "3".to_owned()],
            ["kiwi".to_owned(), "12".to_owned()],
        ]
    }

    fn warnings() -> [[String; 2]; 1] {
        [[
            "Warnings".to_owned(),
            "one two three four five six seven eight".to_owned(),
        ]]
    }

    #[test]
    fn cli_tables_001_a_titled_table_has_its_title_centred_above_the_headers() {
        let expected = "\
┌───────────────┐
│     Fruit     │
├───────┬───────┤
│ Name  │ Count │
├───────┼───────┤
│ apple │ 3     │
├───────┼───────┤
│ kiwi  │ 12    │
└───────┴───────┘";
        let layout = TableLayout {
            title: Some("Fruit"),
            ..TableLayout::default()
        };
        assert_eq!(render_table(["Name", "Count"], &fruit(), &layout), expected);
    }

    #[test]
    fn cli_tables_002_the_columns_named_right_aligned_are_and_the_others_are_left_aligned() {
        let expected = "\
┌───────┬───────┐
│ Name  │ Count │
├───────┼───────┤
│ apple │     3 │
├───────┼───────┤
│ kiwi  │    12 │
└───────┴───────┘";
        let layout = TableLayout {
            right: &[1],
            ..TableLayout::default()
        };
        assert_eq!(render_table(["Name", "Count"], &fruit(), &layout), expected);
    }

    #[test]
    fn cli_tables_003_a_table_with_no_rows_still_shows_its_headers() {
        let expected = "\
┌──────┬───────┐
│ Name │ Count │
└──────┴───────┘";
        assert_eq!(
            render_table(["Name", "Count"], &[], &TableLayout::default()),
            expected
        );
    }

    #[test]
    fn cli_tables_004_a_table_with_no_header_row_shows_its_title_and_rows_only() {
        let expected = "\
┌────────────┐
│    Lab     │
├──────┬─────┤
│ ID   │ abc │
├──────┼─────┤
│ Name │ x   │
└──────┴─────┘";
        let rows = [
            ["ID".to_owned(), "abc".to_owned()],
            ["Name".to_owned(), "x".to_owned()],
        ];
        let layout = TableLayout {
            title: Some("Lab"),
            header_row: false,
            ..TableLayout::default()
        };
        assert_eq!(
            render_table(["Property", "Value"], &rows, &layout),
            expected
        );
    }

    #[test]
    fn cli_tables_005_a_table_given_a_width_wraps_its_widest_column_between_words() {
        let expected = "\
┌──────────┬─────────────────┐
│ Item     │ Details         │
├──────────┼─────────────────┤
│ Warnings │ one two three   │
│          │ four five six   │
│          │ seven eight     │
└──────────┴─────────────────┘";
        let layout = TableLayout {
            wrap_width: Some(30),
            ..TableLayout::default()
        };
        let table = render_table(["Item", "Details"], &warnings(), &layout);
        assert!(
            table.lines().all(|line| line.chars().count() <= 30),
            "{table}"
        );
        assert_eq!(table, expected);
    }

    #[test]
    fn cli_tables_006_a_title_wrapping_and_right_alignment_work_together() {
        let expected = "\
┌────────────────────────────┐
│           Status           │
├──────────┬─────────────────┤
│     Item │ Details         │
├──────────┼─────────────────┤
│ Warnings │ one two three   │
│          │ four five six   │
│          │ seven eight     │
└──────────┴─────────────────┘";
        let layout = TableLayout {
            title: Some("Status"),
            right: &[0],
            wrap_width: Some(30),
            ..TableLayout::default()
        };
        let table = render_table(["Item", "Details"], &warnings(), &layout);
        assert!(
            table.lines().all(|line| line.chars().count() <= 30),
            "{table}"
        );
        assert_eq!(table, expected);
    }
}
