//! Progress messages the server streams during a long operation.

use shared::data::{StatusKind, StatusMessage};
use shared::util::Emoji;

/// The emoji shown before a status message of `kind`.
pub fn status_emoji(kind: StatusKind) -> Emoji {
    match kind {
        StatusKind::Progress => Emoji::Progress,
        StatusKind::Done => Emoji::Success,
        StatusKind::Info => Emoji::Info,
        StatusKind::Waiting => Emoji::Hourglass,
    }
}

/// Show a status message the server streams; other messages are ignored.
pub fn print_status(text: &str) {
    if let Ok(status) = serde_json::from_str::<StatusMessage>(text)
        && status.r#type == "status"
    {
        println!("{} {}", status_emoji(status.kind), status.message);
    }
}
