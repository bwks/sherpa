use rand::Rng;

pub fn random_mac_suffix() -> String {
    // Generate a 24-bit random number (between 0 and 0xFFFFFF)
    let random_number: u32 = rand::thread_rng().gen_range(0..=0xFFFFFF);

    // Format as a 6-character hexadecimal string
    let hex = format!("{:06X}", random_number);

    // Insert colons between each two characters (aa:bb:cc)
    format!("{}:{}:{}", &hex[0..2], &hex[2..4], &hex[4..6])
}
