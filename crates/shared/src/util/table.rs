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
    /// Shown in a centred panel above the table.
    pub title: Option<&'a str>,
    /// The indices of the columns whose cells are right-aligned, such as columns of numbers.
    pub right: &'a [usize],
    /// The widest the table may be, title, borders and padding included. The widest column is
    /// wrapped first, at word boundaries where it can be.
    pub wrap_width: Option<usize>,
    /// Whether the headers are shown as the first row. `false` for a table of label and value
    /// pairs, whose first column names each row.
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
    fn a_titled_table_has_a_centred_title_above_its_headers() {
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
    fn an_untitled_table_right_aligns_the_columns_it_is_given() {
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
    fn right_aligned_columns_keep_the_title_centred() {
        let expected = "\
┌───────────────┐
│     Fruit     │
├───────┬───────┤
│ Name  │ Count │
├───────┼───────┤
│ apple │     3 │
├───────┼───────┤
│ kiwi  │    12 │
└───────┴───────┘";
        let layout = TableLayout {
            title: Some("Fruit"),
            right: &[1],
            ..TableLayout::default()
        };
        assert_eq!(render_table(["Name", "Count"], &fruit(), &layout), expected);
    }

    #[test]
    fn an_empty_table_still_shows_its_headers() {
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
    fn a_table_without_a_header_row_shows_only_its_rows() {
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
    fn a_wrapped_table_wraps_its_widest_column_between_words() {
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
    fn title_wrapping_and_right_alignment_compose() {
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

    #[test]
    fn the_default_layout_has_a_header_row_and_nothing_else() {
        let layout = TableLayout::default();
        assert_eq!(layout.title, None);
        assert_eq!(layout.right, &[] as &[usize]);
        assert_eq!(layout.wrap_width, None);
        assert!(layout.header_row);
    }
}
