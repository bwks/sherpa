//! Image tables: the images list, a scan's results and one image's detail.

use shared::data::{ImageSummary, NodeConfig, ScannedImage};
use shared::util::{render_property_table, render_table};

const IMAGE_HEADERS: [&str; 4] = ["Model", "Kind", "Version", "Default"];
const SCANNED_IMAGE_HEADERS: [&str; 4] = ["Model", "Version", "Kind", "Status"];

/// Renders a table of images with their model, kind, version, and default status
#[must_use]
pub fn render_images_table(images: &[ImageSummary]) -> String {
    let rows: Vec<[String; 4]> = images.iter().map(image_cells).collect();
    render_table(Some("Images"), IMAGE_HEADERS, &rows, &[])
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
    render_table(Some("Scanned Images"), SCANNED_IMAGE_HEADERS, &rows, &[])
}

fn scanned_image_cells(image: &ScannedImage) -> [String; 4] {
    [
        image.model.to_string(),
        image.version.clone(),
        image.kind.to_string(),
        image.status.clone(),
    ]
}

/// Renders a two-column key-value table with all `NodeConfig` fields for an image
#[must_use]
pub fn render_image_detail_table(image: &NodeConfig) -> String {
    let rows = [
        ("Model", image.model.to_string()),
        ("Kind", image.kind.to_string()),
        ("Version", image.version.clone()),
        ("Default", image.default.to_string()),
        ("OS Variant", image.os_variant.to_string()),
        ("BIOS", image.bios.to_string()),
        ("CPU Architecture", image.cpu_architecture.to_string()),
        ("CPU Model", image.cpu_model.to_string()),
        ("CPU Count", image.cpu_count.to_string()),
        ("Memory (MB)", image.memory.to_string()),
        ("Machine Type", image.machine_type.to_string()),
        ("VMX Enabled", image.vmx_enabled.to_string()),
        ("HDD Bus", image.hdd_bus.to_string()),
        ("CDROM", image.cdrom.as_deref().unwrap_or("none").to_owned()),
        ("CDROM Bus", image.cdrom_bus.to_string()),
        ("Interface Type", image.interface_type.to_string()),
        ("Interface Prefix", image.interface_prefix.clone()),
        ("Interface MTU", image.interface_mtu.to_string()),
        (
            "Data Interface Count",
            image.data_interface_count.to_string(),
        ),
        (
            "First Interface Index",
            image.first_interface_index.to_string(),
        ),
        (
            "Management Interface",
            image.management_interface.to_string(),
        ),
        (
            "Reserved Interface Count",
            image.reserved_interface_count.to_string(),
        ),
        ("Repo", image.repo.as_deref().unwrap_or("none").to_owned()),
        ("ZTP", image.ztp.mode.to_string()),
        (
            "ZTP Username",
            image.ztp.username.as_deref().unwrap_or("none").to_owned(),
        ),
        ("ZTP Password Auth", image.ztp.password_auth.to_string()),
    ];
    render_property_table("Image Detail", &rows)
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
    fn test_render_image_detail_table() {
        let image = NodeConfig::get_model(NodeModel::CiscoCat8000v);

        let table = render_image_detail_table(&image);

        // Check that all property labels are present
        assert!(table.contains("Model"));
        assert!(table.contains("Kind"));
        assert!(table.contains("Version"));
        assert!(table.contains("Default"));
        assert!(table.contains("OS Variant"));
        assert!(table.contains("BIOS"));
        assert!(table.contains("CPU Architecture"));
        assert!(table.contains("CPU Model"));
        assert!(table.contains("CPU Count"));
        assert!(table.contains("Memory (MB)"));
        assert!(table.contains("Machine Type"));
        assert!(table.contains("VMX Enabled"));
        assert!(table.contains("HDD Bus"));
        assert!(table.contains("CDROM"));
        assert!(table.contains("CDROM Bus"));
        assert!(table.contains("Interface Type"));
        assert!(table.contains("Interface Prefix"));
        assert!(table.contains("Interface MTU"));
        assert!(table.contains("Data Interface Count"));
        assert!(table.contains("First Interface Index"));
        assert!(table.contains("Management Interface"));
        assert!(table.contains("Reserved Interface Count"));
        assert!(table.contains("Repo"));
        assert!(table.contains("│ ZTP "));
        assert!(table.contains("cdrom"));
        assert!(!table.contains("ZTP Enabled"));
        assert!(!table.contains("ZTP Method"));
        assert!(table.contains("ZTP Username"));
        assert!(table.contains("ZTP Password Auth"));

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
