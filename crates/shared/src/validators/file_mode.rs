use anyhow::{Context, Result};

/// The permission bits of a file mode written as octal digits, such as `644` for `0o644`, as
/// manifests write modes.
///
/// # Errors
///
/// Fails when `digits` are not octal digits up to `777`.
pub fn mode_from_octal_digits(digits: u32) -> Result<u32> {
    u32::from_str_radix(&digits.to_string(), 8)
        .ok()
        .filter(|bits| *bits <= 0o777)
        .with_context(|| format!("Mode {digits} is not octal digits up to 777, such as 644"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_digits_convert_to_permission_bits() {
        assert_eq!(mode_from_octal_digits(644).unwrap(), 0o644);
        assert_eq!(mode_from_octal_digits(7).unwrap(), 0o7);
        assert_eq!(mode_from_octal_digits(0).unwrap(), 0);
        assert_eq!(mode_from_octal_digits(777).unwrap(), 0o777);
    }

    #[test]
    fn a_mode_that_is_not_octal_digits_up_to_777_is_refused() {
        for digits in [8, 9, 448, 800, 999, 1000] {
            let error = mode_from_octal_digits(digits).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Mode {digits} is not octal digits up to 777, such as 644")
            );
        }
    }
}
