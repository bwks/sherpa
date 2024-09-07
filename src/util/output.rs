/// Prints a message to the terminal
pub fn term_msg(message: &str) {
    let msg_len = message.len();
    let surround = std::iter::repeat("=").take(msg_len).collect::<String>();
    println!(
        r#"
{surround}
{message}
{surround}
"#,
    );
}
