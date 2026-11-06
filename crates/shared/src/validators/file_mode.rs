/// The permission bits of a file mode written as octal digits, such as `644` for `0o644`, as
/// manifests write modes. `None` when `digits` are not octal digits up to `777`.
#[must_use]
pub fn mode_from_octal_digits(digits: u32) -> Option<u32> {
    u32::from_str_radix(&digits.to_string(), 8)
        .ok()
        .filter(|bits| *bits <= 0o777)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_digits_convert_to_permission_bits() {
        assert_eq!(mode_from_octal_digits(644), Some(0o644));
        assert_eq!(mode_from_octal_digits(7), Some(0o7));
        assert_eq!(mode_from_octal_digits(0), Some(0));
        assert_eq!(mode_from_octal_digits(777), Some(0o777));
        for digits in [8, 9, 448, 800, 999, 1000] {
            assert_eq!(mode_from_octal_digits(digits), None, "{digits}");
        }
    }
}
