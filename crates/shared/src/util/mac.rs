use rand::Rng;

/// Creates a random colon delimited hexadecimal string for use as MAC address.
pub fn random_mac(vendor_oui: &str) -> String {
    // Generate a 24-bit random number (between 0 and 0xFFFFFF)
    let random_number: u32 = rand::thread_rng().gen_range(0..=0xFFFFFF);

    // Format as a 6-character hexadecimal string
    let hex = format!("{:06X}", random_number);

    // Insert colons between each two characters (aa:bb:cc)
    format!(
        "{}:{}:{}:{}",
        vendor_oui,
        &hex[0..2],
        &hex[2..4],
        &hex[4..6]
    )
}

/// Clean a address by removing known MAC delimiters,
/// trimming any whitespace and transforming to lowercase
pub fn clean_mac(mac_address: &str) -> String {
    mac_address
        .trim()
        .replace([':', '-', '.', ' '], "")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konst::KVM_OUI;
    use regex::Regex;

    #[test]
    fn test_random_mac_format() {
        let mac = random_mac(KVM_OUI);
        let re = Regex::new(r"^52:54:00:[0-9A-F]{2}:[0-9A-F]{2}:[0-9A-F]{2}$").unwrap();
        assert!(
            re.is_match(&mac),
            "MAC address format is incorrect: {}",
            mac
        );
    }

    #[test]
    fn test_random_mac_oui() {
        let mac = random_mac(KVM_OUI);
        assert!(mac.starts_with(KVM_OUI), "MAC should start with KVM OUI");
    }

    #[test]
    fn test_random_mac_length() {
        let mac = random_mac(KVM_OUI);
        assert_eq!(mac.len(), 17, "MAC address should be 17 characters long");
    }

    #[test]
    fn test_random_mac_colon_count() {
        let mac = random_mac(KVM_OUI);
        assert_eq!(
            mac.chars().filter(|&c| c == ':').count(),
            5,
            "MAC should have 5 colons"
        );
    }

    #[test]
    fn test_random_mac_uniqueness() {
        let mac1 = random_mac(KVM_OUI);
        let mac2 = random_mac(KVM_OUI);
        assert_ne!(mac1, mac2, "Two calls should produce different MACs");
    }

    #[test]
    fn test_clean_mac_colons() {
        assert_eq!(clean_mac("52:54:00:AA:BB:CC"), "525400aabbcc");
    }

    #[test]
    fn test_clean_mac_dashes() {
        assert_eq!(clean_mac("52-54-00-AA-BB-CC"), "525400aabbcc");
    }

    #[test]
    fn test_clean_mac_dots() {
        assert_eq!(clean_mac("5254.00AA.BBCC"), "525400aabbcc");
    }

    #[test]
    fn test_clean_mac_whitespace() {
        assert_eq!(clean_mac("  52:54:00:AA:BB:CC  "), "525400aabbcc");
    }

    #[test]
    fn test_clean_mac_already_clean() {
        assert_eq!(clean_mac("525400aabbcc"), "525400aabbcc");
    }
}
