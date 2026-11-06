use std::io;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::ImportImageOptionsBuilder;
use futures_util::{StreamExt, future};
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};
use tracing::instrument;

/// Load a container image from a tar archive into the Docker daemon.
///
/// Equivalent to `docker load -i <src>`. Accepts both `.tar` and `.tar.gz`
/// files (Docker's load endpoint handles gzip decompression natively).
///
/// The optional `on_progress` callback is invoked with status messages
/// as the load progresses.
///
/// # Errors
///
/// Fails when the archive cannot be opened or read, or Docker cannot load it. A read error is
/// reported ahead of the error Docker gives for the archive it cut short.
#[instrument(skip(docker, on_progress), level = "debug")]
pub async fn load_image<F>(docker: &Docker, src: &str, on_progress: F) -> Result<()>
where
    F: Fn(&str),
{
    tracing::info!(src = %src, "Loading container image from tar archive");
    on_progress(&format!("Loading image from {src}..."));

    let file = File::open(src)
        .await
        .with_context(|| format!("Failed to open tar archive: {src}"))?;

    // Docker takes the archive as a stream of plain chunks, so a read error ends the stream and
    // is kept here to be reported.
    let read_error = Arc::new(Mutex::new(None::<io::Error>));
    let stream_error = Arc::clone(&read_error);
    let byte_stream = FramedRead::new(file, BytesCodec::new()).scan((), move |(), chunk| {
        future::ready(match chunk {
            Ok(bytes) => Some(bytes.freeze()),
            Err(error) => {
                if let Ok(mut slot) = stream_error.lock() {
                    *slot = Some(error);
                }
                None
            }
        })
    });

    let options = ImportImageOptionsBuilder::default().quiet(false).build();
    let mut response_stream = docker.import_image_stream(options, byte_stream, None);

    while let Some(result) = response_stream.next().await {
        match result {
            Ok(info) => {
                if let Some(ref status) = info.status {
                    on_progress(status);
                    tracing::debug!(status = %status, "Image load progress");
                }
            }
            Err(error) => {
                check_read(&read_error, src)?;
                return Err(error).with_context(|| format!("Error loading image from {src}"));
            }
        }
    }
    check_read(&read_error, src)?;

    tracing::info!(src = %src, "Successfully loaded image from tar archive");
    on_progress(&format!("Successfully loaded image from {src}"));

    Ok(())
}

/// Fail with the error that ended reading `src`, if one did.
fn check_read(read_error: &Mutex<Option<io::Error>>, src: &str) -> Result<()> {
    let error = read_error.lock().map_or(None, |mut slot| slot.take());
    match error {
        Some(error) => Err(error).with_context(|| format!("Failed to read image archive {src}")),
        None => Ok(()),
    }
}
