// compile-flags: --test

use std::io;

fn failing() -> io::Error {
    io::Error::other("out of space")
}

fn outside_a_test() {
    if failing().to_string().contains("space") {}
}

#[test]
fn in_a_test_function() {
    assert!(failing().to_string().contains("space"));
    assert_eq!(format!("{}", failing()), "out of space");
}

#[cfg(test)]
mod tests {
    use super::failing;

    fn helper() -> bool {
        failing().to_string().starts_with("out")
    }
}
