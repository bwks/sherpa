//! ISO 9660 images for node configuration media: ZTP config drives, NoCloud seeds and the
//! PAN-OS bootstrap. See `docs/architecture/ZTP_ISO.md`.

#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod error;
mod writer;

// re-export
pub use error::IsoError;
pub use writer::create_iso;
