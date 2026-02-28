/// Replaces chars with dashes.
pub fn dasher(text: &str) -> String {
    text.replace(['/', ':', '.'], "-")
}
