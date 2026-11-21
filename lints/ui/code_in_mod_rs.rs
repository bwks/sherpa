// The files under test are the two `mod.rs` files in `auxiliary/`; this file only loads them.
// `clean/mod.rs` holds what a `mod.rs` may hold and reports nothing; `with_code/mod.rs` holds
// one of each kind of item it may not.

#[path = "auxiliary/clean/mod.rs"]
mod clean;
#[path = "auxiliary/with_code/mod.rs"]
mod with_code;

// An inline module is not a `mod.rs` file, whatever it is called.
mod not_a_file {
    pub fn code() -> u8 {
        1
    }
}

fn main() {
    let _ = clean::Thing;
    clean::other();
    let _ = with_code::Thing;
    let _ = with_code::Holder;
    let _ = with_code::show(with_code::LIMIT);
    let _ = with_code::inline::code();
    let _ = not_a_file::code();
}
