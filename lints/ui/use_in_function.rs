use std::collections::BTreeMap;

// A `use` that a macro writes into a function is not the caller's to move.
macro_rules! fn_with_generated_use {
    ($name:ident) => {
        fn $name() -> String {
            use std::fmt::Write;
            let mut text = String::new();
            let _ = write!(text, "generated");
            text
        }
    };
}

fn_with_generated_use!(generated);

fn at_the_top_of_a_body() -> String {
    use std::fmt::Write;
    let mut text = String::new();
    let _ = write!(text, "top");
    text
}

fn after_a_statement() -> usize {
    let map: BTreeMap<u8, u8> = BTreeMap::new();
    use std::collections::BTreeSet;
    let set: BTreeSet<u8> = BTreeSet::new();
    map.len() + set.len()
}

fn in_a_nested_block(flag: bool) -> usize {
    if flag {
        use std::collections::VecDeque;
        VecDeque::<u8>::new().len()
    } else {
        0
    }
}

fn in_a_closure() -> usize {
    let count = || {
        use std::collections::HashSet;
        HashSet::<u8>::new().len()
    };
    count()
}

struct Holder;

impl Holder {
    fn in_a_method(&self) -> usize {
        use std::collections::BinaryHeap;
        BinaryHeap::<u8>::new().len()
    }
}

mod inner {
    // A `use` in a module, inline or not, is at the top of its scope: not reported.
    use std::collections::LinkedList;

    pub fn in_a_module() -> usize {
        LinkedList::<u8>::new().len()
    }
}

fn main() {
    let _ = generated();
    let _ = at_the_top_of_a_body();
    let _ = after_a_statement();
    let _ = in_a_nested_block(true);
    let _ = in_a_closure();
    let _ = Holder.in_a_method();
    let _ = inner::in_a_module();
}
