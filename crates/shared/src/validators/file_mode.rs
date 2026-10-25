/// A file mode written as octal digits, such as `444`, as manifests write them.
pub fn valid_octal_mode(mode: u32) -> bool {
    if mode > 777 {
        return false;
    }
    let mut rest = mode;
    while rest > 0 {
        if rest % 10 > 7 {
            return false;
        }
        rest /= 10;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_digit_modes_up_to_777_are_valid() {
        for mode in [0, 7, 400, 644, 777] {
            assert!(valid_octal_mode(mode), "{mode}");
        }
        for mode in [8, 9, 448, 800, 1000] {
            assert!(!valid_octal_mode(mode), "{mode}");
        }
    }
}
