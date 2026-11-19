/// Replaces chars with dashes.
#[must_use]
pub fn dasher(text: &str) -> String {
    text.replace(['/', ':', '.'], "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dasher_replaces_slashes() {
        assert_eq!(dasher("GigabitEthernet0/0/1"), "GigabitEthernet0-0-1");
    }

    #[test]
    fn test_dasher_replaces_colons() {
        assert_eq!(dasher("52:54:00:aa:bb:cc"), "52-54-00-aa-bb-cc");
    }

    #[test]
    fn test_dasher_replaces_dots() {
        assert_eq!(dasher("192.168.1.1"), "192-168-1-1");
    }
}
