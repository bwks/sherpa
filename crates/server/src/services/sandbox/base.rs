//! The Flatcar release sandboxes boot: its base image, its kernel and the kernel's command line.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str;

use anyhow::{Context, Result};

use shared::data::ModelKind;

use crate::services::error::ServiceError;

/// The Flatcar Container Linux release sandboxes boot.
pub(crate) const FLATCAR_SANDBOX_VERSION: &str = "4757.2.4";
/// The folder under the images folder that holds the sandbox base image and its kernel.
pub(crate) const SANDBOX_IMAGE_DIR: &str = "sandbox";
/// The GPT partition UUID of Flatcar's first `/usr` partition, the one sandboxes boot.
const FLATCAR_USR_A_PARTUUID: &str = "7130c94a-213a-4e5a-8e26-6cce9662f132";
/// Where in Flatcar's kernel image the hex hash of its `/usr` partition starts.
const KERNEL_USR_HASH_OFFSET: usize = 64;
/// The length of that hash in hex digits.
const KERNEL_USR_HASH_LEN: usize = 64;

/// The sandbox base image in images folder `images_dir`.
pub(crate) fn base_image(images_dir: &str) -> String {
    format!("{images_dir}/{SANDBOX_IMAGE_DIR}/flatcar-{FLATCAR_SANDBOX_VERSION}.qcow2")
}

/// The kernel of the sandbox base image in images folder `images_dir`.
pub(crate) fn base_kernel(images_dir: &str) -> String {
    format!("{images_dir}/{SANDBOX_IMAGE_DIR}/flatcar-{FLATCAR_SANDBOX_VERSION}.vmlinuz")
}

/// Refuse when one of `model_kinds` runs in a sandbox and the base image or its kernel is
/// missing from images folder `images_dir`.
///
/// # Errors
///
/// Fails with [`ServiceError::SandboxBaseImageMissing`] when the base image or its kernel is
/// needed and not there.
pub(crate) fn require_base_image(
    images_dir: &str,
    model_kinds: &[ModelKind],
) -> Result<(), ServiceError> {
    if !model_kinds.iter().any(|kind| kind.runs_in_sandbox()) {
        return Ok(());
    }
    for path in [base_image(images_dir), base_kernel(images_dir)] {
        if !Path::new(&path).is_file() {
            return Err(ServiceError::SandboxBaseImageMissing {
                path: PathBuf::from(path),
            });
        }
    }
    Ok(())
}

/// The hash of the `/usr` partition in Flatcar kernel image `header`, its first bytes.
fn usr_hash(header: &[u8]) -> Option<&str> {
    let hash = header.get(KERNEL_USR_HASH_OFFSET..KERNEL_USR_HASH_OFFSET + KERNEL_USR_HASH_LEN)?;
    hash.iter()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        .then(|| str::from_utf8(hash).ok())
        .flatten()
}

/// The hash of the `/usr` partition of the sandbox kernel in images folder `images_dir`.
///
/// # Errors
///
/// Fails when the kernel cannot be read or holds no hash.
pub(super) fn kernel_usr_hash(images_dir: &str) -> Result<String> {
    let path = base_kernel(images_dir);
    let mut header = [0; KERNEL_USR_HASH_OFFSET + KERNEL_USR_HASH_LEN];
    File::open(&path)
        .and_then(|mut file| file.read_exact(&mut header))
        .with_context(|| format!("Cannot read the sandbox kernel {path}"))?;
    Ok(usr_hash(&header)
        .ok_or(ServiceError::SandboxKernelNoUsrHash {
            path: PathBuf::from(&path),
        })?
        .to_owned())
}

/// The kernel command line of a sandbox, which runs Ignition when `first_boot`.
pub(super) fn kernel_cmdline(usr_hash: &str, first_boot: bool) -> String {
    let mut cmdline = format!(
        "mount.usr=/dev/mapper/usr verity.usr=PARTUUID={FLATCAR_USR_A_PARTUUID} \
         rootflags=rw,noinit_itable \
         mount.usrflags=ro consoleblank=0 root=LABEL=ROOT flatcar.oem.id=openstack \
         verity.usrhash={usr_hash}"
    );
    if first_boot {
        cmdline.push_str(" flatcar.first_boot=detected");
    }
    cmdline
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use shared::data::{NodeKind, NodeModel};

    use super::*;

    #[test]
    fn sandbox_033_the_usr_hash_is_read_from_the_kernel_header() {
        let hash = "6f38dc650e110fc0c1d2211a9c0ac3d165c7613fd81c9df9a4f3a5e59d6cd0a8";
        let mut header = vec![0u8; KERNEL_USR_HASH_OFFSET];
        header.extend_from_slice(hash.as_bytes());
        assert_eq!(usr_hash(&header), Some(hash));
        assert_eq!(usr_hash(&header[..100]), None);
        header[KERNEL_USR_HASH_OFFSET] = b'G';
        assert_eq!(usr_hash(&header), None);
    }

    #[test]
    fn sandbox_042_the_root_filesystem_is_mounted_without_zeroing_its_inode_tables() {
        assert!(
            kernel_cmdline("abc", true)
                .split(' ')
                .any(|arg| arg == "rootflags=rw,noinit_itable")
        );
    }

    #[test]
    fn sandbox_034_the_kernel_boots_without_a_console_and_runs_ignition_on_the_first_boot_only() {
        let first = kernel_cmdline("abc", true);
        let args: Vec<&str> = first.split(' ').collect();
        assert!(args.contains(&"verity.usrhash=abc"));
        assert!(args.contains(&"verity.usr=PARTUUID=7130c94a-213a-4e5a-8e26-6cce9662f132"));
        assert_eq!(args.last(), Some(&"flatcar.first_boot=detected"));
        assert!(!args.iter().any(|arg| arg.starts_with("console=")));
        assert_eq!(
            kernel_cmdline("abc", false),
            args[..args.len() - 1].join(" ")
        );
    }

    #[test]
    fn sandbox_018_a_lab_with_a_sandbox_is_refused_without_the_base_image() {
        let dir = TempDir::new().unwrap();
        let images = dir.path().to_str().unwrap();
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap();
        let frr = ModelKind::new(NodeModel::FrrLinux, Some(NodeKind::Container)).unwrap();
        assert!(require_base_image(images, &[frr]).is_ok());
        assert!(matches!(
            require_base_image(images, &[frr, ceos]),
            Err(ServiceError::SandboxBaseImageMissing { .. })
        ));
        fs::create_dir_all(dir.path().join(SANDBOX_IMAGE_DIR)).unwrap();
        fs::write(base_image(images), b"qcow2").unwrap();
        assert!(matches!(
            require_base_image(images, &[ceos]),
            Err(ServiceError::SandboxBaseImageMissing { path })
                if path == Path::new(&base_kernel(images))
        ));
        fs::write(base_kernel(images), b"kernel").unwrap();
        assert!(require_base_image(images, &[frr, ceos]).is_ok());
    }
}
