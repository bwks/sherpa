//! The web UI's admin image pages and upload.

use axum::extract::{Multipart, Path, State};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde::de::value::{Error as DeError, StrDeserializer};
use serde::de::{DeserializeOwned, IntoDeserializer};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::iter;
use std::str::FromStr;
use strum::IntoEnumIterator;
use tokio::sync::mpsc;

use db::{DefaultRequest, LastDefaultImage};

use crate::api::request_parse::{FormBody, InvalidRequest};
use crate::daemon::state::AppState;
use crate::services::error::{ServiceError, caller_message};
use crate::services::image_lifecycle::lock_image;
use crate::services::import;
use crate::services::progress::ProgressSender;
use crate::templates::{
    AdminImageEditTemplate, AdminImageUploadTemplate, AdminNotificationErrorTemplate,
    DeviceEditChoices, FormChoice, KindBadge, ModelChoice, NavPage, VmEditChoices, kind_choices,
    kind_label,
};

use super::images_api::path_model_kind;
use crate::api::errors::ApiError;
use crate::api::extractors::AdminUser;

use crate::validate::validate_node_image_update;
use shared::data::kind_settings::{self, KindSetting};
use shared::data::{
    BiosTypes, BootDiskSettings, CdromSettings, ContainerSettings, CpuArchitecture, CpuModels,
    CpuSettings, DiskBuses, ImportRequest, InterfaceDeviceSettings, InterfaceSettings,
    InterfaceType, KindSettings, MachineSettings, MachineType, ModelKind, NodeConfig, NodeKind,
    NodeModel, OsVariant, UnikernelCpuSettings, UnikernelMachineSettings, UnikernelSettings,
    VmCpuSettings, VmSettings, Ztp, ZtpSettings,
};

/// The image a page's path names: a model, one of its kinds and, optionally, a version.
#[derive(Debug, Deserialize)]
pub struct ImagePath {
    pub model: String,
    pub kind: String,
    pub version: Option<String>,
}

/// The model and kind of an image versions page's path.
#[derive(Debug, Deserialize)]
pub struct ImageVersionsPath {
    pub model: String,
    pub kind: String,
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
    let configs = db::list_node_images(&state.db)
        .await
        .map_err(|e| ApiError::internal(e, "Failed to load image configurations"))?;

    // One row per model and kind, ordered by model and then by the model's order of kinds,
    // showing the default version, or else the first one found.
    let mut best_by_model_kind: BTreeMap<(String, usize), ImageSummary> = BTreeMap::new();

    for config in configs {
        let kind_order = config
            .model
            .kinds()
            .iter()
            .position(|kind| *kind == config.kind())
            .unwrap_or(usize::MAX);
        let key = (config.model.to_string(), kind_order);
        let summary = ImageSummary {
            model: key.0.clone(),
            kind_badge: KindBadge::short(config.kind()),
            version: config.version,
            cpu_count: config.cpu.count,
            memory: config.memory,
            data_interface_count: config.interfaces.data_count,
            default: config.default,
        };

        let dominated = match best_by_model_kind.get(&key) {
            None => true,
            Some(existing) => summary.default && !existing.default,
        };

        if dominated {
            best_by_model_kind.insert(key, summary);
        }
    }

    let summaries: Vec<ImageSummary> = best_by_model_kind.into_values().collect();

    let template = crate::templates::AdminImagesListTemplate {
        username: admin.username.clone(),
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        configs: summaries,
    };

    Ok(template)
}

/// The image a page or update names: the URL's version of the model and kind, or their
/// default version.
async fn find_image_config(state: &AppState, params: &ImagePath) -> Result<NodeConfig, ApiError> {
    let model_kind = path_model_kind(&params.model, &params.kind).inspect_err(|e| {
        tracing::warn!(
            "Invalid model or kind in URL: {}/{} - {e:?}",
            params.model,
            params.kind
        );
    })?;
    let node_model = model_kind.model();
    let node_kind = model_kind.kind();

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

/// The ZTP settings the edit form offers for an image of `model_kind`: off, then the methods
/// the model and kind take, with the image's `current` setting selected.
fn ztp_choices(model_kind: ModelKind, current: Ztp) -> Vec<FormChoice> {
    iter::once(Ztp::Disabled)
        .chain(model_kind.ztp_methods().iter().copied())
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
fn parse_form_choice<T: DeserializeOwned>(value: &str, field_name: &str) -> anyhow::Result<T> {
    let deserializer: StrDeserializer<'_, DeError> = value.into_deserializer();
    T::deserialize(deserializer)
        .map_err(|_| ServiceError::Invalid(format!("Invalid {field_name}: {value}")).into())
}

/// The image's new configuration from the form: the settings every kind takes, and the
/// image's own kind's group. Its id, model, kind, version, management interface and, for a
/// unikernel, boot mode are kept; checkboxes are on when present, and blank optional fields are
/// unset. The version names the image's files, so the form does not send it.
///
/// # Errors
///
/// Fails when the form sets a field the image's kind does not take, as a page for another kind
/// would post, lacks one its kind needs, or sends a choice that is not one of its options.
fn updated_image_config(config: NodeConfig, form: ImageForm) -> anyhow::Result<NodeConfig> {
    let kind = config.kind();
    if let Some((key, ..)) = kind_settings::first_not_taken(kind, &form.kind_fields()) {
        return Err(ServiceError::Invalid(format!(
            "The form sets `{key}`, which a {kind} image does not take"
        ))
        .into());
    }
    let settings = match &config.settings {
        KindSettings::VirtualMachine(_) => KindSettings::VirtualMachine(VmSettings {
            os_variant: form_choice(form.os_variant.as_deref(), "os_variant", kind)?,
            cpu: VmCpuSettings {
                architecture: form_choice(
                    form.cpu_architecture.as_deref(),
                    "cpu_architecture",
                    kind,
                )?,
                model: form_choice(form.cpu_model.as_deref(), "cpu_model", kind)?,
                vmx: form.vmx_enabled.is_some(),
            },
            machine: MachineSettings {
                kind: form_choice(form.machine_type.as_deref(), "machine_type", kind)?,
                bios: form_choice(form.bios.as_deref(), "bios", kind)?,
            },
            boot_disk: BootDiskSettings {
                bus: form_choice(form.hdd_bus.as_deref(), "hdd_bus", kind)?,
            },
            cdrom: CdromSettings {
                image: form.cdrom.filter(|s| !s.trim().is_empty()),
                bus: form_choice(form.cdrom_bus.as_deref(), "cdrom_bus", kind)?,
            },
            interfaces: InterfaceDeviceSettings {
                kind: form_choice(form.interface_type.as_deref(), "interface_type", kind)?,
                mtu: form_field(form.interface_mtu, "interface_mtu", kind)?,
            },
            ztp: ZtpSettings {
                method: form_field(form.ztp, "ztp", kind)?,
            },
        }),
        KindSettings::Container(_) => KindSettings::Container(ContainerSettings {
            repo: form.repo.filter(|s| !s.trim().is_empty()),
            ztp: ZtpSettings {
                method: form_field(form.ztp, "ztp", kind)?,
            },
        }),
        KindSettings::Unikernel(unikernel) => KindSettings::Unikernel(UnikernelSettings {
            boot_mode: unikernel.boot_mode.clone(), // Keep original (names the image's file)
            cpu: UnikernelCpuSettings {
                architecture: form_choice(
                    form.cpu_architecture.as_deref(),
                    "cpu_architecture",
                    kind,
                )?,
                model: form_choice(form.cpu_model.as_deref(), "cpu_model", kind)?,
            },
            machine: UnikernelMachineSettings {
                kind: form_choice(form.machine_type.as_deref(), "machine_type", kind)?,
            },
            interfaces: InterfaceDeviceSettings {
                kind: form_choice(form.interface_type.as_deref(), "interface_type", kind)?,
                mtu: form_field(form.interface_mtu, "interface_mtu", kind)?,
            },
        }),
    };
    Ok(NodeConfig {
        id: config.id,
        model: config.model,     // Keep original (read-only)
        version: config.version, // Keep original (names the image's files)
        default: config.default, // Decided by the default rule in `db`
        memory: form.memory,
        cpu: CpuSettings {
            count: form.cpu_count,
        },
        interfaces: InterfaceSettings {
            data_count: form.data_interface_count,
            management: config.interfaces.management, // Keep original (read-only)
            reserved_count: form.reserved_interface_count,
        },
        settings,
    })
}

/// A form field an image of `kind` needs.
fn form_field<T>(value: Option<T>, field: &str, kind: NodeKind) -> anyhow::Result<T> {
    value.ok_or_else(|| {
        ServiceError::Invalid(format!(
            "The form has no `{field}`, which a {kind} image needs"
        ))
        .into()
    })
}

/// A choice an image of `kind` needs, by the enum's serde name.
fn form_choice<T: DeserializeOwned>(
    value: Option<&str>,
    field: &str,
    kind: NodeKind,
) -> anyhow::Result<T> {
    parse_form_choice(form_field(value, field, kind)?, field)
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
        kind_badge: KindBadge::full(config.kind()),
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

    image_edit_template(admin.username, config)
}

/// The edit page for `config`: the settings every kind takes, and its own kind's, with every
/// dropdown's options and the image's own selected.
///
/// # Errors
///
/// Fails when the image's model does not take its kind.
fn image_edit_template(
    username: String,
    config: NodeConfig,
) -> Result<AdminImageEditTemplate, ApiError> {
    let vm = config.vm().map(|vm| VmEditChoices {
        os_variant_choices: form_choices::<OsVariant>(&vm.os_variant),
        bios_choices: form_choices::<BiosTypes>(&vm.machine.bios),
        hdd_bus_choices: form_choices::<DiskBuses>(&vm.boot_disk.bus),
        cdrom_bus_choices: form_choices::<DiskBuses>(&vm.cdrom.bus),
        cdrom: vm.cdrom.image.clone().unwrap_or_default(),
        vmx: vm.cpu.vmx,
    });
    let device = match &config.settings {
        KindSettings::VirtualMachine(vm) => Some(DeviceEditChoices {
            cpu_architecture_choices: form_choices::<CpuArchitecture>(&vm.cpu.architecture),
            cpu_model_choices: form_choices::<CpuModels>(&vm.cpu.model),
            machine_type_choices: form_choices::<MachineType>(&vm.machine.kind),
            interface_type_choices: form_choices::<InterfaceType>(&vm.interfaces.kind),
            interface_mtu: vm.interfaces.mtu,
        }),
        KindSettings::Unikernel(unikernel) => Some(DeviceEditChoices {
            cpu_architecture_choices: form_choices::<CpuArchitecture>(&unikernel.cpu.architecture),
            cpu_model_choices: form_choices::<CpuModels>(&unikernel.cpu.model),
            machine_type_choices: form_choices::<MachineType>(&unikernel.machine.kind),
            interface_type_choices: form_choices::<InterfaceType>(&unikernel.interfaces.kind),
            interface_mtu: unikernel.interfaces.mtu,
        }),
        KindSettings::Container(_) => None,
    };
    let ztp_choices = match &config.settings {
        KindSettings::Unikernel(_) => Vec::new(),
        KindSettings::VirtualMachine(_) | KindSettings::Container(_) => {
            ztp_choices(config.model_kind()?, config.ztp())
        }
    };
    Ok(AdminImageEditTemplate {
        username,
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        kind_label: kind_label(config.kind()),
        repo: config
            .container()
            .map(|container| container.repo.clone().unwrap_or_default()),
        vm,
        device,
        ztp_choices,
        config,
    })
}

/// Form data for updating an image configuration: the settings every kind takes, and each
/// kind's own, of which the page for an image posts its kind's alone. It denies unknown fields,
/// so a form that still sends `interface_prefix` or `first_interface_index`, removed in 0.7.0,
/// or `ztp_username`, `ztp_password`, `ztp_password_remove` or `ztp_password_auth`, as an
/// editor page loaded from an older server does, is refused naming the field instead of saved
/// without it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageForm {
    // Model, kind, version and management_interface come from URL path/existing config, not
    // form
    pub cpu_count: u8,
    pub memory: u16,
    pub data_interface_count: u8,
    pub reserved_interface_count: u8,
    pub default: Option<String>, // checkbox
    // The `vm` group's.
    pub os_variant: Option<String>,
    pub bios: Option<String>,
    pub vmx_enabled: Option<String>, // checkbox
    pub hdd_bus: Option<String>,
    pub cdrom: Option<String>,
    pub cdrom_bus: Option<String>,
    // The `vm` and `unikernel` groups'.
    pub cpu_architecture: Option<String>,
    pub cpu_model: Option<String>,
    pub machine_type: Option<String>,
    pub interface_type: Option<String>,
    pub interface_mtu: Option<u16>,
    /// The ZTP setting, by the word the image stores: the `vm` and `container` groups'.
    pub ztp: Option<Ztp>,
    // The `container` group's.
    pub repo: Option<String>,
}

impl ImageForm {
    /// The kind-only fields the form sets, each with the kinds of image that take it.
    fn kind_fields(&self) -> [KindSetting<'static>; 13] {
        [
            ("os_variant", self.os_variant.is_some(), kind_settings::VM),
            ("bios", self.bios.is_some(), kind_settings::VM),
            ("vmx_enabled", self.vmx_enabled.is_some(), kind_settings::VM),
            ("hdd_bus", self.hdd_bus.is_some(), kind_settings::VM),
            ("cdrom", self.cdrom.is_some(), kind_settings::VM),
            ("cdrom_bus", self.cdrom_bus.is_some(), kind_settings::VM),
            (
                "cpu_architecture",
                self.cpu_architecture.is_some(),
                kind_settings::VM_AND_UNIKERNEL,
            ),
            (
                "cpu_model",
                self.cpu_model.is_some(),
                kind_settings::VM_AND_UNIKERNEL,
            ),
            (
                "machine_type",
                self.machine_type.is_some(),
                kind_settings::VM_AND_UNIKERNEL,
            ),
            (
                "interface_type",
                self.interface_type.is_some(),
                kind_settings::VM_AND_UNIKERNEL,
            ),
            (
                "interface_mtu",
                self.interface_mtu.is_some(),
                kind_settings::VM_AND_UNIKERNEL,
            ),
            ("ztp", self.ztp.is_some(), kind_settings::VM_AND_CONTAINER),
            ("repo", self.repo.is_some(), kind_settings::CONTAINER),
        ]
    }
}

/// Admin handler to process image configuration update. A form the server cannot read, as one
/// with a field it does not take, is refused with a notification naming the problem.
pub async fn admin_image_update_handler(
    State(state): State<AppState>,
    Path(params): Path<ImagePath>,
    _admin: AdminUser,
    form: Result<FormBody<ImageForm>, InvalidRequest>,
) -> Result<Response, ApiError> {
    let form = match form {
        Ok(FormBody(form)) => form,
        Err(rejection) => {
            tracing::warn!(%rejection, "Refused an image form");
            return Ok(AdminNotificationErrorTemplate {
                message: format!("The image was not saved: {rejection}"),
            }
            .into_response());
        }
    };
    tracing::info!(
        "Admin updating image: {} (version: {:?})",
        params.model,
        params.version
    );

    let config = find_image_config(&state, &params).await?;
    let model_kind = config.model_kind()?;

    let request_default = if form.default.is_some() {
        DefaultRequest::Make
    } else {
        DefaultRequest::Unset
    };

    let _image_guard = lock_image(model_kind, &config.version).await;
    let updated_config = match updated_image_config(config, form) {
        Ok(updated) => updated,
        Err(error) => {
            tracing::warn!("Refused an image form: {error:?}");
            return Ok(AdminNotificationErrorTemplate {
                message: format!("The image was not saved: {error}"),
            }
            .into_response());
        }
    };
    if let Err(e) = validate_node_image_update(model_kind, &updated_config) {
        tracing::warn!("Node config validation failed: {:?}", e);
        return Ok(AdminNotificationErrorTemplate {
            message: e.to_string(),
        }
        .into_response());
    }

    if let Err(error) = db::update_node_image(&state.db, updated_config, request_default).await {
        if let Some(last_default) = error.downcast_ref::<LastDefaultImage>() {
            tracing::warn!("{last_default}");
            return Ok(AdminNotificationErrorTemplate {
                message: last_default.to_string(),
            }
            .into_response());
        }
        return Err(ApiError::internal(error, "Failed to update configuration"));
    }

    tracing::info!(
        "Successfully updated image: {} (version: {:?})",
        params.model,
        params.version
    );

    // Redirect to detail page using HX-Redirect header
    // Include version in URL if it was specified
    let redirect_url = if let Some(version) = &params.version {
        format!("/admin/images/{}/{}/{}", params.model, params.kind, version)
    } else {
        format!("/admin/images/{}/{}", params.model, params.kind)
    };

    let mut response = Html("").into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("hx-redirect"),
        header::HeaderValue::from_str(&redirect_url)
            .map_err(|e| ApiError::internal(e, "Failed to create redirect header value"))?,
    );

    Ok(response)
}

/// Handler for listing all versions of an image (GET /admin/images/{model}/{kind}/versions)
pub async fn admin_image_versions_handler(
    State(state): State<AppState>,
    AdminUser { username, .. }: AdminUser,
    Path(ImageVersionsPath { model, kind }): Path<ImageVersionsPath>,
) -> Result<Response, ApiError> {
    tracing::debug!(
        "Node config versions list request: model={}, kind={}",
        model,
        kind
    );

    let model_kind = path_model_kind(&model, &kind)?;
    let model_enum = model_kind.model();
    let kind_enum = model_kind.kind();

    // Fetch all versions for this model/kind
    let versions = db::get_node_image_versions(&state.db, &model_enum, &kind_enum)
        .await
        .map_err(|e| ApiError::internal(e, "Failed to fetch versions"))?;

    tracing::debug!("Found {} versions for {}", versions.len(), model);

    // Create template and render
    let template = crate::templates::AdminImageVersionsTemplate {
        username,
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        model,
        kind_label: kind_label(kind_enum),
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
    pub model_kind: ModelKind,
    pub version: String,
    pub file_data: Vec<u8>,
    pub default: bool,
}

/// Raw string fields extracted from multipart before validation.
pub struct RawUploadFields {
    pub model: Option<String>,
    /// The image's kind; absent or empty, the model's default kind.
    pub kind: Option<String>,
    pub version: Option<String>,
    pub file_data: Option<Vec<u8>>,
    pub default: Option<String>,
}

/// Validate raw upload fields and convert to typed `UploadFields`.
///
/// Checks that all required fields are present and valid, and that the model takes the kind:
/// a model with no default kind (`generic`) needs one.
pub fn validate_upload_fields(raw: RawUploadFields) -> Result<UploadFields, String> {
    let model_str = raw.model.ok_or("Missing required field: model")?;
    let parsed_model = NodeModel::from_str(&model_str)?;
    let parsed_kind = raw
        .kind
        .filter(|kind| !kind.is_empty())
        .map(|kind| NodeKind::from_str(&kind))
        .transpose()?;
    let model_kind = ModelKind::new(parsed_model, parsed_kind).map_err(|e| e.to_string())?;

    let version_str = raw.version.ok_or("Missing required field: version")?;
    if version_str.is_empty() {
        return Err("Version cannot be empty".to_string());
    }

    let data = raw.file_data.ok_or("Missing required field: file")?;

    let default = raw.default.as_deref() == Some("on");

    Ok(UploadFields {
        model_kind,
        version: version_str,
        file_data: data,
        default,
    })
}

/// Parse multipart form fields for image upload.
///
/// Extracts model, kind, version, file, and default fields from the multipart stream.
/// Returns an error if any required field is missing or invalid.
pub async fn parse_upload_fields(mut multipart: Multipart) -> Result<UploadFields, String> {
    let mut model: Option<String> = None;
    let mut kind: Option<String> = None;
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
            "kind" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read kind field: {e}"))?;
                kind = Some(text);
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
        kind,
        version,
        file_data,
        default,
    })
}

/// Admin handler to render the image upload form (GET /admin/images/upload)
pub async fn admin_image_upload_page_handler(
    admin: AdminUser,
) -> Result<impl IntoResponse, ApiError> {
    let mut models: Vec<ModelChoice> = NodeModel::iter().map(ModelChoice::from).collect();
    models.sort_by(|a, b| a.model.cmp(&b.model));

    let template = AdminImageUploadTemplate {
        username: admin.username,
        is_admin: true,
        nav: NavPage::AdminImages.into(),
        models,
        kinds: kind_choices(),
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
        "sherpa_upload_{}_{}_{}_{}",
        fields.model_kind.model(),
        fields.model_kind.kind(),
        fields.version,
        uuid::Uuid::new_v4()
    ));
    let temp_path = temp_file.to_string_lossy().to_string();

    if let Err(e) = tokio::fs::write(&temp_file, &fields.file_data).await {
        tracing::error!("Failed to write temp file {}: {:?}", temp_path, e);
        return Ok(AdminNotificationErrorTemplate {
            message: format!(
                "Failed to save uploaded file: {}",
                caller_message(
                    &anyhow::Error::from(e),
                    "Failed to save the uploaded image file"
                )
            ),
        }
        .into_response());
    }

    // Construct import request and call the import service
    let request = ImportRequest {
        model: fields.model_kind.model(),
        kind: Some(fields.model_kind.kind()),
        version: fields.version,
        src: temp_path.clone(),
        default: fields.default,
    };

    let (progress_tx, _progress_rx) = mpsc::unbounded_channel();
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
                message: format!(
                    "Import failed: {}",
                    caller_message(&e, "Image import failed")
                ),
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
            kind: None,
            version: version.map(std::string::ToString::to_string),
            file_data,
            default: default.map(std::string::ToString::to_string),
        }
    }

    /// An upload of `model` with `kind`, otherwise valid.
    fn upload_of(model: &str, kind: Option<&str>) -> Result<UploadFields, String> {
        validate_upload_fields(RawUploadFields {
            kind: kind.map(str::to_owned),
            ..raw_fields(Some(model), Some("4.32.0F"), Some(vec![0x01]), None)
        })
    }

    #[test]
    fn image_import_018_an_upload_without_a_kind_takes_the_models_default_and_with_one_that_kind() {
        let fields = validate_upload_fields(raw_fields(
            Some("ubuntu_linux"),
            Some("24.04"),
            Some(vec![0x00, 0x01, 0x02]),
            Some("on"),
        ))
        .unwrap();
        assert_eq!(
            fields.model_kind,
            ModelKind::default_for(NodeModel::UbuntuLinux)
        );
        assert_eq!(fields.version, "24.04");
        assert_eq!(fields.file_data, vec![0x00, 0x01, 0x02]);
        assert!(fields.default);

        for (kind, expected) in [
            (Some("virtual_machine"), NodeKind::VirtualMachine),
            (Some("container"), NodeKind::Container),
            (None, NodeKind::VirtualMachine),
            (Some(""), NodeKind::VirtualMachine),
        ] {
            let fields = upload_of("arista_eos", kind).unwrap();
            assert_eq!(fields.model_kind.model(), NodeModel::AristaEos, "{kind:?}");
            assert_eq!(fields.model_kind.kind(), expected, "{kind:?}");
        }
        let fields = upload_of("generic", Some("unikernel")).unwrap();
        assert_eq!(fields.model_kind.kind(), NodeKind::Unikernel);
    }

    #[test]
    fn image_import_017_an_upload_missing_a_field_or_with_a_bad_model_version_or_kind_is_refused() {
        for (raw, expected) in [
            (
                raw_fields(None, Some("24.04"), Some(vec![0x01]), None),
                "model",
            ),
            (
                raw_fields(Some("ubuntu_linux"), None, Some(vec![0x01]), None),
                "version",
            ),
            (
                raw_fields(Some("ubuntu_linux"), Some("24.04"), None, None),
                "file",
            ),
            (
                raw_fields(
                    Some("not_a_real_model"),
                    Some("24.04"),
                    Some(vec![0x01]),
                    None,
                ),
                "Unknown node model",
            ),
            (
                raw_fields(Some("ubuntu_linux"), Some(""), Some(vec![0x01]), None),
                "empty",
            ),
        ] {
            let err = validate_upload_fields(raw).unwrap_err();
            assert!(err.contains(expected), "expected {expected}, got: {err}");
        }
        let err = upload_of("arista_eos", Some("unikernel")).unwrap_err();
        assert!(err.contains("is not a unikernel"), "{err}");
        let err = upload_of("arista_eos", Some("vm")).unwrap_err();
        assert!(err.contains("Unknown node kind: vm"), "{err}");
        for kind in [None, Some("")] {
            let err = upload_of("generic", kind).unwrap_err();
            assert!(err.contains("has no default kind"), "{kind:?}: {err}");
        }
    }

    /// A choice's serde name, as the form sends it.
    fn choice<T: Serialize>(value: &T) -> String {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The form the edit page for `config` posts, unchanged.
    fn form_for(config: &NodeConfig) -> ImageForm {
        let mut form = ImageForm {
            cpu_count: config.cpu.count,
            memory: config.memory,
            data_interface_count: config.interfaces.data_count,
            reserved_interface_count: config.interfaces.reserved_count,
            default: None,
            os_variant: None,
            bios: None,
            vmx_enabled: None,
            hdd_bus: None,
            cdrom: None,
            cdrom_bus: None,
            cpu_architecture: None,
            cpu_model: None,
            machine_type: None,
            interface_type: None,
            interface_mtu: None,
            ztp: None,
            repo: None,
        };
        match &config.settings {
            KindSettings::VirtualMachine(vm) => {
                form.os_variant = Some(choice(&vm.os_variant));
                form.bios = Some(choice(&vm.machine.bios));
                form.vmx_enabled = vm.cpu.vmx.then(|| "on".to_owned());
                form.hdd_bus = Some(choice(&vm.boot_disk.bus));
                form.cdrom = Some(vm.cdrom.image.clone().unwrap_or_default());
                form.cdrom_bus = Some(choice(&vm.cdrom.bus));
                form.cpu_architecture = Some(choice(&vm.cpu.architecture));
                form.cpu_model = Some(choice(&vm.cpu.model));
                form.machine_type = Some(choice(&vm.machine.kind));
                form.interface_type = Some(choice(&vm.interfaces.kind));
                form.interface_mtu = Some(vm.interfaces.mtu);
                form.ztp = Some(vm.ztp.method);
            }
            KindSettings::Container(container) => {
                form.repo = Some(container.repo.clone().unwrap_or_default());
                form.ztp = Some(container.ztp.method);
            }
            KindSettings::Unikernel(unikernel) => {
                form.cpu_architecture = Some(choice(&unikernel.cpu.architecture));
                form.cpu_model = Some(choice(&unikernel.cpu.model));
                form.machine_type = Some(choice(&unikernel.machine.kind));
                form.interface_type = Some(choice(&unikernel.interfaces.kind));
                form.interface_mtu = Some(unikernel.interfaces.mtu);
            }
        }
        form
    }

    /// The three kinds' images: a VM, a container and a unikernel.
    fn one_image_of_each_kind() -> [NodeConfig; 3] {
        [
            ModelKind::default_for(NodeModel::CiscoCat8000).node_config(),
            ModelKind::default_for(NodeModel::NokiaSrlinux).node_config(),
            ModelKind::default_for(NodeModel::NanosUnikernel).node_config(),
        ]
    }

    #[test]
    fn image_settings_001_a_form_saved_unchanged_leaves_an_image_of_each_kind_as_it_was() {
        for mut config in one_image_of_each_kind() {
            "24.04".clone_into(&mut config.version);
            config.default = true;
            let form = form_for(&config);

            let updated = updated_image_config(config.clone(), form).unwrap();

            assert_eq!(updated, config, "{}", config.model);
        }
    }

    #[test]
    fn image_settings_004_a_form_with_a_field_of_another_kind_is_refused_naming_the_field() {
        let [vm, container, unikernel] = one_image_of_each_kind();
        for (config, field, setter) in [
            (
                &container,
                "bios",
                (|form: &mut ImageForm| form.bios = Some("uefi".to_owned())) as fn(&mut ImageForm),
            ),
            (&container, "interface_mtu", |form: &mut ImageForm| {
                form.interface_mtu = Some(1500);
            }),
            (&container, "cpu_model", |form: &mut ImageForm| {
                form.cpu_model = Some("host-model".to_owned());
            }),
            (&unikernel, "ztp", |form: &mut ImageForm| {
                form.ztp = Some(Ztp::Disabled);
            }),
            (&unikernel, "hdd_bus", |form: &mut ImageForm| {
                form.hdd_bus = Some("virtio".to_owned());
            }),
            (&vm, "repo", |form: &mut ImageForm| {
                form.repo = Some("x".to_owned());
            }),
        ] {
            let mut form = form_for(config);
            setter(&mut form);
            let error = updated_image_config(config.clone(), form)
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
                format!(
                    "The form sets `{field}`, which a {} image does not take",
                    config.kind()
                )
            );
        }
    }

    #[test]
    fn image_settings_005_a_form_without_a_field_its_kind_needs_is_refused_naming_the_field() {
        let [vm, container, _] = one_image_of_each_kind();
        let mut form = form_for(&vm);
        form.bios = None;
        let error = updated_image_config(vm, form).unwrap_err().to_string();
        assert!(error.contains("no `bios`"), "{error}");
        let mut form = form_for(&container);
        form.ztp = None;
        let error = updated_image_config(container, form)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no `ztp`"), "{error}");
    }

    #[tokio::test]
    async fn image_settings_007_an_edit_keeps_the_images_version_and_a_unikernels_boot_mode() {
        let [_, _, unikernel] = one_image_of_each_kind();
        let mut form = form_for(&unikernel);
        form.interface_mtu = Some(9000);
        let updated = updated_image_config(unikernel.clone(), form).unwrap();
        assert_eq!(
            updated.unikernel().unwrap().boot_mode,
            unikernel.unikernel().unwrap().boot_mode
        );
        assert_eq!(updated.interface_device().unwrap().mtu, 9000);

        let db = db::open_in_memory().await.unwrap();
        db::apply_schema(&db).await.unwrap();
        let mut renamed = db::upsert_node_image(&db, unikernel.clone(), DefaultRequest::Keep)
            .await
            .unwrap();
        "other".clone_into(&mut renamed.version);
        let stored = db::update_node_image(&db, renamed, DefaultRequest::Keep)
            .await
            .unwrap();
        assert_eq!(stored.version, unikernel.version);
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
    fn image_settings_010_the_edit_page_shows_the_images_own_settings_chosen_and_names_its_kind() {
        let mut config = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        let KindSettings::VirtualMachine(vm) = &mut config.settings else {
            panic!("ubuntu_linux is a VM");
        };
        vm.os_variant = OsVariant::iter().next_back().expect("an OS variant");
        vm.cpu.architecture = CpuArchitecture::iter()
            .next_back()
            .expect("an architecture");
        vm.cpu.model = CpuModels::iter().next_back().expect("a CPU model");
        vm.machine = MachineSettings {
            kind: MachineType::iter().next_back().expect("a machine type"),
            bios: BiosTypes::iter().next_back().expect("a BIOS"),
        };
        vm.boot_disk.bus = DiskBuses::iter().next_back().expect("a bus");
        vm.cdrom.bus = DiskBuses::iter().next().expect("a bus");
        vm.interfaces.kind = InterfaceType::iter()
            .next_back()
            .expect("an interface type");
        let vm = vm.clone();
        let html = image_edit_template("admin".to_owned(), config.clone())
            .unwrap()
            .render()
            .expect("template should render");
        for (name, value) in [
            ("os_variant", vm.os_variant.to_string()),
            ("bios", vm.machine.bios.to_string()),
            ("cpu_architecture", vm.cpu.architecture.to_string()),
            ("cpu_model", vm.cpu.model.to_string()),
            ("machine_type", vm.machine.kind.to_string()),
            ("hdd_bus", vm.boot_disk.bus.to_string()),
            ("cdrom_bus", vm.cdrom.bus.to_string()),
            ("ztp", config.ztp().to_string()),
            ("interface_type", vm.interfaces.kind.to_string()),
        ] {
            assert_eq!(selected_option(&html, name), value, "{name}");
        }
        assert!(html.contains(r#"<input type="text" value="Virtual Machine" disabled"#));
        assert!(!html.contains(r#"name="repo""#), "{html}");
    }

    /// The offered values, and the one selected.
    fn offered(model: NodeModel, current: Ztp) -> (Vec<String>, Vec<String>) {
        let choices = ztp_choices(ModelKind::default_for(model), current);
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
    fn ztp_setting_009_the_image_edit_page_offers_disabled_and_the_models_methods() {
        // Two methods, one method, and a container with none.
        assert_eq!(
            offered(NodeModel::JuniperEvolved, Ztp::Usb),
            (
                vec!["disabled".to_owned(), "tftp".to_owned(), "usb".to_owned()],
                vec!["usb".to_owned()]
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
    async fn posted_form(ztp: &str) -> Result<FormBody<ImageForm>, InvalidRequest> {
        let body = format!(
            "os_variant=linux&bios=seabios&cpu_count=1&cpu_architecture=x86_64\
             &cpu_model=host-model&machine_type=q35&memory=1024&hdd_bus=virtio\
             &cdrom_bus=sata&ztp={ztp}&data_interface_count=1\
             &interface_type=virtio&interface_mtu=1500&reserved_interface_count=0"
        );
        let request = Request::builder()
            .method(Method::POST)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        FormBody::<ImageForm>::from_request(request, &()).await
    }

    #[tokio::test]
    async fn ztp_setting_010_the_edit_form_reads_the_setting_by_its_word_and_refuses_any_other() {
        for (word, ztp) in [("cloud-init", Ztp::CloudInit), ("disabled", Ztp::Disabled)] {
            let FormBody(form) = posted_form(word).await.unwrap();
            assert_eq!(form.ztp, Some(ztp));
        }
        for word in ["none", "ipxe", "on", ""] {
            assert!(
                posted_form(word).await.is_err(),
                "{word:?} was read as a ZTP setting"
            );
        }
    }

    #[tokio::test]
    async fn image_settings_006_a_form_with_a_field_the_server_does_not_take_is_refused() {
        let rejection = posted_form("cloud-init&version=1")
            .await
            .err()
            .expect("version was accepted");
        assert_eq!(rejection.message, "the request is not valid at version");
    }

    #[test]
    fn ztp_setting_011_a_saved_edit_form_carries_its_setting_to_the_image() {
        let config = ModelKind::default_for(NodeModel::JuniperEvolved).node_config();
        let mut form = form_for(&config);
        form.ztp = Some(Ztp::Usb);

        let updated = updated_image_config(config, form).unwrap();

        assert_eq!(updated.ztp(), Ztp::Usb);
    }
}
