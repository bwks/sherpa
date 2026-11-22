//! Image tables: the images list, a scan's results and one image's detail.

use shared::data::{ImageSummary, KindSettings, NodeConfig, ScannedImage, SkippedModel};
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

/// Renders a two-column key-value table with the `NodeConfig` fields of an image. Each row
/// is named by its key in the image JSON, as `group.field` for the grouped settings: the groups
/// every kind takes, then the image's own kind's group, `vm`, `container` or `unikernel`.
#[must_use]
pub fn render_image_detail_table(image: &NodeConfig) -> String {
    let optional = |value: Option<&str>| value.unwrap_or("none").to_owned();
    let mut rows = vec![
        ["model".to_owned(), image.model.to_string()],
        ["kind".to_owned(), image.kind().to_string()],
        ["version".to_owned(), image.version.clone()],
        ["default".to_owned(), image.default.to_string()],
        ["memory".to_owned(), format!("{} MB", image.memory)],
        ["cpu.count".to_owned(), image.cpu.count.to_string()],
        [
            "interfaces.data_count".to_owned(),
            image.interfaces.data_count.to_string(),
        ],
        [
            "interfaces.management".to_owned(),
            image.interfaces.management.to_string(),
        ],
        [
            "interfaces.reserved_count".to_owned(),
            image.interfaces.reserved_count.to_string(),
        ],
    ];
    match &image.settings {
        KindSettings::VirtualMachine(vm) => rows.extend([
            ["vm.os_variant".to_owned(), vm.os_variant.to_string()],
            [
                "vm.cpu.architecture".to_owned(),
                vm.cpu.architecture.to_string(),
            ],
            ["vm.cpu.model".to_owned(), vm.cpu.model.to_string()],
            ["vm.cpu.vmx".to_owned(), vm.cpu.vmx.to_string()],
            ["vm.machine.kind".to_owned(), vm.machine.kind.to_string()],
            ["vm.machine.bios".to_owned(), vm.machine.bios.to_string()],
            ["vm.boot_disk.bus".to_owned(), vm.boot_disk.bus.to_string()],
            [
                "vm.cdrom.image".to_owned(),
                optional(vm.cdrom.image.as_deref()),
            ],
            ["vm.cdrom.bus".to_owned(), vm.cdrom.bus.to_string()],
            [
                "vm.interfaces.kind".to_owned(),
                vm.interfaces.kind.to_string(),
            ],
            [
                "vm.interfaces.mtu".to_owned(),
                vm.interfaces.mtu.to_string(),
            ],
            ["vm.ztp.method".to_owned(), vm.ztp.method.to_string()],
        ]),
        KindSettings::Container(container) => rows.extend([
            [
                "container.repo".to_owned(),
                optional(container.repo.as_deref()),
            ],
            [
                "container.ztp.method".to_owned(),
                container.ztp.method.to_string(),
            ],
        ]),
        KindSettings::Unikernel(unikernel) => rows.extend([
            [
                "unikernel.boot_mode".to_owned(),
                unikernel.boot_mode.to_string(),
            ],
            [
                "unikernel.cpu.architecture".to_owned(),
                unikernel.cpu.architecture.to_string(),
            ],
            [
                "unikernel.cpu.model".to_owned(),
                unikernel.cpu.model.to_string(),
            ],
            [
                "unikernel.machine.kind".to_owned(),
                unikernel.machine.kind.to_string(),
            ],
            [
                "unikernel.interfaces.kind".to_owned(),
                unikernel.interfaces.kind.to_string(),
            ],
            [
                "unikernel.interfaces.mtu".to_owned(),
                unikernel.interfaces.mtu.to_string(),
            ],
        ]),
    }
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
    use shared::data::{ModelKind, NodeKind, NodeModel};

    #[test]
    fn image_list_008_image_list_prints_each_images_model_kind_version_and_default() {
        let images = [
            ImageSummary {
                model: NodeModel::CiscoCat8000,
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
┌───────────────────────────────────────────────────────┐
│                        Images                         │
├───────────────┬─────────────────┬───────────┬─────────┤
│ Model         │ Kind            │ Version   │ Default │
├───────────────┼─────────────────┼───────────┼─────────┤
│ cisco_cat8000 │ virtual_machine │ 17.13.01a │ true    │
├───────────────┼─────────────────┼───────────┼─────────┤
│ nokia_srlinux │ container       │ latest    │ false   │
└───────────────┴─────────────────┴───────────┴─────────┘";
        assert_eq!(render_images_table(&images), expected);
    }

    #[test]
    fn image_scan_019_the_cli_lists_each_image_found_with_its_status_and_each_skipped_model_with_the_reason()
     {
        let scanned = [
            ScannedImage {
                model: NodeModel::CiscoCat8000,
                version: "17.13.01a".to_string(),
                kind: NodeKind::VirtualMachine,
                status: "imported".to_string(),
                failed: false,
            },
            ScannedImage {
                model: NodeModel::AristaEos,
                version: "4.33".to_string(),
                kind: NodeKind::VirtualMachine,
                status: "already present".to_string(),
                failed: false,
            },
        ];
        let expected = "\
┌───────────────────────────────────────────────────────────────┐
│                        Scanned Images                         │
├───────────────┬───────────┬─────────────────┬─────────────────┤
│ Model         │ Version   │ Kind            │ Status          │
├───────────────┼───────────┼─────────────────┼─────────────────┤
│ cisco_cat8000 │ 17.13.01a │ virtual_machine │ imported        │
├───────────────┼───────────┼─────────────────┼─────────────────┤
│ arista_eos    │ 4.33      │ virtual_machine │ already present │
└───────────────┴───────────┴─────────────────┴─────────────────┘";
        assert_eq!(render_scanned_images_table(&scanned), expected);

        let skipped = [
            SkippedModel {
                model: NodeModel::MysqlDb,
                reason: "no built-in repo".to_string(),
            },
            SkippedModel {
                model: NodeModel::Generic,
                reason: "no built-in repo".to_string(),
            },
        ];
        let expected_skipped = "\
┌─────────────────────────────┐
│       Skipped Models        │
├──────────┬──────────────────┤
│ Model    │ Reason           │
├──────────┼──────────────────┤
│ mysql_db │ no built-in repo │
├──────────┼──────────────────┤
│ generic  │ no built-in repo │
└──────────┴──────────────────┘";
        assert_eq!(render_skipped_models_table(&skipped), expected_skipped);
    }

    /// The rows of an image's detail table, by their names.
    fn row_names(table: &str) -> Vec<String> {
        table
            .lines()
            .filter_map(|line| line.strip_prefix("│ "))
            .filter_map(|line| line.split(" │").next())
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty() && name != "Image Detail")
            .collect()
    }

    const SHARED_ROWS: [&str; 9] = [
        "model",
        "kind",
        "version",
        "default",
        "memory",
        "cpu.count",
        "interfaces.data_count",
        "interfaces.management",
        "interfaces.reserved_count",
    ];

    #[test]
    fn image_list_009_image_show_prints_the_shared_rows_then_its_kinds_group() {
        let image = ModelKind::default_for(NodeModel::CiscoCat8000).node_config();
        let table = render_image_detail_table(&image);
        let mut expected: Vec<&str> = SHARED_ROWS.to_vec();
        expected.extend([
            "vm.os_variant",
            "vm.cpu.architecture",
            "vm.cpu.model",
            "vm.cpu.vmx",
            "vm.machine.kind",
            "vm.machine.bios",
            "vm.boot_disk.bus",
            "vm.cdrom.image",
            "vm.cdrom.bus",
            "vm.interfaces.kind",
            "vm.interfaces.mtu",
            "vm.ztp.method",
        ]);
        assert_eq!(row_names(&table), expected, "{table}");
        assert!(table.contains("cisco_cat8000"));
        assert!(table.contains("virtual_machine"));
        assert!(table.contains("Image Detail"));
    }
}
