use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use shared::data::{Config, ImportResponse, ListImagesResponse, NodeKind, NodeModel, Sherpa};
use shared::error::RpcErrorCode;
use shared::util::{Emoji, render_images_table, term_msg_surround};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

#[derive(Debug, Subcommand)]
pub enum ImageCommands {
    /// List all boxes
    List {
        /// Optional: List all boxes for a model
        #[arg(value_enum)]
        model: Option<NodeModel>,
        /// List container images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        container: bool,
        /// List unikernel images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        unikernel: bool,
        /// List virtual machine images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        virtual_machine: bool,
    },

    /// Import a disk image
    Import {
        /// Source path of the disk image
        #[arg(short, long)]
        src: String,
        /// Version of the device model
        #[arg(short, long)]
        version: String,
        /// Model of Device
        #[arg(short, long, value_enum)]
        model: NodeModel,
        /// Import the disk image as the latest version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        latest: bool,
    },
}

/// Parse the commands for Image
pub async fn parse_image_commands(
    commands: &ImageCommands,
    _config: &Sherpa,
    server_config: &Config,
    server_url: &str,
) -> Result<()> {
    match commands {
        ImageCommands::List {
            model,
            container,
            unikernel,
            virtual_machine,
        } => {
            let kind = if *container {
                Some(NodeKind::Container)
            } else if *unikernel {
                Some(NodeKind::Unikernel)
            } else if *virtual_machine {
                Some(NodeKind::VirtualMachine)
            } else {
                None
            };

            list_images_rpc(*model, kind, server_config, server_url).await?;
        }
        ImageCommands::Import {
            src,
            version,
            model,
            latest,
        } => import_rpc(src, version, model, *latest, server_config, server_url).await?,
    }
    Ok(())
}

async fn list_images_rpc(
    model: Option<NodeModel>,
    kind: Option<NodeKind>,
    config: &Config,
    server_url: &str,
) -> Result<()> {
    // Load authentication token
    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("\n{} Authentication required", Emoji::Error);
            eprintln!("   Please run: sherpa login");
            eprintln!("   Error: {}", e);
            bail!("Authentication token not found");
        }
    };

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    let list_request = RpcRequest::new(
        "image.list",
        serde_json::json!({
            "model": model,
            "kind": kind,
            "token": token,
        }),
    );

    let list_response = rpc_client
        .call(list_request)
        .await
        .context("Image list RPC call failed")?;

    rpc_client.close().await.ok();

    // Handle errors
    if let Some(error) = list_response.error {
        eprintln!("\n{} Server Error:", Emoji::Error);
        eprintln!("   Message: {}", error.message);
        eprintln!("   Code: {}", error.code);
        if let Some(context) = error.context {
            eprintln!("   Context:\n{}", context);
        }

        if error.code == RpcErrorCode::AuthRequired {
            eprintln!(
                "\n{} Your authentication token has expired or is invalid",
                Emoji::Error
            );
            eprintln!("   Please run: sherpa login");
        }

        bail!("Image list failed");
    }

    let result = list_response.result.context("No result in list response")?;
    let response: ListImagesResponse =
        serde_json::from_value(result).context("Failed to parse list images response")?;

    if response.images.is_empty() {
        println!("\n{} No images found", Emoji::Warning);
    } else {
        println!("\n{}", render_images_table(&response.images));
    }

    Ok(())
}

async fn import_rpc(
    src: &str,
    version: &str,
    model: &NodeModel,
    latest: bool,
    config: &Config,
    server_url: &str,
) -> Result<()> {
    term_msg_surround("Importing disk image");

    // Load authentication token
    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("\n{} Authentication required", Emoji::Error);
            eprintln!("   Please run: sherpa login");
            eprintln!("   Error: {}", e);
            bail!("Authentication token not found");
        }
    };

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    let import_request = RpcRequest::new(
        "image.import",
        serde_json::json!({
            "model": model,
            "version": version,
            "src": src,
            "latest": latest,
            "token": token,
        }),
    );

    let import_response = rpc_client
        .call(import_request)
        .await
        .context("Image import RPC call failed")?;

    rpc_client.close().await.ok();

    // Handle errors
    if let Some(error) = import_response.error {
        eprintln!("\n{} Server Error:", Emoji::Error);
        eprintln!("   Message: {}", error.message);
        eprintln!("   Code: {}", error.code);
        if let Some(context) = error.context {
            eprintln!("   Context:\n{}", context);
        }

        if error.code == RpcErrorCode::AuthRequired {
            eprintln!(
                "\n{} Your authentication token has expired or is invalid",
                Emoji::Error
            );
            eprintln!("   Please run: sherpa login");
        }

        bail!("Image import failed");
    }

    let result = import_response
        .result
        .context("No result in import response")?;
    let response: ImportResponse =
        serde_json::from_value(result).context("Failed to parse import response")?;

    // Display results
    if response.success {
        println!("\n{} Image imported successfully", Emoji::Success);
        println!("   Model:    {}", response.model);
        println!("   Kind:     {}", response.kind);
        println!("   Version:  {}", response.version);
        println!("   Path:     {}", response.image_path);
        println!(
            "   DB Track: {}",
            if response.db_tracked { "yes" } else { "no" }
        );
    } else {
        eprintln!("\n{} Image import failed", Emoji::Error);
    }

    Ok(())
}
