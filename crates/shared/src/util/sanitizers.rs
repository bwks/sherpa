/// Replaces chars with dashes.
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

    #[test]
    fn test_dasher_mixed() {
        assert_eq!(dasher("a/b:c.d"), "a-b-c-d");
    }

    #[test]
    fn test_dasher_no_replacements() {
        assert_eq!(dasher("no-change-needed"), "no-change-needed");
    }

    #[test]
    fn test_dasher_empty() {
        assert_eq!(dasher(""), "");
    }
}
