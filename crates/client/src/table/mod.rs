//! The CLI's terminal tables. Each maps the items it shows to rows of cells and renders them
//! with `shared::util::render_table`.

mod image;
mod lab;
mod server;
mod ssh_config;

pub use image::{render_image_detail_table, render_images_table, render_scanned_images_table};
pub use lab::{
    render_bridges_table, render_devices_table, render_lab_info_table, render_links_table,
    render_nodes_table, render_provision_table, render_tailnet_table,
};
pub use server::{
    CertificateTableInfo, render_backup_table, render_certificates_table,
    render_server_status_table, render_ssh_keys_table,
};
pub use ssh_config::render_ssh_config_inspection_table;
