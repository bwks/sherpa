use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use jiff::Timestamp;
use thiserror::Error;

/// Why an ISO image could not be written.
#[derive(Debug, Error)]
pub enum IsoError {
    #[error(
        "Invalid ISO volume label {0:?}: it must be 1 to 16 ASCII letters, digits or punctuation, without spaces"
    )]
    InvalidVolumeId(String),

    #[error("ISO path has no file name: {}", .0.display())]
    NoFileName(PathBuf),

    #[error("Failed to resolve the directory the ISO is written to, {}", path.display())]
    OutputDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Failed to read ISO source directory {}", path.display())]
    SourceDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Failed to read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("{} is not a regular file or directory", .0.display())]
    NotRegularFile(PathBuf),

    #[error("File name is not UTF-8: {0:?}")]
    NotUtf8(OsString),

    #[error("File name longer than {limit} characters: {name}")]
    NameTooLong { name: String, limit: usize },

    #[error("{first} and {second} have the same ISO 9660 name")]
    NameClash { first: String, second: String },

    #[error("{} is nested deeper than the {limit} levels ISO 9660 allows", path.display())]
    TooDeep { path: PathBuf, limit: usize },

    #[error("{} is too large for ISO 9660", .0.display())]
    FileTooLarge(PathBuf),

    #[error("Too many directories for an ISO 9660 path table")]
    TooManyDirectories,

    #[error("ISO 9660 {0} too large")]
    TooLarge(&'static str),

    #[error("ISO 9660 directory record too long for {0}")]
    RecordTooLong(String),

    #[error("Failed to create ISO {}", path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Failed to copy {} into the ISO", path.display())]
    Copy {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("{} changed size while the ISO was written", .0.display())]
    ChangedSize(PathBuf),

    #[error("Failed to write ISO {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Failed to move the finished ISO into place at {}", path.display())]
    Rename {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("ISO layout error: writing sector {actual}, expected {expected}")]
    Layout { actual: u32, expected: u32 },

    #[error("{0} is outside the dates ISO 9660 can record")]
    DateOutOfRange(Timestamp),
}
