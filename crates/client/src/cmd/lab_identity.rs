//! Which lab a folder holds. `.sherpa/lab-info.toml`, written by `sherpa up` and
//! `sherpa download`, holds the lab's ID as the server gave it, and is the lab's identity
//! whenever it is present: a downloaded lab, or one another OS user started, is found by it.
//! When `manifest.toml` is present too, it must be for the lab of the same name. A folder with
//! only a manifest is for the lab of that name owned by the Sherpa user logged in, whose ID is
//! a hash of the user's Sherpa name and the lab's name, the ID `sherpa up` gives a new lab.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use shared::data::{LabIdentity, LabInfo};
use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::{LabFolder, get_id_for_user, load_file};
use topology::Manifest;

use crate::token::{load_token, username_from_token};

/// The lab a folder names, before any ID is worked out.
#[derive(Debug, PartialEq, Eq)]
enum FolderLab {
    Recorded(LabIdentity),
    Named(String),
}

/// The lab of the folder `dir`, from `lab-info.toml` or `manifest.toml`.
///
/// # Errors
///
/// Fails when the folder has neither file, when `lab-info.toml` is there but does not parse,
/// when `manifest.toml` does not load and there is no `lab-info.toml`, when the two name
/// different labs, or when only a manifest names the lab and there is no login to work out its
/// ID from.
pub fn resolve_lab_identity(dir: &Path) -> Result<LabIdentity> {
    let lab_info = lab_info_identity(dir)?;
    let manifest_path = dir.join(SHERPA_MANIFEST_FILE);
    let manifest_name = if manifest_path.exists() {
        match Manifest::load_file(&manifest_path.to_string_lossy()) {
            Ok(manifest) => Some(manifest.name),
            Err(error) if lab_info.is_some() => {
                tracing::debug!(error = %format!("{error:#}"), "Using lab-info.toml: the manifest does not load");
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    identity_of(folder_lab(dir, lab_info, manifest_name)?)
}

/// The lab of `manifest` in the folder `dir`, which must be the lab `lab-info.toml` names, if
/// there is one.
///
/// # Errors
///
/// Fails when `lab-info.toml` does not parse or names another lab, or when there is none and no
/// login to work out the lab's ID from.
pub fn manifest_lab_identity(dir: &Path, manifest: &Manifest) -> Result<LabIdentity> {
    identity_of(folder_lab(
        dir,
        lab_info_identity(dir)?,
        Some(manifest.name.clone()),
    )?)
}

/// The lab's source manifest at `path`, for `command`, which builds nodes from it. A folder
/// made by `sherpa download` has none, and is told where to run `command` instead.
///
/// # Errors
///
/// Fails when there is no file at `path`, or it does not load.
pub fn load_source_manifest(path: &Path, command: &str) -> Result<Manifest> {
    if !path.exists() {
        bail!(
            "This folder has no {SHERPA_MANIFEST_FILE}, and 'sherpa {command}' needs the lab's \
             source manifest. A folder made by 'sherpa download' has none: run 'sherpa \
             {command}' in the folder the lab was started from, or copy its \
             {SHERPA_MANIFEST_FILE} here."
        );
    }
    Manifest::load_file(&path.to_string_lossy())
}

/// The ID `sherpa up` gives a new lab named `lab_name` for the user logged in with `token`: a
/// hash of their Sherpa username and the lab name, as the web UI gives one.
///
/// # Errors
///
/// Fails when `token` does not name a user.
fn new_lab_id(token: &str, lab_name: &str) -> Result<String> {
    Ok(get_id_for_user(&username_from_token(token)?, lab_name))
}

/// The identity of `lab`: a recorded lab's as recorded, a named one's from the login.
fn identity_of(lab: FolderLab) -> Result<LabIdentity> {
    match lab {
        FolderLab::Recorded(identity) => Ok(identity),
        FolderLab::Named(name) => Ok(LabIdentity {
            id: new_lab_id(&load_token()?, &name)?,
            name,
        }),
    }
}

/// `.sherpa/lab-info.toml` in the folder `dir`.
fn lab_info_file(dir: &Path) -> PathBuf {
    LabFolder::new(dir).lab_info()
}

/// The lab `.sherpa/lab-info.toml` in `dir` names, if there is one.
///
/// # Errors
///
/// Fails when the file is there but cannot be read or does not parse.
fn lab_info_identity(dir: &Path) -> Result<Option<LabIdentity>> {
    Ok(folder_lab_info(dir)?.map(|lab_info| LabIdentity {
        name: lab_info.name,
        id: lab_info.id,
    }))
}

/// `.sherpa/lab-info.toml` in `dir`, if there is one. A file that is there but does not parse
/// stops the command: it may record a lab another user started or one downloaded here, so
/// falling back to the manifest's lab could act on a different lab.
///
/// # Errors
///
/// Fails when the file is there but cannot be read or does not parse, naming the file.
fn folder_lab_info(dir: &Path) -> Result<Option<LabInfo>> {
    let path = lab_info_file(dir);
    if !path.exists() {
        return Ok(None);
    }
    load_file(&path.to_string_lossy())
        .and_then(|content| content.parse::<LabInfo>())
        .map(Some)
        .with_context(|| {
            format!(
                "{} does not parse; restore it with 'sherpa download' of the lab, or remove it \
                 if this folder is for the lab named in {SHERPA_MANIFEST_FILE}",
                path.display()
            )
        })
}

/// Stop unless the user logged in owns the lab of the folder `dir`: only a lab's owner can
/// connect to its nodes, admins included, so `sherpa ssh` and provisioners
/// call this before they try anything. A folder with no `lab-info.toml` is for the logged-in
/// user's own lab of its manifest's name.
///
/// # Errors
///
/// Fails when `lab-info.toml` does not parse or names another owner, or there is no login to
/// compare it with.
pub fn require_lab_owner(dir: &Path) -> Result<()> {
    let Some(lab_info) = folder_lab_info(dir)? else {
        return Ok(());
    };
    check_lab_owner(&lab_info, &username_from_token(&load_token()?)?)
}

/// Whether `username` owns the lab of `lab_info`, and may connect to its nodes.
fn check_lab_owner(lab_info: &LabInfo, username: &str) -> Result<()> {
    ensure!(
        lab_info.user == username,
        "Only the owner of lab '{}' can connect to its nodes",
        lab_info.id
    );
    Ok(())
}

/// The lab of the folder `dir`, named by its `lab_info` and by the name in its manifest,
/// whichever are present. When both are, the names must agree, and `lab_info`'s ID is the lab's.
fn folder_lab(
    dir: &Path,
    lab_info: Option<LabIdentity>,
    manifest_name: Option<String>,
) -> Result<FolderLab> {
    match (lab_info, manifest_name) {
        (Some(lab_info), Some(manifest_name)) => {
            let path = lab_info_file(dir);
            ensure!(
                lab_info.name == manifest_name,
                "{} is for lab {}-{}, but {SHERPA_MANIFEST_FILE} is for lab {}; destroy lab \
                 {}-{} or remove {} before using this folder for lab {}",
                path.display(),
                lab_info.name,
                lab_info.id,
                manifest_name,
                lab_info.name,
                lab_info.id,
                path.display(),
                manifest_name
            );
            Ok(FolderLab::Recorded(lab_info))
        }
        (Some(lab_info), None) => Ok(FolderLab::Recorded(lab_info)),
        (None, Some(manifest_name)) => Ok(FolderLab::Named(manifest_name)),
        (None, None) => bail!(
            "This folder has no {SHERPA_MANIFEST_FILE} or {}",
            lab_info_file(dir).display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use base64::{Engine as _, engine::general_purpose};

    use super::*;

    fn lab(name: &str, id: &str) -> LabIdentity {
        LabIdentity {
            name: name.to_string(),
            id: id.to_string(),
        }
    }

    /// A folder holding what `sherpa download` writes, `.sherpa/lab-info.toml` for lab `name`
    /// with `id`, and nothing else.
    fn downloaded_folder(name: &str, id: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = lab_info_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "id = '{id}'\nname = '{name}'\nuser = 'dev'\nipv4_network = '172.31.1.0/24'\n\
                 ipv4_gateway = '172.31.1.1'\nipv4_router = '172.31.1.2'\n"
            ),
        )
        .unwrap();
        dir
    }

    fn token_for(username: &str) -> String {
        let claims =
            format!(r#"{{"sub":"{username}","sid":"s","exp":1,"iat":1,"is_admin":false}}"#);
        format!(
            "header.{}.signature",
            general_purpose::URL_SAFE_NO_PAD.encode(claims)
        )
    }

    /// A manifest that does not load: its node list is never closed.
    const BROKEN_MANIFEST: &str = "name = 'core'\nnodes = [\n";

    #[test]
    fn lab_identity_001_a_folder_with_only_lab_info_is_the_lab_it_records() {
        let dir = downloaded_folder("core", "0a1b2c3d");
        assert!(!dir.path().join(SHERPA_MANIFEST_FILE).exists());
        let identity = resolve_lab_identity(dir.path()).unwrap();
        assert_eq!(
            (identity.name.as_str(), identity.id.as_str()),
            ("core", "0a1b2c3d")
        );
    }

    #[test]
    fn lab_identity_002_the_recorded_id_is_used_beside_a_manifest_of_the_same_name() {
        // The ID was made for another user's name, so no hash of this OS user's name gives
        // it; with a manifest for the same lab beside it, the folder still names it.
        let dir = downloaded_folder("core", "0a1b2c3d");
        fs::write(
            dir.path().join(SHERPA_MANIFEST_FILE),
            "name = 'core'\nnodes = []\n",
        )
        .unwrap();
        let identity = resolve_lab_identity(dir.path()).unwrap();
        assert_eq!(identity.id, "0a1b2c3d");

        let manifest: Manifest = toml::from_str("name = 'core'\nnodes = []\n").unwrap();
        assert_eq!(
            manifest_lab_identity(dir.path(), &manifest).unwrap().id,
            "0a1b2c3d"
        );
    }

    #[test]
    fn lab_identity_003_a_folder_with_only_a_manifest_is_the_logins_lab_of_its_name() {
        assert_eq!(
            folder_lab(Path::new(""), None, Some("core".to_string())).unwrap(),
            FolderLab::Named("core".to_string())
        );
        assert_eq!(new_lab_id(&token_for("dev"), "core").unwrap(), "b886ee2b");
        // Nothing from the OS user goes in: both run as the same OS user, so only the Sherpa
        // login tells them apart.
        assert_ne!(
            new_lab_id(&token_for("alice"), "core").unwrap(),
            new_lab_id(&token_for("bob"), "core").unwrap()
        );
    }

    #[test]
    fn lab_identity_005_files_naming_different_labs_are_refused() {
        let error =
            folder_lab(Path::new(""), Some(lab("a", "1")), Some("b".to_string())).unwrap_err();
        // The path is the platform's, `.sherpa\lab-info.toml` on Windows.
        let path = Path::new(".sherpa").join("lab-info.toml");
        assert_eq!(
            error.to_string(),
            format!(
                "{} is for lab a-1, but manifest.toml is for lab b; destroy lab a-1 or \
                 remove {} before using this folder for lab b",
                path.display(),
                path.display()
            )
        );
    }

    #[test]
    fn lab_identity_006_a_folder_with_neither_file_is_refused() {
        let error = folder_lab(Path::new(""), None, None).unwrap_err();
        let path = Path::new(".sherpa").join("lab-info.toml");
        assert_eq!(
            error.to_string(),
            format!("This folder has no manifest.toml or {}", path.display())
        );
    }

    #[test]
    fn lab_identity_007_up_redeploy_and_provision_without_a_manifest_say_where_to_run() {
        let dir = downloaded_folder("core", "0a1b2c3d");
        for command in ["up", "redeploy", "provision"] {
            let error =
                load_source_manifest(&dir.path().join(SHERPA_MANIFEST_FILE), command).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "This folder has no manifest.toml, and 'sherpa {command}' needs the lab's \
                     source manifest. A folder made by 'sherpa download' has none: run 'sherpa \
                     {command}' in the folder the lab was started from, or copy its \
                     manifest.toml here."
                )
            );
        }
    }

    #[test]
    fn lab_identity_008_a_manifest_that_does_not_load_is_passed_over_only_beside_lab_info() {
        let dir = downloaded_folder("core", "0a1b2c3d");
        fs::write(dir.path().join(SHERPA_MANIFEST_FILE), BROKEN_MANIFEST).unwrap();
        let identity = resolve_lab_identity(dir.path()).unwrap();
        assert_eq!(
            (identity.name.as_str(), identity.id.as_str()),
            ("core", "0a1b2c3d")
        );

        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join(SHERPA_MANIFEST_FILE);
        fs::write(&manifest, BROKEN_MANIFEST).unwrap();
        let error = resolve_lab_identity(dir.path()).unwrap_err().to_string();
        assert_eq!(
            error,
            format!("Failed to parse manifest {}", manifest.display())
        );
    }

    #[test]
    fn lab_identity_009_a_lab_info_that_does_not_parse_stops_the_command_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = lab_info_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "id = '0a1b2c3d'\nname = \n").unwrap();
        fs::write(
            dir.path().join(SHERPA_MANIFEST_FILE),
            "name = 'core'\nnodes = []\n",
        )
        .unwrap();
        let manifest: Manifest = toml::from_str("name = 'core'\nnodes = []\n").unwrap();
        let expected = format!(
            "{} does not parse; restore it with 'sherpa download' of the lab, or remove it if \
             this folder is for the lab named in manifest.toml",
            path.display()
        );

        for error in [
            resolve_lab_identity(dir.path()).unwrap_err(),
            manifest_lab_identity(dir.path(), &manifest).unwrap_err(),
            require_lab_owner(dir.path()).unwrap_err(),
        ] {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn access_control_016_a_folder_lab_of_another_user_is_refused_and_a_manifest_alone_is_own() {
        let dir = downloaded_folder("core", "0a1b2c3d");
        let lab_info = folder_lab_info(dir.path()).unwrap().unwrap();
        assert!(check_lab_owner(&lab_info, "dev").is_ok());
        for other in ["admin", "dana"] {
            assert_eq!(
                check_lab_owner(&lab_info, other).unwrap_err().to_string(),
                "Only the owner of lab '0a1b2c3d' can connect to its nodes"
            );
        }

        // No lab-info.toml: the lab's ID comes from the login, so it is the user's own lab and
        // no token is read.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(SHERPA_MANIFEST_FILE), "name = 'core'\n").unwrap();
        assert!(require_lab_owner(dir.path()).is_ok());
    }
}
