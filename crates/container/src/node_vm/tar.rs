//! A ustar archive of the files Sherpa puts into a node VM's container, the form Docker's
//! archive API takes.

use crate::error::ContainerError;

/// The size of a tar block: every header, and every file's data padded to it.
const BLOCK: usize = 512;
/// The longest name a ustar header holds in its name field.
const NAME_LEN: usize = 100;
/// The longest path prefix a ustar header holds.
const PREFIX_LEN: usize = 155;

/// A file to put into a container: its absolute path there, its contents and its mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeVmFile {
    pub path: String,
    pub contents: Vec<u8>,
    pub mode: u32,
}

/// The ustar archive of `files`, each at its path relative to the root, owned by root. No
/// folder entries are written, so the folders a file goes into keep their owner and mode, and
/// Docker makes any that are missing.
///
/// # Errors
///
/// Fails when a path is not absolute, has a `..` part, or is too long for a ustar header.
pub fn archive(files: &[NodeVmFile]) -> Result<Vec<u8>, ContainerError> {
    let mut tar = Vec::new();
    for file in files {
        let path = relative_path(&file.path)?;
        tar.extend_from_slice(&header(&file.path, path, file.contents.len(), file.mode)?);
        tar.extend_from_slice(&file.contents);
        tar.resize(tar.len().next_multiple_of(BLOCK), 0);
    }
    tar.resize(tar.len() + 2 * BLOCK, 0);
    Ok(tar)
}

/// `path` without its leading `/`, refused when it is not absolute or climbs out with `..`.
fn relative_path(path: &str) -> Result<&str, ContainerError> {
    let relative = path
        .strip_prefix('/')
        .ok_or_else(|| ContainerError::NodeVmFilePath(path.to_owned()))?;
    if relative.is_empty()
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "..")
    {
        return Err(ContainerError::NodeVmFilePath(path.to_owned()));
    }
    Ok(relative)
}

/// The ustar header of a regular file at `path`, `size` bytes long with `mode`.
fn header(
    original: &str,
    path: &str,
    size: usize,
    mode: u32,
) -> Result<[u8; BLOCK], ContainerError> {
    let too_long = || ContainerError::NodeVmFilePath(original.to_owned());
    let (prefix, name) = if path.len() <= NAME_LEN {
        ("", path)
    } else {
        let split = path[..=PREFIX_LEN.min(path.len() - 1)]
            .rfind('/')
            .ok_or_else(too_long)?;
        (&path[..split], &path[split + 1..])
    };
    if name.len() > NAME_LEN || prefix.len() > PREFIX_LEN {
        return Err(too_long());
    }
    let mut block = [0u8; BLOCK];
    block[..name.len()].copy_from_slice(name.as_bytes());
    octal(&mut block[100..108], u64::from(mode & 0o7777));
    octal(&mut block[108..116], 0);
    octal(&mut block[116..124], 0);
    octal(&mut block[124..136], size as u64);
    octal(&mut block[136..148], 0);
    block[148..156].fill(b' ');
    block[156] = b'0';
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");
    block[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    let checksum: u64 = block.iter().map(|byte| u64::from(*byte)).sum();
    octal(&mut block[148..155], checksum);
    block[155] = b' ';
    Ok(block)
}

/// Write `value` in octal into `field`, zero-padded, ending in NUL.
fn octal(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    let text = format!("{value:0digits$o}");
    field[..digits].copy_from_slice(&text.as_bytes()[text.len() - digits..]);
    field[digits] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, contents: &[u8]) -> NodeVmFile {
        NodeVmFile {
            path: path.to_owned(),
            contents: contents.to_vec(),
            mode: 0o644,
        }
    }

    #[test]
    fn node_vm_006_a_node_file_is_archived_at_its_container_path_padded_to_blocks() {
        let tar = archive(&[file("/mnt/flash/startup-config", b"hostname r1\n")]).unwrap();
        assert_eq!(tar.len(), 4 * BLOCK);
        assert!(tar.starts_with(b"mnt/flash/startup-config\0"));
        assert_eq!(&tar[124..136], b"00000000014\0");
        assert_eq!(&tar[257..262], b"ustar");
        assert_eq!(&tar[BLOCK..BLOCK + 12], b"hostname r1\n");
        let sum: u64 = tar[..BLOCK]
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    u64::from(*b)
                }
            })
            .sum();
        let stored = u64::from_str_radix(str::from_utf8(&tar[148..154]).unwrap(), 8).unwrap();
        assert_eq!(stored, sum);
    }

    #[test]
    fn node_vm_007_a_long_path_goes_into_the_prefix_and_a_relative_or_climbing_one_is_refused() {
        let long = format!("/{}/{}", "d".repeat(120), "config.json");
        let tar = archive(&[file(&long, b"{}")]).unwrap();
        assert!(tar.starts_with(b"config.json\0"));
        assert_eq!(&tar[345..465], "d".repeat(120).as_bytes());
        for path in ["etc/passwd", "/etc/../passwd", "/", "/etc//passwd"] {
            assert!(archive(&[file(path, b"")]).is_err(), "{path}");
        }
    }
}
