use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Subcommand};

use shared::data::{
    ClientConfig, ImpairmentDirection, UpdateImpairmentRequest, UpdateImpairmentResponse,
};
use shared::error::RpcErrorCode;
use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::{Emoji, get_id};
use topology::{Link2, Manifest};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

#[derive(Debug, Subcommand)]
pub enum LinkCommands {
    /// Change the impairment on a running P2p link. The values replace the impairment on the
    /// directions named; the other direction keeps its own.
    #[command(group(
        ArgGroup::new("impairment")
            .required(true)
            .multiple(true)
            .args(["delay", "jitter", "loss", "reorder", "corrupt", "clear"])
    ))]
    Impair {
        /// Either end of the link, as the manifest names it (`node::interface`)
        endpoint: String,
        /// Which direction to change, by the link's manifest `src` and `dst`
        #[arg(long, value_enum, default_value_t = ImpairmentDirection::Both)]
        direction: ImpairmentDirection,
        /// One-way delay in milliseconds
        #[arg(long)]
        delay: Option<u32>,
        /// Delay jitter in milliseconds
        #[arg(long)]
        jitter: Option<u32>,
        /// Packet loss percentage (0.0-100.0)
        #[arg(long)]
        loss: Option<f32>,
        /// Packet reordering percentage (0.0-100.0); needs a delay
        #[arg(long)]
        reorder: Option<f32>,
        /// Bit-flip corruption percentage (0.0-100.0)
        #[arg(long)]
        corrupt: Option<f32>,
        /// Remove the impairment from the directions named
        #[arg(long, conflicts_with_all = ["delay", "jitter", "loss", "reorder", "corrupt"])]
        clear: bool,
    },
}

pub async fn parse_link_commands(
    commands: &LinkCommands,
    config: &ClientConfig,
    server_url: &str,
) -> Result<()> {
    match commands {
        LinkCommands::Impair {
            endpoint,
            direction,
            delay,
            jitter,
            loss,
            reorder,
            corrupt,
            // --clear only stands in for the values; clap keeps it apart from them.
            clear: _,
        } => {
            let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
            let lab_id = get_id(&manifest.name)?;
            let link_index =
                find_link_index(manifest.links.as_deref().unwrap_or_default(), endpoint)?;
            // A value left out is zero, so --clear sends all zeros, which removes the impairment.
            let request = UpdateImpairmentRequest {
                lab_id,
                link_index,
                direction: *direction,
                delay: delay.unwrap_or(0),
                jitter: jitter.unwrap_or(0),
                loss_percent: loss.unwrap_or(0.0),
                reorder_percent: reorder.unwrap_or(0.0),
                corrupt_percent: corrupt.unwrap_or(0.0),
            };
            update_impairment_rpc(request, config, server_url).await?;
        }
    }
    Ok(())
}

/// The index of the manifest link with `endpoint` at either end. Links are numbered in manifest
/// order, as `sherpa up` numbers them.
fn find_link_index(links: &[Link2], endpoint: &str) -> Result<u16> {
    let Some(position) = links
        .iter()
        .position(|link| link.src == endpoint || link.dst == endpoint)
    else {
        bail!("no link in {SHERPA_MANIFEST_FILE} has the end {endpoint}");
    };
    u16::try_from(position).context("link index does not fit in 16 bits")
}

async fn update_impairment_rpc(
    request: UpdateImpairmentRequest,
    config: &ClientConfig,
    server_url: &str,
) -> Result<()> {
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

    let mut params =
        serde_json::to_value(&request).context("Failed to encode impairment request")?;
    params["token"] = serde_json::Value::String(token);
    let response = rpc_client
        .call(RpcRequest::new("link.update_impairment", params))
        .await
        .context("RPC call failed")?;
    rpc_client.close().await.ok();

    if let Some(error) = response.error {
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
        bail!("Link impairment update failed");
    }

    let result = response.result.context("No result in response")?;
    let response: UpdateImpairmentResponse =
        serde_json::from_value(result).context("Failed to parse impairment response")?;
    println!("{} {}", Emoji::Success, response.message);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(src: &str, dst: &str) -> Link2 {
        Link2 {
            src: src.to_string(),
            dst: dst.to_string(),
            p2p: None,
            impairment: None,
        }
    }

    #[test]
    fn test_find_link_index_by_either_end() {
        let links = vec![link("u1::eth1", "u2::eth1"), link("u1::eth2", "c1::eth1")];
        assert_eq!(find_link_index(&links, "u1::eth2").unwrap(), 1);
        assert_eq!(find_link_index(&links, "c1::eth1").unwrap(), 1);
        assert_eq!(find_link_index(&links, "u2::eth1").unwrap(), 0);
    }

    #[test]
    fn test_find_link_index_unknown_end_fails() {
        let links = vec![link("u1::eth1", "u2::eth1")];
        let error = find_link_index(&links, "u1::eth9").unwrap_err().to_string();
        assert!(error.contains("u1::eth9"), "{error}");
    }
}
