// aux-build:clap.rs

extern crate clap;

/// Type docs are fine.
#[derive(Debug, Clone)]
pub struct Named {
    /// Field doc.
    pub a: u8,
    // Plain comment.
    b: u16,
    c: u32, // Trailing comment.
    /** Block doc. */
    d: u64,
    #[doc = "Doc attribute."]
    e: bool,
    #[doc(hidden)]
    f: bool,
    g: &'static str,
}

/// Tuple type docs.
pub struct Tuple<T: Fn(u8)>(
    /// Tuple field doc.
    pub T,
    String,
);

/// Enum docs.
pub enum Kind {
    /// Variant doc.
    Unit,
    /* Block comment. */
    Struct {
        /// Variant field doc.
        x: u8,
    },
}

pub union Bits {
    /// Union field doc.
    a: u8,
    b: i8,
}

/// A type with a comment between its name and its body. // not a field comment
pub struct Bounded<T>
where
    // A bound's comment is not a field comment.
    T: Clone,
{
    inner: T,
}

/// A command line type, with clap's trait implemented by hand.
pub struct Cli {
    /// Help text.
    verbose: bool,
}

impl clap::Parser for Cli {}

/// A subcommand type, with the trait reached through another name.
pub enum Command {
    /// Help text.
    Run {
        /// Help text.
        fast: bool,
    },
}

use clap::Subcommand as Sub;

impl Sub for Command {}

/// A type that only looks like a clap type.
pub struct Lookalike {
    /// Not help text.
    verbose: bool,
}

trait Parser {}

impl Parser for Lookalike {}

macro_rules! generated {
    ($name:ident) => {
        pub struct $name {
            /// Written by a macro.
            x: u8,
        }
    };
}

generated!(FromMacro);

fn function() {
    // A comment in a function body.
    let quoted = "// not a comment";
    struct Local {
        /// Local field doc.
        x: u8,
        y: &'static str,
    }
    let _ = Local { x: 0, y: quoted };
}

fn main() {}
