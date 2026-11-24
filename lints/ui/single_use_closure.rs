// edition:2021

fn reported(values: &[u32]) -> u32 {
    let double = |value: u32| value * 2;
    let total: u32 = values.iter().copied().map(double).sum();

    let add_one = |value: u32| value + 1;
    add_one(total)
}

fn not_reported(values: &[u32]) -> u32 {
    let double = |value: u32| value * 2;
    let first = double(1);
    let second = double(2);

    let unused = |value: u32| value;

    let total: u32 = values.iter().map(|value| value + 1).sum();
    first + second + total
}

async fn async_block_not_reported() -> u32 {
    let work = async { 1 };
    work.await
}

fn main() {}
