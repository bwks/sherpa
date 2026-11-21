//! A `mod.rs` with only what it may hold: docs, `mod` declarations and re-exports.

mod inner;

pub use self::inner::Thing;
pub(crate) use self::inner::other;
