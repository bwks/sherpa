// aux-build:anyhow.rs

extern crate anyhow;

use std::error::Error;
use std::fmt;
use std::io;

#[derive(Debug)]
enum Refused {
    Busy,
    Missing,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("busy"),
            Self::Missing => f.write_str("missing"),
        }
    }
}

impl Error for Refused {}

struct Label(&'static str);

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

fn reported(io_error: &io::Error, refused: Refused, boxed: Box<dyn Error + Send + Sync>, any: anyhow::Error) {
    if io_error.to_string().contains("No such device") {}
    if refused.to_string() == "busy" {}
    if "busy" != refused.to_string() {}
    if boxed.to_string().to_lowercase().starts_with("timed out") {}
    if any.to_string().ends_with("refused") {}
    if format!("{any:#}").contains("refused") {}
    if format!("{:?}", refused).contains("Busy") {}

    let message = format!("{io_error}");
    if message.contains("denied") {}

    let lowered = refused.to_string().to_lowercase();
    let trimmed = lowered.trim();
    if trimmed.starts_with("bus") {}

    match refused.to_string().as_str() {
        "busy" => {},
        _ => {},
    }

    let cause: &dyn Error = &refused;
    if cause.to_string().contains("busy") {}

    let known = vec![String::from("busy")];
    if known.contains(&refused.to_string()) {}

    assert_eq!(refused.to_string(), "busy");
    assert_ne!(format!("{any}"), "busy");
}

fn not_reported(label: Label, refused: Refused, io_error: &io::Error) {
    if label.to_string().contains("x") {}
    if format!("{label}") == "x" {}
    if matches!(refused, Refused::Missing) {}
    if io_error.kind() == io::ErrorKind::NotFound {}
    let shown = refused.to_string();
    println!("{shown}");
    let mut changed = refused.to_string();
    changed.push('!');
    if changed.contains("busy") {}
    assert_eq!(label.to_string(), "x");
}

fn main() {}
