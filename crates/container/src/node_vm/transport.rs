//! Docker's API of a node VM, reached over vsock with mutual TLS. Every request opens its own
//! connection: the host connects to the VM's vsock context ID, the guest's socket proxy hands
//! it to dockerd's TLS listener, and both ends prove themselves with the node's certificates.

use std::io::{self, BufReader};
use std::os::fd::{AsRawFd, OwnedFd};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, ready};

use bollard::errors::Error as DockerError;
use bollard::{BollardRequest, Docker};
use hyper::Response;
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper::header::{HOST, HeaderValue};
use hyper::http::uri::PathAndQuery;
use hyper_util::rt::TokioIo;
use nix::errno::Errno;
use nix::sys::socket::{
    AddressFamily, MsgFlags, Shutdown, SockFlag, SockType, VsockAddr, connect, getsockopt, recv,
    send, shutdown, socket, sockopt,
};
use rustls::ClientConfig;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::ServerName;
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::TlsConnector;

use crate::error::ContainerError;

/// The address bollard builds request URIs on. Requests go over vsock, so it is never
/// resolved; only the path reaches dockerd.
const NODE_VM_CLIENT_ADDRESS: &str = "http://node-vm";

/// The certificates of a node VM's Docker API: the node's CA, which signed both ends, sherpad's
/// client certificate and key, and the name in the VM's server certificate.
#[derive(Clone)]
pub struct NodeVmTls {
    pub ca_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
    pub server_name: String,
}

/// A vsock stream that tokio polls.
struct VsockStream {
    fd: AsyncFd<OwnedFd>,
}

impl VsockStream {
    /// Connect to `port` on the VM with context ID `cid`.
    async fn connect(cid: u32, port: u32) -> io::Result<Self> {
        let fd = socket(
            AddressFamily::Vsock,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )?;
        match connect(fd.as_raw_fd(), &VsockAddr::new(cid, port)) {
            Ok(()) | Err(Errno::EINPROGRESS) => {}
            Err(errno) => return Err(errno.into()),
        }
        let fd = AsyncFd::new(fd)?;
        // Writable once the connection is made or has failed; the readiness is kept, as the
        // socket stays writable for the first request.
        drop(fd.writable().await?);
        let status = getsockopt(fd.get_ref(), sockopt::SocketError)?;
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        Ok(Self { fd })
    }
}

impl AsyncRead for VsockStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut guard = ready!(self.fd.poll_read_ready(cx))?;
            let unfilled = buf.initialize_unfilled();
            match guard.try_io(|fd| {
                recv(fd.get_ref().as_raw_fd(), unfilled, MsgFlags::empty()).map_err(io::Error::from)
            }) {
                Ok(Ok(read)) => {
                    buf.advance(read);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(error)) => return Poll::Ready(Err(error)),
                Err(_would_block) => {}
            }
        }
    }
}

impl AsyncWrite for VsockStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = ready!(self.fd.poll_write_ready(cx))?;
            match guard.try_io(|fd| {
                send(fd.get_ref().as_raw_fd(), buf, MsgFlags::MSG_NOSIGNAL).map_err(io::Error::from)
            }) {
                Ok(result) => return Poll::Ready(result),
                Err(_would_block) => {}
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(
            shutdown(self.fd.get_ref().as_raw_fd(), Shutdown::Write).map_err(io::Error::from),
        )
    }
}

/// The TLS client config of a node VM's Docker API: only the node's CA is trusted, and
/// sherpad presents its client certificate.
fn client_config(tls: &NodeVmTls) -> Result<ClientConfig, ContainerError> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut BufReader::new(tls.ca_pem.as_bytes())) {
        let cert = cert.map_err(|_| ContainerError::NodeVmCertificate("CA certificate"))?;
        roots
            .add(cert)
            .map_err(|_| ContainerError::NodeVmCertificate("CA certificate"))?;
    }
    let chain = rustls_pemfile::certs(&mut BufReader::new(tls.client_cert_pem.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContainerError::NodeVmCertificate("client certificate"))?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(tls.client_key_pem.as_bytes()))
        .map_err(|_| ContainerError::NodeVmCertificate("client key"))?
        .ok_or(ContainerError::NodeVmCertificate("client key"))?;
    ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|_| ContainerError::NodeVmCertificate("TLS versions"))?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .map_err(|_| ContainerError::NodeVmCertificate("client certificate"))
}

/// Send `request` to the Docker API of the VM with context ID `cid`, on a connection of its own
/// that is kept for an upgrade, such as an attach.
async fn send_request(
    cid: u32,
    port: u32,
    connector: TlsConnector,
    server_name: ServerName<'static>,
    request: BollardRequest,
) -> Result<Response<Incoming>, DockerError> {
    let stream = VsockStream::connect(cid, port).await?;
    let tls = connector.connect(server_name, stream).await?;
    let (mut sender, connection) = http1::handshake(TokioIo::new(tls)).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.with_upgrades().await {
            tracing::debug!(%error, "Node VM Docker connection ended");
        }
    });
    let (mut parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));
    parts.uri = path.into();
    parts
        .headers
        .insert(HOST, HeaderValue::from_static("docker"));
    Ok(sender
        .send_request(BollardRequest::from_parts(parts, body))
        .await?)
}

/// A Docker client for the node VM with vsock context ID `cid`, whose dockerd listens on vsock
/// port `port`, authenticated both ways by `tls`. Each request may take up to
/// `timeout_secs`.
///
/// # Errors
///
/// Fails when a certificate or the key in `tls` cannot be read, or the server name is not a
/// valid DNS name.
pub fn node_vm_docker(
    cid: u32,
    port: u32,
    tls: &NodeVmTls,
    timeout_secs: u64,
) -> Result<Docker, ContainerError> {
    let connector = TlsConnector::from(Arc::new(client_config(tls)?));
    let server_name = ServerName::try_from(tls.server_name.clone())
        .map_err(|_| ContainerError::NodeVmCertificate("server name"))?;
    let transport = move |request: BollardRequest| {
        send_request(cid, port, connector.clone(), server_name.clone(), request)
    };
    Docker::connect_with_custom_transport(
        transport,
        Some(NODE_VM_CLIENT_ADDRESS),
        timeout_secs,
        bollard::API_DEFAULT_VERSION,
    )
    .map_err(ContainerError::NodeVmClient)
}
