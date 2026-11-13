//! Image tables: the images list, a scan's results and one image's detail.

use shared::data::{ImageSummary, NodeConfig, ScannedImage, SkippedModel};
use shared::util::{TableLayout, render_table};

const IMAGE_HEADERS: [&str; 4] = ["Model", "Kind", "Version", "Default"];
const SCANNED_IMAGE_HEADERS: [&str; 4] = ["Model", "Version", "Kind", "Status"];
const SKIPPED_MODEL_HEADERS: [&str; 2] = ["Model", "Reason"];
/// The image detail table has no header row: its first column names each row.
const IMAGE_DETAIL_HEADERS: [&str; 2] = ["Property", "Value"];

/// Renders a table of images with their model, kind, version, and default status
#[must_use]
pub fn render_images_table(images: &[ImageSummary]) -> String {
    let rows: Vec<[String; 4]> = images.iter().map(image_cells).collect();
    render_table(
        IMAGE_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Images"),
            ..TableLayout::default()
        },
    )
}

fn image_cells(image: &ImageSummary) -> [String; 4] {
    [
        image.model.to_string(),
        image.kind.to_string(),
        image.version.clone(),
        image.default.to_string(),
    ]
}

/// Renders a table of scanned images with their model, version, kind, and status
#[must_use]
pub fn render_scanned_images_table(images: &[ScannedImage]) -> String {
    let rows: Vec<[String; 4]> = images.iter().map(scanned_image_cells).collect();
    render_table(
        SCANNED_IMAGE_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Scanned Images"),
            ..TableLayout::default()
        },
    )
}

fn scanned_image_cells(image: &ScannedImage) -> [String; 4] {
    [
        image.model.to_string(),
        image.version.clone(),
        image.kind.to_string(),
        image.status.clone(),
    ]
}

/// Renders a table of the models a scan skipped, with why
#[must_use]
pub fn render_skipped_models_table(models: &[SkippedModel]) -> String {
    let rows: Vec<[String; 2]> = models.iter().map(skipped_model_cells).collect();
    render_table(
        SKIPPED_MODEL_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Skipped Models"),
            ..TableLayout::default()
        },
    )
}

fn skipped_model_cells(model: &SkippedModel) -> [String; 2] {
    [model.model.to_string(), model.reason.clone()]
}

/// Renders a two-column key-value table with all `NodeConfig` fields for an image. Each row
/// is named by its key in the image JSON, as `group.field` for the grouped settings.
#[must_use]
pub fn render_image_detail_table(image: &NodeConfig) -> String {
    let optional = |value: Option<&str>| value.unwrap_or("none").to_owned();
    let rows = [
        ["model".to_owned(), image.model.to_string()],
        ["kind".to_owned(), image.kind.to_string()],
        ["version".to_owned(), image.version.clone()],
        ["default".to_owned(), image.default.to_string()],
        ["os_variant".to_owned(), image.os_variant.to_string()],
        ["repo".to_owned(), optional(image.repo.as_deref())],
        ["memory".to_owned(), format!("{} MB", image.memory)],
        ["cpu.count".to_owned(), image.cpu.count.to_string()],
        [
            "cpu.architecture".to_owned(),
            image.cpu.architecture.to_string(),
        ],
        ["cpu.model".to_owned(), image.cpu.model.to_string()],
        ["cpu.vmx".to_owned(), image.cpu.vmx.to_string()],
        ["machine.kind".to_owned(), image.machine.kind.to_string()],
        ["machine.bios".to_owned(), image.machine.bios.to_string()],
        ["boot_disk.bus".to_owned(), image.boot_disk.bus.to_string()],
        [
            "cdrom.image".to_owned(),
            optional(image.cdrom.image.as_deref()),
        ],
        ["cdrom.bus".to_owned(), image.cdrom.bus.to_string()],
        [
            "interfaces.data_count".to_owned(),
            image.interfaces.data_count.to_string(),
        ],
        [
            "interfaces.kind".to_owned(),
            image.interfaces.kind.to_string(),
        ],
        [
            "interfaces.mtu".to_owned(),
            image.interfaces.mtu.to_string(),
        ],
        [
            "interfaces.prefix".to_owned(),
            image.interfaces.prefix.clone(),
        ],
        [
            "interfaces.first_index".to_owned(),
            image.interfaces.first_index.to_string(),
        ],
        [
            "interfaces.management".to_owned(),
            image.interfaces.management.to_string(),
        ],
        [
            "interfaces.reserved_count".to_owned(),
            image.interfaces.reserved_count.to_string(),
        ],
        ["ztp.method".to_owned(), image.ztp.method.to_string()],
        [
            "ztp.username".to_owned(),
            optional(image.ztp.username.as_deref()),
        ],
        [
            "ztp.password_auth".to_owned(),
            image.ztp.password_auth.to_string(),
        ],
        [
            "unikernel.boot_mode".to_owned(),
            image.unikernel.as_ref().map_or_else(
                || "none".to_owned(),
                |unikernel| unikernel.boot_mode.to_string(),
            ),
        ],
    ];
    render_table(
        IMAGE_DETAIL_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Image Detail"),
            header_row: false,
            ..TableLayout::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeKind, NodeModel};

    #[test]
    fn images_table_shows_each_image_and_whether_it_is_the_default() {
        let images = [
            ImageSummary {
                model: NodeModel::CiscoCat8000v,
                kind: NodeKind::VirtualMachine,
                version: "17.13.01a".to_string(),
                default: true,
            },
            ImageSummary {
                model: NodeModel::NokiaSrlinux,
                kind: NodeKind::Container,
                version: "latest".to_string(),
                default: false,
            },
        ];
        let expected = "\
┌────────────────────────────────────────────────────────┐
│                         Images                         │
├────────────────┬─────────────────┬───────────┬─────────┤
│ Model          │ Kind            │ Version   │ Default │
├────────────────┼─────────────────┼───────────┼─────────┤
│ cisco_cat8000v │ virtual_machine │ 17.13.01a │ true    │
├────────────────┼─────────────────┼───────────┼─────────┤
│ nokia_srlinux  │ container       │ latest    │ false   │
└────────────────┴─────────────────┴───────────┴─────────┘";
        assert_eq!(render_images_table(&images), expected);
    }

    #[test]
    fn empty_images_table_still_has_headers() {
        let expected = "\
┌──────────────────────────────────┐
│              Images              │
├───────┬──────┬─────────┬─────────┤
│ Model │ Kind │ Version │ Default │
└───────┴──────┴─────────┴─────────┘";
        assert_eq!(render_images_table(&[]), expected);
    }

    #[test]
    fn scanned_images_table_shows_each_image_with_its_status() {
        let scanned = [
            ScannedImage {
                model: NodeModel::CiscoCat8000v,
                version: "17.13.01a".to_string(),
                kind: NodeKind::VirtualMachine,
                status: "imported".to_string(),
            },
            ScannedImage {
                model: NodeModel::AristaVeos,
                version: "4.33".to_string(),
                kind: NodeKind::VirtualMachine,
                status: "already present".to_string(),
            },
        ];
        let expected = "\
┌────────────────────────────────────────────────────────────────┐
│                         Scanned Images                         │
├────────────────┬───────────┬─────────────────┬─────────────────┤
│ Model          │ Version   │ Kind            │ Status          │
├────────────────┼───────────┼─────────────────┼─────────────────┤
│ cisco_cat8000v │ 17.13.01a │ virtual_machine │ imported        │
├────────────────┼───────────┼─────────────────┼─────────────────┤
│ arista_veos    │ 4.33      │ virtual_machine │ already present │
└────────────────┴───────────┴─────────────────┴─────────────────┘";
        assert_eq!(render_scanned_images_table(&scanned), expected);
    }

    #[test]
    fn skipped_models_table_shows_each_model_with_its_reason() {
        let skipped = [
            SkippedModel {
                model: NodeModel::MysqlDb,
                reason: "no built-in repo".to_string(),
            },
            SkippedModel {
                model: NodeModel::GenericContainer,
                reason: "no built-in repo".to_string(),
            },
        ];
        let expected = "\
┌──────────────────────────────────────┐
│            Skipped Models            │
├───────────────────┬──────────────────┤
│ Model             │ Reason           │
├───────────────────┼──────────────────┤
│ mysql_db          │ no built-in repo │
├───────────────────┼──────────────────┤
│ generic_container │ no built-in repo │
└───────────────────┴──────────────────┘";
        assert_eq!(render_skipped_models_table(&skipped), expected);
    }

    #[test]
    fn test_render_image_detail_table() {
        let image = NodeConfig::get_model(NodeModel::CiscoCat8000v);

        let table = render_image_detail_table(&image);

        // Each row is named by its key in the image JSON, groups included.
        for key in [
            "model",
            "kind",
            "version",
            "default",
            "os_variant",
            "repo",
            "memory",
            "cpu.count",
            "cpu.architecture",
            "cpu.model",
            "cpu.vmx",
            "machine.kind",
            "machine.bios",
            "boot_disk.bus",
            "cdrom.image",
            "cdrom.bus",
            "interfaces.data_count",
            "interfaces.kind",
            "interfaces.mtu",
            "interfaces.prefix",
            "interfaces.first_index",
            "interfaces.management",
            "interfaces.reserved_count",
            "ztp.method",
            "ztp.username",
            "ztp.password_auth",
            "unikernel.boot_mode",
        ] {
            assert!(table.contains(&format!("│ {key} ")), "{key}\n{table}");
        }
        assert!(table.contains("cdrom"));

        // Check that model-specific values are present
        assert!(table.contains("cisco_cat8000v"));
        assert!(table.contains("virtual_machine"));

        // Check for header panel
        assert!(table.contains("Image Detail"));

        // Check for modern style box-drawing characters
        assert!(table.contains("┌") || table.contains("│"));

        println!("\n{table}");
    }

    #[test]
    fn test_render_image_detail_table_container_model() {
        let image = NodeConfig::get_model(NodeModel::NokiaSrlinux);

        let table = render_image_detail_table(&image);

        // Check container-specific values
        assert!(table.contains("nokia_srlinux"));
        assert!(table.contains("container"));

        // Repo should be populated for container models
        assert!(!table.contains("Repo") || !table.contains("none") || table.contains("ghcr.io"));

        println!("\n{table}");
    }

    #[test]
    fn test_render_image_detail_table_optional_fields() {
        let image = NodeConfig::default();

        let table = render_image_detail_table(&image);

        // Optional fields should show "none" when not set
        assert!(table.contains("none")); // repo, cdrom, ztp_username are None by default

        println!("\n{table}");
    }
}
