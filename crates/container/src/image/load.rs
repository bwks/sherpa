use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::ImportImageOptionsBuilder;
use futures_util::StreamExt;
use tokio_util::codec;

/// Load a container image from a tar archive into the Docker daemon.
///
/// Equivalent to `docker load -i <src>`. Accepts both `.tar` and `.tar.gz`
/// files (Docker's load endpoint handles gzip decompression natively).
///
/// The optional `on_progress` callback is invoked with status messages
/// as the load progresses.
pub async fn load_image<F>(docker: &Docker, src: &str, on_progress: F) -> Result<()>
where
    F: Fn(&str),
{
    tracing::info!(src = %src, "Loading container image from tar archive");
    on_progress(&format!("Loading image from {}...", src));

    let file = tokio::fs::File::open(src)
        .await
        .with_context(|| format!("Failed to open tar archive: {}", src))?;

    let byte_stream = codec::FramedRead::new(file, codec::BytesCodec::new())
        .map(|r| r.map(|bytes| bytes.freeze()));

    let options = ImportImageOptionsBuilder::default().quiet(false).build();

    let mut response_stream = docker.import_image_stream(
        options,
        byte_stream.filter_map(|r| async {
            match r {
                Ok(bytes) => Some(bytes),
                Err(e) => {
                    tracing::error!("Error reading tar archive: {}", e);
                    None
                }
            }
        }),
        None,
    );

    while let Some(result) = response_stream.next().await {
        match result {
            Ok(info) => {
                if let Some(ref status) = info.status {
                    on_progress(status);
                    tracing::debug!(status = %status, "Image load progress");
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Error loading image from {}: {}", src, e));
            }
        }
    }

    tracing::info!(src = %src, "Successfully loaded image from tar archive");
    on_progress(&format!("Successfully loaded image from {}", src));

    Ok(())
}
