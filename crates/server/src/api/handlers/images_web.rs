//! The web UI's admin image pages and upload.

use axum::Form;
use axum::extract::{Multipart, Path, State};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde::de::value::{Error as DeError, StrDeserializer};
use serde::de::{DeserializeOwned, IntoDeserializer};
use std::fmt::Display;
use std::iter;
use std::str::FromStr;
use strum::IntoEnumIterator;

use db::{DefaultRequest, LastDefaultImage};

use crate::daemon::state::AppState;
use crate::services::image_lifecycle::lock_image;
use crate::services::import;
use crate::services::progress::ProgressSender;
use crate::templates::{
    AdminImageEditTemplate, AdminImageUploadTemplate, AdminNotificationErrorTemplate, FormChoice,
    KindBadge, NavPage, kind_label,
};

use crate::api::errors::ApiError;
use crate::api::extractors::AdminUser;

use crate::validate::validate_node_image_update;
use shared::data::{
    BiosTypes, ComputeSettings, CpuArchitecture, CpuModels, DiskBuses, DiskSettings, ImportRequest,
    InterfaceSettings, InterfaceType, MachineType, NodeConfig, NodeModel, OsVariant, Ztp,
    ZtpSettings,
};

/// Helper struct for extracting image path parameters with optional version
#[derive(Debug, Deserialize)]
pub struct ImagePath {
    pub model: String,
    pub version: Option<String>,
}

/// Helper struct to summarize image data for list view
#[derive(Debug, Clone)]
pub struct ImageSummary {
    pub model: String,
    pub kind_badge: KindBadge,
    pub version: String,
    pub cpu_count: u8,
    pub memory: u16,
    pub data_interface_count: u8,
    pub default: bool,
}

/// Admin handler to list all image configurations
pub async fn admin_images_list_handler(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    tracing::debug!("Admin requesting images list");

    // Fetch all node images from database
    let configs = db::list_node_images(&state.db).await.map_err(|e| {
        tracing::error!("Failed to list images: {:?}", e);
        ApiError::internal("Failed to load image configurations")
    })?;

    // Group configs by model and pick the best representative:
    // - Prefer default=true, otherwise pick the first entry
    let mut best_by_model: std::collections::HashMap<String, ImageSummary> =
        std::collections::HashMap::new();

    for config in configs {
        let model_key = config.model.to_string();
        let summary = ImageSummary {
            model: model_key.clone(),
            kind_badge: KindBadge::short(config.kind),
            version: config.version,
            cpu_count: config.compute.cpu_count,
            memory: config.compute.memory,
            data_interface_count: config.interfaces.data_count,
            default: config.default,
        };

        let dominated = match best_by_model.get(&model_key) {
            None => true,
            Some(existing) => summary.default && !existing.default,
        };

        if dominated {
            best_by_model.insert(model_key, summary);
        }
    }

    let mut summaries: Vec<ImageSummary> = best_by_model.into_values().collect();
    summaries.sort_by(|a, b| a.model.cmp(&b.model));

    let template = crate::templates::AdminImagesListTemplate {
        username: admin.username.clone(),
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        configs: summaries,
    };

    Ok(template)
}

/// The image a page or update names: the URL's version of the model, or its default version.
async fn find_image_config(state: &AppState, params: &ImagePath) -> Result<NodeConfig, ApiError> {
    let node_model = NodeModel::from_str(&params.model).map_err(|e| {
        tracing::warn!("Invalid node model in URL: {} - {}", params.model, e);
        ApiError::not_found("Image", format!("Invalid model: {}", params.model))
    })?;

    // Derive kind from model
    let node_kind = node_model.kind();

    // Fetch the specific config from database
    // If version is specified, fetch that specific version; otherwise fetch default
    let config = if let Some(version) = &params.version {
        db::find_node_image_by_model_kind_version(&state.db, &node_model, &node_kind, version)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to get image for {}/{}: {:?}",
                    params.model,
                    version,
                    e
                );
                ApiError::not_found(
                    "Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Node config not found for {}/{}", params.model, version);
                ApiError::not_found(
                    "Image",
                    format!(
                        "Configuration for {} version {} not found",
                        params.model, version
                    ),
                )
            })?
    } else {
        db::find_default_node_image(&state.db, &node_model, &node_kind)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get default image for {}: {:?}", params.model, e);
                ApiError::not_found(
                    "Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
            .ok_or_else(|| {
                tracing::warn!("Default image not found for {}", params.model);
                ApiError::not_found(
                    "Image",
                    format!("Default configuration for {} not found", params.model),
                )
            })?
    };
    Ok(config)
}

/// The image form's choices, parsed from their `snake_case` names.
struct ImageFormChoices {
    os_variant: OsVariant,
    bios: BiosTypes,
    cpu_architecture: CpuArchitecture,
    cpu_model: CpuModels,
    machine_type: MachineType,
    hdd_bus: DiskBuses,
    cdrom_bus: DiskBuses,
    interface_type: InterfaceType,
}

impl ImageFormChoices {
    fn parse(form: &ImageForm) -> Result<Self, ApiError> {
        Ok(Self {
            os_variant: parse_form_choice(&form.os_variant, "OS variant")?,
            bios: parse_form_choice(&form.bios, "BIOS type")?,
            cpu_architecture: parse_form_choice(&form.cpu_architecture, "CPU architecture")?,
            cpu_model: parse_form_choice(&form.cpu_model, "CPU model")?,
            machine_type: parse_form_choice(&form.machine_type, "machine type")?,
            hdd_bus: parse_form_choice(&form.hdd_bus, "HDD bus")?,
            cdrom_bus: parse_form_choice(&form.cdrom_bus, "CDROM bus")?,
            interface_type: parse_form_choice(&form.interface_type, "interface type")?,
        })
    }
}

/// The ZTP settings the edit form offers for an image of `model`: off, then the methods the
/// model takes, with the image's `current` setting selected.
fn ztp_choices(model: NodeModel, current: Ztp) -> Vec<FormChoice> {
    iter::once(Ztp::Disabled)
        .chain(model.ztp_methods().iter().copied())
        .map(|ztp| FormChoice {
            value: ztp.to_string(),
            label: ztp.to_string(),
            selected: ztp == current,
        })
        .collect()
}

/// The edit form's options for a field of type `T`: every variant, with the image's `current`
/// value selected.
fn form_choices<T: IntoEnumIterator + Display + PartialEq>(current: &T) -> Vec<FormChoice> {
    T::iter()
        .map(|variant| FormChoice {
            value: variant.to_string(),
            label: variant.to_string(),
            selected: variant == *current,
        })
        .collect()
}

/// One of the form's choices, by the enum's serde name.
fn parse_form_choice<T: DeserializeOwned>(value: &str, field_name: &str) -> Result<T, ApiError> {
    let deserializer: StrDeserializer<'_, DeError> = value.into_deserializer();
    T::deserialize(deserializer)
        .map_err(|_| ApiError::bad_request(format!("Invalid {field_name}: {value}")))
}

/// The image's new configuration from the form. Its id, model, kind, version, management
/// interface and boot mode are kept; checkboxes are on when present, and blank optional fields
/// are unset. The version names the image's files, so a form that sends another is ignored.
fn updated_image_config(
    config: NodeConfig,
    form: ImageForm,
    choices: ImageFormChoices,
) -> NodeConfig {
    NodeConfig {
        id: config.id,
        model: config.model,     // Keep original (read-only)
        kind: config.kind,       // Keep original (read-only)
        version: config.version, // Keep original (names the image's files)
        repo: form.repo.filter(|s| !s.trim().is_empty()),
        os_variant: choices.os_variant,
        compute: ComputeSettings {
            bios: choices.bios,
            cpu_count: form.cpu_count,
            cpu_architecture: choices.cpu_architecture,
            cpu_model: choices.cpu_model,
            machine_type: choices.machine_type,
            vmx_enabled: form.vmx_enabled.is_some(),
            memory: form.memory,
            boot_mode: config.compute.boot_mode, // Keep original
        },
        disks: DiskSettings {
            hdd_bus: choices.hdd_bus,
            cdrom: form.cdrom.filter(|s| !s.trim().is_empty()),
            cdrom_bus: choices.cdrom_bus,
        },
        ztp: ZtpSettings {
            mode: form.ztp,
            username: form.ztp_username.filter(|s| !s.trim().is_empty()),
            password: form.ztp_password.filter(|s| !s.trim().is_empty()),
            password_auth: form.ztp_password_auth.is_some(),
        },
        interfaces: InterfaceSettings {
            data_count: form.data_interface_count,
            prefix: form.interface_prefix,
            kind: choices.interface_type,
            mtu: form.interface_mtu,
            first_index: form.first_interface_index,
            management: config.interfaces.management, // Keep original (read-only)
            reserved_count: form.reserved_interface_count,
        },
        default: config.default, // Decided by the default rule in `db`
    }
}

/// Admin handler to view a single image configuration detail
pub async fn admin_image_detail_handler(
    State(state): State<AppState>,
    Path(params): Path<ImagePath>,
    admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    tracing::debug!(
        "Admin requesting image detail: {} (version: {:?})",
        params.model,
        params.version
    );

    let config = find_image_config(&state, &params).await?;

    let template = crate::templates::AdminImageDetailTemplate {
        username: admin.username.clone(),
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        kind_badge: KindBadge::full(config.kind.clone()),
        config,
    };

    Ok(template)
}

/// Admin handler to render the edit page for an image configuration
pub async fn admin_image_edit_page_handler(
    State(state): State<AppState>,
    Path(params): Path<ImagePath>,
    admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    tracing::debug!(
        "Admin requesting image edit page: {} (version: {:?})",
        params.model,
        params.version
    );

    let config = find_image_config(&state, &params).await?;

    Ok(image_edit_template(admin.username, config))
}

/// The edit page for `config`, with every dropdown's options and the image's own selected.
fn image_edit_template(username: String, config: NodeConfig) -> AdminImageEditTemplate {
    AdminImageEditTemplate {
        username,
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        kind_label: kind_label(&config.kind),
        os_variant_choices: form_choices::<OsVariant>(&config.os_variant),
        bios_choices: form_choices::<BiosTypes>(&config.compute.bios),
        cpu_architecture_choices: form_choices::<CpuArchitecture>(&config.compute.cpu_architecture),
        cpu_model_choices: form_choices::<CpuModels>(&config.compute.cpu_model),
        machine_type_choices: form_choices::<MachineType>(&config.compute.machine_type),
        hdd_bus_choices: form_choices::<DiskBuses>(&config.disks.hdd_bus),
        cdrom_bus_choices: form_choices::<DiskBuses>(&config.disks.cdrom_bus),
        ztp_choices: ztp_choices(config.model, config.ztp.mode),
        interface_type_choices: form_choices::<InterfaceType>(&config.interfaces.kind),
        config,
    }
}

/// Form data for updating an image configuration
#[derive(Deserialize)]
pub struct ImageForm {
    // Model, kind, and management_interface come from URL path/existing config, not form
    pub version: String,
    pub repo: Option<String>,
    pub os_variant: String,
    pub bios: String,
    pub cpu_count: u8,
    pub cpu_architecture: String,
    pub cpu_model: String,
    pub machine_type: String,
    pub vmx_enabled: Option<String>, // checkbox
    pub memory: u16,
    pub hdd_bus: String,
    pub cdrom: Option<String>,
    pub cdrom_bus: String,
    /// The ZTP setting, by the word the image stores.
    pub ztp: Ztp,
    pub ztp_username: Option<String>,
    pub ztp_password: Option<String>,
    pub ztp_password_auth: Option<String>, // checkbox
    pub data_interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: String,
    pub interface_mtu: u16,
    pub first_interface_index: u8,
    pub reserved_interface_count: u8,
    pub default: Option<String>, // checkbox
}

/// Admin handler to process image configuration update
pub async fn admin_image_update_handler(
    State(state): State<AppState>,
    Path(params): Path<ImagePath>,
    _admin: AdminUser,
    Form(form): Form<ImageForm>,
) -> Result<Response, ApiError> {
    tracing::info!(
        "Admin updating image: {} (version: {:?})",
        params.model,
        params.version
    );

    let config = find_image_config(&state, &params).await?;

    let choices = ImageFormChoices::parse(&form)?;

    let request_default = if form.default.is_some() {
        DefaultRequest::Make
    } else {
        DefaultRequest::Unset
    };

    // Validate form data
    if let Err(e) = validate_node_image_update(
        config.model,
        form.ztp,
        form.cpu_count,
        form.memory,
        form.interface_mtu,
        &form.version,
        &form.interface_prefix,
    ) {
        tracing::warn!("Node config validation failed: {:?}", e);
        return Ok(AdminNotificationErrorTemplate {
            message: e.to_string(),
        }
        .into_response());
    }

    let _image_guard = lock_image(config.model, &config.version).await;
    let updated_config = updated_image_config(config, form, choices);

    if let Err(error) = db::update_node_image(&state.db, updated_config, request_default).await {
        if let Some(last_default) = error.downcast_ref::<LastDefaultImage>() {
            tracing::warn!("{last_default}");
            return Ok(AdminNotificationErrorTemplate {
                message: last_default.to_string(),
            }
            .into_response());
        }
        tracing::error!("Failed to update image: {:?}", error);
        return Err(ApiError::internal(format!(
            "Failed to update configuration: {error}"
        )));
    }

    tracing::info!(
        "Successfully updated image: {} (version: {:?})",
        params.model,
        params.version
    );

    // Redirect to detail page using HX-Redirect header
    // Include version in URL if it was specified
    let redirect_url = if let Some(version) = &params.version {
        format!("/admin/images/{}/{}", params.model, version)
    } else {
        format!("/admin/images/{}", params.model)
    };

    let mut response = Html("").into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("hx-redirect"),
        header::HeaderValue::from_str(&redirect_url).map_err(|e| {
            ApiError::internal(format!("Failed to create redirect header value: {e}"))
        })?,
    );

    Ok(response)
}

/// Handler for listing all versions of an image (GET /admin/images/{model}/versions)
pub async fn admin_image_versions_handler(
    State(state): State<AppState>,
    AdminUser { username, .. }: AdminUser,
    Path(model): Path<String>,
) -> Result<Response, ApiError> {
    tracing::debug!("Node config versions list request: model={}", model);

    // Parse model enum
    let model_enum = NodeModel::from_str(&model)
        .map_err(|_| ApiError::bad_request(format!("Invalid model: {model}")))?;

    // Derive kind from model
    let kind_enum = model_enum.kind();

    // Fetch all versions for this model/kind
    let versions = db::get_node_image_versions(&state.db, &model_enum, &kind_enum)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch image versions: {:?}", e);
            ApiError::internal(format!("Failed to fetch versions: {e}"))
        })?;

    tracing::debug!("Found {} versions for {}", versions.len(), model);

    // Create template and render
    let template = crate::templates::AdminImageVersionsTemplate {
        username,
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        model,
        kind_label: kind_label(&kind_enum),
        kind: kind_enum,
        versions,
    };

    Ok(template.into_response())
}

// =============================================================================
// Admin — Image Upload
// =============================================================================

/// Parsed fields from a multipart upload form.
#[derive(Debug)]
pub struct UploadFields {
    pub model: NodeModel,
    pub version: String,
    pub file_data: Vec<u8>,
    pub default: bool,
}

/// Raw string fields extracted from multipart before validation.
pub struct RawUploadFields {
    pub model: Option<String>,
    pub version: Option<String>,
    pub file_data: Option<Vec<u8>>,
    pub default: Option<String>,
}

/// Validate raw upload fields and convert to typed `UploadFields`.
///
/// Checks that all required fields are present and valid.
pub fn validate_upload_fields(raw: RawUploadFields) -> Result<UploadFields, String> {
    let model_str = raw.model.ok_or("Missing required field: model")?;
    let parsed_model =
        NodeModel::from_str(&model_str).map_err(|_| format!("Invalid model: {model_str}"))?;

    let version_str = raw.version.ok_or("Missing required field: version")?;
    if version_str.is_empty() {
        return Err("Version cannot be empty".to_string());
    }

    let data = raw.file_data.ok_or("Missing required field: file")?;

    let default = raw.default.as_deref() == Some("on");

    Ok(UploadFields {
        model: parsed_model,
        version: version_str,
        file_data: data,
        default,
    })
}

/// Parse multipart form fields for image upload.
///
/// Extracts model, version, file, and default fields from the multipart stream.
/// Returns an error if any required field is missing or invalid.
pub async fn parse_upload_fields(mut multipart: Multipart) -> Result<UploadFields, String> {
    let mut model: Option<String> = None;
    let mut version: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;
    let mut default: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| format!("Failed to read form field: {e}"))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "model" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read model field: {e}"))?;
                model = Some(text);
            }
            "version" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read version field: {e}"))?;
                version = Some(text);
            }
            "file" => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| format!("Failed to read file data: {e}"))?;
                if !bytes.is_empty() {
                    file_data = Some(bytes.to_vec());
                }
            }
            "default" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read default field: {e}"))?;
                default = Some(text);
            }
            _ => {
                tracing::debug!("Ignoring unknown upload field: {}", name);
            }
        }
    }

    validate_upload_fields(RawUploadFields {
        model,
        version,
        file_data,
        default,
    })
}

/// Admin handler to render the image upload form (GET /admin/images/upload)
pub async fn admin_image_upload_page_handler(
    admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    let mut models: Vec<String> = NodeModel::to_vec()
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    models.sort();

    let template = AdminImageUploadTemplate {
        username: admin.username,
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        models,
    };

    Ok(template)
}

/// Admin handler to process image upload (POST /admin/images/upload)
pub async fn admin_image_upload_handler(
    _admin: AdminUser,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    tracing::info!("Admin uploading image");

    // Parse multipart fields
    let fields = match parse_upload_fields(multipart).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Upload form validation failed: {}", e);
            return Ok(AdminNotificationErrorTemplate { message: e }.into_response());
        }
    };

    // Write file data to a temp file
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!(
        "sherpa_upload_{}_{}_{}",
        fields.model,
        fields.version,
        uuid::Uuid::new_v4()
    ));
    let temp_path = temp_file.to_string_lossy().to_string();

    if let Err(e) = tokio::fs::write(&temp_file, &fields.file_data).await {
        tracing::error!("Failed to write temp file {}: {:?}", temp_path, e);
        return Ok(AdminNotificationErrorTemplate {
            message: format!("Failed to save uploaded file: {e}"),
        }
        .into_response());
    }

    // Construct import request and call the import service
    let request = ImportRequest {
        model: fields.model,
        version: fields.version,
        src: temp_path.clone(),
        default: fields.default,
    };

    let (progress_tx, _progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = ProgressSender::new(progress_tx);

    let result = import::import_image(request, &state, progress).await;

    // Clean up temp file regardless of result
    if let Err(e) = tokio::fs::remove_file(&temp_file).await {
        tracing::warn!("Failed to clean up temp file {}: {:?}", temp_path, e);
    }

    match result {
        Ok(response) => {
            tracing::info!(
                "Image upload successful: {} {} -> {}",
                response.model,
                response.version,
                response.image_path
            );

            // Redirect to the images list
            let mut resp = Html("").into_response();
            resp.headers_mut().insert(
                header::HeaderName::from_static("hx-redirect"),
                header::HeaderValue::from_static("/admin/images"),
            );
            Ok(resp)
        }
        Err(e) => {
            tracing::error!("Image upload failed: {:?}", e);
            Ok(AdminNotificationErrorTemplate {
                message: format!("Import failed: {e}"),
            }
            .into_response())
        }
    }
}

// =============================================================================
// REST API — Lab Management
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use askama::Template;
    use axum::body::Body;
    use axum::extract::FromRequest;
    use axum::extract::rejection::FormRejection;
    use axum::http::{Method, Request};
    use serde::Serialize;

    fn raw_fields(
        model: Option<&str>,
        version: Option<&str>,
        file_data: Option<Vec<u8>>,
        default: Option<&str>,
    ) -> RawUploadFields {
        RawUploadFields {
            model: model.map(std::string::ToString::to_string),
            version: version.map(std::string::ToString::to_string),
            file_data,
            default: default.map(std::string::ToString::to_string),
        }
    }

    #[test]
    fn test_validate_upload_fields_valid() {
        let result = validate_upload_fields(raw_fields(
            Some("ubuntu_linux"),
            Some("24.04"),
            Some(vec![0x00, 0x01, 0x02]),
            Some("on"),
        ));
        let fields = result.expect("should succeed");
        assert_eq!(fields.model, NodeModel::UbuntuLinux);
        assert_eq!(fields.version, "24.04");
        assert_eq!(fields.file_data, vec![0x00, 0x01, 0x02]);
        assert!(fields.default);
    }

    #[test]
    fn test_validate_upload_fields_missing_model() {
        let result =
            validate_upload_fields(raw_fields(None, Some("24.04"), Some(vec![0x01]), None));
        let err = result.unwrap_err();
        assert!(err.contains("model"), "expected model error, got: {err}");
    }

    #[test]
    fn test_validate_upload_fields_missing_version() {
        let result = validate_upload_fields(raw_fields(
            Some("ubuntu_linux"),
            None,
            Some(vec![0x01]),
            None,
        ));
        let err = result.unwrap_err();
        assert!(
            err.contains("version"),
            "expected version error, got: {err}"
        );
    }

    #[test]
    fn test_validate_upload_fields_missing_file() {
        let result =
            validate_upload_fields(raw_fields(Some("ubuntu_linux"), Some("24.04"), None, None));
        let err = result.unwrap_err();
        assert!(err.contains("file"), "expected file error, got: {err}");
    }

    #[test]
    fn test_validate_upload_fields_invalid_model() {
        let result = validate_upload_fields(raw_fields(
            Some("not_a_real_model"),
            Some("24.04"),
            Some(vec![0x01]),
            None,
        ));
        let err = result.unwrap_err();
        assert!(
            err.contains("Invalid model"),
            "expected invalid model error, got: {err}"
        );
    }

    #[test]
    fn test_validate_upload_fields_empty_version() {
        let result = validate_upload_fields(raw_fields(
            Some("ubuntu_linux"),
            Some(""),
            Some(vec![0x01]),
            None,
        ));
        let err = result.unwrap_err();
        assert!(
            err.contains("empty"),
            "expected empty version error, got: {err}"
        );
    }

    #[test]
    fn test_validate_upload_fields_default_absent() {
        let result = validate_upload_fields(raw_fields(
            Some("ubuntu_linux"),
            Some("24.04"),
            Some(vec![0x01]),
            None,
        ));
        let fields = result.expect("should succeed");
        assert!(!fields.default);
    }

    #[test]
    fn test_validate_upload_fields_default_on() {
        let result = validate_upload_fields(raw_fields(
            Some("ubuntu_linux"),
            Some("24.04"),
            Some(vec![0x01]),
            Some("on"),
        ));
        let fields = result.expect("should succeed");
        assert!(fields.default);
    }

    /// A choice's serde name, as the form sends it.
    fn choice<T: Serialize>(value: &T) -> String {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn form_for(config: &NodeConfig, version: &str) -> ImageForm {
        ImageForm {
            version: version.to_owned(),
            repo: None,
            os_variant: choice(&config.os_variant),
            bios: choice(&config.compute.bios),
            cpu_count: config.compute.cpu_count,
            cpu_architecture: choice(&config.compute.cpu_architecture),
            cpu_model: choice(&config.compute.cpu_model),
            machine_type: choice(&config.compute.machine_type),
            vmx_enabled: None,
            memory: config.compute.memory,
            hdd_bus: choice(&config.disks.hdd_bus),
            cdrom: None,
            cdrom_bus: choice(&config.disks.cdrom_bus),
            ztp: config.ztp.mode,
            ztp_username: None,
            ztp_password: None,
            ztp_password_auth: None,
            data_interface_count: config.interfaces.data_count,
            interface_prefix: config.interfaces.prefix.clone(),
            interface_type: choice(&config.interfaces.kind),
            interface_mtu: config.interfaces.mtu,
            first_interface_index: config.interfaces.first_index,
            reserved_interface_count: config.interfaces.reserved_count,
            default: None,
        }
    }

    #[test]
    fn the_form_cannot_change_an_images_version_or_default_flag() {
        let mut config = NodeConfig::get_model(NodeModel::UbuntuLinux);
        "24.04".clone_into(&mut config.version);
        config.default = true;
        let form = form_for(&config, "renamed");
        let choices = ImageFormChoices::parse(&form).unwrap();

        let updated = updated_image_config(config, form, choices);

        assert_eq!(updated.version, "24.04");
        assert!(updated.default);
    }

    /// Each variant of `T` in turn as the image's value: the form offers every variant, and
    /// selects that one alone.
    fn assert_every_variant_offered_and_the_current_selected<T>()
    where
        T: IntoEnumIterator + Display + PartialEq,
    {
        let all: Vec<String> = T::iter().map(|variant| variant.to_string()).collect();
        for current in T::iter() {
            let choices = form_choices(&current);
            let values: Vec<String> = choices.iter().map(|choice| choice.value.clone()).collect();
            let labels: Vec<String> = choices.iter().map(|choice| choice.label.clone()).collect();
            let selected: Vec<String> = choices
                .iter()
                .filter(|choice| choice.selected)
                .map(|choice| choice.value.clone())
                .collect();
            assert_eq!(values, all);
            assert_eq!(labels, all);
            assert_eq!(selected, vec![current.to_string()]);
        }
    }

    #[test]
    fn the_edit_form_offers_every_option_and_selects_the_images_own() {
        assert_every_variant_offered_and_the_current_selected::<OsVariant>();
        assert_every_variant_offered_and_the_current_selected::<BiosTypes>();
        assert_every_variant_offered_and_the_current_selected::<CpuArchitecture>();
        assert_every_variant_offered_and_the_current_selected::<CpuModels>();
        assert_every_variant_offered_and_the_current_selected::<MachineType>();
        assert_every_variant_offered_and_the_current_selected::<DiskBuses>();
        assert_every_variant_offered_and_the_current_selected::<InterfaceType>();
    }

    /// The `<option>` the edit page selects in the `<select>` named `name`.
    fn selected_option(html: &str, name: &str) -> String {
        let select = &html[html.find(&format!(r#"<select id="{name}""#)).expect(name)..];
        let select = &select[..select.find("</select>").expect("select end")];
        let marked: Vec<&str> = select
            .match_indices(r#"" selected>"#)
            .map(|(at, _)| {
                let start = select[..at].rfind(r#"value=""#).expect("value") + 7;
                &select[start..at]
            })
            .collect();
        assert_eq!(marked.len(), 1, "{name}: {select}");
        marked[0].to_owned()
    }

    #[test]
    fn the_edit_page_selects_the_images_own_settings_and_names_its_kind() {
        let ubuntu = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let config = NodeConfig {
            os_variant: OsVariant::iter().next_back().expect("an OS variant"),
            compute: ComputeSettings {
                bios: BiosTypes::iter().next_back().expect("a BIOS"),
                cpu_architecture: CpuArchitecture::iter()
                    .next_back()
                    .expect("an architecture"),
                cpu_model: CpuModels::iter().next_back().expect("a CPU model"),
                machine_type: MachineType::iter().next_back().expect("a machine type"),
                ..ubuntu.compute
            },
            disks: DiskSettings {
                hdd_bus: DiskBuses::iter().next_back().expect("a bus"),
                cdrom_bus: DiskBuses::iter().next().expect("a bus"),
                ..ubuntu.disks
            },
            interfaces: InterfaceSettings {
                kind: InterfaceType::iter()
                    .next_back()
                    .expect("an interface type"),
                ..ubuntu.interfaces
            },
            ..ubuntu
        };
        let html = image_edit_template("admin".to_owned(), config.clone())
            .render()
            .expect("template should render");
        for (name, value) in [
            ("os_variant", config.os_variant.to_string()),
            ("bios", config.compute.bios.to_string()),
            (
                "cpu_architecture",
                config.compute.cpu_architecture.to_string(),
            ),
            ("cpu_model", config.compute.cpu_model.to_string()),
            ("machine_type", config.compute.machine_type.to_string()),
            ("hdd_bus", config.disks.hdd_bus.to_string()),
            ("cdrom_bus", config.disks.cdrom_bus.to_string()),
            ("ztp", config.ztp.mode.to_string()),
            ("interface_type", config.interfaces.kind.to_string()),
        ] {
            assert_eq!(selected_option(&html, name), value, "{name}");
        }
        assert!(html.contains(r#"<input type="text" value="Virtual Machine" disabled"#));
    }

    /// The offered values, and the one selected.
    fn offered(model: NodeModel, current: Ztp) -> (Vec<String>, Vec<String>) {
        let choices = ztp_choices(model, current);
        let selected = choices
            .iter()
            .filter(|choice| choice.selected)
            .map(|choice| choice.value.clone())
            .collect();
        (
            choices.into_iter().map(|choice| choice.value).collect(),
            selected,
        )
    }

    #[test]
    fn the_ztp_choices_are_disabled_and_the_models_methods() {
        // Two methods, one method, and a container with none.
        assert_eq!(
            offered(NodeModel::JuniperVsrxv3, Ztp::Cdrom),
            (
                vec!["disabled".to_owned(), "tftp".to_owned(), "cdrom".to_owned()],
                vec!["cdrom".to_owned()]
            )
        );
        assert_eq!(
            offered(NodeModel::UbuntuLinux, Ztp::Disabled),
            (
                vec!["disabled".to_owned(), "cloud-init".to_owned()],
                vec!["disabled".to_owned()]
            )
        );
        assert_eq!(
            offered(NodeModel::MongoDb, Ztp::Disabled),
            (vec!["disabled".to_owned()], vec!["disabled".to_owned()])
        );
    }

    /// The image form as a browser posts it, with `ztp` as its ZTP setting.
    async fn posted_form(ztp: &str) -> Result<Form<ImageForm>, FormRejection> {
        let body = format!(
            "version=1&os_variant=linux&bios=seabios&cpu_count=1&cpu_architecture=x86_64\
             &cpu_model=host-model&machine_type=q35&memory=1024&hdd_bus=virtio\
             &cdrom_bus=sata&ztp={ztp}&data_interface_count=1&interface_prefix=eth\
             &interface_type=virtio&interface_mtu=1500&first_interface_index=0\
             &reserved_interface_count=0"
        );
        let request = Request::builder()
            .method(Method::POST)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        Form::<ImageForm>::from_request(request, &()).await
    }

    #[tokio::test]
    async fn the_form_reads_the_ztp_word_and_refuses_any_other() {
        for (word, ztp) in [("cloud-init", Ztp::CloudInit), ("disabled", Ztp::Disabled)] {
            let Form(form) = posted_form(word).await.unwrap();
            assert_eq!(form.ztp, ztp);
        }
        for word in ["none", "ipxe", "on", ""] {
            assert!(
                posted_form(word).await.is_err(),
                "{word:?} was read as a ZTP setting"
            );
        }
    }

    #[test]
    fn a_saved_form_carries_its_ztp_setting_to_the_image() {
        let config = NodeConfig::get_model(NodeModel::JuniperVsrxv3);
        let mut form = form_for(&config, "1");
        form.ztp = Ztp::Tftp;
        let choices = ImageFormChoices::parse(&form).unwrap();

        let updated = updated_image_config(config, form, choices);

        assert_eq!(updated.ztp.mode, Ztp::Tftp);
    }
}
