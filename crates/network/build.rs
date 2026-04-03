fn main() {
    // Tell cargo to recompile when the eBPF binary changes.
    // This is needed because include_bytes!() doesn't register as a dependency.
    println!(
        "cargo::rerun-if-changed=../ebpf-redirect/target/bpfel-unknown-none/release/ebpf-redirect"
    );
}
