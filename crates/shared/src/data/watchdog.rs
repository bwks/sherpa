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
    /// Reset the VM, as a hardware reset.
    Reset,
    /// Power the VM off at once.
    Poweroff,
    /// Ask the guest to shut down, as the ACPI power button does.
    Shutdown,
    /// Pause the VM.
    Pause,
    /// Dump the VM's memory, then keep running.
    Dump,
    /// Send the guest a non-maskable interrupt.
    InjectNmi,
    /// Do nothing; the device is present but expiring has no effect.
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
