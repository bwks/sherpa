//! ISO 9660 images for node configuration media: ZTP config drives, NoCloud seeds and the
//! PAN-OS bootstrap. Only what those readers need is written: a primary tree with relaxed names
//! and Rock Ridge, and a Joliet tree, both over the same file data. See
//! `docs/architecture/ZTP_ISO.md`.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use tracing::instrument;

use crate::konst::{
    ISO_APPLICATION_ID, ISO_DIR_MODE, ISO_FILE_MODE, ISO_JOLIET_ESCAPE, ISO_JOLIET_RESERVED,
    ISO_MAX_DEPTH, ISO_MAX_JOLIET_UNITS, ISO_MAX_JOLIET_VOLUME_ID_UNITS, ISO_MAX_NAME_BYTES,
    ISO_MAX_VOLUME_ID_BYTES, ISO_PADDING_SECTORS, ISO_RR_FLAGS_DOT, ISO_RR_FLAGS_NAMED,
    ISO_RRIP_DESCRIPTOR, ISO_RRIP_ID, ISO_RRIP_SOURCE, ISO_SECTOR_SIZE, ISO_STANDARD_ID,
    ISO_SYSTEM_AREA_SECTORS, ISO_SYSTEM_ID, ISO_TF_FLAGS,
};

/// The two directory trees on an image. Both point at the same file data.
#[derive(Clone, Copy, PartialEq)]
enum Tree {
    Primary,
    Joliet,
}

/// What a directory entry refers to: an index into `Source::dirs` or `Source::files`.
#[derive(Clone, Copy)]
enum Target {
    Dir(usize),
    File(usize),
}

/// A directory of the source, with its entries as read from disk.
struct SourceDir {
    parent: usize,
    entries: Vec<(String, Target)>,
    subdirs: u32,
}

/// A regular file of the source, and the sector its data is written at.
struct SourceFile {
    path: PathBuf,
    size: u32,
    extent: u32,
}

/// The source directory tree. The root is `dirs[0]`.
struct Source {
    dirs: Vec<SourceDir>,
    files: Vec<SourceFile>,
}

/// A directory entry as one tree records it.
struct Child {
    id: Vec<u8>,
    name: String,
    target: Target,
}

/// One tree's directories, indexed like `Source::dirs`: identifiers, children in record order,
/// path-table order and numbers, and where each directory's records sit.
struct TreeLayout {
    tree: Tree,
    dir_id: Vec<Vec<u8>>,
    children: Vec<Vec<Child>>,
    order: Vec<usize>,
    number: Vec<u16>,
    extent: Vec<u32>,
    size: Vec<u32>,
}

/// Where a tree's little- and big-endian path tables sit, and their length in bytes.
struct PathTables {
    little: u32,
    big: u32,
    len: u32,
}

/// Writes the image a sector at a time, so each part can be checked against the layout.
struct SectorWriter {
    out: BufWriter<File>,
    sector: u32,
}

/// Write an ISO 9660 image of the directory `src_dir` to `iso_dst`, labelled `volume_id`.
///
/// `iso_dst` may be inside `src_dir`; it is left out of the image. Names keep their case,
/// hyphens and dots in the primary tree, files are read-only and owned by root in Rock Ridge,
/// and Joliet carries the names unchanged. Symlinks and special files are refused.
#[instrument(level = "debug")]
pub fn create_iso(iso_dst: &str, src_dir: &str, volume_id: &str) -> Result<()> {
    validate_volume_id(volume_id)?;
    let output = output_path(Path::new(iso_dst))?;
    let root = fs::canonicalize(src_dir)
        .with_context(|| format!("Failed to read ISO source directory {src_dir}"))?;
    let mut source = Source {
        dirs: vec![SourceDir {
            parent: 0,
            entries: Vec::new(),
            subdirs: 0,
        }],
        files: Vec::new(),
    };
    scan_dir(&mut source, 0, &root, 1, &output)?;

    let now = Timestamp::now();
    let date = record_date(now);
    let mut primary = plan_tree(&source, Tree::Primary)?;
    let mut joliet = plan_tree(&source, Tree::Joliet)?;

    // Sector layout: system area, the three volume descriptors, path tables, primary
    // directories, the Rock Ridge continuation area, Joliet directories, file data, padding.
    let mut next = ISO_SYSTEM_AREA_SECTORS + 3;
    let primary_tables = place_path_tables(&source, &primary, &mut next)?;
    let joliet_tables = place_path_tables(&source, &joliet, &mut next)?;
    place_dirs(&mut primary, &mut next);
    let continuation = next;
    next += 1;
    place_dirs(&mut joliet, &mut next);
    let file_order = place_files(&mut source, &primary, &mut next);
    let volume_sectors = next + ISO_PADDING_SECTORS;
    let er = su_entry(b"ER", &er_data());

    let file = File::create(iso_dst).with_context(|| format!("Failed to create ISO {iso_dst}"))?;
    let mut writer = SectorWriter {
        out: BufWriter::new(file),
        sector: 0,
    };
    writer.zero_sectors(ISO_SYSTEM_AREA_SECTORS)?;
    for (layout, tables) in [(&primary, &primary_tables), (&joliet, &joliet_tables)] {
        writer.padded(&volume_descriptor(
            layout,
            tables,
            volume_id,
            volume_sectors,
            now,
        )?)?;
    }
    writer.padded(&terminator())?;
    for (layout, tables) in [(&primary, &primary_tables), (&joliet, &joliet_tables)] {
        writer.check_sector(tables.little)?;
        writer.padded(&path_table(&source, layout, false))?;
        writer.check_sector(tables.big)?;
        writer.padded(&path_table(&source, layout, true))?;
    }
    write_dirs(&mut writer, &source, &primary, date, continuation, er.len())?;
    writer.check_sector(continuation)?;
    writer.padded(&er)?;
    write_dirs(&mut writer, &source, &joliet, date, continuation, er.len())?;
    for f in file_order {
        let file = &source.files[f];
        writer.check_sector(file.extent)?;
        writer.file(file)?;
    }
    writer.zero_sectors(ISO_PADDING_SECTORS)?;
    writer.check_sector(volume_sectors)?;
    writer
        .out
        .flush()
        .with_context(|| format!("Failed to write ISO {iso_dst}"))?;
    tracing::debug!(path = %iso_dst, source_dir = %src_dir, %volume_id, sectors = volume_sectors, "ISO created");
    Ok(())
}

fn validate_volume_id(volume_id: &str) -> Result<()> {
    if volume_id.is_empty()
        || volume_id.len() > ISO_MAX_VOLUME_ID_BYTES
        || volume_id.encode_utf16().count() > ISO_MAX_JOLIET_VOLUME_ID_UNITS
        || !volume_id.bytes().all(|b| b.is_ascii_graphic())
    {
        bail!(
            "Invalid ISO volume label {volume_id:?}: it must be 1 to {ISO_MAX_JOLIET_VOLUME_ID_UNITS} printable ASCII characters"
        );
    }
    Ok(())
}

/// The canonical path the image is written to, so the walk can leave it out.
fn output_path(iso_dst: &Path) -> Result<PathBuf> {
    let name = iso_dst
        .file_name()
        .ok_or_else(|| anyhow!("ISO path has no file name: {}", iso_dst.display()))?;
    let parent = match iso_dst.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let parent = fs::canonicalize(parent)
        .with_context(|| format!("Failed to read ISO directory {}", parent.display()))?;
    Ok(parent.join(name))
}

/// Add the entries of the directory at `path` to `source` as `dir`, recursing into
/// subdirectories. `depth` counts the root as 1.
fn scan_dir(
    source: &mut Source,
    dir: usize,
    path: &Path,
    depth: usize,
    output: &Path,
) -> Result<()> {
    if depth > ISO_MAX_DEPTH {
        bail!(
            "{} is nested deeper than the {ISO_MAX_DEPTH} levels ISO 9660 allows",
            path.display()
        );
    }
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("Failed to read {}", path.display()))?
        .collect::<io::Result<Vec<_>>>()
        .with_context(|| format!("Failed to read {}", path.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let entry_path = entry.path();
        if entry_path == output {
            tracing::debug!(path = %entry_path.display(), "Leaving the ISO out of its own source");
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| anyhow!("File name is not UTF-8: {name:?}"))?;
        let meta = fs::symlink_metadata(&entry_path)
            .with_context(|| format!("Failed to read {}", entry_path.display()))?;
        if meta.is_dir() {
            let idx = source.dirs.len();
            source.dirs.push(SourceDir {
                parent: dir,
                entries: Vec::new(),
                subdirs: 0,
            });
            source.dirs[dir].entries.push((name, Target::Dir(idx)));
            source.dirs[dir].subdirs += 1;
            scan_dir(source, idx, &entry_path, depth + 1, output)?;
        } else if meta.is_file() {
            let size = u32::try_from(meta.len())
                .with_context(|| format!("{} is too large for ISO 9660", entry_path.display()))?;
            let idx = source.files.len();
            source.files.push(SourceFile {
                path: entry_path,
                size,
                extent: 0,
            });
            source.dirs[dir].entries.push((name, Target::File(idx)));
        } else {
            bail!(
                "{} is not a regular file or directory",
                entry_path.display()
            );
        }
    }
    Ok(())
}

/// The identifier `name` gets in a tree: relaxed names with `;1` on files in the primary tree,
/// UTF-16 big-endian in Joliet.
fn identifier(name: &str, is_dir: bool, tree: Tree) -> Result<Vec<u8>> {
    match tree {
        Tree::Primary => {
            let mut id: Vec<u8> = name
                .chars()
                .map(|c| {
                    if c.is_ascii_graphic() && c != ';' && c != '/' {
                        c as u8
                    } else {
                        b'_'
                    }
                })
                .collect();
            if id.len() > ISO_MAX_NAME_BYTES {
                bail!("File name longer than {ISO_MAX_NAME_BYTES} characters: {name}");
            }
            if !is_dir {
                if !id.contains(&b'.') {
                    id.push(b'.');
                }
                id.extend_from_slice(b";1");
            }
            Ok(id)
        }
        Tree::Joliet => {
            let units: Vec<u16> = name
                .chars()
                .map(|c| {
                    if ISO_JOLIET_RESERVED.contains(&c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect::<String>()
                .encode_utf16()
                .collect();
            if units.len() > ISO_MAX_JOLIET_UNITS {
                bail!("File name longer than {ISO_MAX_JOLIET_UNITS} characters: {name}");
            }
            Ok(units.iter().flat_map(|unit| unit.to_be_bytes()).collect())
        }
    }
}

/// A directory's entries as `tree` records them, in identifier order.
fn tree_children(dir: &SourceDir, tree: Tree) -> Result<Vec<Child>> {
    let mut children = Vec::with_capacity(dir.entries.len());
    for (name, target) in &dir.entries {
        children.push(Child {
            id: identifier(name, matches!(target, Target::Dir(_)), tree)?,
            name: name.clone(),
            target: *target,
        });
    }
    children.sort_by(|a, b| a.id.cmp(&b.id));
    for pair in children.windows(2) {
        if pair[0].id == pair[1].id {
            bail!(
                "{} and {} have the same ISO 9660 name",
                pair[0].name,
                pair[1].name
            );
        }
    }
    Ok(children)
}

/// Lay out one tree: identifiers, children, path-table order and numbers, directory sizes.
fn plan_tree(source: &Source, tree: Tree) -> Result<TreeLayout> {
    let count = source.dirs.len();
    let mut children = Vec::with_capacity(count);
    for dir in &source.dirs {
        children.push(tree_children(dir, tree)?);
    }
    let mut dir_id = vec![vec![0u8]; count];
    for child in children.iter().flatten() {
        if let Target::Dir(d) = child.target {
            dir_id[d] = child.id.clone();
        }
    }
    // Path tables list directories by level, each parent's subdirectories in identifier order.
    let mut order = vec![0];
    let mut at = 0;
    while at < order.len() {
        for child in &children[order[at]] {
            if let Target::Dir(d) = child.target {
                order.push(d);
            }
        }
        at += 1;
    }
    let mut number = vec![0u16; count];
    for (n, dir) in order.iter().enumerate() {
        number[*dir] =
            u16::try_from(n + 1).context("Too many directories for an ISO 9660 path table")?;
    }
    let mut layout = TreeLayout {
        tree,
        dir_id,
        children,
        order,
        number,
        extent: vec![0; count],
        size: vec![0; count],
    };
    for dir in 0..count {
        let len = dir_extent(source, &layout, dir, [0; 7], 0, 0)?.len();
        layout.size[dir] = u32::try_from(len).context("ISO 9660 directory too large")?;
    }
    Ok(layout)
}

fn place_path_tables(source: &Source, layout: &TreeLayout, next: &mut u32) -> Result<PathTables> {
    let len = path_table(source, layout, false).len();
    let sectors = sector_count(len);
    let tables = PathTables {
        little: *next,
        big: *next + sectors,
        len: u32::try_from(len).context("ISO 9660 path table too large")?,
    };
    *next += 2 * sectors;
    Ok(tables)
}

fn place_dirs(layout: &mut TreeLayout, next: &mut u32) {
    for dir in &layout.order {
        layout.extent[*dir] = *next;
        *next += sector_count(layout.size[*dir] as usize);
    }
}

/// Give each non-empty file its sectors, in primary-tree order, and return that order.
/// Empty files keep extent 0.
fn place_files(source: &mut Source, primary: &TreeLayout, next: &mut u32) -> Vec<usize> {
    let mut order = Vec::new();
    for dir in &primary.order {
        for child in &primary.children[*dir] {
            if let Target::File(f) = child.target {
                let file = &mut source.files[f];
                if file.size > 0 {
                    file.extent = *next;
                    *next += sector_count(file.size as usize);
                    order.push(f);
                }
            }
        }
    }
    order
}

fn path_table(source: &Source, layout: &TreeLayout, big_endian: bool) -> Vec<u8> {
    let mut table = Vec::new();
    for dir in &layout.order {
        let id = &layout.dir_id[*dir];
        let extent = layout.extent[*dir];
        let parent = layout.number[source.dirs[*dir].parent];
        table.push(id.len() as u8);
        table.push(0);
        if big_endian {
            table.extend_from_slice(&extent.to_be_bytes());
            table.extend_from_slice(&parent.to_be_bytes());
        } else {
            table.extend_from_slice(&extent.to_le_bytes());
            table.extend_from_slice(&parent.to_le_bytes());
        }
        table.extend_from_slice(id);
        if !id.len().is_multiple_of(2) {
            table.push(0);
        }
    }
    table
}

fn write_dirs(
    writer: &mut SectorWriter,
    source: &Source,
    layout: &TreeLayout,
    date: [u8; 7],
    continuation: u32,
    er_len: usize,
) -> Result<()> {
    for dir in &layout.order {
        writer.check_sector(layout.extent[*dir])?;
        writer.padded(&dir_extent(
            source,
            layout,
            *dir,
            date,
            continuation,
            er_len,
        )?)?;
    }
    Ok(())
}

/// A directory's records, packed so none crosses a sector boundary, padded to whole sectors.
/// In the primary tree each record carries Rock Ridge fields; the root's `.` also points at the
/// continuation area holding the `ER` entry.
fn dir_extent(
    source: &Source,
    layout: &TreeLayout,
    dir: usize,
    date: [u8; 7],
    continuation: u32,
    er_len: usize,
) -> Result<Vec<u8>> {
    let rock_ridge = layout.tree == Tree::Primary;
    let parent = source.dirs[dir].parent;
    let mut dot = Vec::new();
    let mut dotdot = Vec::new();
    if rock_ridge {
        if dir == 0 {
            dot.extend(su_entry(b"SP", &[0xbe, 0xef, 0]));
        }
        dot.extend(su_entry(b"RR", &[ISO_RR_FLAGS_DOT]));
        dot.extend(px_entry(ISO_DIR_MODE, 2 + source.dirs[dir].subdirs));
        dot.extend(tf_entry(date));
        if dir == 0 {
            let mut ce = Vec::with_capacity(24);
            ce.extend(both32(continuation));
            ce.extend(both32(0));
            ce.extend(both32(er_len as u32));
            dot.extend(su_entry(b"CE", &ce));
        }
        dotdot.extend(su_entry(b"RR", &[ISO_RR_FLAGS_DOT]));
        dotdot.extend(px_entry(ISO_DIR_MODE, 2 + source.dirs[parent].subdirs));
        dotdot.extend(tf_entry(date));
    }
    let mut records = vec![
        record(&[0], layout.extent[dir], layout.size[dir], true, date, &dot)?,
        record(
            &[1],
            layout.extent[parent],
            layout.size[parent],
            true,
            date,
            &dotdot,
        )?,
    ];
    for child in &layout.children[dir] {
        let (extent, size, is_dir, mode, nlink) = match child.target {
            Target::Dir(d) => (
                layout.extent[d],
                layout.size[d],
                true,
                ISO_DIR_MODE,
                2 + source.dirs[d].subdirs,
            ),
            Target::File(f) => (
                source.files[f].extent,
                source.files[f].size,
                false,
                ISO_FILE_MODE,
                1,
            ),
        };
        let mut system_use = Vec::new();
        if rock_ridge {
            let mut nm = vec![0u8];
            nm.extend_from_slice(child.name.as_bytes());
            system_use.extend(su_entry(b"RR", &[ISO_RR_FLAGS_NAMED]));
            system_use.extend(su_entry(b"NM", &nm));
            system_use.extend(px_entry(mode, nlink));
            system_use.extend(tf_entry(date));
        }
        records.push(record(&child.id, extent, size, is_dir, date, &system_use)?);
    }
    let mut out = Vec::new();
    for record in records {
        let used = out.len() % ISO_SECTOR_SIZE;
        if used + record.len() > ISO_SECTOR_SIZE {
            out.resize(out.len() + ISO_SECTOR_SIZE - used, 0);
        }
        out.extend(record);
    }
    out.resize(sector_count(out.len()) as usize * ISO_SECTOR_SIZE, 0);
    Ok(out)
}

/// One directory record. Its length is padded to even, as ECMA-119 requires.
fn record(
    id: &[u8],
    extent: u32,
    size: u32,
    is_dir: bool,
    date: [u8; 7],
    system_use: &[u8],
) -> Result<Vec<u8>> {
    let mut record = vec![0u8; 33];
    record[2..10].copy_from_slice(&both32(extent));
    record[10..18].copy_from_slice(&both32(size));
    record[18..25].copy_from_slice(&date);
    record[25] = if is_dir { 2 } else { 0 };
    record[28..32].copy_from_slice(&both16(1));
    record[32] = id.len() as u8;
    record.extend_from_slice(id);
    if id.len().is_multiple_of(2) {
        record.push(0);
    }
    record.extend_from_slice(system_use);
    if !record.len().is_multiple_of(2) {
        record.push(0);
    }
    record[0] = u8::try_from(record.len()).with_context(|| {
        format!(
            "ISO 9660 directory record too long for {}",
            String::from_utf8_lossy(id)
        )
    })?;
    Ok(record)
}

/// A System Use Sharing Protocol entry: signature, length, version 1, data.
fn su_entry(signature: &[u8; 2], data: &[u8]) -> Vec<u8> {
    let mut entry = Vec::with_capacity(4 + data.len());
    entry.extend_from_slice(signature);
    entry.push((4 + data.len()) as u8);
    entry.push(1);
    entry.extend_from_slice(data);
    entry
}

/// Rock Ridge `PX`: mode, links, and uid and gid 0.
fn px_entry(mode: u32, nlink: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(32);
    for value in [mode, nlink, 0, 0] {
        data.extend(both32(value));
    }
    su_entry(b"PX", &data)
}

/// Rock Ridge `TF`: modify, access and attribute-change times, all `date`.
fn tf_entry(date: [u8; 7]) -> Vec<u8> {
    let mut data = vec![ISO_TF_FLAGS];
    for _ in 0..3 {
        data.extend_from_slice(&date);
    }
    su_entry(b"TF", &data)
}

/// Rock Ridge `ER` data: the extension's identifier, descriptor and source.
fn er_data() -> Vec<u8> {
    let mut data = vec![
        ISO_RRIP_ID.len() as u8,
        ISO_RRIP_DESCRIPTOR.len() as u8,
        ISO_RRIP_SOURCE.len() as u8,
        1,
    ];
    data.extend_from_slice(ISO_RRIP_ID);
    data.extend_from_slice(ISO_RRIP_DESCRIPTOR);
    data.extend_from_slice(ISO_RRIP_SOURCE);
    data
}

/// The primary (type 1) or Joliet supplementary (type 2) volume descriptor.
fn volume_descriptor(
    layout: &TreeLayout,
    tables: &PathTables,
    volume_id: &str,
    volume_sectors: u32,
    now: Timestamp,
) -> Result<Vec<u8>> {
    let tree = layout.tree;
    let mut d = vec![0u8; ISO_SECTOR_SIZE];
    d[0] = match tree {
        Tree::Primary => 1,
        Tree::Joliet => 2,
    };
    d[1..6].copy_from_slice(ISO_STANDARD_ID);
    d[6] = 1;
    put_text(&mut d[8..40], ISO_SYSTEM_ID, tree);
    put_text(&mut d[40..72], volume_id, tree);
    d[80..88].copy_from_slice(&both32(volume_sectors));
    if tree == Tree::Joliet {
        d[88..91].copy_from_slice(ISO_JOLIET_ESCAPE);
    }
    d[120..124].copy_from_slice(&both16(1));
    d[124..128].copy_from_slice(&both16(1));
    d[128..132].copy_from_slice(&both16(ISO_SECTOR_SIZE as u16));
    d[132..140].copy_from_slice(&both32(tables.len));
    d[140..144].copy_from_slice(&tables.little.to_le_bytes());
    d[148..152].copy_from_slice(&tables.big.to_be_bytes());
    let root = record(
        &[0],
        layout.extent[0],
        layout.size[0],
        true,
        record_date(now),
        &[],
    )?;
    d[156..156 + root.len()].copy_from_slice(&root);
    // Volume set, publisher, preparer, application, copyright, abstract and bibliographic ids.
    for (range, value) in [
        (190..318, ""),
        (318..446, ""),
        (446..574, ""),
        (574..702, ISO_APPLICATION_ID),
        (702..739, ""),
        (739..776, ""),
        (776..813, ""),
    ] {
        put_text(&mut d[range], value, tree);
    }
    let stamp = volume_date(now);
    d[813..830].copy_from_slice(&stamp);
    d[830..847].copy_from_slice(&stamp);
    d[847..863].fill(b'0');
    d[864..881].copy_from_slice(&stamp);
    d[881] = 1;
    Ok(d)
}

fn terminator() -> Vec<u8> {
    let mut d = vec![0u8; ISO_SECTOR_SIZE];
    d[0] = 255;
    d[1..6].copy_from_slice(ISO_STANDARD_ID);
    d[6] = 1;
    d
}

/// Fill a descriptor text field: space-padded ASCII in the primary descriptor, space-padded
/// UTF-16 big-endian in Joliet. Values are validated or constant, so they fit.
fn put_text(field: &mut [u8], value: &str, tree: Tree) {
    match tree {
        Tree::Primary => {
            field.fill(b' ');
            field[..value.len()].copy_from_slice(value.as_bytes());
        }
        Tree::Joliet => {
            for pair in field.chunks_mut(2) {
                pair[0] = 0;
                if let Some(low) = pair.get_mut(1) {
                    *low = b' ';
                }
            }
            for (i, unit) in value.encode_utf16().enumerate() {
                field[2 * i..2 * i + 2].copy_from_slice(&unit.to_be_bytes());
            }
        }
    }
}

/// A directory record date: years since 1900, month, day, hour, minute, second, UTC offset 0.
fn record_date(now: Timestamp) -> [u8; 7] {
    let t = now.to_zoned(TimeZone::UTC);
    [
        (t.year() - 1900) as u8,
        t.month() as u8,
        t.day() as u8,
        t.hour() as u8,
        t.minute() as u8,
        t.second() as u8,
        0,
    ]
}

/// A volume descriptor date: `YYYYMMDDHHMMSScc` digits, then a UTC offset of 0.
fn volume_date(now: Timestamp) -> [u8; 17] {
    let digits = now
        .to_zoned(TimeZone::UTC)
        .strftime("%Y%m%d%H%M%S00")
        .to_string();
    let mut date = [0u8; 17];
    date[..16].copy_from_slice(&digits.as_bytes()[..16]);
    date
}

/// A 32-bit value little-endian then big-endian, as ISO 9660 stores most numbers.
fn both32(value: u32) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&value.to_le_bytes());
    out[4..].copy_from_slice(&value.to_be_bytes());
    out
}

fn both16(value: u16) -> [u8; 4] {
    let mut out = [0u8; 4];
    out[..2].copy_from_slice(&value.to_le_bytes());
    out[2..].copy_from_slice(&value.to_be_bytes());
    out
}

fn sector_count(len: usize) -> u32 {
    len.div_ceil(ISO_SECTOR_SIZE) as u32
}

impl SectorWriter {
    /// Write `data`, zero-padded to whole sectors.
    fn padded(&mut self, data: &[u8]) -> Result<()> {
        let sectors = sector_count(data.len());
        self.out.write_all(data).context("Failed to write ISO")?;
        self.out
            .write_all(&vec![0u8; sectors as usize * ISO_SECTOR_SIZE - data.len()])
            .context("Failed to write ISO")?;
        self.sector += sectors;
        Ok(())
    }

    fn zero_sectors(&mut self, count: u32) -> Result<()> {
        self.out
            .write_all(&vec![0u8; count as usize * ISO_SECTOR_SIZE])
            .context("Failed to write ISO")?;
        self.sector += count;
        Ok(())
    }

    /// Copy a source file's data, zero-padded to whole sectors.
    fn file(&mut self, file: &SourceFile) -> Result<()> {
        let mut input = File::open(&file.path)
            .with_context(|| format!("Failed to open {}", file.path.display()))?;
        let copied = io::copy(&mut input, &mut self.out)
            .with_context(|| format!("Failed to copy {} into the ISO", file.path.display()))?;
        if copied != u64::from(file.size) {
            bail!(
                "{} changed size while the ISO was written",
                file.path.display()
            );
        }
        let sectors = sector_count(file.size as usize);
        self.out
            .write_all(&vec![
                0u8;
                sectors as usize * ISO_SECTOR_SIZE - file.size as usize
            ])
            .context("Failed to write ISO")?;
        self.sector += sectors;
        Ok(())
    }

    /// Fail unless the next sector written is `sector`.
    fn check_sector(&self, sector: u32) -> Result<()> {
        if self.sector != sector {
            bail!(
                "ISO layout error: writing sector {}, expected {sector}",
                self.sector
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use anyhow::Result;
    use tempfile::TempDir;

    use super::*;
    use crate::konst::{
        ISO_DIR_MODE, ISO_FILE_MODE, ISO_PADDING_SECTORS, ISO_RRIP_ID, ISO_SECTOR_SIZE,
        PALOALTO_BOOTSTRAP_VOLUME_ID, ZTP_ISO, ZTP_ISO_VOLUME_ID,
    };

    const META_DATA: &str = "instance-id: iid-node1\nlocal-hostname: node1\n";
    const USER_DATA: &str = "#cloud-config\nhostname: node1\n";
    const NETWORK_CONFIG: &str = "version: 2\n";
    const INIT_CFG: &str = "type=static\nhostname=pan1\n";
    const BOOTSTRAP_XML: &str = "<config version=\"11.1.0\"/>\n";
    const BIG_FILE_LEN: usize = 10_000;
    const MANY_FILES: usize = 300;

    /// One directory record read back from an image.
    struct Record {
        id: Vec<u8>,
        extent: u32,
        size: u32,
        is_dir: bool,
        system_use: Vec<u8>,
    }

    /// Which names a test reads a tree by.
    #[derive(Clone, Copy)]
    enum Names {
        Primary,
        RockRidge,
        Joliet,
    }

    fn sector(image: &[u8], n: u32) -> &[u8] {
        let start = n as usize * ISO_SECTOR_SIZE;
        &image[start..start + ISO_SECTOR_SIZE]
    }

    fn le32(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes[..4].try_into().unwrap())
    }

    fn be32(bytes: &[u8]) -> u32 {
        u32::from_be_bytes(bytes[..4].try_into().unwrap())
    }

    fn le16(bytes: &[u8]) -> u16 {
        u16::from_le_bytes(bytes[..2].try_into().unwrap())
    }

    fn be16(bytes: &[u8]) -> u16 {
        u16::from_be_bytes(bytes[..2].try_into().unwrap())
    }

    /// Parse one directory record, checking its both-endian fields agree.
    fn parse_record(bytes: &[u8]) -> Record {
        let len = bytes[0] as usize;
        let record = &bytes[..len];
        assert_eq!(le32(&record[2..]), be32(&record[6..]), "extent both-endian");
        assert_eq!(le32(&record[10..]), be32(&record[14..]), "size both-endian");
        let id_len = record[32] as usize;
        let su_start = 33 + id_len + (id_len + 1) % 2;
        Record {
            id: record[33..33 + id_len].to_vec(),
            extent: le32(&record[2..]),
            size: le32(&record[10..]),
            is_dir: record[25] & 2 != 0,
            system_use: record[su_start..].to_vec(),
        }
    }

    /// The records of a directory extent. Fails if a record crosses a sector boundary.
    fn records(image: &[u8], extent: u32, size: u32) -> Vec<Record> {
        let mut out = Vec::new();
        for n in 0..size.div_ceil(ISO_SECTOR_SIZE as u32) {
            let data = sector(image, extent + n);
            let mut at = 0;
            while at < ISO_SECTOR_SIZE && data[at] != 0 {
                let len = data[at] as usize;
                assert!(
                    at + len <= ISO_SECTOR_SIZE,
                    "record crosses a sector boundary"
                );
                out.push(parse_record(&data[at..]));
                at += len;
            }
        }
        out
    }

    fn root_record(image: &[u8], descriptor: u32) -> Record {
        parse_record(&sector(image, descriptor)[156..190])
    }

    /// The system use entries of a record, as (signature, data) pairs.
    fn su_entries(system_use: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 4 <= system_use.len() && system_use[at + 2] != 0 {
            let len = system_use[at + 2] as usize;
            out.push((
                String::from_utf8_lossy(&system_use[at..at + 2]).to_string(),
                system_use[at + 4..at + len].to_vec(),
            ));
            at += len;
        }
        out
    }

    fn su_entry(record: &Record, sig: &str) -> Option<Vec<u8>> {
        su_entries(&record.system_use)
            .into_iter()
            .find(|(s, _)| s == sig)
            .map(|(_, data)| data)
    }

    fn name(record: &Record, names: Names) -> String {
        match names {
            Names::Primary => String::from_utf8(record.id.clone()).unwrap(),
            Names::RockRidge => {
                let nm = su_entry(record, "NM").expect("NM entry");
                String::from_utf8(nm[1..].to_vec()).unwrap()
            }
            Names::Joliet => {
                let units: Vec<u16> = record
                    .id
                    .chunks(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16(&units).unwrap()
            }
        }
    }

    /// Every file and directory under a tree's root, by path, with file contents.
    fn walk(
        image: &[u8],
        dir: &Record,
        names: Names,
        prefix: &str,
        out: &mut BTreeMap<String, Option<Vec<u8>>>,
    ) {
        for record in records(image, dir.extent, dir.size) {
            if record.id == [0] || record.id == [1] {
                continue;
            }
            let path = format!("{prefix}{}", name(&record, names));
            if record.is_dir {
                out.insert(format!("{path}/"), None);
                walk(image, &record, names, &format!("{path}/"), out);
            } else {
                let start = record.extent as usize * ISO_SECTOR_SIZE;
                let data = image[start..start + record.size as usize].to_vec();
                out.insert(path, Some(data));
            }
        }
    }

    fn tree(image: &[u8], names: Names) -> BTreeMap<String, Option<Vec<u8>>> {
        let descriptor = match names {
            Names::Joliet => 17,
            Names::Primary | Names::RockRidge => 16,
        };
        let mut out = BTreeMap::new();
        walk(image, &root_record(image, descriptor), names, "", &mut out);
        out
    }

    fn ztp_source(dir: &Path) -> Result<()> {
        fs::write(dir.join("meta-data"), META_DATA)?;
        fs::write(dir.join("user-data"), USER_DATA)?;
        fs::write(dir.join("network-config"), NETWORK_CONFIG)?;
        Ok(())
    }

    fn panos_source(dir: &Path) -> Result<()> {
        for sub in ["config", "content", "license", "software"] {
            fs::create_dir(dir.join(sub))?;
        }
        fs::write(dir.join("config/init-cfg.txt"), INIT_CFG)?;
        fs::write(dir.join("config/bootstrap.xml"), BOOTSTRAP_XML)?;
        Ok(())
    }

    /// Build an image of `src` into a separate directory and read it back.
    fn build(src: &Path, volume_id: &str) -> Result<Vec<u8>> {
        let out = TempDir::new()?;
        let iso = out.path().join(ZTP_ISO);
        create_iso(iso.to_str().unwrap(), src.to_str().unwrap(), volume_id)?;
        Ok(fs::read(iso)?)
    }

    /// Build `src` and return the full error chain, failing the test if the build succeeds.
    fn build_error(src: &Path, volume_id: &str) -> String {
        match build(src, volume_id) {
            Ok(_) => panic!("expected the ISO build to fail"),
            Err(err) => format!("{err:#}"),
        }
    }

    fn contents(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<Vec<u8>>> {
        entries
            .iter()
            .map(|(path, data)| (path.to_string(), data.map(|d| d.as_bytes().to_vec())))
            .collect()
    }

    #[test]
    fn test_create_iso_writes_labelled_volume_descriptors() -> Result<()> {
        let src = TempDir::new()?;
        ztp_source(src.path())?;
        let image = build(src.path(), ZTP_ISO_VOLUME_ID)?;

        assert_eq!(image.len() % ISO_SECTOR_SIZE, 0);
        let pvd = sector(&image, 16);
        assert_eq!(&pvd[..7], b"\x01CD001\x01");
        assert_eq!(&pvd[40..72], format!("{ZTP_ISO_VOLUME_ID:<32}").as_bytes());
        assert_eq!(le32(&pvd[80..]) as usize * ISO_SECTOR_SIZE, image.len());
        assert_eq!(le32(&pvd[80..]), be32(&pvd[84..]));
        assert_eq!(le16(&pvd[128..]) as usize, ISO_SECTOR_SIZE);
        assert_eq!(pvd[881], 1);

        let svd = sector(&image, 17);
        assert_eq!(&svd[..7], b"\x02CD001\x01");
        assert_eq!(&svd[88..91], b"%/E");
        let label: Vec<u8> = ZTP_ISO_VOLUME_ID
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(&svd[40..40 + label.len()], label.as_slice());
        assert_eq!(&svd[40 + label.len()..42 + label.len()], &[0, b' ']);

        assert_eq!(&sector(&image, 18)[..7], b"\xffCD001\x01");
        let tail = &image[image.len() - ISO_PADDING_SECTORS as usize * ISO_SECTOR_SIZE..];
        assert!(tail.iter().all(|b| *b == 0));
        Ok(())
    }

    #[test]
    fn test_create_iso_keeps_relaxed_primary_names() -> Result<()> {
        let src = TempDir::new()?;
        ztp_source(src.path())?;
        panos_source(src.path())?;
        let names: Vec<String> = tree(&build(src.path(), ZTP_ISO_VOLUME_ID)?, Names::Primary)
            .into_keys()
            .collect();
        assert_eq!(
            names,
            [
                "config/",
                "config/bootstrap.xml;1",
                "config/init-cfg.txt;1",
                "content/",
                "license/",
                "meta-data.;1",
                "network-config.;1",
                "software/",
                "user-data.;1",
            ]
        );
        Ok(())
    }

    #[test]
    fn test_create_iso_rock_ridge_names_and_modes() -> Result<()> {
        let src = TempDir::new()?;
        panos_source(src.path())?;
        fs::write(src.path().join("day0-config"), "hostname asa1\n")?;
        let image = build(src.path(), PALOALTO_BOOTSTRAP_VOLUME_ID)?;

        let root = root_record(&image, 16);
        let entries = records(&image, root.extent, root.size);
        let dot = su_entries(&entries[0].system_use);
        assert_eq!(dot[0], ("SP".to_string(), vec![0xbe, 0xef, 0]));
        let ce = su_entry(&entries[0], "CE").expect("CE on the root");
        let er = &sector(&image, le32(&ce))[..le32(&ce[16..]) as usize];
        assert_eq!(&er[..2], b"ER");
        assert_eq!(&er[8..8 + ISO_RRIP_ID.len()], ISO_RRIP_ID);

        for record in &entries[2..] {
            let px = su_entry(record, "PX").expect("PX entry");
            let (mode, nlink) = (le32(&px), le32(&px[8..]));
            if record.is_dir {
                assert_eq!(mode, ISO_DIR_MODE);
                assert_eq!(nlink, 2);
            } else {
                assert_eq!(mode, ISO_FILE_MODE);
                assert_eq!(nlink, 1);
            }
        }
        let root_px = su_entry(&entries[0], "PX").unwrap();
        assert_eq!(
            le32(&root_px[8..]),
            2 + 4,
            "root links: . plus four subdirectories"
        );

        let names: Vec<String> = tree(&image, Names::RockRidge).into_keys().collect();
        assert_eq!(
            names,
            [
                "config/",
                "config/bootstrap.xml",
                "config/init-cfg.txt",
                "content/",
                "day0-config",
                "license/",
                "software/",
            ]
        );
        Ok(())
    }

    #[test]
    fn test_create_iso_joliet_tree_matches_rock_ridge() -> Result<()> {
        let src = TempDir::new()?;
        ztp_source(src.path())?;
        panos_source(src.path())?;
        let image = build(src.path(), ZTP_ISO_VOLUME_ID)?;
        assert_eq!(tree(&image, Names::Joliet), tree(&image, Names::RockRidge));
        Ok(())
    }

    #[test]
    fn test_create_iso_round_trips_file_contents() -> Result<()> {
        let src = TempDir::new()?;
        panos_source(src.path())?;
        let big = "x".repeat(BIG_FILE_LEN);
        fs::write(src.path().join("big.bin"), &big)?;
        fs::write(src.path().join("empty"), "")?;
        let image = build(src.path(), PALOALTO_BOOTSTRAP_VOLUME_ID)?;
        let expected = contents(&[
            ("big.bin", Some(big.as_str())),
            ("config/", None),
            ("config/bootstrap.xml", Some(BOOTSTRAP_XML)),
            ("config/init-cfg.txt", Some(INIT_CFG)),
            ("content/", None),
            ("empty", Some("")),
            ("license/", None),
            ("software/", None),
        ]);
        assert_eq!(tree(&image, Names::RockRidge), expected);
        assert_eq!(tree(&image, Names::Joliet), expected);
        Ok(())
    }

    #[test]
    fn test_create_iso_directory_spans_sectors() -> Result<()> {
        let src = TempDir::new()?;
        for n in 0..MANY_FILES {
            fs::write(
                src.path().join(format!("config-file-number-{n}.txt")),
                n.to_string(),
            )?;
        }
        let image = build(src.path(), ZTP_ISO_VOLUME_ID)?;
        assert!(root_record(&image, 16).size as usize > ISO_SECTOR_SIZE);
        let files = tree(&image, Names::RockRidge);
        assert_eq!(files.len(), MANY_FILES);
        assert_eq!(files["config-file-number-299.txt"], Some(b"299".to_vec()));
        assert_eq!(tree(&image, Names::Joliet), files);
        Ok(())
    }

    #[test]
    fn test_create_iso_path_tables_list_directories() -> Result<()> {
        let src = TempDir::new()?;
        panos_source(src.path())?;
        fs::create_dir(src.path().join("config/nested"))?;
        let image = build(src.path(), ZTP_ISO_VOLUME_ID)?;
        let pvd = sector(&image, 16);
        let len = le32(&pvd[132..]) as usize;
        let little = &sector(&image, le32(&pvd[140..]))[..len];
        let big = &sector(&image, be32(&pvd[148..]))[..len];

        let mut entries = Vec::new();
        let mut at = 0;
        while at < len {
            let id_len = little[at] as usize;
            assert_eq!(little[at], big[at]);
            assert_eq!(le32(&little[at + 2..]), be32(&big[at + 2..]));
            assert_eq!(le16(&little[at + 6..]), be16(&big[at + 6..]));
            let id = String::from_utf8_lossy(&little[at + 8..at + 8 + id_len]).to_string();
            entries.push((id, le16(&little[at + 6..]), le32(&little[at + 2..])));
            at += 8 + id_len + id_len % 2;
        }
        let ids: Vec<(&str, u16)> = entries
            .iter()
            .map(|(id, parent, _)| (id.as_str(), *parent))
            .collect();
        assert_eq!(
            ids,
            [
                ("\0", 1),
                ("config", 1),
                ("content", 1),
                ("license", 1),
                ("software", 1),
                ("nested", 2),
            ]
        );
        assert_eq!(entries[0].2, root_record(&image, 16).extent);
        Ok(())
    }

    #[test]
    fn test_create_iso_leaves_out_its_own_output() -> Result<()> {
        let src = TempDir::new()?;
        ztp_source(src.path())?;
        let iso = src.path().join(ZTP_ISO);
        for _ in 0..2 {
            create_iso(
                iso.to_str().unwrap(),
                src.path().to_str().unwrap(),
                ZTP_ISO_VOLUME_ID,
            )?;
        }
        let names: Vec<String> = tree(&fs::read(&iso)?, Names::RockRidge)
            .into_keys()
            .collect();
        assert_eq!(names, ["meta-data", "network-config", "user-data"]);
        Ok(())
    }

    #[test]
    fn test_create_iso_rejects_missing_source() -> Result<()> {
        let dir = TempDir::new()?;
        let iso = dir.path().join(ZTP_ISO);
        let missing = dir.path().join("missing");
        let err = create_iso(
            iso.to_str().unwrap(),
            missing.to_str().unwrap(),
            ZTP_ISO_VOLUME_ID,
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("ISO source directory"),
            "{err:#}"
        );
        assert!(!iso.exists());
        Ok(())
    }

    #[test]
    fn test_create_iso_rejects_invalid_volume_ids() -> Result<()> {
        let src = TempDir::new()?;
        ztp_source(src.path())?;
        for volume_id in ["", "with space", "a-label-longer-than-sixteen"] {
            let err = build_error(src.path(), volume_id);
            assert!(
                err.contains("Invalid ISO volume label"),
                "{volume_id:?}: {err}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_create_iso_rejects_long_names() -> Result<()> {
        let src = TempDir::new()?;
        fs::write(src.path().join("a".repeat(32)), "")?;
        let err = build_error(src.path(), ZTP_ISO_VOLUME_ID);
        assert!(err.contains("longer than 31"), "{err}");
        Ok(())
    }

    #[test]
    fn test_create_iso_rejects_clashing_names() -> Result<()> {
        let src = TempDir::new()?;
        fs::write(src.path().join("a;b"), "")?;
        fs::write(src.path().join("a_b"), "")?;
        let err = build_error(src.path(), ZTP_ISO_VOLUME_ID);
        assert!(err.contains("same ISO 9660 name"), "{err}");
        Ok(())
    }

    #[test]
    fn test_create_iso_limits_nesting_to_eight_levels() -> Result<()> {
        let src = TempDir::new()?;
        let deepest = (1..=8).fold(src.path().to_path_buf(), |path, n| {
            path.join(format!("d{n}"))
        });
        fs::create_dir_all(&deepest)?;
        let err = build_error(src.path(), ZTP_ISO_VOLUME_ID);
        assert!(err.contains("nested deeper"), "{err}");
        fs::remove_dir(&deepest)?;
        build(src.path(), ZTP_ISO_VOLUME_ID)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_create_iso_rejects_symlinks() -> Result<()> {
        let src = TempDir::new()?;
        ztp_source(src.path())?;
        symlink(src.path().join("meta-data"), src.path().join("link"))?;
        let err = build_error(src.path(), ZTP_ISO_VOLUME_ID);
        assert!(err.contains("not a regular file or directory"), "{err}");
        Ok(())
    }
}
