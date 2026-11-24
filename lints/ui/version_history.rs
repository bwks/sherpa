//! This module no longer reads the old file.

/// Returns the name. Previously this returned the path.
fn name() -> &'static str {
    // Since 0.7.0 the name is cached.
    "name"
}

/* Before 0.6 this was a macro. */
fn messages() -> Vec<String> {
    vec![
        String::from("the --all flag is deprecated"),
        format!("removed in {}", 3),
        String::from(r#"the "legacy" layout"#),
        String::from("This option used to be called --force"),
    ]
}

/// The value is used to build the address, which is in 10.0.0.0/24.
fn not_reported() -> &'static str {
    // A node that is no_longer_needed keeps its name; listed in 4 places.
    let previously_seen = "seen";
    let no_longer = previously_seen;
    // Wait until 2.5 seconds have passed; it isn't used to decide anything.
    no_longer
}

fn main() {}
