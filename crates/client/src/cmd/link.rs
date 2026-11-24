use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Subcommand};

use shared::data::{
    ClientConfig, ImpairmentDirection, LinkInfo, UpdateImpairmentRequest, UpdateImpairmentResponse,
};
use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::Emoji;
use topology::{Link2, Manifest};

use super::inspect::fetch_inspect;
use super::lab_identity::resolve_lab_identity;
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
        /// Either end of the link, `node::interface`, as the manifest and `sherpa inspect` name it
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
        loss: Option<f64>,
        /// Packet reordering percentage (0.0-100.0); needs a delay
        #[arg(long)]
        reorder: Option<f64>,
        /// Bit-flip corruption percentage (0.0-100.0)
        #[arg(long)]
        corrupt: Option<f64>,
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
            let lab_id = resolve_lab_identity(Path::new(""))?.id;
            let inspect = fetch_inspect(&lab_id, server_url, config).await?;
            let link_index = match inspect_link_index(&inspect.links, endpoint)? {
                Some(index) => index,
                None => manifest_link_index(Path::new(SHERPA_MANIFEST_FILE), endpoint)?,
            };
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

/// The index of the lab's link with `endpoint` (`node::interface`) at either end, from the
/// lab's `links` as `inspect` gives them. `None` when the server gives no link indexes, as
/// servers before link indexes do.
fn inspect_link_index(links: &[LinkInfo], endpoint: &str) -> Result<Option<u16>> {
    let found = links.iter().find(|link| {
        format!("{}::{}", link.node_a_name, link.int_a) == endpoint
            || format!("{}::{}", link.node_b_name, link.int_b) == endpoint
    });
    match found {
        Some(link) => Ok(link.index),
        None if links.iter().any(|link| link.index.is_none()) => Ok(None),
        None => bail!("no link in the lab has the end {endpoint}"),
    }
}

/// The index of the link with `endpoint` at either end in the manifest at `path`, for a server
/// that gives no link indexes.
fn manifest_link_index(path: &Path, endpoint: &str) -> Result<u16> {
    if !path.exists() {
        bail!(
            "The Sherpa server did not give the lab's link indexes: it is older than this \
             client. Upgrade the server, or run 'sherpa link impair' in the folder the lab was \
             started from, with its {SHERPA_MANIFEST_FILE}."
        );
    }
    let manifest = Manifest::load_file(&path.to_string_lossy())?;
    find_link_index(manifest.links.as_deref().unwrap_or_default(), endpoint)
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
    let token = load_token()?;

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

    let response: UpdateImpairmentResponse =
        response.into_result("Link impairment update failed")?;
    println!("{} {}", Emoji::Success, response.message);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use shared::data::BridgeKind;

    use super::*;

    fn link(src: &str, dst: &str) -> Link2 {
        Link2 {
            src: src.to_string(),
            dst: dst.to_string(),
            impairment: None,
        }
    }

    fn link_info(a: &str, b: &str, index: Option<u16>) -> LinkInfo {
        let (first, first_int) = a.split_once("::").unwrap();
        let (second, second_int) = b.split_once("::").unwrap();
        LinkInfo {
            node_a_name: first.to_string(),
            int_a: first_int.to_string(),
            node_b_name: second.to_string(),
            int_b: second_int.to_string(),
            kind: BridgeKind::P2p,
            state: None,
            index,
        }
    }

    #[test]
    fn link_impair_006_the_link_is_found_by_either_end_and_an_end_on_no_link_is_refused() {
        // Indexes as the server numbered them, not by position in the list.
        let links = [
            link_info("u1::eth2", "c1::eth1", Some(1)),
            link_info("u1::eth1", "u2::eth1", Some(0)),
        ];
        assert_eq!(inspect_link_index(&links, "u1::eth2").unwrap(), Some(1));
        assert_eq!(inspect_link_index(&links, "c1::eth1").unwrap(), Some(1));
        assert_eq!(inspect_link_index(&links, "u2::eth1").unwrap(), Some(0));

        let links = [link_info("u1::eth1", "u2::eth1", Some(0))];
        let error = inspect_link_index(&links, "u1::eth9").unwrap_err();
        assert_eq!(error.to_string(), "no link in the lab has the end u1::eth9");
        assert!(inspect_link_index(&[], "u1::eth1").is_err());
    }

    #[test]
    fn link_impair_007_with_no_link_indexes_the_link_is_found_in_the_manifest_or_refused() {
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(SHERPA_MANIFEST_FILE);
            fs::write(
                &path,
                "name='lab'\nnodes=[]\nlinks=[{src='u1::eth1',dst='u2::eth1'},{src='u1::eth2',dst='c1::eth1'}]\n",
            )
            .unwrap();
            assert_eq!(manifest_link_index(&path, "c1::eth1").unwrap(), 1);
        }

        {
            let links = [link_info("u1::eth1", "u2::eth1", None)];
            assert_eq!(inspect_link_index(&links, "u1::eth1").unwrap(), None);
            assert_eq!(inspect_link_index(&links, "u1::eth9").unwrap(), None);
        }

        {
            let dir = tempfile::tempdir().unwrap();
            let error = manifest_link_index(&dir.path().join(SHERPA_MANIFEST_FILE), "u1::eth1")
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "The Sherpa server did not give the lab's link indexes: it is older than this \
                 client. Upgrade the server, or run 'sherpa link impair' in the folder the lab was \
                 started from, with its manifest.toml."
            );
        }

        {
            let links = vec![link("u1::eth1", "u2::eth1"), link("u1::eth2", "c1::eth1")];
            assert_eq!(find_link_index(&links, "u1::eth2").unwrap(), 1);
            assert_eq!(find_link_index(&links, "c1::eth1").unwrap(), 1);
            assert_eq!(find_link_index(&links, "u2::eth1").unwrap(), 0);
        }

        {
            let links = vec![link("u1::eth1", "u2::eth1")];
            let error = find_link_index(&links, "u1::eth9").unwrap_err().to_string();
            assert!(error.contains("u1::eth9"), "{error}");
        }
    }
}
