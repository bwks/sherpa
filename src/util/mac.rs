use rand::Rng;

use crate::core::konst::KVM_OUI;

/// Creates a random colon delimited hexadecimal string for use as MAC address.
/// MAC addresses have the same OUI assigned via KVM (52:54:00).
pub fn random_mac() -> String {
    // Generate a 24-bit random number (between 0 and 0xFFFFFF)
    let random_number: u32 = rand::thread_rng().gen_range(0..=0xFFFFFF);

    // Format as a 6-character hexadecimal string
    let hex = format!("{:06X}", random_number);

    // Insert colons between each two characters (aa:bb:cc)
    format!("{}:{}:{}:{}", KVM_OUI, &hex[0..2], &hex[2..4], &hex[4..6])
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    use crate::core::konst::KVM_OUI;

    #[test]
    fn test_random_mac_format() {
        let mac = random_mac();
        let re = Regex::new(r"^52:54:00:[0-9A-F]{2}:[0-9A-F]{2}:[0-9A-F]{2}$").unwrap();
        assert!(
            re.is_match(&mac),
            "MAC address format is incorrect: {}",
            mac
        );
    }

    #[test]
    fn test_random_mac_uniqueness() {
        let mac1 = random_mac();
        let mac2 = random_mac();
        assert_ne!(
            mac1, mac2,
            "Two consecutive calls should generate different MACs"
        );
    }

    #[test]
    fn test_random_mac_oui() {
        let mac = random_mac();
        assert!(mac.starts_with(KVM_OUI), "MAC should start with KVM OUI");
    }

    #[test]
    fn test_random_mac_length() {
        let mac = random_mac();
        assert_eq!(mac.len(), 17, "MAC address should be 17 characters long");
    }

    #[test]
    fn test_random_mac_colon_positions() {
        let mac = random_mac();
        assert_eq!(
            mac.chars().filter(|&c| c == ':').count(),
            5,
            "MAC should have 5 colons"
        );
        assert_eq!(
            mac.char_indices()
                .filter(|(_, c)| *c == ':')
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            vec![2, 5, 8, 11, 14],
            "Colons should be at positions 2, 5, 8, 11, and 14"
        );
    }

    #[test]
    fn test_random_mac_distribution() {
        let macs: Vec<String> = (0..1000).map(|_| random_mac()).collect();
        let unique_macs: std::collections::HashSet<_> = macs.iter().cloned().collect();
        assert!(
            unique_macs.len() > 999,
            "Less than 99.9% of generated MACs are unique"
        );
    }
}
