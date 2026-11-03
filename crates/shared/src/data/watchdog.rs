use std::fmt;

use serde_derive::{Deserialize, Serialize};

/// What QEMU does when a VM's hardware watchdog expires: `watchdog` on a node in the manifest.
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

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIONS: [WatchdogAction; 7] = [
        WatchdogAction::Reset,
        WatchdogAction::Poweroff,
        WatchdogAction::Shutdown,
        WatchdogAction::Pause,
        WatchdogAction::Dump,
        WatchdogAction::InjectNmi,
        WatchdogAction::None,
    ];

    #[test]
    fn manifest_names_are_the_libvirt_names() {
        for action in ACTIONS {
            let json = serde_json::to_value(action).unwrap();
            assert_eq!(json.as_str().unwrap(), action.to_string());
        }
        assert_eq!(WatchdogAction::InjectNmi.to_string(), "inject-nmi");
    }

    #[test]
    fn unknown_actions_are_refused() {
        assert!(serde_json::from_str::<WatchdogAction>("\"reboot\"").is_err());
        assert!(serde_json::from_str::<WatchdogAction>("\"inject_nmi\"").is_err());
    }
}
