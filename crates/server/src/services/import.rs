use anyhow::{Context, Result, bail};
use db::{Database, DefaultRequest};
use futures_util::{Stream, StreamExt};
use opentelemetry::KeyValue;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::io::AsyncWriteExt;

use shared::data::{
    DownloadImageRequest, ImageSummary, ImportRequest, ImportResponse, KindSettings,
    ListImagesRequest, ListImagesResponse, ModelKind, NodeConfig, NodeKind, NodeModel,
    ScanImagesRequest, ScanImagesResponse, ScannedImage, SetDefaultImageRequest,
    SetDefaultImageResponse, ShowImageRequest, ShowImageResponse, SkippedModel, StatusKind,
};
use shared::konst::SHERPA_IMAGES_PATH;
use shared::util::{file_exists, image_filename};

use tracing::instrument;

use crate::daemon::state::AppState;
use crate::services::container_scan::{ScanTarget, plan_container_scan, tagged_versions};
use crate::services::error::{ServiceError, Unexpected};
use crate::services::image_lifecycle::{
    BackedVolumes, ensure_image_has_no_overlays, image_model_kind, lock_image,
};
use crate::services::image_store::{ExistingDisk, ImagePaths, PlacedDisk, StagedDisk};
use crate::services::progress::ProgressSender;
use crate::validate::{validate_image_scan, validate_image_version};

/// Talos Image Factory base URL for downloading Talos Linux images.
const TALOS_IMAGE_FACTORY_URL: &str = "https://factory.talos.dev";
/// Image Factory schematic for vanilla Talos Linux (no system extensions). The factory
/// returns its ID when it is posted to `/schematics`.
const TALOS_VANILLA_SCHEMATIC_YAML: &str = "customization: {}\n";

/// Ubuntu cloud image releases, each with its images and their `SHA256SUMS`.
const UBUNTU_RELEASES_URL: &str = "https://cloud-images.ubuntu.com/releases";

/// Bytes between progress reports while downloading an image.
const DOWNLOAD_REPORT_INTERVAL: u64 = 5 * 1024 * 1024;

/// Where a download comes from: its URL, and the SHA-256 the source publishes for the image, if
/// any.
#[derive(Debug)]
struct DownloadSource {
    url: String,
    sha256: Option<String>,
}

/// What a download's source says the image is: its length, when the server declares one, and its
/// SHA-256, when the source publishes one.
#[derive(Debug, Default)]
struct SourceImage {
    len: Option<u64>,
    sha256: Option<String>,
}

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
    let model_kind = image_model_kind(request.model, request.kind)?;
    let _image_guard = lock_image(model_kind, &request.version).await;
    let config = model_kind.node_config();
    let kind = model_kind.kind();

    let _ = progress.send_status(
        format!("Validating source file: {}", request.src),
        StatusKind::Info,
    );

    // Validate source file exists on the server
    if !file_exists(&request.src) {
        return Err(
            ServiceError::NotFound(format!("Source file does not exist: {}", request.src)).into(),
        );
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
                state.qemu.as_ref(),
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

    let kind = db_config.kind();
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
    pool: &impl BackedVolumes,
    progress: &ProgressSender,
) -> Result<ImportResponse> {
    let kind = db_config.kind();
    let filename = image_filename(&db_config.settings)
        .with_context(|| format!("{} is a container image, which has no disk", request.model))?;
    let version_dir = paths.version_dir(request.model, &request.version);
    let version_disk = version_dir.join(filename);

    let source_len = tokio::fs::metadata(&request.src)
        .await
        .with_context(|| format!("Failed to read source file {}", request.src))?
        .len();
    let existing_len = file_len(&version_disk).await?;

    let placed = match ExistingDisk::compare(Some(source_len), existing_len) {
        ExistingDisk::Same => {
            let _ = progress.send_status(
                "Image already exists on disk with the same size, skipping copy".to_string(),
                StatusKind::Info,
            );
            None
        }
        existing => {
            if existing == ExistingDisk::Different {
                ensure_disk_replaceable(pool, &version_dir, &version_disk, progress)?;
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

/// Refuse to replace `version_disk`, which differs from the source, while lab disks are built on
/// it; otherwise say that it is being replaced.
fn ensure_disk_replaceable(
    pool: &impl BackedVolumes,
    version_dir: &Path,
    version_disk: &Path,
    progress: &ProgressSender,
) -> Result<()> {
    ensure_image_has_no_overlays(pool, version_dir).map_err(|error| {
        let context = format!(
            "{} differs from the source and lab disks are built on it; destroy those labs or add \
             the image as a new version",
            version_disk.display()
        );
        match ServiceError::find(&error) {
            Some(ServiceError::Conflict(_)) => {
                ServiceError::Conflict(format!("{context}: {error}")).into()
            }
            _ => error.context(context),
        }
    })?;
    let _ = progress.send_status(
        "Image on disk differs from the source, replacing it".to_string(),
        StatusKind::Info,
    );
    Ok(())
}

/// Read a file's length, or `None` when there is no file.
async fn file_len(path: &Path) -> Result<Option<u64>> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(Some(metadata.len())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
    }
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
            let mut configs = Vec::new();
            for kind in model.kinds() {
                configs.extend(
                    db::get_node_image_versions(&state.db, model, kind)
                        .await
                        .context("Failed to query node images by model")?,
                );
            }
            configs
        }
        (None, None) => db::list_node_images(&state.db)
            .await
            .context("Failed to query all node images")?,
    };

    let images: Vec<ImageSummary> = configs
        .into_iter()
        .map(|c| ImageSummary {
            model: c.model,
            kind: c.kind(),
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
    let kind = image_model_kind(request.model, request.kind)?.kind();

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
        ServiceError::NotFound(format!(
            "No image found for model '{}' (kind: {})",
            request.model, kind
        ))
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
        .map(|(model, repo)| ScanTarget {
            model,
            repo: repo.to_string(),
        });
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

/// One model and kind during a scan: its config, and whether it has a default image yet.
struct ModelScan {
    model_kind: ModelKind,
    config: NodeConfig,
    /// Whether the model has a default, or a version this scan imported or would import became it.
    has_default: bool,
    /// What the import log calls the record.
    label: &'static str,
}

impl ModelScan {
    /// Start scanning `model_kind`, from its current default.
    async fn new(database: &Database, model_kind: ModelKind, label: &'static str) -> Result<Self> {
        let default =
            db::find_default_node_image(database, &model_kind.model(), &model_kind.kind())
                .await
                .context("Failed to query the model's default image")?;
        Ok(Self {
            model_kind,
            config: model_kind.node_config(),
            has_default: default.is_some(),
            label,
        })
    }

    /// The scan's row for `version` of this model and kind. `failed` when it was not recorded.
    fn scanned(&self, version: String, status: String, failed: bool) -> ScannedImage {
        ScannedImage {
            model: self.model_kind.model(),
            version,
            kind: self.model_kind.kind(),
            status,
            failed,
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

        // A model's folder holds the disks of its virtual machine and unikernel kinds, each
        // kind under its own file name in a version folder. Containers live in Docker.
        let model_dir = format!("{images_dir}/{dir_name}");
        for &kind in model.kinds() {
            if kind == NodeKind::Container || request.kind.is_some_and(|filter| filter != kind) {
                continue;
            }
            let model_kind = ModelKind::new(model, Some(kind))?;
            let mut scan = ModelScan::new(&state.db, model_kind, "node_image").await?;
            scan_version_dirs(state, &mut scan, &model_dir, request.dry_run, tally).await?;
        }
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
        let filename = image_filename(&scan.config.settings).with_context(|| {
            format!(
                "{} is a container image, which has no disk",
                scan.model_kind
            )
        })?;
        let disk_path = format!("{model_dir}/{version}/{filename}");

        // Check if image file exists in this version directory
        if !file_exists(&disk_path) {
            tracing::debug!(
                "No {} found for model={} version={}",
                filename,
                scan.model_kind.model(),
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

/// Record the local Docker tags of each repo [`plan_container_scan`] gives, as versions of its
/// model, each with the repo it was asked for. Docker matches the repo to its images.
#[instrument(skip_all, level = "debug")]
async fn scan_container_images(
    target: Option<ScanTarget>,
    dry_run: bool,
    state: &AppState,
    tally: &mut ScanTally,
) -> Result<()> {
    let plan = plan_container_scan(target);
    for model in &plan.skipped {
        tracing::debug!(model = %model.model, reason = %model.reason, "Skipping container model");
    }
    tally.skipped = plan.skipped;

    for target in plan.targets {
        let repo_tags = container::get_local_images(&state.docker, Some(&target.repo))
            .await
            .with_context(|| format!("Failed to list local Docker images of {}", target.repo))?;
        let versions = tagged_versions(&repo_tags);
        if versions.is_empty() {
            continue;
        }
        let model_kind = ModelKind::new(target.model, Some(NodeKind::Container))?;
        let mut scan = ModelScan::new(&state.db, model_kind, "container node_image").await?;
        let KindSettings::Container(container) = &mut scan.config.settings else {
            bail!("{model_kind} has no container image settings");
        };
        container.repo = Some(target.repo);
        for version in versions {
            scan_version(
                &state.db,
                &mut scan,
                version.to_string(),
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
    let _image_guard = lock_image(scan.model_kind, &version).await;
    if let Some(disk) = &disk
        && !tokio::fs::try_exists(disk).await.unwrap_or(false)
    {
        tracing::debug!(model = %scan.model_kind.model(), %version, "Image removed while the scan waited");
        return Ok(());
    }
    let existing = db::find_node_image_by_model_kind_version(
        database,
        &scan.model_kind.model(),
        &scan.model_kind.kind(),
        &version,
    )
    .await
    .context("Failed to check existing node_image")?;
    if existing.is_some() {
        tally
            .scanned
            .push(scan.scanned(version, "already_exists".to_string(), false));
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
            .push(scan.scanned(version, status.to_string(), false));
        tally.total_imported += 1;
        return Ok(());
    }

    let mut db_config = scan.config.clone();
    db_config.version.clone_from(&version);
    db_config.id = None;

    let (status, failed) =
        match db::upsert_node_image(database, db_config, DefaultRequest::Keep).await {
            Ok(recorded) => {
                tracing::info!(
                    "Scanned and imported {} for model={} version={} default={}",
                    scan.label,
                    scan.model_kind.model(),
                    version,
                    recorded.default
                );
                scan.has_default = true;
                tally.total_imported += 1;
                let status = if recorded.default {
                    "imported (default)"
                } else {
                    "imported"
                };
                (status.to_string(), false)
            }
            Err(e) => {
                tracing::error!(
                    "Failed to upsert node_image for model={} version={}: {:?}",
                    scan.model_kind.model(),
                    version,
                    e
                );
                (
                    format!(
                        "error: {}",
                        Unexpected::log(&e, "Failed to record a scanned image version")
                    ),
                    true,
                )
            }
        };

    tally.scanned.push(scan.scanned(version, status, failed));
    Ok(())
}

/// Set the default version for a node image
#[instrument(skip(state), fields(model = %request.model, version = %request.version), level = "debug")]
pub async fn set_default_image(
    request: SetDefaultImageRequest,
    state: &AppState,
) -> Result<SetDefaultImageResponse> {
    let model_kind = image_model_kind(request.model, request.kind)?;
    let _image_guard = lock_image(model_kind, &request.version).await;
    let kind = model_kind.kind();

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
        return Err(ServiceError::NotFound(format!(
            "Image not found for model '{}' with version '{}'. Use 'server image list' to see available images.",
            request.model, request.version
        ))
        .into());
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
            "{UBUNTU_RELEASES_URL}/{version}/release/{}",
            ubuntu_image_name(version)
        )),
        NodeModel::TalosLinux => {
            let schematic_id = talos_vanilla_schematic_id().await?;
            Ok(talos_image_url(&schematic_id, version))
        }
        _ => Err(ServiceError::Invalid(format!(
            "No auto-download URL for model '{model}'. Provide --url."
        ))
        .into()),
    }
}

/// The file name of an Ubuntu release's cloud image.
fn ubuntu_image_name(version: &str) -> String {
    format!("ubuntu-{version}-server-cloudimg-amd64.img")
}

/// The SHA-256 the source of a download publishes for its image: an Ubuntu release's
/// `SHA256SUMS` entry for a download without `--url`. Other sources publish none.
async fn published_sha256(
    model: &NodeModel,
    version: &str,
    url: Option<&str>,
) -> Result<Option<String>> {
    if url.is_some() || *model != NodeModel::UbuntuLinux {
        return Ok(None);
    }
    let sums_url = format!("{UBUNTU_RELEASES_URL}/{version}/release/SHA256SUMS");
    fetch_sha256(&sums_url, &ubuntu_image_name(version))
        .await
        .map(Some)
}

/// Fetch a `SHA256SUMS` file and return the SHA-256 it lists for `file_name`.
#[instrument(level = "debug")]
async fn fetch_sha256(sums_url: &str, file_name: &str) -> Result<String> {
    let response = reqwest::get(sums_url)
        .await
        .with_context(|| format!("Failed to fetch the image's checksums from {sums_url}"))?;
    if !response.status().is_success() {
        bail!(
            "Failed to fetch the image's checksums: HTTP {} from {sums_url}",
            response.status()
        );
    }
    let body = response
        .text()
        .await
        .with_context(|| format!("Failed to read the image's checksums from {sums_url}"))?;
    parse_sha256sums(&body, file_name)
        .with_context(|| format!("{sums_url} lists no SHA-256 for {file_name}"))
}

/// Find the SHA-256 of `file_name` in a `SHA256SUMS` body: lines of a hex digest, whitespace, and
/// the file name, marked `*` when it was hashed in binary mode.
fn parse_sha256sums(body: &str, file_name: &str) -> Result<String> {
    let digest = body
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim_start().trim_start_matches('*') == file_name)
        .map(|(digest, _)| digest)
        .context("no entry for the file")?;
    anyhow::ensure!(
        digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()),
        "the entry is not a SHA-256: {digest}"
    );
    Ok(digest.to_ascii_lowercase())
}

/// The SHA-256 of a file, as lowercase hex. It blocks, so async callers run it with
/// `spawn_blocking`.
fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    Ok(hex::encode(hasher.finalize()))
}

/// Compare the file at `version_disk`, if any, with what the source says the image is: by size,
/// as an import does, then by SHA-256 when the source publishes one.
async fn compare_existing_download(
    version_disk: &Path,
    source: &SourceImage,
) -> Result<ExistingDisk> {
    let existing = ExistingDisk::compare(source.len, file_len(version_disk).await?);
    let Some(expected) = source.sha256.clone() else {
        return Ok(existing);
    };
    if existing != ExistingDisk::Same {
        return Ok(existing);
    }
    let path = version_disk.to_owned();
    let actual = tokio::task::spawn_blocking(move || sha256_file(&path))
        .await
        .context("The image checksum task failed")??;
    Ok(if actual == expected {
        ExistingDisk::Same
    } else {
        ExistingDisk::Different
    })
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
/// The rename happens only when the whole body arrived and matches what the source says the image
/// is: its declared length and its published SHA-256, when it gives them. A download that is
/// interrupted or does not match leaves nothing at `dest` for a later run to take as a complete
/// image.
async fn write_download<S, B, E>(
    mut body: S,
    dest: &Path,
    source: &SourceImage,
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
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_reported: u64 = 0;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.context("Failed to read response chunk")?;
        let chunk = chunk.as_ref();
        file.write_all(chunk)
            .await
            .context("Failed to write chunk to disk")?;
        hasher.update(chunk);
        downloaded += chunk.len() as u64;
        if downloaded - last_reported >= DOWNLOAD_REPORT_INTERVAL {
            last_reported = downloaded;
            let _ = progress.send_status(
                download_progress(downloaded, source.len),
                StatusKind::Progress,
            );
        }
    }
    file.sync_all()
        .await
        .context("Failed to flush image file")?;
    drop(file);
    if let Some(expected) = source.len
        && downloaded != expected
    {
        bail!("Download ended after {downloaded} of {expected} bytes");
    }
    if let Some(expected) = &source.sha256 {
        let actual = hex::encode(hasher.finalize());
        if actual != *expected {
            bail!("The downloaded image's SHA-256 is {actual}, not the published {expected}");
        }
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
    let model_kind = image_model_kind(request.model, request.kind)?;
    let _image_guard = lock_image(model_kind, &request.version).await;
    let kind = model_kind.kind();

    if kind != NodeKind::VirtualMachine {
        return Err(ServiceError::Invalid(format!(
            "Image download is only supported for virtual machine models, got '{kind}'"
        ))
        .into());
    }

    let _ = progress.send_status("Resolving download URL...".to_string(), StatusKind::Info);

    let url = request.url.as_deref();
    let source = DownloadSource {
        url: resolve_download_url(&request.model, &request.version, url).await?,
        sha256: published_sha256(&request.model, &request.version, url).await?,
    };
    let mut db_config = model_kind.node_config();
    db_config.version.clone_from(&request.version);
    db_config.id = None;

    download_disk_image(
        &request,
        &source,
        db_config,
        &state.db,
        &ImagePaths::installed(),
        state.qemu.as_ref(),
        &progress,
    )
    .await
}

/// Download a VM image from `download.url` into the images directory, then record `db_config`.
///
/// A file already at the destination is kept only when it has the length the server declares
/// for the download and, when the source publishes one, its SHA-256. Otherwise it is downloaded
/// again and replaced, unless lab overlays are built on it.
#[instrument(skip_all, fields(model = %request.model, version = %request.version), level = "debug")]
async fn download_disk_image(
    request: &DownloadImageRequest,
    download: &DownloadSource,
    db_config: NodeConfig,
    database: &Database,
    paths: &ImagePaths,
    pool: &impl BackedVolumes,
    progress: &ProgressSender,
) -> Result<ImportResponse> {
    let kind = db_config.kind();
    let filename = image_filename(&db_config.settings)
        .with_context(|| format!("{} is a container image, which has no disk", request.model))?;
    let version_dir = paths.version_dir(request.model, &request.version);
    let version_disk = version_dir.join(filename);
    tokio::fs::create_dir_all(&version_dir)
        .await
        .with_context(|| format!("Failed to create {}", version_dir.display()))?;

    let download_url = &download.url;
    let response = reqwest::get(download_url)
        .await
        .context(format!("Failed to download image from {download_url}"))?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download image: HTTP {} from {}",
            response.status(),
            download_url
        );
    }
    let source = SourceImage {
        len: response.content_length(),
        sha256: download.sha256.clone(),
    };

    let placed = match compare_existing_download(&version_disk, &source).await? {
        ExistingDisk::Same => {
            tracing::info!(path = %version_disk.display(), "Image already exists, skipping download");
            let _ = progress.send_status(
                "Image already exists on disk and matches the source, skipping download"
                    .to_string(),
                StatusKind::Info,
            );
            None
        }
        existing => {
            if existing == ExistingDisk::Different {
                ensure_disk_replaceable(pool, &version_dir, &version_disk, progress)?;
            }
            let _ = progress.send_status(
                format!(
                    "Downloading {} {} from {}...",
                    request.model, request.version, download_url
                ),
                StatusKind::Progress,
            );
            let (placed, downloaded) =
                write_download(response.bytes_stream(), &version_disk, &source, progress).await?;
            tracing::info!(path = %placed.path().display(), "Download complete");
            let _ = progress.send_status(
                format!("Download complete: {} MB", megabytes(downloaded)),
                StatusKind::Done,
            );
            Some(placed)
        }
    };

    let _ = progress.send_status("Updating database...".to_string(), StatusKind::Progress);
    let request_default = if request.default {
        DefaultRequest::Make
    } else {
        DefaultRequest::Keep
    };
    let recorded = match db::upsert_node_image(database, db_config, request_default).await {
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
        version: request.version.clone(),
        image_path: version_disk.to_string_lossy().into_owned(),
        db_tracked: true,
        default: recorded.default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::ws::Message;
    use axum::routing::get;
    use futures_util::stream;
    use libvirt::BackedVolume;
    use std::io;
    use tokio::sync::mpsc;

    fn quiet_progress() -> ProgressSender {
        let (tx, _rx) = mpsc::unbounded_channel();
        ProgressSender::new(tx)
    }

    fn declared(len: u64) -> SourceImage {
        SourceImage {
            len: Some(len),
            sha256: None,
        }
    }

    fn chunks(parts: Vec<io::Result<Vec<u8>>>) -> impl Stream<Item = io::Result<Vec<u8>>> + Unpin {
        stream::iter(parts)
    }

    #[tokio::test]
    async fn image_download_001_a_complete_download_is_at_the_images_path() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("disk.qcow2");
        let body = chunks(vec![Ok(b"abc".to_vec()), Ok(b"def".to_vec())]);

        let (placed, size) = write_download(body, &dest, &declared(6), &quiet_progress())
            .await
            .unwrap();

        assert_eq!(size, 6);
        assert_eq!(placed.path(), dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"abcdef");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn image_download_002_a_download_that_breaks_off_leaves_nothing_at_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("disk.qcow2");
        let body = chunks(vec![
            Ok(b"abc".to_vec()),
            Err(io::Error::other("connection reset")),
        ]);

        assert!(
            write_download(body, &dest, &SourceImage::default(), &quiet_progress())
                .await
                .is_err()
        );

        assert!(!dest.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn image_download_003_a_body_shorter_than_its_declared_length_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("disk.qcow2");
        let body = chunks(vec![Ok(b"abc".to_vec())]);

        let error = write_download(body, &dest, &declared(10), &quiet_progress())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("3 of 10 bytes"), "{error}");
        assert!(!dest.exists());
    }

    #[test]
    fn image_download_008_a_talos_version_is_downloaded_from_the_vanilla_schematic() {
        let id = "a".repeat(64);
        for version in ["1.14.4", "v1.14.4"] {
            assert_eq!(
                talos_image_url(&id, version),
                format!("https://factory.talos.dev/image/{id}/v1.14.4/nocloud-amd64.qcow2")
            );
        }
    }

    #[test]
    fn image_download_009_a_factory_answer_without_a_schematic_id_is_refused() {
        let id = "0123456789abcdef".repeat(4);
        let body = format!(r#"{{"id":"{id}","schematic":"customization: {{}}\n"}}"#);
        assert_eq!(parse_schematic_id(body.as_bytes()).unwrap(), id);
        for bad in [r#"{"id":"short"}"#, r#"{"schematic":"x"}"#, "not json"] {
            assert!(parse_schematic_id(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    #[ignore = "requires internet access to factory.talos.dev"]
    async fn image_download_010_the_image_factory_answers_with_the_vanilla_schematic_id() {
        let url = resolve_download_url(&NodeModel::TalosLinux, "1.14.4", None)
            .await
            .unwrap();
        assert!(url.starts_with("https://factory.talos.dev/image/"));
        assert!(url.ends_with("/v1.14.4/nocloud-amd64.qcow2"));
    }

    #[tokio::test]
    async fn image_download_006_the_url_given_is_the_url_downloaded() {
        let custom = "https://example.com/my-image.qcow2";
        let url = resolve_download_url(&NodeModel::FedoraLinux, "42", Some(custom))
            .await
            .unwrap();
        assert_eq!(url, custom);
    }

    #[tokio::test]
    async fn image_download_007_without_a_url_a_model_other_than_ubuntu_or_talos_is_refused() {
        let result = resolve_download_url(&NodeModel::FedoraLinux, "42", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No auto-download URL"));
        assert!(err.contains("fedora_linux"));
    }

    #[test]
    fn a_status_message_reaches_the_client_as_a_status_frame() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);

        progress
            .send_status("Copying image...".to_string(), StatusKind::Progress)
            .unwrap();

        let Message::Text(text) = rx.try_recv().unwrap() else {
            panic!("the status was not sent as text");
        };
        let frame: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
        assert_eq!(frame["type"], "status", "{frame}");
        assert_eq!(frame["message"], "Copying image...", "{frame}");
        assert_eq!(frame["kind"], "progress", "{frame}");
        assert!(frame.get("phase").is_none(), "{frame}");
        assert!(rx.try_recv().is_err());
    }

    /// A disk import of `model` version `version` from `source`.
    fn disk_import(source: &Path, version: &str) -> (ImportRequest, NodeConfig) {
        let request = ImportRequest {
            model: NodeModel::UbuntuLinux,
            kind: None,
            version: version.to_owned(),
            src: source.to_string_lossy().into_owned(),
            default: false,
        };
        let mut config = ModelKind::default_for(request.model).node_config();
        config.version = request.version.clone();
        (request, config)
    }

    fn test_paths(root: &Path) -> ImagePaths {
        ImagePaths {
            images: root.join("images"),
        }
    }

    /// A storage pool of these volumes, for tests.
    struct TestPool(Vec<BackedVolume>);

    impl BackedVolumes for TestPool {
        fn backed_volumes(&self) -> Result<Vec<BackedVolume>> {
            Ok(self.0.clone())
        }
    }

    fn disk_path(paths: &ImagePaths, version: &str) -> PathBuf {
        let config = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        paths
            .version_dir(config.model, version)
            .join(image_filename(&config.settings).unwrap())
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
        import_disk_on(database, paths, &TestPool(Vec::new()), source, version).await
    }

    async fn import_disk_on(
        database: &Database,
        paths: &ImagePaths,
        pool: &TestPool,
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
            pool,
            &quiet_progress(),
        )
        .await
    }

    async fn recorded(database: &Database, version: &str) -> Option<NodeConfig> {
        let config = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        db::find_node_image_by_model_kind_version(database, &config.model, &config.kind(), version)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn image_import_001_an_imported_disk_is_in_the_images_directory_and_recorded() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let source = root.path().join("source.qcow2");
        std::fs::write(&source, b"whole image").unwrap();
        let database = schema_db().await;

        import_disk(&database, &paths, &source, "1").await.unwrap();

        assert_eq!(
            std::fs::read(disk_path(&paths, "1")).unwrap(),
            b"whole image"
        );
        assert!(recorded(&database, "1").await.is_some());
        let version_dir = paths.version_dir(NodeModel::UbuntuLinux, "1");
        assert_eq!(std::fs::read_dir(version_dir).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn image_import_004_an_import_whose_record_cannot_be_written_removes_the_file_it_added() {
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
    async fn image_import_003_a_source_that_cannot_be_read_leaves_no_file_no_partial_file_and_no_record()
     {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let database = schema_db().await;

        let missing = root.path().join("missing.qcow2");
        assert!(import_disk(&database, &paths, &missing, "1").await.is_err());

        assert!(recorded(&database, "1").await.is_none());
        let version_dir = paths.version_dir(NodeModel::UbuntuLinux, "1");
        let files = std::fs::read_dir(version_dir).map_or(0, Iterator::count);
        assert_eq!(files, 0, "the import left a file or a partial file");
    }

    #[tokio::test]
    async fn image_import_009_a_file_of_another_size_is_replaced() {
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
    async fn image_import_008_a_file_of_the_sources_size_skips_the_copy() {
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

    #[tokio::test]
    async fn image_import_010_a_file_of_another_size_with_lab_disks_built_on_it_is_refused_and_kept()
     {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let source = root.path().join("source.qcow2");
        std::fs::write(&source, b"another image").unwrap();
        let disk = disk_path(&paths, "1");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        let base = b"base".to_vec();
        std::fs::write(&disk, &base).unwrap();
        let pool = TestPool(vec![BackedVolume {
            name: "node-lab-hdd.qcow2".to_owned(),
            backing_file: disk.clone(),
        }]);
        let database = schema_db().await;

        let error = import_disk_on(&database, &paths, &pool, &source, "1")
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("new version"), "{error:#}");
        assert_eq!(std::fs::read(&disk).unwrap(), base);
        assert!(recorded(&database, "1").await.is_none());
    }

    /// Serve `body` at `/image` on a local port, declaring its length when `declare_len` is set,
    /// and return the image's URL.
    async fn serve_image(body: &'static [u8], declare_len: bool) -> String {
        let router = Router::new().route(
            "/image",
            get(move || async move {
                if declare_len {
                    Body::from(body)
                } else {
                    Body::from_stream(stream::iter([Ok::<_, io::Error>(body)]))
                }
            }),
        );
        format!("{}/image", serve(router).await)
    }

    /// Serve `router` on a local port and return its base URL.
    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await });
        format!("http://{address}")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    /// A download of the default Ubuntu VM at `version` from `url`, checked against `sha256`.
    async fn download_disk_on(
        database: &Database,
        paths: &ImagePaths,
        pool: &TestPool,
        url: String,
        sha256: Option<String>,
        version: &str,
    ) -> Result<ImportResponse> {
        let request = DownloadImageRequest {
            model: NodeModel::UbuntuLinux,
            kind: None,
            version: version.to_owned(),
            url: Some(url.clone()),
            default: false,
        };
        let mut config = ModelKind::default_for(request.model).node_config();
        config.version = request.version.clone();
        download_disk_image(
            &request,
            &DownloadSource { url, sha256 },
            config,
            database,
            paths,
            pool,
            &quiet_progress(),
        )
        .await
    }

    async fn download_disk(
        database: &Database,
        paths: &ImagePaths,
        url: String,
        sha256: Option<String>,
    ) -> Result<ImportResponse> {
        download_disk_on(database, paths, &TestPool(Vec::new()), url, sha256, "1").await
    }

    /// Put `contents` at version `1`'s image path, as a file an earlier run left.
    fn existing_disk(paths: &ImagePaths, contents: &[u8]) -> PathBuf {
        let disk = disk_path(paths, "1");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        std::fs::write(&disk, contents).unwrap();
        disk
    }

    #[tokio::test]
    async fn image_download_005_a_file_matching_the_sources_length_and_checksum_is_recorded_without_a_download()
     {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let disk = existing_disk(&paths, b"old image");
        let url = serve_image(b"new image", true).await;
        let database = schema_db().await;

        download_disk(&database, &paths, url, Some(sha256_hex(b"old image")))
            .await
            .unwrap();

        assert_eq!(std::fs::read(&disk).unwrap(), b"old image");
        assert!(recorded(&database, "1").await.is_some());
    }

    #[tokio::test]
    async fn image_download_012_a_file_shorter_than_the_sources_length_is_downloaded_again() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let disk = existing_disk(&paths, b"whole");
        let url = serve_image(b"whole image", true).await;
        let database = schema_db().await;

        download_disk(&database, &paths, url, None).await.unwrap();

        assert_eq!(std::fs::read(&disk).unwrap(), b"whole image");
        assert!(recorded(&database, "1").await.is_some());
    }

    #[tokio::test]
    async fn image_download_013_a_file_of_the_sources_length_with_another_checksum_is_downloaded_again()
     {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let disk = existing_disk(&paths, b"old image");
        let url = serve_image(b"new image", true).await;
        let database = schema_db().await;

        download_disk(&database, &paths, url, Some(sha256_hex(b"new image")))
            .await
            .unwrap();

        assert_eq!(std::fs::read(&disk).unwrap(), b"new image");
    }

    #[tokio::test]
    async fn image_download_014_a_file_is_downloaded_again_when_the_source_declares_no_length() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let disk = existing_disk(&paths, b"old image");
        let url = serve_image(b"new image", false).await;
        let database = schema_db().await;

        download_disk(&database, &paths, url, None).await.unwrap();

        assert_eq!(std::fs::read(&disk).unwrap(), b"new image");
    }

    #[tokio::test]
    async fn image_download_015_a_download_whose_checksum_is_not_the_published_one_is_refused_and_leaves_nothing()
     {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let url = serve_image(b"tampered image", true).await;
        let database = schema_db().await;

        let error = download_disk(&database, &paths, url, Some(sha256_hex(b"real image")))
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("SHA-256"), "{error:#}");
        let version_dir = paths.version_dir(NodeModel::UbuntuLinux, "1");
        assert_eq!(std::fs::read_dir(version_dir).unwrap().count(), 0);
        assert!(recorded(&database, "1").await.is_none());
    }

    #[tokio::test]
    async fn image_download_016_checksums_that_cannot_be_fetched_or_do_not_list_the_image_are_refused()
     {
        let digest = "AB".repeat(32);
        let sums = format!("{digest} *other.img\n");
        let router = Router::new().route("/SHA256SUMS", get(move || async move { sums }));
        let base = serve(router).await;

        assert!(
            fetch_sha256(&format!("{base}/missing"), "image.img")
                .await
                .is_err()
        );
        let unlisted = fetch_sha256(&format!("{base}/SHA256SUMS"), "image.img").await;
        assert!(unlisted.is_err(), "{unlisted:?}");
        assert_eq!(
            fetch_sha256(&format!("{base}/SHA256SUMS"), "other.img")
                .await
                .unwrap(),
            digest.to_ascii_lowercase()
        );
    }

    #[tokio::test]
    async fn image_download_017_a_file_to_replace_with_lab_disks_built_on_it_is_refused_and_kept() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let disk = existing_disk(&paths, b"base");
        let url = serve_image(b"another image", true).await;
        let pool = TestPool(vec![BackedVolume {
            name: "node-lab-hdd.qcow2".to_owned(),
            backing_file: disk.clone(),
        }]);
        let database = schema_db().await;

        let error = download_disk_on(&database, &paths, &pool, url, None, "1")
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("new version"), "{error:#}");
        assert_eq!(std::fs::read(&disk).unwrap(), b"base");
        assert!(recorded(&database, "1").await.is_none());
    }

    #[tokio::test]
    async fn image_scan_016_a_scan_that_waited_for_a_delete_does_not_record_the_deleted_image() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let disk = disk_path(&paths, "scan-race");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        std::fs::write(&disk, b"image").unwrap();
        let database = schema_db().await;
        let mut scan = ModelScan::new(
            &database,
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "node_image",
        )
        .await
        .unwrap();
        let mut tally = ScanTally::default();

        // A delete holds the image's lock; it removes the files while the scan waits.
        let delete = lock_image(ModelKind::default_for(NodeModel::UbuntuLinux), "scan-race").await;
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
