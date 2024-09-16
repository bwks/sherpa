/// Prints a message to the terminal
pub fn term_msg(message: &str) {
    let msg_len = message.len();
    let surround = "=".repeat(msg_len);
    println!(
        r#"
{surround}
{message}
{surround}
"#,
    );
}
