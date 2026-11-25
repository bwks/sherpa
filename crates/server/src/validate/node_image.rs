use anyhow::{Context, Result, bail};

use crate::services::error::ServiceError;
use shared::data::{ModelKind, NodeConfig};
use shared::error::SharedError;
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
    if let Err(error) = check_ztp(model_kind, image.ztp()) {
        let refusal = error
            .downcast::<SharedError>()
            .context("Failed to check the image's ZTP setting")?;
        bail!(ServiceError::ImageZtpUnsupported { refusal });
    }
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
        bail!(ServiceError::ImageCpuCountZero);
    }
    Ok(())
}

fn validate_memory(mb: u16) -> Result<()> {
    if mb < 64 {
        bail!(ServiceError::ImageMemoryTooLow);
    }
    Ok(())
}

fn validate_interface_mtu(mtu: u16) -> Result<()> {
    if !(576..=9600).contains(&mtu) {
        bail!(ServiceError::ImageMtuOutOfRange);
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
    fn image_import_021_versions_vendors_use_are_taken() {
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
    fn image_import_022_a_version_that_could_be_a_path_or_has_another_character_is_refused() {
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
    fn image_settings_008_a_cpu_count_memory_or_mtu_out_of_range_is_refused_naming_the_setting() {
        let ubuntu = NodeModel::UbuntuLinux;
        assert!(validate(ubuntu, &image(ubuntu, Ztp::CloudInit, 2, 1024, 1500)).is_ok());

        let error = validate(ubuntu, &image(ubuntu, Ztp::CloudInit, 0, 1024, 1500)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::ImageCpuCountZero)
        );

        let error = validate(ubuntu, &image(ubuntu, Ztp::CloudInit, 1, 32, 1500)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::ImageMemoryTooLow)
        );

        for model in [NodeModel::UbuntuLinux, NodeModel::NanosUnikernel] {
            let error = validate(model, &image(model, Ztp::Disabled, 1, 1024, 100)).unwrap_err();
            assert_eq!(
                error.downcast_ref::<ServiceError>(),
                Some(&ServiceError::ImageMtuOutOfRange),
                "{model}"
            );
        }
    }

    #[test]
    fn image_settings_009_a_container_image_has_no_interface_mtu_to_check() {
        let mongo = NodeModel::MongoDb;
        assert!(validate(mongo, &image(mongo, Ztp::Disabled, 1, 1024, 100)).is_ok());
    }

    #[test]
    fn ztp_setting_012_an_edit_to_a_method_the_model_does_not_take_is_refused() {
        let ubuntu = NodeModel::UbuntuLinux;
        let error = validate(ubuntu, &image(ubuntu, Ztp::Tftp, 1, 1024, 1500)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::ImageZtpUnsupported {
                refusal: SharedError::UnsupportedZtp {
                    ztp: Ztp::Tftp,
                    model: NodeModel::UbuntuLinux,
                    allowed: vec![Ztp::Disabled, Ztp::CloudInit],
                },
            })
        );
        let evolved = NodeModel::JuniperEvolved;
        for ztp in [Ztp::Disabled, Ztp::Tftp, Ztp::Usb] {
            assert!(
                validate(evolved, &image(evolved, ztp, 1, 1024, 1500)).is_ok(),
                "{ztp}"
            );
        }
    }
}
