use thiserror::Error;

/// A network operation's failure that a caller tells apart from the others.
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("Link {name} not found")]
    LinkNotFound { name: String },

    #[error("{what} of {micros} µs is too long for netem")]
    TooLongForNetem { what: &'static str, micros: u32 },
}
