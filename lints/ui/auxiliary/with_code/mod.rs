//! A `mod.rs` with one of each kind of item it may not hold.

use std::fmt::Display;

mod inner;

pub use self::inner::Thing;

pub const LIMIT: u8 = 1;

pub struct Holder;

pub fn show(value: impl Display) -> String {
    value.to_string()
}

pub mod inline {
    pub fn code() -> u8 {
        2
    }
}
