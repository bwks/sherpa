//! Which lab the current folder holds. `.sherpa/lab-info.toml`, written by `sherpa up` and
//! `sherpa download`, and `manifest.toml` must name the same lab when both are present, so every
//! command acts on the same one.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail, ensure};

use shared::data::{LabIdentity, LabInfo};
use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::{LabFolder, get_id, load_file};
use topology::Manifest;

/// The lab of the current folder, from `lab-info.toml` or `manifest.toml`.
///
/// # Errors
///
/// Fails when the folder has neither file, when `manifest.toml` does not load and there is no
/// `lab-info.toml`, or when the two name different labs.
pub fn resolve_lab_identity() -> Result<LabIdentity> {
    let lab_info = lab_info_identity();
    let manifest = if Path::new(SHERPA_MANIFEST_FILE).exists() {
        match Manifest::load_file(SHERPA_MANIFEST_FILE) {
            Ok(manifest) => Some(identity_of(&manifest)?),
            Err(error) if lab_info.is_some() => {
                tracing::debug!(error = %format!("{error:#}"), "Using lab-info.toml: the manifest does not load");
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    agreed_identity(lab_info, manifest)
}

/// The lab of `manifest`, which must be the lab `lab-info.toml` names, if there is one.
///
/// # Errors
///
/// Fails when the current user is unknown, or `lab-info.toml` names another lab.
pub fn manifest_lab_identity(manifest: &Manifest) -> Result<LabIdentity> {
    agreed_identity(lab_info_identity(), Some(identity_of(manifest)?))
}

/// The lab `manifest` is for: its name, and the ID the current user's labs of that name get.
fn identity_of(manifest: &Manifest) -> Result<LabIdentity> {
    Ok(LabIdentity {
        name: manifest.name.clone(),
        id: get_id(&manifest.name)?,
    })
}

/// `.sherpa/lab-info.toml`, relative to the current folder.
fn lab_info_file() -> PathBuf {
    LabFolder::new(Path::new("")).lab_info()
}

/// The lab `.sherpa/lab-info.toml` in the current folder names, if there is one that parses.
fn lab_info_identity() -> Option<LabIdentity> {
    let path = lab_info_file();
    if !path.exists() {
        return None;
    }
    match load_file(&path.to_string_lossy()).and_then(|content| content.parse::<LabInfo>()) {
        Ok(lab_info) => Some(LabIdentity {
            name: lab_info.name,
            id: lab_info.id,
        }),
        Err(error) => {
            tracing::debug!(error = %format!("{error:#}"), "Ignoring lab-info.toml");
            None
        }
    }
}

/// The lab named by `lab_info` and `manifest`, whichever are present; they must agree.
fn agreed_identity(
    lab_info: Option<LabIdentity>,
    manifest: Option<LabIdentity>,
) -> Result<LabIdentity> {
    match (lab_info, manifest) {
        (Some(lab_info), Some(manifest)) => {
            ensure!(
                lab_info.id == manifest.id,
                "{} is for lab {}-{}, but {SHERPA_MANIFEST_FILE} is for lab {}-{}; \
                 destroy lab {}-{} or remove {} before using this folder for lab {}",
                lab_info_file().display(),
                lab_info.name,
                lab_info.id,
                manifest.name,
                manifest.id,
                lab_info.name,
                lab_info.id,
                lab_info_file().display(),
                manifest.name
            );
            Ok(lab_info)
        }
        (Some(lab), None) | (None, Some(lab)) => Ok(lab),
        (None, None) => bail!(
            "This folder has no {SHERPA_MANIFEST_FILE} or {}",
            lab_info_file().display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab(name: &str, id: &str) -> LabIdentity {
        LabIdentity {
            name: name.to_string(),
            id: id.to_string(),
        }
    }

    #[test]
    fn either_file_alone_names_the_lab() {
        assert_eq!(agreed_identity(Some(lab("a", "1")), None).unwrap().id, "1");
        assert_eq!(agreed_identity(None, Some(lab("a", "1"))).unwrap().id, "1");
    }

    #[test]
    fn both_files_naming_the_same_lab_agree() {
        assert_eq!(
            agreed_identity(Some(lab("a", "1")), Some(lab("a", "1")))
                .unwrap()
                .id,
            "1"
        );
    }

    #[test]
    fn files_naming_different_labs_are_refused() {
        let error = agreed_identity(Some(lab("a", "1")), Some(lab("b", "2"))).unwrap_err();
        assert_eq!(
            error.to_string(),
            ".sherpa/lab-info.toml is for lab a-1, but manifest.toml is for lab b-2; destroy lab \
             a-1 or remove .sherpa/lab-info.toml before using this folder for lab b"
        );
    }

    #[test]
    fn a_folder_with_neither_file_names_no_lab() {
        assert!(agreed_identity(None, None).is_err());
    }
}
