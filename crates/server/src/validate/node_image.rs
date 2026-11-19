use anyhow::{Result, bail};

use crate::services::error::ServiceError;
use shared::data::{ModelKind, NodeConfig};
use shared::validators::check_ztp;

/// The longest image version.
const IMAGE_VERSION_MAX_LEN: usize = 64;

/// Validate an image edited in the web form before it is saved. The ZTP setting must be one
/// the image's model and kind take; the CPU count, memory and, for a virtual machine or
/// unikernel, the interface MTU must be in range. The settings of other kinds the type keeps
/// out.
///
/// # Errors
///
/// Fails on the first setting out of range, naming it.
pub fn validate_node_image_update(model_kind: ModelKind, image: &NodeConfig) -> Result<()> {
    check_ztp(model_kind, image.ztp())?;
    validate_cpu_count(image.cpu.count)?;
    validate_memory(image.memory)?;
    if let Some(device) = image.interface_device() {
        validate_interface_mtu(device.mtu)?;
    }
    validate_image_version(&image.version)?;
    Ok(())
}

fn validate_cpu_count(count: u8) -> Result<()> {
    if count < 1 {
        bail!("CPU count must be at least 1");
    }
    Ok(())
}

fn validate_memory(mb: u16) -> Result<()> {
    if mb < 64 {
        bail!("Memory must be at least 64 MB");
    }
    Ok(())
}

fn validate_interface_mtu(mtu: u16) -> Result<()> {
    if !(576..=9600).contains(&mtu) {
        bail!("Interface MTU must be between 576 and 9600");
    }
    Ok(())
}

/// Check an image version, which names the image's folder and its container tag.
///
/// A version is 1 to 64 letters, digits, `.`, `_`, `+` or `-`, starting with a letter or digit,
/// so it can never be a path such as `..` or `a/b`.
///
/// # Errors
///
/// Fails with `ServiceError::Invalid` when `version` breaks the rule.
pub fn validate_image_version(version: &str) -> Result<()> {
    let starts_well = version
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let plain = version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
    if starts_well && plain && version.len() <= IMAGE_VERSION_MAX_LEN {
        return Ok(());
    }
    Err(ServiceError::Invalid(format!(
        "Image version '{version}' is not allowed: use 1 to {IMAGE_VERSION_MAX_LEN} letters, \
         digits, '.', '_', '+' or '-', starting with a letter or digit"
    ))
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{KindSettings, NodeModel, Ztp};

    #[test]
    fn test_validate_cpu_count() {
        assert!(validate_cpu_count(1).is_ok());
        assert!(validate_cpu_count(255).is_ok());
        assert!(validate_cpu_count(0).is_err());
    }

    #[test]
    fn test_validate_memory() {
        assert!(validate_memory(64).is_ok());
        assert!(validate_memory(1024).is_ok());
        assert!(validate_memory(63).is_err());
    }

    #[test]
    fn test_validate_interface_mtu() {
        assert!(validate_interface_mtu(576).is_ok());
        assert!(validate_interface_mtu(1500).is_ok());
        assert!(validate_interface_mtu(9600).is_ok());
        assert!(validate_interface_mtu(575).is_err());
        assert!(validate_interface_mtu(9601).is_err());
    }

    #[test]
    fn real_image_versions_are_accepted() {
        let longest = "1".repeat(64);
        for version in [
            "1.0",
            "24.4",
            "17",
            "9-23-1",
            "latest",
            "26.2R1.7-EVO",
            "v1.9.0+k3s1",
            "abc_1",
            longest.as_str(),
        ] {
            assert!(validate_image_version(version).is_ok(), "{version}");
        }
    }

    #[test]
    fn versions_that_are_paths_or_odd_names_are_refused() {
        let too_long = "1".repeat(65);
        for version in [
            "",
            " ",
            "..",
            ".",
            "../images",
            "a/b",
            ".hidden",
            "-x",
            "_x",
            "1 0",
            "1.0\n",
            too_long.as_str(),
        ] {
            assert!(validate_image_version(version).is_err(), "{version:?}");
        }
    }

    /// An image of `model`'s default kind with these settings.
    fn image(model: NodeModel, ztp: Ztp, cpu_count: u8, memory: u16, mtu: u16) -> NodeConfig {
        let mut image = ModelKind::default_for(model).node_config();
        image.cpu.count = cpu_count;
        image.memory = memory;
        "1.0.0".clone_into(&mut image.version);
        match &mut image.settings {
            KindSettings::VirtualMachine(vm) => {
                vm.ztp.method = ztp;
                vm.interfaces.mtu = mtu;
            }
            KindSettings::Container(container) => container.ztp.method = ztp,
            KindSettings::Unikernel(unikernel) => unikernel.interfaces.mtu = mtu,
        }
        image
    }

    fn validate(model: NodeModel, image: &NodeConfig) -> Result<()> {
        validate_node_image_update(ModelKind::default_for(model), image)
    }

    #[test]
    fn test_validate_node_image_update_all_valid() {
        let ubuntu = NodeModel::UbuntuLinux;
        assert!(validate(ubuntu, &image(ubuntu, Ztp::CloudInit, 2, 1024, 1500)).is_ok());
    }

    #[test]
    fn test_validate_node_image_update_cpu_zero() {
        let ubuntu = NodeModel::UbuntuLinux;
        let error = validate(ubuntu, &image(ubuntu, Ztp::CloudInit, 0, 1024, 1500)).unwrap_err();
        assert!(error.to_string().contains("CPU count"));
    }

    #[test]
    fn test_validate_node_image_update_memory_too_low() {
        let ubuntu = NodeModel::UbuntuLinux;
        let error = validate(ubuntu, &image(ubuntu, Ztp::CloudInit, 1, 32, 1500)).unwrap_err();
        assert!(error.to_string().contains("Memory"));
    }

    #[test]
    fn test_validate_node_image_update_mtu_out_of_range() {
        for model in [NodeModel::UbuntuLinux, NodeModel::NanosUnikernel] {
            let error = validate(model, &image(model, Ztp::Disabled, 1, 1024, 100)).unwrap_err();
            assert!(error.to_string().contains("MTU"), "{model}");
        }
    }

    #[test]
    fn a_container_image_has_no_mtu_to_check() {
        let mongo = NodeModel::MongoDb;
        assert!(validate(mongo, &image(mongo, Ztp::Disabled, 1, 1024, 100)).is_ok());
    }

    #[test]
    fn test_validate_node_image_update_empty_version() {
        let ubuntu = NodeModel::UbuntuLinux;
        let mut image = image(ubuntu, Ztp::CloudInit, 1, 1024, 1500);
        image.version.clear();
        let error = validate(ubuntu, &image).unwrap_err();
        assert!(error.to_string().contains("Image version"));
    }

    #[test]
    fn test_validate_node_image_update_refuses_a_ztp_method_the_model_does_not_take() {
        let ubuntu = NodeModel::UbuntuLinux;
        let error = validate(ubuntu, &image(ubuntu, Ztp::Tftp, 1, 1024, 1500))
            .unwrap_err()
            .to_string();
        assert!(error.contains("allows: disabled, cloud-init"), "{error}");
        let evolved = NodeModel::JuniperEvolved;
        for ztp in [Ztp::Disabled, Ztp::Tftp, Ztp::Usb] {
            assert!(
                validate(evolved, &image(evolved, ztp, 1, 1024, 1500)).is_ok(),
                "{ztp}"
            );
        }
    }
}
