//! Terminal tables in the house style: `Style::modern()`, with an optional title panel.
//!
//! Callers map each item they show to one row of cells, `[String; N]`, and pass `N` headers, so
//! the compiler checks that every row has a cell for each header. Behind the `table` feature,
//! which is the only thing that builds `tabled`.

use tabled::{
    Table,
    builder::Builder,
    settings::{
        Alignment, Modify, Panel, Style, Width,
        object::{Columns, Rows},
        peaker::PriorityMax,
        themes::BorderCorrection,
    },
};

/// Renders `rows` under `headers`. A `title` goes in a centred panel above the headers; the
/// columns named in `right` (by index) are right-aligned.
#[must_use]
pub fn render_table<const N: usize>(
    title: Option<&str>,
    headers: [&str; N],
    rows: &[[String; N]],
    right: &[usize],
) -> String {
    let mut table = build(Some(headers), rows);
    for column in right {
        table.with(Modify::new(Columns::one(*column)).with(Alignment::right()));
    }
    finish(table, title)
}

/// Renders label and value pairs under `title`, without a header row.
#[must_use]
pub fn render_property_table(title: &str, rows: &[(&str, String)]) -> String {
    let mut builder = Builder::default();
    for (label, value) in rows {
        builder.push_record([*label, value.as_str()]);
    }
    finish(builder.build(), Some(title))
}

/// Renders `rows` under `headers`, no wider than `width` characters, borders and padding
/// included. The widest column is wrapped first, at word boundaries where it can be.
#[must_use]
pub fn render_wrapped_table<const N: usize>(
    headers: [&str; N],
    rows: &[[String; N]],
    width: usize,
) -> String {
    let mut table = build(Some(headers), rows);
    table.with(Style::modern()).with(
        Width::wrap(width)
            .keep_words(true)
            .priority(PriorityMax::right()),
    );
    table.to_string()
}

/// A table of `rows`, with `headers` as its first row when there are any.
fn build<const N: usize>(headers: Option<[&str; N]>, rows: &[[String; N]]) -> Table {
    let mut builder = Builder::default();
    if let Some(headers) = headers {
        builder.push_record(headers);
    }
    for row in rows {
        builder.push_record(row.iter().map(String::as_str));
    }
    builder.build()
}

/// `table` in the modern style, under a centred `title` panel when there is one.
fn finish(mut table: Table, title: Option<&str>) -> String {
    table.with(Style::modern());
    if let Some(title) = title {
        table
            .with(Panel::header(title))
            .with(Modify::new(Rows::first()).with(Alignment::center()))
            .with(BorderCorrection::span());
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
        assert_eq!(
            render_table(Some("Fruit"), ["Name", "Count"], &fruit(), &[]),
            expected
        );
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
        assert_eq!(
            render_table(None, ["Name", "Count"], &fruit(), &[1]),
            expected
        );
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
        assert_eq!(
            render_table(Some("Fruit"), ["Name", "Count"], &fruit(), &[1]),
            expected
        );
    }

    #[test]
    fn an_empty_table_still_shows_its_headers() {
        let expected = "\
┌──────┬───────┐
│ Name │ Count │
└──────┴───────┘";
        assert_eq!(render_table(None, ["Name", "Count"], &[], &[]), expected);
    }

    #[test]
    fn a_property_table_has_no_header_row() {
        let expected = "\
┌────────────┐
│    Lab     │
├──────┬─────┤
│ ID   │ abc │
├──────┼─────┤
│ Name │ x   │
└──────┴─────┘";
        let rows = [("ID", "abc".to_owned()), ("Name", "x".to_owned())];
        assert_eq!(render_property_table("Lab", &rows), expected);
    }

    #[test]
    fn a_wrapped_table_is_no_wider_than_its_width() {
        let rows = [[
            "Warnings".to_owned(),
            "one two three four five six seven eight".to_owned(),
        ]];
        let table = render_wrapped_table(["Item", "Details"], &rows, 30);
        assert!(
            table.lines().all(|line| line.chars().count() <= 30),
            "{table}"
        );
        // The label column is left whole; the details column is wrapped between words.
        assert!(table.contains("│ Warnings │"), "{table}");
        assert!(table.lines().any(|line| line.contains("eight")), "{table}");
        assert!(table.lines().count() > 5, "{table}");
    }
}
