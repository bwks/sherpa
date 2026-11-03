//! Compile-time checks that SQL statements and row types agree with the table schema.
//!
//! The `CREATE TABLE` statements in `schema/` are the single definition of each table. `sql!`
//! checks a statement against its table's `CREATE TABLE`, and `row!` (in `persistence`) checks a
//! row type, while the crate compiles: the checks run as constants, so a mismatch fails the build
//! with the panic message as the error. The same functions panic normally when called at runtime,
//! which is how the tests below exercise them.
//!
//! The parser understands the statement shapes the crate writes, not SQL in general:
//! - `SELECT <columns> FROM <table> ...`: the columns are the table's, in schema order.
//! - `INSERT INTO <table> (<columns>) VALUES (?1, ..., ?n) [RETURNING <columns>]`: the columns are
//!   the table's in schema order, with one numbered placeholder per column, in column order.
//! - `UPDATE <table> SET <column> = ..., ... [WHERE ...] [RETURNING <columns>]`: every column set
//!   is one of the table's.
//! - `DELETE FROM <table> ... [RETURNING <columns>]`.
//!
//! Column lists hold bare column names, optionally double-quoted (`"index"`). Names compare
//! without regard to ASCII case, as SQLite does.

/// Byte range of a name in a string.
#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

/// Checks `sql` against the `CREATE TABLE <table>` statement in `schema`, and returns it.
macro_rules! sql {
    ($table:ident, $sql:literal) => {{
        const SQL: &str = $sql;
        const _: () = $crate::schema_check::check_statement(
            $crate::schema::$table::TABLES,
            stringify!($table),
            SQL,
        );
        SQL
    }};
}

pub(crate) use sql;

/// Check a statement against the `CREATE TABLE <table>` statement in `schema`.
pub(crate) const fn check_statement(schema: &str, table: &str, sql: &str) {
    let schema = schema.as_bytes();
    let table = table.as_bytes();
    let s = sql.as_bytes();
    let start = skip_space(s, 0);
    if keyword_at(s, start, b"select") {
        let list = start + 6;
        if !list_matches_schema(schema, table, s, list) {
            panic!("SELECT columns do not match the table's columns in the schema");
        }
        match find_keyword(s, list, b"from") {
            Some(from) => {
                if !names_table(s, from + 4, table) {
                    panic!("SELECT reads a different table");
                }
            }
            None => panic!("SELECT has no FROM"),
        }
    } else if keyword_at(s, start, b"insert") {
        let into = skip_space(s, start + 6);
        if !keyword_at(s, into, b"into") || !names_table(s, into + 4, table) {
            panic!("INSERT writes a different table");
        }
        let (_, after_table) = identifier(s, into + 4);
        let open = skip_space(s, after_table);
        if open >= s.len() || s[open] != b'(' {
            panic!("INSERT has no column list");
        }
        if !list_matches_schema(schema, table, s, open + 1) {
            panic!("INSERT columns do not match the table's columns in the schema");
        }
        match find_keyword(s, open, b"values") {
            Some(values) => check_placeholders(schema, table, s, values + 6),
            None => panic!("INSERT has no VALUES"),
        }
        check_returning(schema, table, s, open);
    } else if keyword_at(s, start, b"update") {
        if !names_table(s, start + 6, table) {
            panic!("UPDATE writes a different table");
        }
        match find_keyword(s, start, b"set") {
            Some(set) => check_assignments(schema, table, s, set + 3),
            None => panic!("UPDATE has no SET"),
        }
        check_returning(schema, table, s, start);
    } else if keyword_at(s, start, b"delete") {
        let from = skip_space(s, start + 6);
        if !keyword_at(s, from, b"from") || !names_table(s, from + 4, table) {
            panic!("DELETE removes from a different table");
        }
        check_returning(schema, table, s, start);
    } else {
        panic!("sql! checks SELECT, INSERT, UPDATE and DELETE statements only");
    }
}

/// Check a row type's field names against the table's columns, in schema order.
pub(crate) const fn check_row(schema: &str, table: &str, fields: &[&str]) {
    let schema = schema.as_bytes();
    let table = table.as_bytes();
    let mut n = 0;
    loop {
        let column = schema_column(schema, table, n);
        if n == fields.len() {
            if column.is_some() {
                panic!("The row type has fewer fields than the table has columns");
            }
            return;
        }
        let field = fields[n].as_bytes();
        match column {
            Some(column) => {
                if !same(schema, column, field, whole(field)) {
                    panic!("A row type field does not match the table's column in schema order");
                }
            }
            None => panic!("The row type has more fields than the table has columns"),
        }
        n += 1;
    }
}

/// The `n`th column of `CREATE TABLE <table>` in `schema`, skipping table constraints.
const fn schema_column(schema: &[u8], table: &[u8], n: usize) -> Option<Span> {
    let mut pos = table_body(schema, table);
    let mut index = 0;
    loop {
        let item = skip_space(schema, pos);
        if item >= schema.len() || schema[item] == b')' {
            return None;
        }
        let (name, after) = identifier(schema, item);
        let constraint = keyword_at(schema, item, b"primary")
            || keyword_at(schema, item, b"unique")
            || keyword_at(schema, item, b"foreign")
            || keyword_at(schema, item, b"check")
            || keyword_at(schema, item, b"constraint");
        if !constraint {
            if index == n {
                return Some(name);
            }
            index += 1;
        }
        let end = item_end(schema, after);
        if end >= schema.len() || schema[end] == b')' {
            return None;
        }
        pos = end + 1;
    }
}

/// Number of columns of `CREATE TABLE <table>` in `schema`.
const fn schema_column_count(schema: &[u8], table: &[u8]) -> usize {
    let mut n = 0;
    while schema_column(schema, table, n).is_some() {
        n += 1;
    }
    n
}

/// Whether `column` is one of the table's columns.
const fn schema_has(schema: &[u8], table: &[u8], s: &[u8], column: Span) -> bool {
    let mut n = 0;
    while let Some(name) = schema_column(schema, table, n) {
        if same(schema, name, s, column) {
            return true;
        }
        n += 1;
    }
    false
}

/// Index just after the `(` that opens `CREATE TABLE <table>` in `schema`.
const fn table_body(schema: &[u8], table: &[u8]) -> usize {
    let mut i = 0;
    while i < schema.len() {
        if keyword_at(schema, i, b"create") {
            let word = skip_space(schema, i + 6);
            if keyword_at(schema, word, b"table") && names_table(schema, word + 5, table) {
                let (_, after) = identifier(schema, word + 5);
                let open = skip_space(schema, after);
                if open < schema.len() && schema[open] == b'(' {
                    return open + 1;
                }
            }
        }
        i += 1;
    }
    panic!("The schema has no CREATE TABLE for this table");
}

/// Whether the comma-separated column names starting at `list` are the table's columns, in
/// schema order.
const fn list_matches_schema(schema: &[u8], table: &[u8], s: &[u8], list: usize) -> bool {
    let mut n = 0;
    loop {
        let expected = schema_column(schema, table, n);
        let found = list_column(s, list, n);
        match (expected, found) {
            (None, None) => return true,
            (Some(expected), Some(found)) => {
                if !same(schema, expected, s, found) {
                    return false;
                }
            }
            _ => return false,
        }
        n += 1;
    }
}

/// The `n`th name of the comma-separated column list starting at `list`.
const fn list_column(s: &[u8], list: usize, n: usize) -> Option<Span> {
    let mut pos = list;
    let mut index = 0;
    loop {
        let (name, after) = identifier(s, pos);
        if name.start == name.end {
            panic!("A column list holds something other than a column name");
        }
        if index == n {
            return Some(name);
        }
        let next = skip_space(s, after);
        if next >= s.len() || s[next] != b',' {
            return None;
        }
        pos = next + 1;
        index += 1;
    }
}

/// Check that `VALUES (` at `values` holds `?1, ..., ?n`, one per column of the table.
const fn check_placeholders(schema: &[u8], table: &[u8], s: &[u8], values: usize) {
    let open = skip_space(s, values);
    if open >= s.len() || s[open] != b'(' {
        panic!("INSERT VALUES has no parenthesised list");
    }
    let mut pos = open + 1;
    let mut count = 0;
    loop {
        let mark = skip_space(s, pos);
        if mark >= s.len() || s[mark] != b'?' {
            panic!("INSERT values must be numbered placeholders");
        }
        let mut digit = mark + 1;
        let mut number = 0;
        while digit < s.len() && s[digit].is_ascii_digit() {
            number = number * 10 + (s[digit] - b'0') as usize;
            digit += 1;
        }
        count += 1;
        if number != count {
            panic!("INSERT placeholders must be ?1, ?2, ... in column order");
        }
        let next = skip_space(s, digit);
        if next < s.len() && s[next] == b',' {
            pos = next + 1;
        } else if next < s.len() && s[next] == b')' {
            break;
        } else {
            panic!("INSERT VALUES list is not closed");
        }
    }
    if count != schema_column_count(schema, table) {
        panic!("INSERT has a different number of placeholders than the table has columns");
    }
}

/// Check that every `column = ...` after `SET` at `set` names one of the table's columns.
const fn check_assignments(schema: &[u8], table: &[u8], s: &[u8], set: usize) {
    let mut pos = set;
    loop {
        let (column, after) = identifier(s, pos);
        if column.start == column.end {
            panic!("UPDATE SET holds something other than a column name");
        }
        if !schema_has(schema, table, s, column) {
            panic!("UPDATE sets a column the table does not have");
        }
        let equals = skip_space(s, after);
        if equals >= s.len() || s[equals] != b'=' {
            panic!("UPDATE SET column has no =");
        }
        let end = assignment_end(s, equals + 1);
        if end >= s.len() || s[end] != b',' {
            return;
        }
        pos = end + 1;
    }
}

/// Index of the comma, `WHERE`, `RETURNING` or end of text that ends a `SET` value.
const fn assignment_end(s: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < s.len() {
        let b = s[i];
        if b == b'\'' || b == b'"' {
            i = quote_end(s, i);
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth = depth.saturating_sub(1);
        } else if depth == 0
            && (b == b',' || keyword_at(s, i, b"where") || keyword_at(s, i, b"returning"))
        {
            return i;
        }
        i += 1;
    }
    i
}

/// If the statement has `RETURNING`, check its columns are the table's, in schema order.
const fn check_returning(schema: &[u8], table: &[u8], s: &[u8], from: usize) {
    if let Some(returning) = find_keyword(s, from, b"returning")
        && !list_matches_schema(schema, table, s, returning + 9)
    {
        panic!("RETURNING columns do not match the table's columns in the schema");
    }
}

/// Whether the name after `i` is `table`.
const fn names_table(s: &[u8], i: usize, table: &[u8]) -> bool {
    let (name, _) = identifier(s, i);
    same(s, name, table, whole(table))
}

/// The name at or after `i`, without its quotes, and the index after it.
const fn identifier(s: &[u8], i: usize) -> (Span, usize) {
    let i = skip_space(s, i);
    if i < s.len() && s[i] == b'"' {
        let end = quote_end(s, i);
        (Span { start: i + 1, end }, end + 1)
    } else {
        let mut end = i;
        while end < s.len() && is_word(s[end]) {
            end += 1;
        }
        (Span { start: i, end }, end)
    }
}

/// Index of the comma or closing parenthesis ending the definition from `i`, skipping nested
/// parentheses and quoted text.
const fn item_end(s: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < s.len() {
        let b = s[i];
        if b == b'\'' || b == b'"' {
            i = quote_end(s, i);
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            if depth == 0 {
                return i;
            }
            depth -= 1;
        } else if b == b',' && depth == 0 {
            return i;
        }
        i += 1;
    }
    i
}

/// Index of the quote closing the quoted text that opens at `i`.
const fn quote_end(s: &[u8], i: usize) -> usize {
    let quote = s[i];
    let mut end = i + 1;
    while end < s.len() && s[end] != quote {
        end += 1;
    }
    end
}

/// Index of the first `word` from `from` that is outside quoted text.
const fn find_keyword(s: &[u8], from: usize, word: &[u8]) -> Option<usize> {
    let mut i = from;
    while i < s.len() {
        if s[i] == b'\'' || s[i] == b'"' {
            i = quote_end(s, i);
        } else if keyword_at(s, i, word) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Whether the keyword `word`, written in lower case, starts at `i` as a whole word.
const fn keyword_at(s: &[u8], i: usize, word: &[u8]) -> bool {
    if i + word.len() > s.len() || (i > 0 && is_word(s[i - 1])) {
        return false;
    }
    let mut k = 0;
    while k < word.len() {
        if s[i + k].to_ascii_lowercase() != word[k] {
            return false;
        }
        k += 1;
    }
    i + word.len() == s.len() || !is_word(s[i + word.len()])
}

/// Whether two names are equal without regard to ASCII case.
const fn same(a: &[u8], x: Span, b: &[u8], y: Span) -> bool {
    if x.end - x.start != y.end - y.start {
        return false;
    }
    let mut k = 0;
    while k < x.end - x.start {
        if !a[x.start + k].eq_ignore_ascii_case(&b[y.start + k]) {
            return false;
        }
        k += 1;
    }
    true
}

const fn whole(s: &[u8]) -> Span {
    Span {
        start: 0,
        end: s.len(),
    }
}

const fn skip_space(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

const fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = "
CREATE TABLE thing (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
        CHECK (length(name) BETWEEN 3 AND 32 AND name NOT GLOB '*[^a-z0-9,()]*'),
    \"index\" INTEGER NOT NULL,
    owner TEXT NOT NULL REFERENCES user (id) ON DELETE RESTRICT,
    UNIQUE (owner, \"index\")
) STRICT;
CREATE INDEX thing_owner ON thing (owner);
";

    fn check(sql: &str) {
        check_statement(SCHEMA, "thing", sql);
    }

    #[test]
    fn schema_columns_skip_types_checks_and_table_constraints() {
        let schema = SCHEMA.as_bytes();
        let names: Vec<&str> = (0..)
            .map_while(|n| schema_column(schema, b"thing", n))
            .map(|span| &SCHEMA[span.start..span.end])
            .collect();
        assert_eq!(names, ["id", "name", "index", "owner"]);
    }

    #[test]
    fn accepts_statements_matching_the_schema() {
        check("SELECT id, name, \"index\", owner FROM thing WHERE name = ?1");
        check(
            "INSERT INTO thing (id, name, \"index\", owner) VALUES (?1, ?2, ?3, ?4)
             RETURNING id, name, \"index\", owner",
        );
        check(
            "UPDATE thing SET name = ?2, \"index\" = ?3 WHERE id = ?1 RETURNING id, name, \"index\", owner",
        );
        check("DELETE FROM thing WHERE id = ?1 RETURNING id, name, \"index\", owner");
        check("select ID, Name, \"index\", owner from thing");
    }

    #[test]
    #[should_panic(expected = "SELECT columns do not match")]
    fn rejects_a_select_missing_a_column() {
        check("SELECT id, name, owner FROM thing");
    }

    #[test]
    #[should_panic(expected = "SELECT columns do not match")]
    fn rejects_a_select_out_of_order() {
        check("SELECT id, \"index\", name, owner FROM thing");
    }

    #[test]
    #[should_panic(expected = "SELECT reads a different table")]
    fn rejects_a_select_from_another_table() {
        check("SELECT id, name, \"index\", owner FROM other");
    }

    #[test]
    #[should_panic(expected = "INSERT columns do not match")]
    fn rejects_an_insert_with_a_misspelt_column() {
        check("INSERT INTO thing (id, nmae, \"index\", owner) VALUES (?1, ?2, ?3, ?4)");
    }

    #[test]
    #[should_panic(expected = "INSERT placeholders must be ?1, ?2, ... in column order")]
    fn rejects_insert_placeholders_out_of_order() {
        check("INSERT INTO thing (id, name, \"index\", owner) VALUES (?1, ?3, ?2, ?4)");
    }

    #[test]
    #[should_panic(expected = "INSERT has a different number of placeholders")]
    fn rejects_an_insert_with_a_missing_placeholder() {
        check("INSERT INTO thing (id, name, \"index\", owner) VALUES (?1, ?2, ?3)");
    }

    #[test]
    #[should_panic(expected = "UPDATE sets a column the table does not have")]
    fn rejects_an_update_of_an_unknown_column() {
        check("UPDATE thing SET colour = ?2 WHERE id = ?1");
    }

    #[test]
    #[should_panic(expected = "RETURNING columns do not match")]
    fn rejects_a_short_returning_list() {
        check("DELETE FROM thing WHERE id = ?1 RETURNING id, name");
    }

    #[test]
    fn accepts_row_fields_in_schema_order() {
        check_row(SCHEMA, "thing", &["id", "name", "index", "owner"]);
    }

    #[test]
    #[should_panic(expected = "A row type field does not match")]
    fn rejects_row_fields_out_of_order() {
        check_row(SCHEMA, "thing", &["id", "index", "name", "owner"]);
    }

    #[test]
    #[should_panic(expected = "fewer fields")]
    fn rejects_a_row_missing_a_column() {
        check_row(SCHEMA, "thing", &["id", "name", "index"]);
    }
}
