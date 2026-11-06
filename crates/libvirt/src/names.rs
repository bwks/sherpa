//! Which libvirt domains and volumes belong to a lab or a node, by name.
//!
//! A node's domain is named `<node>-<lab_id>` and its disks add one of `LAB_VOLUME_SUFFIXES`.
//! Names are matched whole: a substring match would take another lab's resources when one of
//! its node names contains this lab's ID, and destroy deletes what matches.

use shared::konst::LAB_VOLUME_SUFFIXES;

/// Whether `name` is `<node>-<lab_id>`, a node's device in the lab `lab_id`.
#[must_use]
pub fn is_lab_device(name: &str, lab_id: &str) -> bool {
    name.strip_suffix(lab_id)
        .and_then(|rest| rest.strip_suffix('-'))
        .is_some_and(|node| !node.is_empty())
}

/// Whether `volume` is a disk of a node in the lab `lab_id`.
#[must_use]
pub fn is_lab_volume(volume: &str, lab_id: &str) -> bool {
    LAB_VOLUME_SUFFIXES.iter().any(|suffix| {
        volume
            .strip_suffix(suffix)
            .is_some_and(|device| is_lab_device(device, lab_id))
    })
}

/// Whether `volume` is a disk of the device `device_name`, `<node>-<lab_id>`.
#[must_use]
pub fn is_device_volume(volume: &str, device_name: &str) -> bool {
    LAB_VOLUME_SUFFIXES
        .iter()
        .any(|suffix| volume.strip_suffix(suffix) == Some(device_name))
}

#[cfg(test)]
mod tests {
    use shared::konst::CONTAINER_DISK_NAME;

    use super::*;

    const LAB_ID: &str = "fecb1a12";

    #[test]
    fn lab_volumes_are_matched_by_their_whole_name() {
        for volume in [
            "dev-fecb1a12.qcow2",
            "dev-fecb1a12-hdd.qcow2",
            "dev-fecb1a12-cfg.img",
            "dev-fecb1a12-cfg.ign",
            "dev-fecb1a12.iso",
            "dev-fecb1a12.elf",
            "web-1-fecb1a12-disk.img",
        ] {
            assert!(is_lab_volume(volume, LAB_ID), "{volume}");
        }
        // Nodes in another lab named after this lab's ID, and volumes Sherpa did not name.
        for volume in [
            "fecb1a12-0a1b2c3d.qcow2",
            "x-fecb1a12-0a1b2c3d-hdd.qcow2",
            "-fecb1a12.qcow2",
            "dev-fecb1a12.txt",
            "dev-fecb1a12",
            "dev-fecb1a12x.qcow2",
        ] {
            assert!(!is_lab_volume(volume, LAB_ID), "{volume}");
        }
    }

    #[test]
    fn lab_devices_are_a_node_name_and_the_lab_id() {
        assert!(is_lab_device("dev-fecb1a12", LAB_ID));
        assert!(is_lab_device("web-1-fecb1a12", LAB_ID));
        assert!(!is_lab_device("fecb1a12-0a1b2c3d", LAB_ID));
        assert!(!is_lab_device("fecb1a12", LAB_ID));
        assert!(!is_lab_device("-fecb1a12", LAB_ID));
    }

    #[test]
    fn device_volumes_belong_to_one_device() {
        assert!(is_device_volume(
            "dev01-fecb1a12-hdd.qcow2",
            "dev01-fecb1a12"
        ));
        assert!(is_device_volume("dev01-fecb1a12.iso", "dev01-fecb1a12"));
        assert!(!is_device_volume(
            "dev01-fecb1a12-hdd.qcow2",
            "v01-fecb1a12"
        ));
        assert!(!is_device_volume("xdev01-fecb1a12.qcow2", "dev01-fecb1a12"));
    }

    #[test]
    fn container_disks_have_a_lab_volume_suffix() {
        assert!(LAB_VOLUME_SUFFIXES.contains(&format!("-{CONTAINER_DISK_NAME}").as_str()));
    }
}
