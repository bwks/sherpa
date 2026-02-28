use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use shared::data::{ClientConfig, ListImagesResponse, NodeKind, NodeModel};
use shared::error::RpcErrorCode;
use shared::util::{Emoji, render_images_table};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

#[derive(Debug, Subcommand)]
pub enum ImageCommands {
    /// List all images
    List {
        /// Optional: List all images for a model
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
}

/// Parse the commands for Image
pub async fn parse_image_commands(
    commands: &ImageCommands,
    server_config: &ClientConfig,
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
    }
    Ok(())
}

async fn list_images_rpc(
    model: Option<NodeModel>,
    kind: Option<NodeKind>,
    config: &ClientConfig,
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
