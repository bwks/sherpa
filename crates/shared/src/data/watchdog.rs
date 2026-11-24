use std::fmt;

use serde_derive::{Deserialize, Serialize};

/// What QEMU does when a VM's hardware watchdog expires: `vm.watchdog` on a node in the manifest.
///
/// The watchdog is an emulated Intel 6300ESB timer. It does nothing until the guest arms it, and
/// the guest must keep resetting it; if the guest hangs and stops, QEMU takes this action. The
/// names are libvirt's.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WatchdogAction {
    Reset,
    Poweroff,
    Shutdown,
    Pause,
    Dump,
    InjectNmi,
    None,
}

impl fmt::Display for WatchdogAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchdogAction::Reset => write!(f, "reset"),
            WatchdogAction::Poweroff => write!(f, "poweroff"),
            WatchdogAction::Shutdown => write!(f, "shutdown"),
            WatchdogAction::Pause => write!(f, "pause"),
            WatchdogAction::Dump => write!(f, "dump"),
            WatchdogAction::InjectNmi => write!(f, "inject-nmi"),
            WatchdogAction::None => write!(f, "none"),
        }
    }
}
