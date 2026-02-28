#[cfg(feature = "local")]
mod boot_containers;
mod cert;
mod clean;
mod cli;
mod console;
mod destroy;
mod down;
mod image;
mod init;
mod inspect;
mod new;
mod login;
mod manifest_processing;
mod resume;
mod ssh;
mod up;
mod validate;
mod virtual_machine;

pub use cli::Cli;
