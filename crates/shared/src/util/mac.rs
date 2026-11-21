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
