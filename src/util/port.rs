use crate::core::konst::BASE_PORT;

/// Returns a high port number based from id
pub fn id_to_port(id: u8) -> u16 {
    return BASE_PORT + id as u16;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_to_port_zero() {
        assert_eq!(id_to_port(0), BASE_PORT);
    }

    #[test]
    fn test_id_to_port_one() {
        assert_eq!(id_to_port(1), BASE_PORT + 1);
    }

    #[test]
    fn test_id_to_port_max_u8() {
        assert_eq!(id_to_port(u8::MAX), BASE_PORT + u8::MAX as u16);
    }

    #[test]
    fn test_id_to_port_range() {
        for id in 0..=u8::MAX {
            let port = id_to_port(id);
            assert!(port >= BASE_PORT && port <= BASE_PORT + u8::MAX as u16);
        }
    }
}
