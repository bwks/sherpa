use clap::Subcommand;

#[derive(Debug, Subcommand)]
#[group(
    id = "image_selector",
    args = ["name", "model"],
    required = true
)]
pub enum VirtualMachineCommands {
    /// Pull a container image from an image hosting service.
    Pull {
        // /// Image name - srlinux
        // #[arg(short, long, requires_all = ["repo", "version"])]
        // name: Option<String>,
        // /// Image Repository - ghcr.io/nokia/srlinux
        // #[arg(short, long)]
        // repo: Option<String>,
        // /// Image version - 1.2.3
        // #[arg(short, long)]
        // version: Option<String>,
        // #[arg(short, long)]
        // /// Container Model
        // model: Option<ContainerModel>,
    },
    /// Import a local container image as a Sherpa box.
    Import {
        // /// Source container image
        // #[arg(short, long)]
        // image: String,
        // /// Version of the device model
        // #[arg(short, long)]
        // version: String,
        // /// Model of Device
        // #[arg(short, long, value_enum)]
        // model: NodeModel,
        // /// Import the container image as the latest version
        // #[arg(long, action = clap::ArgAction::SetTrue)]
        // latest: bool,
    },
}
