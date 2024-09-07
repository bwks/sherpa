use rand::Rng;

/// Generate a unique ID
pub fn generate_id() -> String {
    const ID_LENGTH: usize = 12;
    const CHARSET: &[u8] = b"0123456789abcdef";

    let mut rng = rand::thread_rng();

    (0..ID_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_id_length() {
        let id = generate_id();
        assert_eq!(id.len(), 12, "Generated ID should be 12 characters long");
    }

    #[test]
    fn test_id_charset() {
        let id = generate_id();
        assert!(
            id.chars().all(|c| c.is_digit(16)),
            "Generated ID should only contain hexadecimal characters"
        );
    }

    #[test]
    fn test_id_uniqueness() {
        let mut ids = HashSet::new();
        for _ in 0..1000 {
            let id = generate_id();
            assert!(ids.insert(id), "Generated IDs should be unique");
        }
    }

    #[test]
    fn test_id_distribution() {
        let mut char_counts = [0; 16];
        for _ in 0..10000 {
            let id = generate_id();
            for c in id.chars() {
                let index = c.to_digit(16).unwrap() as usize;
                char_counts[index] += 1;
            }
        }

        let total_chars = 10000 * 12;
        let expected_count = total_chars / 16;
        let tolerance = (expected_count as f64 * 0.1) as i32; // 10% tolerance

        for (i, &count) in char_counts.iter().enumerate() {
            assert!(
                (count as i32 - expected_count as i32).abs() < tolerance,
                "Character '{}' count {} is not within 10% of expected count {}",
                char::from_digit(i as u32, 16).unwrap(),
                count,
                expected_count
            );
        }
    }
}
