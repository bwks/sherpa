#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::TC_ACT_PIPE,
    helpers::bpf_redirect,
    macros::{classifier, map},
    maps::HashMap,
    programs::TcContext,
};

/// Map storing the peer interface index.
/// Key 0 = peer ifindex to redirect packets to.
#[map]
static PEER_IFINDEX: HashMap<u32, u32> = HashMap::with_max_entries(1, 0);

/// TC classifier program that redirects all ingress packets
/// to the peer interface's egress path.
///
/// This enables protocol-transparent point-to-point forwarding
/// between two network interfaces without using Linux bridges.
#[classifier]
pub fn p2p_redirect(_ctx: TcContext) -> i32 {
    match try_redirect() {
        Ok(action) => action,
        Err(_) => TC_ACT_PIPE as i32,
    }
}

fn try_redirect() -> Result<i32, ()> {
    let key: u32 = 0;
    let peer_ifindex = unsafe { PEER_IFINDEX.get(&key).ok_or(())? };
    let ret = unsafe { bpf_redirect(*peer_ifindex, 0) };
    Ok(ret as i32)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
