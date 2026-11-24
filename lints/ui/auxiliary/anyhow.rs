//! A stand-in for the `anyhow` crate: its `Error` does not implement `std::error::Error`.

use std::fmt;

#[derive(Debug)]
pub struct Error;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("anyhow")
    }
}
