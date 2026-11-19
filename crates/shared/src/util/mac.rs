use rand::Rng;

/// Creates a random colon delimited hexadecimal string for use as MAC address.
#[must_use]
pub fn random_mac(vendor_oui: &str) -> String {
    // Generate a 24-bit random number (between 0 and 0xFFFFFF)
    let random_number: u32 = rand::thread_rng().gen_range(0..=0x00FF_FFFF);

    // Format as a 6-character hexadecimal string
    let hex = format!("{random_number:06X}");

    // Insert colons between each two characters (aa:bb:cc)
    format!(
        "{}:{}:{}:{}",
        vendor_oui,
        &hex[0..2],
        &hex[2..4],
        &hex[4..6]
    )
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
        assert!(re.is_match(&mac), "MAC address format is incorrect: {mac}");
    }
}
