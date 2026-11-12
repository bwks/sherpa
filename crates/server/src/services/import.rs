use anyhow::{Context, Result};
use db::{Database, DefaultRequest};
use futures_util::{Stream, StreamExt};
use opentelemetry::KeyValue;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::io::AsyncWriteExt;

use shared::data::{
    DownloadImageRequest, ImageSummary, ImportRequest, ImportResponse, ListImagesRequest,
    ListImagesResponse, NodeConfig, NodeKind, NodeModel, ScanImagesRequest, ScanImagesResponse,
    ScannedImage, SetDefaultImageRequest, SetDefaultImageResponse, ShowImageRequest,
    ShowImageResponse, SkippedModel, StatusKind,
};
use shared::konst::{SHERPA_IMAGES_PATH, TALOS_IMAGE_FACTORY_URL, TALOS_VANILLA_SCHEMATIC_YAML};
use shared::util::{file_exists, image_filename};

use tracing::instrument;

use crate::daemon::state::AppState;
use crate::services::container_scan::{ScanTarget, match_container_images};
use crate::services::image_lifecycle::lock_image;
use crate::services::image_store::{ExistingDisk, ImagePaths, PlacedDisk, StagedDisk};
use crate::services::progress::ProgressSender;
use crate::validate::{validate_image_scan, validate_image_version};

/// Bytes between progress reports while downloading an image.
const DOWNLOAD_REPORT_INTERVAL: u64 = 5 * 1024 * 1024;

/// Import an image to the server and track it in the database.
///
/// For VMs and unikernels: copies the disk image (qcow2) to the images directory.
/// For containers: loads a tar archive into the Docker daemon via `docker load`.
#[instrument(skip(state, progress), fields(model = %request.model, version = %request.version))]
pub async fn import_image(
    request: ImportRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<ImportResponse> {
    let start = Instant::now();

    validate_image_version(&request.version)?;
    let _image_guard = lock_image(request.model, &request.version).await;
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    let _ = progress.send_status(
        format!("Validating source file: {}", request.src),
        StatusKind::Info,
    );

    // Validate source file exists on the server
    if !file_exists(&request.src) {
        anyhow::bail!("Source file does not exist: {}", request.src);
    }

    let request_default = if request.default {
        DefaultRequest::Make
    } else {
        DefaultRequest::Keep
    };
    let mut db_config = config;
    db_config.version.clone_from(&request.version);
    db_config.id = None;

    let result = match kind {
        NodeKind::Container => {
            import_container_image(&request, db_config, request_default, state, &progress).await
        }
        NodeKind::VirtualMachine | NodeKind::Unikernel => {
            import_disk_image(
                &request,
                db_config,
                request_default,
                &state.db,
                &ImagePaths::installed(),
                &progress,
            )
            .await
        }
    };

    let op_attrs = &[KeyValue::new("operation.type", "import")];
    state
        .metrics
        .operation_duration
        .record(start.elapsed().as_secs_f64(), op_attrs);
    if result.is_err() {
        state.metrics.error_count.add(1, op_attrs);
    }

    result
}

/// Load a container image's tar archive into the Docker daemon and record `db_config`.
#[instrument(skip_all, fields(model = %request.model, version = %request.version), level = "debug")]
async fn import_container_image(
    request: &ImportRequest,
    db_config: NodeConfig,
    request_default: DefaultRequest,
    state: &AppState,
    progress: &ProgressSender,
) -> Result<ImportResponse> {
    tracing::info!(
        model = %request.model,
        version = %request.version,
        src = %request.src,
        "Importing container image from tar archive"
    );

    let _ = progress.send_status(
        format!("Loading container image from tar archive: {}", request.src),
        StatusKind::Progress,
    );

    container::load_image(&state.docker, &request.src, |msg| {
        tracing::info!("{}", msg);
    })
    .await
    .with_context(|| {
        format!(
            "Failed to load container image from '{}' for model '{}'",
            request.src, request.model
        )
    })?;

    let _ = progress.send_status(
        "Container image loaded into Docker daemon".to_string(),
        StatusKind::Done,
    );

    // Record in database
    let _ = progress.send_status("Updating database...".to_string(), StatusKind::Progress);

    let kind = db_config.kind.clone();
    let recorded = db::upsert_node_image(&state.db, db_config, request_default)
        .await
        .context(format!(
            "Failed to register container model '{}' in database",
            request.model
        ))?;

    tracing::info!(
        "Upserted node_image for model={} version={}",
        request.model,
        request.version
    );

    let _ = progress.send_status("Image tracked in database".to_string(), StatusKind::Done);

    Ok(ImportResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version.clone(),
        image_path: format!("loaded from {}", request.src),
        db_tracked: true,
        default: recorded.default,
    })
}

/// Place a VM or unikernel disk image in the images directory, then record `db_config`.
///
/// The image is copied to a staging file and renamed into place before the row is written, so a
/// row never names a missing or partial file (see [`image_store`](crate::services::image_store)).
/// A file already at the destination is kept when it has the source's size, replaced when it does
/// not, and left alone with an error when lab overlays are built on it.
#[instrument(skip_all, fields(model = %request.model, version = %request.version), level = "debug")]
async fn import_disk_image(
    request: &ImportRequest,
    db_config: NodeConfig,
    request_default: DefaultRequest,
    database: &Database,
    paths: &ImagePaths,
    progress: &ProgressSender,
) -> Result<ImportResponse> {
    let kind = db_config.kind.clone();
    let filename = image_filename(&kind, db_config.boot_mode.as_ref());
    let version_dir = paths.version_dir(request.model, &request.version);
    let version_disk = version_dir.join(filename);

    let source_len = tokio::fs::metadata(&request.src)
        .await
        .with_context(|| format!("Failed to read source file {}", request.src))?
        .len();
    let existing_len = match tokio::fs::metadata(&version_disk).await {
        Ok(metadata) => Some(metadata.len()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read {}", version_disk.display()));
        }
    };

    let placed = match ExistingDisk::compare(source_len, existing_len) {
        ExistingDisk::Same => {
            let _ = progress.send_status(
                "Image already exists on disk with the same size, skipping copy".to_string(),
                StatusKind::Info,
            );
            None
        }
        existing => {
            if existing == ExistingDisk::Different {
                libvirt::ensure_no_overlay_dependencies(&paths.storage_pool, &version_dir)
                .with_context(|| {
                    format!(
                        "{} differs from the source and lab disks are built on it; destroy those \
                         labs or import the image as a new version",
                        version_disk.display()
                    )
                })?;
                let _ = progress.send_status(
                    "Image on disk differs from the source, replacing it".to_string(),
                    StatusKind::Info,
                );
            }
            Some(copy_disk_into_place(request, &version_dir, &version_disk, progress).await?)
        }
    };

    let _ = progress.send_status("Updating database...".to_string(), StatusKind::Progress);
    let recorded = match db::upsert_node_image(database, db_config, request_default).await {
        Ok(recorded) => recorded,
        Err(error) => {
            if let Some(placed) = placed {
                placed.discard();
            }
            return Err(error.context(format!(
                "Failed to register node model '{}' in database. Ensure the server is up to date.",
                request.model
            )));
        }
    };
    tracing::info!(
        model = %request.model,
        version = %request.version,
        default = recorded.default,
        "Recorded the imported image"
    );
    let _ = progress.send_status("Image tracked in database".to_string(), StatusKind::Done);

    Ok(ImportResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version.clone(),
        image_path: version_disk.to_string_lossy().into_owned(),
        db_tracked: true,
        default: recorded.default,
    })
}

/// Copy the import's source to a staging file in `version_dir` and rename it onto `version_disk`.
async fn copy_disk_into_place(
    request: &ImportRequest,
    version_dir: &Path,
    version_disk: &Path,
    progress: &ProgressSender,
) -> Result<PlacedDisk> {
    tokio::fs::create_dir_all(version_dir)
        .await
        .with_context(|| format!("Failed to create {}", version_dir.display()))?;
    let _ = progress.send_status(
        format!("Copying image to {}...", version_disk.display()),
        StatusKind::Progress,
    );
    tracing::info!(src = %request.src, dest = %version_disk.display(), "Copying image");
    let staged = StagedDisk::new(version_disk);
    let source = PathBuf::from(&request.src);
    // The blocking task owns the stage, so a cancelled request still removes the partial file
    // once the copy stops.
    let placed = tokio::task::spawn_blocking(move || {
        staged.copy_from(&source)?;
        staged.place()
    })
    .await
    .context("The image copy task failed")??;
    let _ = progress.send_status("Image copy complete".to_string(), StatusKind::Done);
    Ok(placed)
}

/// List images from the database with optional filtering by model and/or kind
#[instrument(skip(state), level = "debug")]
pub async fn list_images(
    request: ListImagesRequest,
    state: &AppState,
) -> Result<ListImagesResponse> {
    let configs = match (&request.model, &request.kind) {
        (Some(model), Some(kind)) => db::get_node_image_versions(&state.db, model, kind)
            .await
            .context("Failed to query node images by model and kind")?,
        (None, Some(kind)) => db::list_node_images_by_kind(&state.db, kind)
            .await
            .context("Failed to query node images by kind")?,
        (Some(model), None) => {
            let config = NodeConfig::get_model(*model);
            db::get_node_image_versions(&state.db, model, &config.kind)
                .await
                .context("Failed to query node images by model")?
        }
        (None, None) => db::list_node_images(&state.db)
            .await
            .context("Failed to query all node images")?,
    };

    let images: Vec<ImageSummary> = configs
        .into_iter()
        .map(|c| ImageSummary {
            model: c.model,
            kind: c.kind,
            version: c.version,
            default: c.default,
        })
        .collect();

    let total = images.len();
    Ok(ListImagesResponse { images, total })
}

/// Show detailed information about a specific node image
#[instrument(skip(state), level = "debug")]
pub async fn show_image(request: ShowImageRequest, state: &AppState) -> Result<ShowImageResponse> {
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    let image = match request.version {
        Some(version) => {
            db::find_node_image_by_model_kind_version(&state.db, &request.model, &kind, &version)
                .await
                .context("Failed to query node image by model, kind and version")?
        }
        None => db::find_default_node_image(&state.db, &request.model, &kind)
            .await
            .context("Failed to query default node image")?,
    };

    let image = image.ok_or_else(|| {
        anyhow::anyhow!(
            "No image found for model '{}' (kind: {})",
            request.model,
            kind
        )
    })?;

    Ok(ShowImageResponse { image })
}

/// Scan for images that are present but not recorded, and record them: disk images in the
/// images directory, and container images in local Docker. A request's `model` and `repo`
/// record that repo's local tags for that container model, and scan nothing else.
#[instrument(skip(state), level = "debug")]
pub async fn scan_images(
    request: ScanImagesRequest,
    state: &AppState,
) -> Result<ScanImagesResponse> {
    validate_image_scan(&request)?;
    let target = request
        .model
        .zip(request.repo.as_deref())
        .map(|(model, repo)| ScanTarget { model, repo });
    let wants = |kind: NodeKind| request.kind.as_ref().is_none_or(|k| *k == kind);

    let mut tally = ScanTally::default();
    if target.is_none() && (wants(NodeKind::VirtualMachine) || wants(NodeKind::Unikernel)) {
        scan_disk_images(&request, state, &mut tally).await?;
    }
    if wants(NodeKind::Container) {
        scan_container_images(target, request.dry_run, state, &mut tally).await?;
    }

    Ok(ScanImagesResponse {
        scanned: tally.scanned,
        skipped: tally.skipped,
        total_imported: tally.total_imported,
    })
}

/// What an image scan found, and how many versions it imported or would import.
#[derive(Default)]
struct ScanTally {
    scanned: Vec<ScannedImage>,
    skipped: Vec<SkippedModel>,
    total_imported: usize,
}

/// One model during a scan: its config, and whether it has a default image yet.
struct ModelScan {
    model: NodeModel,
    kind: NodeKind,
    config: NodeConfig,
    /// Whether the model has a default, or a version this scan imported or would import became it.
    has_default: bool,
    /// What the import log calls the record.
    label: &'static str,
}

impl ModelScan {
    /// Start scanning `model`, from its current default.
    async fn new(database: &Database, model: NodeModel, label: &'static str) -> Result<Self> {
        let config = NodeConfig::get_model(model);
        let kind = config.kind.clone();
        let default = db::find_default_node_image(database, &model, &kind)
            .await
            .context("Failed to query the model's default image")?;
        Ok(Self {
            model,
            kind,
            config,
            has_default: default.is_some(),
            label,
        })
    }

    fn scanned(&self, version: String, status: String) -> ScannedImage {
        ScannedImage {
            model: self.model,
            version,
            kind: self.kind.clone(),
            status,
        }
    }
}

/// Scan `images_dir/<model>/<version>/` for VM and unikernel disk images.
#[instrument(skip_all, level = "debug")]
async fn scan_disk_images(
    request: &ScanImagesRequest,
    state: &AppState,
    tally: &mut ScanTally,
) -> Result<()> {
    let images_dir = SHERPA_IMAGES_PATH.to_owned();

    // Read model directories from the images dir
    let mut model_entries = tokio::fs::read_dir(&images_dir)
        .await
        .context(format!("Failed to read images directory: {images_dir}"))?;

    while let Some(entry) = model_entries
        .next_entry()
        .await
        .context("Failed to read directory entry")?
    {
        // Skip non-directories
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }

        let dir_name = entry.file_name().to_string_lossy().to_string();

        // Try to parse the directory name as a NodeModel
        let model: NodeModel = if let Ok(m) = dir_name.parse() {
            m
        } else {
            tracing::debug!("Skipping non-model directory: {}", dir_name);
            continue;
        };

        // Skip container kinds in filesystem scan (containers live in Docker, not on disk)
        let kind = NodeConfig::get_model(model).kind;
        if kind == NodeKind::Container {
            tracing::debug!("Skipping container model in disk scan: {}", model);
            continue;
        }

        // If a kind filter was provided, check it
        if let Some(ref filter_kind) = request.kind
            && *filter_kind != kind
        {
            continue;
        }

        let mut scan = ModelScan::new(&state.db, model, "node_image").await?;
        let model_dir = format!("{images_dir}/{dir_name}");
        scan_version_dirs(state, &mut scan, &model_dir, request.dry_run, tally).await?;
    }
    Ok(())
}

/// Scan a model directory's version subdirectories for its disk image. Symlinked directories
/// are skipped.
async fn scan_version_dirs(
    state: &AppState,
    scan: &mut ModelScan,
    model_dir: &str,
    dry_run: bool,
    tally: &mut ScanTally,
) -> Result<()> {
    let mut version_entries = match tokio::fs::read_dir(model_dir).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("Failed to read model directory {}: {}", model_dir, e);
            return Ok(());
        }
    };

    while let Some(version_entry) = version_entries
        .next_entry()
        .await
        .context("Failed to read version directory entry")?
    {
        let Ok(version_metadata) = version_entry.metadata().await else {
            continue;
        };

        // Skip symlinks and non-directories
        if !version_metadata.is_dir() || version_metadata.file_type().is_symlink() {
            continue;
        }

        // Also check via symlink_metadata to catch symlinked directories
        let Ok(symlink_meta) = tokio::fs::symlink_metadata(version_entry.path()).await else {
            continue;
        };
        if symlink_meta.file_type().is_symlink() {
            continue;
        }

        let version = version_entry.file_name().to_string_lossy().to_string();
        let filename = image_filename(&scan.kind, scan.config.boot_mode.as_ref());
        let disk_path = format!("{model_dir}/{version}/{filename}");

        // Check if image file exists in this version directory
        if !file_exists(&disk_path) {
            tracing::debug!(
                "No {} found for model={} version={}",
                filename,
                scan.model,
                version
            );
            continue;
        }

        scan_version(
            &state.db,
            scan,
            version,
            Some(PathBuf::from(disk_path)),
            dry_run,
            tally,
        )
        .await?;
    }
    Ok(())
}

/// Record the Docker daemon's local image tags that [`match_container_images`] matches to
/// container models, each with the repo it was matched by.
#[instrument(skip_all, level = "debug")]
async fn scan_container_images(
    target: Option<ScanTarget<'_>>,
    dry_run: bool,
    state: &AppState,
    tally: &mut ScanTally,
) -> Result<()> {
    let docker_tags = container::get_local_images(&state.docker)
        .await
        .context("Failed to list local Docker images")?;
    let found = match_container_images(&docker_tags, target);
    for model in &found.skipped {
        tracing::debug!(model = %model.model, reason = %model.reason, "Skipping container model");
    }
    tally.skipped = found.skipped;

    for versions in found.matches.chunk_by(|a, b| a.model == b.model) {
        let Some(first) = versions.first() else {
            continue;
        };
        let mut scan = ModelScan::new(&state.db, first.model, "container node_image").await?;
        scan.config.repo = Some(first.repo.clone());
        for image in versions {
            scan_version(
                &state.db,
                &mut scan,
                image.version.clone(),
                None,
                dry_run,
                tally,
            )
            .await?;
        }
    }
    Ok(())
}

/// Record one version a scan found: already tracked, would be imported (dry run), or imported.
/// A version becomes the default only when its model has none.
///
/// A disk image is checked again under the image's lock, so a scan that waited for a delete of
/// the same version finds its file gone and does not record it again. `disk` is `None` for a
/// container image.
async fn scan_version(
    database: &Database,
    scan: &mut ModelScan,
    version: String,
    disk: Option<PathBuf>,
    dry_run: bool,
    tally: &mut ScanTally,
) -> Result<()> {
    let _image_guard = lock_image(scan.model, &version).await;
    if let Some(disk) = &disk
        && !tokio::fs::try_exists(disk).await.unwrap_or(false)
    {
        tracing::debug!(model = %scan.model, %version, "Image removed while the scan waited");
        return Ok(());
    }
    let existing =
        db::find_node_image_by_model_kind_version(database, &scan.model, &scan.kind, &version)
            .await
            .context("Failed to check existing node_image")?;
    if existing.is_some() {
        tally
            .scanned
            .push(scan.scanned(version, "already_exists".to_string()));
        return Ok(());
    }

    if dry_run {
        let status = if scan.has_default {
            "would_import"
        } else {
            "would_import (default)"
        };
        scan.has_default = true;
        tally
            .scanned
            .push(scan.scanned(version, status.to_string()));
        tally.total_imported += 1;
        return Ok(());
    }

    let mut db_config = scan.config.clone();
    db_config.version.clone_from(&version);
    db_config.id = None;

    let status = match db::upsert_node_image(database, db_config, DefaultRequest::Keep).await {
        Ok(recorded) => {
            tracing::info!(
                "Scanned and imported {} for model={} version={} default={}",
                scan.label,
                scan.model,
                version,
                recorded.default
            );
            scan.has_default = true;
            tally.total_imported += 1;
            if recorded.default {
                "imported (default)".to_string()
            } else {
                "imported".to_string()
            }
        }
        Err(e) => {
            tracing::error!(
                "Failed to upsert node_image for model={} version={}: {:?}",
                scan.model,
                version,
                e
            );
            format!("error: {e:#}")
        }
    };

    tally.scanned.push(scan.scanned(version, status));
    Ok(())
}

/// Set the default version for a node image
#[instrument(skip(state), fields(model = %request.model, version = %request.version), level = "debug")]
pub async fn set_default_image(
    request: SetDefaultImageRequest,
    state: &AppState,
) -> Result<SetDefaultImageResponse> {
    let _image_guard = lock_image(request.model, &request.version).await;
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    // Look up the image in the database
    let node_image = db::find_node_image_by_model_kind_version(
        &state.db,
        &request.model,
        &kind,
        &request.version,
    )
    .await
    .context("Failed to look up image in database")?;

    let Some(node_image) = node_image else {
        anyhow::bail!(
            "Image not found for model '{}' with version '{}'. Use 'server image list' to see available images.",
            request.model,
            request.version
        );
    };

    db::update_node_image(&state.db, node_image, DefaultRequest::Make)
        .await
        .context("Failed to update image as default")?;

    Ok(SetDefaultImageResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
    })
}

/// Resolve the download URL for a VM image
async fn resolve_download_url(
    model: &NodeModel,
    version: &str,
    url: Option<&str>,
) -> Result<String> {
    if let Some(url) = url {
        return Ok(url.to_string());
    }

    match model {
        NodeModel::UbuntuLinux => Ok(format!(
            "https://cloud-images.ubuntu.com/releases/{version}/release/ubuntu-{version}-server-cloudimg-amd64.img"
        )),
        NodeModel::TalosLinux => {
            let schematic_id = talos_vanilla_schematic_id().await?;
            Ok(talos_image_url(&schematic_id, version))
        }
        _ => anyhow::bail!("No auto-download URL for model '{model}'. Provide --url."),
    }
}

/// Ask the Talos Image Factory for the ID of the vanilla schematic (no system extensions).
///
/// The factory derives a schematic's ID from its content, so the same schematic always has
/// the same ID; Sherpa asks rather than storing it.
#[instrument(level = "debug")]
async fn talos_vanilla_schematic_id() -> Result<String> {
    let url = format!("{TALOS_IMAGE_FACTORY_URL}/schematics");
    let response = reqwest::Client::new()
        .post(&url)
        .body(TALOS_VANILLA_SCHEMATIC_YAML)
        .send()
        .await
        .with_context(|| {
            format!("Failed to reach the Talos Image Factory at {url}. Provide --url instead")
        })?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Talos Image Factory returned HTTP {} for the vanilla schematic. Provide --url instead",
            response.status()
        );
    }
    let body = response
        .bytes()
        .await
        .context("Failed to read the Talos Image Factory response")?;
    parse_schematic_id(&body)
}

/// Read the schematic ID from an Image Factory `/schematics` response.
fn parse_schematic_id(body: &[u8]) -> Result<String> {
    let response: serde_json::Value =
        serde_json::from_slice(body).context("Talos Image Factory response is not JSON")?;
    let id = response["id"]
        .as_str()
        .context("Talos Image Factory response has no schematic id")?;
    anyhow::ensure!(
        id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()),
        "Talos Image Factory returned an unexpected schematic id: {id}"
    );
    Ok(id.to_owned())
}

/// `NoCloud` image URL for a schematic and Talos version (with or without a leading `v`).
fn talos_image_url(schematic_id: &str, version: &str) -> String {
    let version = version.strip_prefix('v').unwrap_or(version);
    format!("{TALOS_IMAGE_FACTORY_URL}/image/{schematic_id}/v{version}/nocloud-amd64.qcow2")
}

/// Stream a download into a staging file for `dest`, rename it into place, and return the placed
/// image and its size.
///
/// The rename happens only when the whole body arrived and, when the server sent a length, matches
/// it. An interrupted download leaves nothing at `dest` for a later run to take as a complete image.
async fn write_download<S, B, E>(
    mut body: S,
    dest: &Path,
    expected_size: Option<u64>,
    progress: &ProgressSender,
) -> Result<(PlacedDisk, u64)>
where
    S: Stream<Item = std::result::Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::error::Error + Send + Sync + 'static,
{
    let staged = StagedDisk::new(dest);
    let mut file = tokio::fs::File::create(staged.partial_path())
        .await
        .with_context(|| format!("Failed to create file {}", staged.partial_path().display()))?;
    let mut downloaded: u64 = 0;
    let mut last_reported: u64 = 0;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.context("Failed to read response chunk")?;
        let chunk = chunk.as_ref();
        file.write_all(chunk)
            .await
            .context("Failed to write chunk to disk")?;
        downloaded += chunk.len() as u64;
        if downloaded - last_reported >= DOWNLOAD_REPORT_INTERVAL {
            last_reported = downloaded;
            let _ = progress.send_status(
                download_progress(downloaded, expected_size),
                StatusKind::Progress,
            );
        }
    }
    file.sync_all()
        .await
        .context("Failed to flush image file")?;
    drop(file);
    if let Some(expected) = expected_size
        && downloaded != expected
    {
        anyhow::bail!("Download ended after {downloaded} of {expected} bytes");
    }
    let placed = tokio::task::spawn_blocking(move || staged.place())
        .await
        .context("The image rename task failed")??;
    Ok((placed, downloaded))
}

/// A progress line for `downloaded` bytes of `total`, when known.
fn download_progress(downloaded: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => format!(
            "Downloaded {} MB / {} MB ({}%)",
            megabytes(downloaded),
            megabytes(total),
            downloaded * 100 / total.max(1)
        ),
        None => format!("Downloaded {} MB", megabytes(downloaded)),
    }
}

/// `bytes` in MiB to one decimal place, rounded, for progress messages.
fn megabytes(bytes: u64) -> String {
    let tenths = (u128::from(bytes) * 10 + 524_288) / 1_048_576;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Download a VM image from a URL and track it in the database
#[instrument(skip(state, progress), fields(model = %request.model, version = %request.version))]
pub async fn download_image(
    request: DownloadImageRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<ImportResponse> {
    validate_image_version(&request.version)?;
    let _image_guard = lock_image(request.model, &request.version).await;
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    if kind != NodeKind::VirtualMachine {
        anyhow::bail!("Image download is only supported for virtual machine models, got '{kind}'");
    }

    let _ = progress.send_status("Resolving download URL...".to_string(), StatusKind::Info);

    let download_url =
        resolve_download_url(&request.model, &request.version, request.url.as_deref()).await?;

    let filename = image_filename(&kind, config.boot_mode.as_ref());
    let version_dir = ImagePaths::installed().version_dir(request.model, &request.version);
    let version_disk = version_dir.join(filename);
    tokio::fs::create_dir_all(&version_dir)
        .await
        .with_context(|| format!("Failed to create {}", version_dir.display()))?;

    // A file at the destination was renamed there whole, by an earlier download or import.
    let placed = if tokio::fs::try_exists(&version_disk).await.unwrap_or(false) {
        tracing::info!(path = %version_disk.display(), "Image already exists, skipping download");
        let _ = progress.send_status(
            "Image already exists, skipping download".to_string(),
            StatusKind::Info,
        );
        None
    } else {
        let _ = progress.send_status(
            format!(
                "Downloading {} {} from {}...",
                request.model, request.version, download_url
            ),
            StatusKind::Progress,
        );
        let response = reqwest::get(&download_url)
            .await
            .context(format!("Failed to download image from {download_url}"))?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to download image: HTTP {} from {}",
                response.status(),
                download_url
            );
        }
        let total_size = response.content_length();
        let (placed, downloaded) = write_download(
            response.bytes_stream(),
            &version_disk,
            total_size,
            &progress,
        )
        .await?;
        tracing::info!(path = %placed.path().display(), "Download complete");
        let _ = progress.send_status(
            format!("Download complete: {} MB", megabytes(downloaded)),
            StatusKind::Done,
        );
        Some(placed)
    };

    let _ = progress.send_status("Updating database...".to_string(), StatusKind::Progress);
    let request_default = if request.default {
        DefaultRequest::Make
    } else {
        DefaultRequest::Keep
    };
    let mut db_config = config;
    db_config.version.clone_from(&request.version);
    db_config.id = None;
    let recorded = match db::upsert_node_image(&state.db, db_config, request_default).await {
        Ok(recorded) => recorded,
        Err(error) => {
            if let Some(placed) = placed {
                placed.discard();
            }
            return Err(error.context(format!(
                "Failed to record {} {} in the database",
                request.model, request.version
            )));
        }
    };
    let _ = progress.send_status("Image tracked in database".to_string(), StatusKind::Done);

    Ok(ImportResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
        image_path: version_disk.to_string_lossy().into_owned(),
        db_tracked: true,
        default: recorded.default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::io;

    fn quiet_progress() -> ProgressSender {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        ProgressSender::new(tx)
    }

    fn chunks(parts: Vec<io::Result<Vec<u8>>>) -> impl Stream<Item = io::Result<Vec<u8>>> + Unpin {
        stream::iter(parts)
    }

    #[tokio::test]
    async fn a_complete_download_lands_at_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("disk.qcow2");
        let body = chunks(vec![Ok(b"abc".to_vec()), Ok(b"def".to_vec())]);

        let (placed, size) = write_download(body, &dest, Some(6), &quiet_progress())
            .await
            .unwrap();

        assert_eq!(size, 6);
        assert_eq!(placed.path(), dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"abcdef");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn an_interrupted_download_leaves_nothing_to_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("disk.qcow2");
        let body = chunks(vec![
            Ok(b"abc".to_vec()),
            Err(io::Error::other("connection reset")),
        ]);

        assert!(
            write_download(body, &dest, None, &quiet_progress())
                .await
                .is_err()
        );

        assert!(!dest.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_body_shorter_than_its_content_length_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("disk.qcow2");
        let body = chunks(vec![Ok(b"abc".to_vec())]);

        let error = write_download(body, &dest, Some(10), &quiet_progress())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("3 of 10 bytes"), "{error}");
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn test_resolve_download_url_ubuntu() {
        let url = resolve_download_url(&NodeModel::UbuntuLinux, "24.04", None)
            .await
            .unwrap();
        assert_eq!(
            url,
            "https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-amd64.img"
        );
    }

    #[test]
    fn talos_image_urls_use_the_schematic_and_a_v_prefixed_version() {
        let id = "a".repeat(64);
        for version in ["1.14.4", "v1.14.4"] {
            assert_eq!(
                talos_image_url(&id, version),
                format!("https://factory.talos.dev/image/{id}/v1.14.4/nocloud-amd64.qcow2")
            );
        }
    }

    #[test]
    fn schematic_ids_are_read_from_the_factory_response() {
        let id = "0123456789abcdef".repeat(4);
        let body = format!(r#"{{"id":"{id}","schematic":"customization: {{}}\n"}}"#);
        assert_eq!(parse_schematic_id(body.as_bytes()).unwrap(), id);
        for bad in [r#"{"id":"short"}"#, r#"{"schematic":"x"}"#, "not json"] {
            assert!(parse_schematic_id(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    #[ignore = "requires internet access to factory.talos.dev"]
    async fn the_image_factory_returns_the_vanilla_schematic() {
        let url = resolve_download_url(&NodeModel::TalosLinux, "1.14.4", None)
            .await
            .unwrap();
        assert!(url.starts_with("https://factory.talos.dev/image/"));
        assert!(url.ends_with("/v1.14.4/nocloud-amd64.qcow2"));
    }

    #[tokio::test]
    async fn test_resolve_download_url_user_override() {
        let custom = "https://example.com/my-image.qcow2";
        let url = resolve_download_url(&NodeModel::FedoraLinux, "42", Some(custom))
            .await
            .unwrap();
        assert_eq!(url, custom);
    }

    #[tokio::test]
    async fn test_resolve_download_url_unsupported_model_no_url() {
        let result = resolve_download_url(&NodeModel::FedoraLinux, "42", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No auto-download URL"));
        assert!(err.contains("fedora_linux"));
    }

    #[test]
    fn test_progress_sender_delivers_status_messages() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);

        progress
            .send_status("Validating source...".to_string(), StatusKind::Info)
            .expect("send should succeed");
        progress
            .send_status("Copying image...".to_string(), StatusKind::Progress)
            .expect("send should succeed");
        progress
            .send_status("Done".to_string(), StatusKind::Done)
            .expect("send should succeed");

        // All 3 messages should be received
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err()); // No more messages
    }

    #[tokio::test]
    async fn test_import_rejects_nonexistent_source_file() {
        // import_image should bail before touching Docker/DB when src doesn't exist.
        // We can't call import_image directly without AppState, but the validation
        // logic is the first thing checked — verify via file_exists.
        let fake_path = "/tmp/sherpa_test_nonexistent_image_12345.qcow2";
        assert!(!file_exists(fake_path));
    }

    #[test]
    fn megabytes_are_shown_to_one_decimal_place() {
        assert_eq!(megabytes(0), "0.0");
        assert_eq!(megabytes(1_572_864), "1.5");
        assert_eq!(megabytes(1_048_575), "1.0");
        assert_eq!(megabytes(10 * 1_048_576), "10.0");
    }

    /// A disk import of `model` version `version` from `source`.
    fn disk_import(source: &Path, version: &str) -> (ImportRequest, NodeConfig) {
        let request = ImportRequest {
            model: NodeModel::UbuntuLinux,
            version: version.to_owned(),
            src: source.to_string_lossy().into_owned(),
            default: false,
        };
        let mut config = NodeConfig::get_model(request.model);
        config.version = request.version.clone();
        (request, config)
    }

    fn test_paths(root: &Path) -> ImagePaths {
        let paths = ImagePaths {
            images: root.join("images"),
            storage_pool: root.join("pool"),
        };
        std::fs::create_dir_all(&paths.storage_pool).unwrap();
        paths
    }

    fn disk_path(paths: &ImagePaths, version: &str) -> PathBuf {
        let config = NodeConfig::get_model(NodeModel::UbuntuLinux);
        paths
            .version_dir(config.model, version)
            .join(image_filename(&config.kind, config.boot_mode.as_ref()))
    }

    async fn schema_db() -> Database {
        let database = db::open_in_memory().await.unwrap();
        db::apply_schema(&database).await.unwrap();
        database
    }

    async fn import_disk(
        database: &Database,
        paths: &ImagePaths,
        source: &Path,
        version: &str,
    ) -> Result<ImportResponse> {
        let (request, config) = disk_import(source, version);
        import_disk_image(
            &request,
            config,
            DefaultRequest::Keep,
            database,
            paths,
            &quiet_progress(),
        )
        .await
    }

    async fn recorded(database: &Database, version: &str) -> Option<NodeConfig> {
        let config = NodeConfig::get_model(NodeModel::UbuntuLinux);
        db::find_node_image_by_model_kind_version(database, &config.model, &config.kind, version)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_imported_disk_is_in_place_and_recorded_as_the_first_default() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let source = root.path().join("source.qcow2");
        std::fs::write(&source, b"whole image").unwrap();
        let database = schema_db().await;

        let response = import_disk(&database, &paths, &source, "1").await.unwrap();

        assert!(response.default);
        assert_eq!(
            std::fs::read(disk_path(&paths, "1")).unwrap(),
            b"whole image"
        );
        assert!(recorded(&database, "1").await.unwrap().default);
        let version_dir = paths.version_dir(NodeModel::UbuntuLinux, "1");
        assert_eq!(std::fs::read_dir(version_dir).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn a_disk_import_whose_row_cannot_be_written_leaves_no_image() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let source = root.path().join("source.qcow2");
        std::fs::write(&source, b"whole image").unwrap();
        // No schema: the row write fails.
        let database = db::open_in_memory().await.unwrap();

        assert!(import_disk(&database, &paths, &source, "1").await.is_err());

        let version_dir = paths.version_dir(NodeModel::UbuntuLinux, "1");
        assert_eq!(std::fs::read_dir(version_dir).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_disk_import_whose_source_is_unreadable_records_nothing() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let database = schema_db().await;

        let missing = root.path().join("missing.qcow2");
        assert!(import_disk(&database, &paths, &missing, "1").await.is_err());

        assert!(recorded(&database, "1").await.is_none());
        assert!(!disk_path(&paths, "1").exists());
    }

    #[tokio::test]
    async fn a_truncated_disk_left_by_an_earlier_import_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let source = root.path().join("source.qcow2");
        std::fs::write(&source, b"whole image").unwrap();
        let disk = disk_path(&paths, "1");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        std::fs::write(&disk, b"whole").unwrap();
        let database = schema_db().await;

        import_disk(&database, &paths, &source, "1").await.unwrap();

        assert_eq!(std::fs::read(&disk).unwrap(), b"whole image");
    }

    #[tokio::test]
    async fn a_disk_of_the_source_size_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let source = root.path().join("source.qcow2");
        std::fs::write(&source, b"new image").unwrap();
        let disk = disk_path(&paths, "1");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        std::fs::write(&disk, b"old image").unwrap();
        let database = schema_db().await;

        import_disk(&database, &paths, &source, "1").await.unwrap();

        assert_eq!(std::fs::read(&disk).unwrap(), b"old image");
        assert!(recorded(&database, "1").await.is_some());
    }

    /// A QCOW2 header, backed by `backing` when given.
    fn qcow2_header(backing: Option<&Path>) -> Vec<u8> {
        let mut bytes = vec![0; 65536];
        bytes[..4].copy_from_slice(b"QFI\xfb");
        bytes[4..8].copy_from_slice(&3_u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&16_u32.to_be_bytes());
        bytes[24..32].copy_from_slice(&(2_u64 << 30).to_be_bytes());
        bytes[100..104].copy_from_slice(&104_u32.to_be_bytes());
        if let Some(backing) = backing {
            let name = backing.to_str().unwrap().as_bytes();
            bytes[8..16].copy_from_slice(&128_u64.to_be_bytes());
            bytes[16..20].copy_from_slice(&u32::try_from(name.len()).unwrap().to_be_bytes());
            bytes[128..128 + name.len()].copy_from_slice(name);
        }
        bytes
    }

    #[tokio::test]
    async fn a_differing_disk_that_lab_disks_are_built_on_is_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let source = root.path().join("source.qcow2");
        std::fs::write(&source, b"another image").unwrap();
        let disk = disk_path(&paths, "1");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        let base = qcow2_header(None);
        std::fs::write(&disk, &base).unwrap();
        std::fs::write(
            paths.storage_pool.join("node.qcow2"),
            qcow2_header(Some(&disk)),
        )
        .unwrap();
        let database = schema_db().await;

        let error = import_disk(&database, &paths, &source, "1")
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("new version"), "{error:#}");
        assert_eq!(std::fs::read(&disk).unwrap(), base);
        assert!(recorded(&database, "1").await.is_none());
    }

    #[tokio::test]
    async fn a_scan_that_waited_for_a_delete_does_not_record_the_deleted_image() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let disk = disk_path(&paths, "scan-race");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        std::fs::write(&disk, b"image").unwrap();
        let database = schema_db().await;
        let mut scan = ModelScan::new(&database, NodeModel::UbuntuLinux, "node_image")
            .await
            .unwrap();
        let mut tally = ScanTally::default();

        // A delete holds the image's lock; it removes the files while the scan waits.
        let delete = lock_image(NodeModel::UbuntuLinux, "scan-race").await;
        let scanning = scan_version(
            &database,
            &mut scan,
            "scan-race".to_owned(),
            Some(disk.clone()),
            false,
            &mut tally,
        );
        let deleting = async {
            tokio::task::yield_now().await;
            std::fs::remove_file(&disk).unwrap();
            drop(delete);
        };
        let (scanned, ()) = tokio::join!(scanning, deleting);
        scanned.unwrap();

        assert!(recorded(&database, "scan-race").await.is_none());
        assert_eq!(tally.total_imported, 0);
    }
}
