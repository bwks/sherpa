/// Prints a surrounded message to the terminal
pub fn term_msg_surround(message: &str) {
    let msg_len = message.len();
    let surround = "=".repeat(msg_len);
    println!(
        r#"{surround}
{message}
{surround}"#,
    );
}

/// Prints an underlined message to the terminal
pub fn term_msg_underline(message: &str) {
    let msg_len = message.len();
    let underline = "-".repeat(msg_len);
    println!(
        r#"{message}
{underline}"#,
    );
}

/// Prints an highlighted message to the terminal
pub fn term_msg_highlight(message: &str) {
    println!("- {message} - ");
}
