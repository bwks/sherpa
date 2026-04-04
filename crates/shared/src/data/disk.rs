use std::fmt;

use anyhow::{Result, bail};

use schemars::JsonSchema;
use serde_derive::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiskDrivers {
    #[default]
    Qemu,
}
impl fmt::Display for DiskDrivers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskDrivers::Qemu => write!(f, "qemu"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiskFormats {
    #[default]
    Qcow2,
    Raw,
}
impl fmt::Display for DiskFormats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskFormats::Qcow2 => write!(f, "qcow2"),
            DiskFormats::Raw => write!(f, "raw"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiskDevices {
    Cdrom,
    #[default]
    File,
}
impl fmt::Display for DiskDevices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskDevices::Cdrom => write!(f, "cdrom"),
            DiskDevices::File => write!(f, "file"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DiskBuses {
    Ide,
    Sata,
    Scsi,
    Usb,
    #[default]
    Virtio,
}
impl fmt::Display for DiskBuses {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskBuses::Ide => write!(f, "ide"),
            DiskBuses::Sata => write!(f, "sata"),
            DiskBuses::Scsi => write!(f, "scsi"),
            DiskBuses::Usb => write!(f, "usb"),
            DiskBuses::Virtio => write!(f, "virtio"),
        }
    }
}
impl DiskBuses {
    pub fn to_vec() -> Vec<DiskBuses> {
        DiskBuses::iter().collect()
    }
}
impl_surreal_value_for_enum!(DiskBuses);

#[derive(Copy, Clone, Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiskTargets {
    // IDE
    Hda,
    Hdb,
    Hdc,
    Hdd,
    Hde,
    Hdf,
    Hdg,
    Hdh,
    Hdi,
    Hdj,

    // Sata, USB
    Sda,
    Sdb,
    Sdc,
    Sdd,
    Sde,
    Sdf,
    Sdg,
    Sdh,
    Sdi,
    Sdj,

    // Virtio
    #[default]
    Vda,
    Vdb,
    Vdc,
    Vdd,
    Vde,
    Vdf,
    Vdg,
    Vdh,
    Vdi,
    Vdj,
}

impl fmt::Display for DiskTargets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // IDE
            DiskTargets::Hda => write!(f, "hda"),
            DiskTargets::Hdb => write!(f, "hdb"),
            DiskTargets::Hdc => write!(f, "hdc"),
            DiskTargets::Hdd => write!(f, "hdd"),
            DiskTargets::Hde => write!(f, "hde"),
            DiskTargets::Hdf => write!(f, "hdf"),
            DiskTargets::Hdg => write!(f, "hdg"),
            DiskTargets::Hdh => write!(f, "hdh"),
            DiskTargets::Hdi => write!(f, "hdi"),
            DiskTargets::Hdj => write!(f, "hdj"),
            // Sata
            DiskTargets::Sda => write!(f, "sda"),
            DiskTargets::Sdb => write!(f, "sdb"),
            DiskTargets::Sdc => write!(f, "sdc"),
            DiskTargets::Sdd => write!(f, "sdd"),
            DiskTargets::Sde => write!(f, "sde"),
            DiskTargets::Sdf => write!(f, "sdf"),
            DiskTargets::Sdg => write!(f, "sdg"),
            DiskTargets::Sdh => write!(f, "sdh"),
            DiskTargets::Sdi => write!(f, "sdi"),
            DiskTargets::Sdj => write!(f, "sdj"),
            // Virtio
            DiskTargets::Vda => write!(f, "vda"),
            DiskTargets::Vdb => write!(f, "vdb"),
            DiskTargets::Vdc => write!(f, "vdc"),
            DiskTargets::Vdd => write!(f, "vdd"),
            DiskTargets::Vde => write!(f, "vde"),
            DiskTargets::Vdf => write!(f, "vdf"),
            DiskTargets::Vdg => write!(f, "vdg"),
            DiskTargets::Vdh => write!(f, "vdh"),
            DiskTargets::Vdi => write!(f, "vdi"),
            DiskTargets::Vdj => write!(f, "vdj"),
        }
    }
}

impl DiskTargets {
    pub fn target(disk_bus: &DiskBuses, index: u8) -> Result<DiskTargets> {
        match disk_bus {
            DiskBuses::Ide => match index {
                0 => Ok(DiskTargets::Hda),
                1 => Ok(DiskTargets::Hdb),
                2 => Ok(DiskTargets::Hdc),
                3 => Ok(DiskTargets::Hdd),
                4 => Ok(DiskTargets::Hde),
                5 => Ok(DiskTargets::Hdf),
                6 => Ok(DiskTargets::Hdg),
                7 => Ok(DiskTargets::Hdh),
                8 => Ok(DiskTargets::Hdi),
                9 => Ok(DiskTargets::Hdj),
                _ => bail!("Unsupported disk target index: {index}"),
            },
            DiskBuses::Sata | DiskBuses::Usb => match index {
                0 => Ok(DiskTargets::Sda),
                1 => Ok(DiskTargets::Sdb),
                2 => Ok(DiskTargets::Sdc),
                3 => Ok(DiskTargets::Sdd),
                4 => Ok(DiskTargets::Sde),
                5 => Ok(DiskTargets::Sdf),
                6 => Ok(DiskTargets::Sdg),
                7 => Ok(DiskTargets::Sdh),
                8 => Ok(DiskTargets::Sdi),
                9 => Ok(DiskTargets::Sdj),
                _ => bail!("Unsupported disk target index: {index}"),
            },
            DiskBuses::Virtio => match index {
                0 => Ok(DiskTargets::Vda),
                1 => Ok(DiskTargets::Vdb),
                2 => Ok(DiskTargets::Vdc),
                3 => Ok(DiskTargets::Vdd),
                4 => Ok(DiskTargets::Vde),
                5 => Ok(DiskTargets::Vdf),
                6 => Ok(DiskTargets::Vdg),
                7 => Ok(DiskTargets::Vdh),
                8 => Ok(DiskTargets::Vdi),
                9 => Ok(DiskTargets::Vdj),
                _ => bail!("Unsupported disk target index: {index}"),
            },
            _ => bail!("Unsupported disk target bus: {disk_bus}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disk_drivers_display() {
        assert_eq!(DiskDrivers::Qemu.to_string(), "qemu");
    }

    #[test]
    fn test_disk_formats_display() {
        assert_eq!(DiskFormats::Qcow2.to_string(), "qcow2");
        assert_eq!(DiskFormats::Raw.to_string(), "raw");
    }

    #[test]
    fn test_disk_devices_display() {
        assert_eq!(DiskDevices::Cdrom.to_string(), "cdrom");
        assert_eq!(DiskDevices::File.to_string(), "file");
    }

    #[test]
    fn test_disk_buses_display() {
        assert_eq!(DiskBuses::Ide.to_string(), "ide");
        assert_eq!(DiskBuses::Sata.to_string(), "sata");
        assert_eq!(DiskBuses::Scsi.to_string(), "scsi");
        assert_eq!(DiskBuses::Usb.to_string(), "usb");
        assert_eq!(DiskBuses::Virtio.to_string(), "virtio");
    }

    #[test]
    fn test_disk_buses_to_vec_contains_all() {
        let buses = DiskBuses::to_vec();
        assert_eq!(buses.len(), 5);
    }

    #[test]
    fn test_disk_targets_ide_all_indices() {
        let expected = [
            "hda", "hdb", "hdc", "hdd", "hde", "hdf", "hdg", "hdh", "hdi", "hdj",
        ];
        for (i, name) in expected.iter().enumerate() {
            let t = DiskTargets::target(&DiskBuses::Ide, i as u8).unwrap();
            assert_eq!(t.to_string(), *name);
        }
    }

    #[test]
    fn test_disk_targets_sata_all_indices() {
        let expected = [
            "sda", "sdb", "sdc", "sdd", "sde", "sdf", "sdg", "sdh", "sdi", "sdj",
        ];
        for (i, name) in expected.iter().enumerate() {
            let t = DiskTargets::target(&DiskBuses::Sata, i as u8).unwrap();
            assert_eq!(t.to_string(), *name);
        }
    }

    #[test]
    fn test_disk_targets_usb_uses_sd_prefix() {
        // USB shares the Sata/Usb arm
        let t = DiskTargets::target(&DiskBuses::Usb, 0).unwrap();
        assert_eq!(t.to_string(), "sda");
        let t = DiskTargets::target(&DiskBuses::Usb, 9).unwrap();
        assert_eq!(t.to_string(), "sdj");
    }

    #[test]
    fn test_disk_targets_virtio_all_indices() {
        let expected = [
            "vda", "vdb", "vdc", "vdd", "vde", "vdf", "vdg", "vdh", "vdi", "vdj",
        ];
        for (i, name) in expected.iter().enumerate() {
            let t = DiskTargets::target(&DiskBuses::Virtio, i as u8).unwrap();
            assert_eq!(t.to_string(), *name);
        }
    }

    #[test]
    fn test_disk_targets_ide_out_of_range_returns_err() {
        assert!(DiskTargets::target(&DiskBuses::Ide, 10).is_err());
    }

    #[test]
    fn test_disk_targets_sata_out_of_range_returns_err() {
        assert!(DiskTargets::target(&DiskBuses::Sata, 10).is_err());
    }

    #[test]
    fn test_disk_targets_virtio_out_of_range_returns_err() {
        assert!(DiskTargets::target(&DiskBuses::Virtio, 10).is_err());
    }

    #[test]
    fn test_disk_targets_scsi_unsupported_returns_err() {
        assert!(DiskTargets::target(&DiskBuses::Scsi, 0).is_err());
    }
}
