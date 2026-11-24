//! A stand-in for the `clap` crate: the traits its derives implement, re-exported as clap does.

mod derive {
    pub trait Parser {}
    pub trait Subcommand {}
    pub trait Args {}
    pub trait ValueEnum {}
}

pub use derive::{Args, Parser, Subcommand, ValueEnum};
