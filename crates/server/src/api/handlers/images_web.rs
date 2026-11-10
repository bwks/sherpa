//! The web UI's admin image pages and upload.

use axum::Form;
use axum::extract::{Multipart, Path, State};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use serde::de::value::{Error as DeError, StrDeserializer};
use serde::de::{DeserializeOwned, IntoDeserializer};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use strum::IntoEnumIterator;

use crate::daemon::state::AppState;
use crate::services::import;
use crate::services::progress::ProgressSender;
use crate::templates::{AdminImageUploadTemplate, AdminNotificationErrorTemplate};

use crate::api::errors::ApiError;
use crate::api::extractors::AdminUser;

use crate::validate::validate_node_image_update;
use shared::data::{
    BiosTypes, CpuArchitecture, CpuModels, DiskBuses, ImportRequest, InterfaceType, MachineType,
    NodeConfig, NodeModel, OsVariant, ZtpMethod, ZtpSettings,
};

/// Helper struct for extracting image path parameters with optional version
#[derive(Debug, Deserialize)]
pub struct ImagePath {
    pub model: String,
    pub version: Option<String>,
}

/// Helper struct to summarize image data for list view
#[derive(Debug, Clone, Serialize)]
pub struct ImageSummary {
    pub model: String,
    pub kind: String,
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
            kind: config.kind.to_string(),
            version: config.version,
            cpu_count: config.cpu_count,
            memory: config.memory,
            data_interface_count: config.data_interface_count,
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
        active_page: "admin_images".to_string(),
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
    ztp_method: ZtpMethod,
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
            ztp_method: parse_form_choice(&form.ztp_method, "ZTP method")?,
            interface_type: parse_form_choice(&form.interface_type, "interface type")?,
        })
    }
}

/// One of the form's choices, by the enum's serde name.
fn parse_form_choice<T: DeserializeOwned>(value: &str, field_name: &str) -> Result<T, ApiError> {
    let deserializer: StrDeserializer<'_, DeError> = value.into_deserializer();
    T::deserialize(deserializer)
        .map_err(|_| ApiError::bad_request(format!("Invalid {field_name}: {value}")))
}

/// The image's new configuration from the form. Its id, model, kind, management interface and
/// boot mode are kept; checkboxes are on when present, and blank optional fields are unset.
fn updated_image_config(
    config: NodeConfig,
    form: ImageForm,
    choices: ImageFormChoices,
) -> NodeConfig {
    NodeConfig {
        id: config.id,
        model: config.model, // Keep original (read-only)
        kind: config.kind,   // Keep original (read-only)
        version: form.version,
        repo: form.repo.filter(|s| !s.trim().is_empty()),
        os_variant: choices.os_variant,
        bios: choices.bios,
        cpu_count: form.cpu_count,
        cpu_architecture: choices.cpu_architecture,
        cpu_model: choices.cpu_model,
        machine_type: choices.machine_type,
        vmx_enabled: form.vmx_enabled.is_some(),
        memory: form.memory,
        hdd_bus: choices.hdd_bus,
        cdrom: form.cdrom.filter(|s| !s.trim().is_empty()),
        cdrom_bus: choices.cdrom_bus,
        ztp: ZtpSettings {
            enable: form.ztp_enable.is_some(),
            method: choices.ztp_method,
            username: form.ztp_username.filter(|s| !s.trim().is_empty()),
            password: form.ztp_password.filter(|s| !s.trim().is_empty()),
            password_auth: form.ztp_password_auth.is_some(),
        },
        data_interface_count: form.data_interface_count,
        interface_prefix: form.interface_prefix,
        interface_type: choices.interface_type,
        interface_mtu: form.interface_mtu,
        first_interface_index: form.first_interface_index,
        management_interface: config.management_interface, // Keep original (read-only)
        reserved_interface_count: form.reserved_interface_count,
        default: form.default.is_some(),
        boot_mode: config.boot_mode, // Keep original
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
        active_page: "admin_images".to_string(),
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

    // Generate enum options for dropdowns
    let template = crate::templates::AdminImageEditTemplate {
        username: admin.username.clone(),
        is_admin: true,
        active_page: "admin_images".to_string(),
        config,
        os_variants: OsVariant::iter().map(|v| v.to_string()).collect(),
        bios_types: BiosTypes::iter().map(|v| v.to_string()).collect(),
        cpu_architectures: CpuArchitecture::iter().map(|v| v.to_string()).collect(),
        cpu_models: CpuModels::iter().map(|v| v.to_string()).collect(),
        machine_types: MachineType::iter().map(|v| v.to_string()).collect(),
        disk_buses: DiskBuses::iter().map(|v| v.to_string()).collect(),
        ztp_methods: ZtpMethod::iter().map(|v| v.to_string()).collect(),
        interface_types: InterfaceType::iter().map(|v| v.to_string()).collect(),
    };

    Ok(template)
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
    pub ztp_enable: Option<String>, // checkbox
    pub ztp_method: String,
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

    let form_default = form.default.is_some();

    // Validate: prevent unsetting the last default for this (model, kind)
    if config.default && !form_default {
        // User is trying to unset default - check if this is the only default
        let versions = db::get_node_image_versions(&state.db, &config.model, &config.kind)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get versions for {}: {:?}", params.model, e);
                ApiError::internal("Failed to validate default status")
            })?;

        let default_count = versions.iter().filter(|v| v.default).count();

        if default_count == 1 {
            tracing::warn!(
                "Cannot unset default - only one default exists for {}",
                params.model
            );
            return Ok(AdminNotificationErrorTemplate {
                message: format!(
                    "Cannot unset default - at least one version must be marked as default for {}",
                    params.model
                ),
            }
            .into_response());
        }
    }

    // Validate form data
    if let Err(e) = validate_node_image_update(
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

    let updated_config = updated_image_config(config, form, choices);

    // Update in database
    db::update_node_image(&state.db, updated_config)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update image: {:?}", e);
            ApiError::internal(format!("Failed to update configuration: {e}"))
        })?;

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
        active_page: "admin_images".to_string(),
        model,
        kind: kind_enum.to_string(), // Derive kind for display in template
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
        active_page: "admin_images".to_string(),
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
}
