//! Per-lab Tailscale gateway. All host interaction is through the Docker API.
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, Error, Result, bail, ensure};
use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, DeviceMapping, EndpointIpamConfig,
    EndpointSettings, HostConfig, Mount, MountTypeEnum, NetworkingConfig, VolumeCreateOptions,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptionsBuilder, InspectContainerOptions,
    RemoveContainerOptions, RemoveVolumeOptions, StartContainerOptions, StopContainerOptions,
};
use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;
use tokio::time::{sleep, timeout};
use tracing::instrument;

use shared::data::{TailnetAuthKey, TailnetState, TailnetStatus, TailscaleGatewaySettings};
use shared::konst::TAILNET_DOCS_URL;

use crate::status::is_missing;

const LAB_LABEL: &str = "sh.erpa.tailscale.lab";
const ROLE_LABEL: &str = "sh.erpa.role";
/// The gateway's entrypoint: installs the forwarding rules, then runs the daemon. Its
/// arguments are the daemon's socket path, the addresses the tailnet must not reach, `--`,
/// then the routes.
///
/// The rules live in mangle/FORWARD, before Tailscale's filter/FORWARD accepts, and are
/// installed before the daemon starts, on every container start. `SHERPA-TAILNET` takes what
/// arrives from the tailnet: it drops the refused addresses, returns the routes and drops the
/// rest. `SHERPA-TAILNET-REPLIES` takes what leaves for the tailnet: it returns replies and
/// drops the rest.
const GATEWAY_START_SCRIPT: &str = r#"
socket="$1"; shift
mkdir -p "$(dirname "$socket")"
chmod 0700 /var/lib/tailscale
for tool in iptables-nft ip6tables-nft; do
    $tool -t mangle -N SHERPA-TAILNET
    $tool -t mangle -N SHERPA-TAILNET-REPLIES
done
action=DROP
for address in "$@"; do
    if [ "$address" = -- ]; then action=RETURN; continue; fi
    case $address in *:*) tool=ip6tables-nft;; *) tool=iptables-nft;; esac
    $tool -t mangle -A SHERPA-TAILNET -d "$address" -j $action
done
for tool in iptables-nft ip6tables-nft; do
    $tool -t mangle -A SHERPA-TAILNET -j DROP
    $tool -t mangle -A SHERPA-TAILNET-REPLIES -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN
    $tool -t mangle -A SHERPA-TAILNET-REPLIES -j DROP
    $tool -t mangle -I FORWARD 1 -i tailscale0 -j SHERPA-TAILNET
    $tool -t mangle -I FORWARD 2 -o tailscale0 -j SHERPA-TAILNET-REPLIES
done
exec tailscaled --state=/var/lib/tailscale/tailscaled.state --socket="$socket"
"#;

#[derive(Debug)]
pub struct GatewayConfig {
    pub lab_id: String,
    pub network: String,
    pub ipv4: String,
    pub ipv6: Option<String>,
    pub routes: Vec<String>,
    /// Addresses inside the routes that the tailnet must not reach: the server's and the
    /// router's on the lab's management network.
    pub refused: Vec<IpAddr>,
}

/// The gateway container's name for the lab `lab_id`.
#[must_use]
pub fn gateway_name(lab_id: &str) -> String {
    format!("sherpa-tailnet-{lab_id}")
}

fn volume_name(lab_id: &str) -> String {
    format!("sherpa-tailnet-state-{lab_id}")
}

fn labels(lab_id: &str) -> HashMap<String, String> {
    HashMap::from([
        (LAB_LABEL.into(), lab_id.into()),
        (ROLE_LABEL.into(), "tailnet-gateway".into()),
    ])
}

/// Identify gateways independently of container names or lab ownership.
pub fn is_gateway<S: BuildHasher>(labels: &HashMap<String, String, S>) -> bool {
    labels
        .get(ROLE_LABEL)
        .is_some_and(|value| value == "tailnet-gateway")
}

fn owned(labels: &HashMap<String, String>, lab_id: &str) -> bool {
    labels.get(LAB_LABEL).is_some_and(|value| value == lab_id) && is_gateway(labels)
}

/// The gateway's command: the start script and its arguments.
fn start_command(config: &GatewayConfig, settings: &TailscaleGatewaySettings) -> Vec<String> {
    let mut command = vec![
        GATEWAY_START_SCRIPT.into(),
        "gateway".into(),
        settings.socket_path.clone(),
    ];
    command.extend(config.refused.iter().map(IpAddr::to_string));
    command.push("--".into());
    command.extend(config.routes.iter().cloned());
    command
}

fn container_config(
    config: &GatewayConfig,
    settings: &TailscaleGatewaySettings,
) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(settings.image.clone()),
        hostname: Some(gateway_name(&config.lab_id)),
        labels: Some(labels(&config.lab_id)),
        env: Some(vec!["TS_DEBUG_FIREWALL_MODE=nftables".into()]),
        entrypoint: Some(vec!["sh".into(), "-ec".into()]),
        cmd: Some(start_command(config, settings)),
        host_config: Some(HostConfig {
            cap_add: Some(vec!["NET_ADMIN".into(), "NET_RAW".into()]),
            privileged: Some(false),
            devices: Some(vec![DeviceMapping {
                path_on_host: Some("/dev/net/tun".into()),
                path_in_container: Some("/dev/net/tun".into()),
                cgroup_permissions: Some("rw".into()),
            }]),
            sysctls: Some(HashMap::from([
                ("net.ipv4.ip_forward".into(), "1".into()),
                ("net.ipv6.conf.all.forwarding".into(), "1".into()),
            ])),
            mounts: Some(vec![Mount {
                target: Some("/var/lib/tailscale".into()),
                source: Some(volume_name(&config.lab_id)),
                typ: Some(MountTypeEnum::VOLUME),
                ..Default::default()
            }]),
            tmpfs: Some(HashMap::from([(
                "/run".into(),
                "rw,noexec,nosuid,size=16m,mode=0700".into(),
            )])),
            ..Default::default()
        }),
        networking_config: Some(NetworkingConfig {
            endpoints_config: Some(HashMap::from([(
                config.network.clone(),
                EndpointSettings {
                    ipam_config: Some(EndpointIpamConfig {
                        ipv4_address: Some(config.ipv4.clone()),
                        ipv6_address: config.ipv6.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )])),
        }),
        ..Default::default()
    }
}

/// Create an identity volume and gateway, but do not enroll until its ID is persisted.
///
/// # Errors
///
/// Fails when the lab already has Tailscale state, or Docker cannot pull the image or create
/// the volume or the container.
#[instrument(skip(docker), fields(lab_id = %config.lab_id))]
pub async fn create_gateway(
    docker: &Docker,
    config: &GatewayConfig,
    settings: &TailscaleGatewaySettings,
) -> Result<String> {
    // Never adopt an existing identity when creating a new lab.
    match docker.inspect_volume(&volume_name(&config.lab_id)).await {
        Ok(_) => {
            bail!("Tailscale state already exists for this lab; run lab cleanup before retrying")
        }
        Err(error) if is_missing(&error) => {}
        Err(error) => return Err(error).context("Unable to inspect Tailscale state volume"),
    }
    match docker.inspect_image(&settings.image).await {
        Ok(_) => {}
        Err(error) if is_missing(&error) => {
            let options = CreateImageOptionsBuilder::default()
                .from_image(&settings.image)
                .build();
            let mut stream = docker.create_image(Some(options), None, None);
            while let Some(info) = stream
                .try_next()
                .await
                .context("Unable to pull Tailscale image")?
            {
                ensure!(info.error.is_none(), "Tailscale image pull failed");
            }
        }
        Err(error) => return Err(error).context("Unable to inspect Tailscale image"),
    }
    docker
        .create_volume(VolumeCreateOptions {
            name: Some(volume_name(&config.lab_id)),
            labels: Some(labels(&config.lab_id)),
            ..Default::default()
        })
        .await
        .context("Unable to create Tailscale identity volume")?;
    let response = docker
        .create_container(
            Some(CreateContainerOptions {
                name: Some(gateway_name(&config.lab_id)),
                ..Default::default()
            }),
            container_config(config, settings),
        )
        .await
        .context("Unable to create Tailscale gateway")?;
    Ok(response.id)
}

#[instrument(skip(docker), level = "debug")]
async fn inspect_owned(
    docker: &Docker,
    id: &str,
    lab_id: &str,
) -> Result<ContainerInspectResponse> {
    let details = docker
        .inspect_container(id, None::<InspectContainerOptions>)
        .await
        .context("Unable to inspect Tailscale gateway")?;
    ensure!(
        details
            .config
            .as_ref()
            .and_then(|c| c.labels.as_ref())
            .is_some_and(|l| owned(l, lab_id)),
        "Refusing to operate on a gateway belonging to another lab"
    );
    Ok(details)
}

// Secrets travel over the attached stdin stream, never in Docker exec metadata.
// Output from enrollment is discarded because upstream diagnostics could echo input.
#[instrument(skip(docker, stdin), level = "debug")]
async fn exec(
    docker: &Docker,
    id: &str,
    cmd: Vec<String>,
    stdin: Option<&str>,
    settings: &TailscaleGatewaySettings,
) -> Result<String> {
    timeout(Duration::from_secs(settings.exec_timeout_secs.into()), async {
        let created = docker.create_exec(id, CreateExecOptions {
            cmd: Some(cmd),
            attach_stdin: Some(stdin.is_some()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        }).await.context("Unable to create Tailscale command")?;
        let result = docker.start_exec(&created.id, None).await.context("Unable to start Tailscale command")?;
        let mut bytes = Vec::new();
        if let StartExecResults::Attached { mut input, mut output } = result {
            if let Some(secret) = stdin {
                input.write_all(secret.as_bytes()).await.context("Unable to send enrollment credential")?;
                input.shutdown().await.context("Unable to finish enrollment input")?;
            }
            while let Some(chunk) = output.try_next().await.context("Unable to read Tailscale command status")? {
                if stdin.is_none() {
                    ensure!(bytes.len() + chunk.as_ref().len() <= 1024 * 1024, "Tailscale response exceeds size limit");
                    bytes.extend_from_slice(chunk.as_ref());
                }
            }
        } else {
            bail!("Tailscale command did not attach");
        }
        let status = docker.inspect_exec(&created.id).await.context("Unable to inspect Tailscale command")?;
        ensure!(status.exit_code == Some(0), "Tailscale command failed (exit {:?}); check key validity, device approval, and connectivity", status.exit_code);
        String::from_utf8(bytes).context("Tailscale command returned invalid UTF-8")
    }).await.context("Tailscale command timed out")?
}

async fn wait_for_socket(
    docker: &Docker,
    id: &str,
    settings: &TailscaleGatewaySettings,
) -> Result<()> {
    timeout(
        Duration::from_secs(settings.daemon_ready_timeout_secs.into()),
        async {
            loop {
                if exec(
                    docker,
                    id,
                    vec!["test".into(), "-S".into(), settings.socket_path.clone()],
                    None,
                    settings,
                )
                .await
                .is_ok()
                {
                    return Ok::<(), Error>(());
                }
                sleep(Duration::from_millis(250)).await;
            }
        },
    )
    .await
    .context("Tailscale daemon did not become ready")?
}

fn enrollment_command(routes: &[String], settings: &TailscaleGatewaySettings) -> Vec<String> {
    let script = r#"umask 077; trap 'rm -f /run/enrollment-key' EXIT; cat > /run/enrollment-key; tailscale "$1" up --auth-key=file:/run/enrollment-key --timeout="$3" --accept-dns=false --accept-routes=false --snat-subnet-routes=true --stateful-filtering=true --advertise-routes="$2""#;
    vec![
        "sh".into(),
        "-c".into(),
        script.into(),
        "enroll".into(),
        format!("--socket={}", settings.socket_path),
        routes.join(","),
        format!("{}s", settings.enrollment_timeout_secs),
    ]
}

/// Start the gateway and log it in with `key`. Also used on resume, as stopping logs out.
///
/// # Errors
///
/// Fails when the gateway belongs to another lab, does not start, cannot be enrolled with
/// `key`, or does not connect in time.
#[instrument(skip(docker, key, settings))]
pub async fn enroll_gateway(
    docker: &Docker,
    id: &str,
    lab_id: &str,
    routes: &[String],
    key: &TailnetAuthKey,
    settings: &TailscaleGatewaySettings,
) -> Result<TailnetStatus> {
    inspect_owned(docker, id, lab_id).await?;
    docker
        .start_container(id, None::<StartContainerOptions>)
        .await
        .context("Unable to start Tailscale gateway")?;
    wait_for_socket(docker, id, settings).await?;
    exec(
        docker,
        id,
        vec!["chmod".into(), "0700".into(), "/var/lib/tailscale".into()],
        None,
        settings,
    )
    .await?;
    exec(
        docker,
        id,
        enrollment_command(routes, settings),
        Some(key.expose()),
        settings,
    )
    .await
    .context("Tailscale enrollment failed")?;
    wait_connected(docker, id, lab_id, routes.to_vec(), settings).await
}

async fn wait_connected(
    docker: &Docker,
    id: &str,
    lab_id: &str,
    routes: Vec<String>,
    settings: &TailscaleGatewaySettings,
) -> Result<TailnetStatus> {
    timeout(Duration::from_secs(settings.connection_timeout_secs.into()), async {
        loop {
            let status = gateway_status(docker, id, lab_id, routes.clone(), settings).await?;
            match status.state {
                TailnetState::Connected => return Ok(status),
                TailnetState::NeedsAuthentication => bail!("Tailscale gateway requires reauthentication or device approval; see {TAILNET_DOCS_URL}#recovery"),
                _ => sleep(Duration::from_millis(500)).await,
            }
        }
    }).await.context("Tailscale gateway did not connect")?
}

/// The gateway's tailnet status, or a stopped status when it is not running.
///
/// # Errors
///
/// Fails when the gateway belongs to another lab, or its status cannot be read.
#[instrument(skip(docker), level = "debug")]
pub async fn gateway_status(
    docker: &Docker,
    id: &str,
    lab_id: &str,
    routes: Vec<String>,
    settings: &TailscaleGatewaySettings,
) -> Result<TailnetStatus> {
    let details = inspect_owned(docker, id, lab_id).await?;
    if details
        .state
        .as_ref()
        .is_none_or(|s| s.running != Some(true))
    {
        let mut status = TailnetStatus::unavailable(routes);
        status.state = TailnetState::Stopped;
        status.warnings.clear();
        return Ok(status);
    }
    let raw = exec(
        docker,
        id,
        vec![
            "tailscale".into(),
            format!("--socket={}", settings.socket_path),
            "status".into(),
            "--json".into(),
            "--peers=false".into(),
        ],
        None,
        settings,
    )
    .await?;
    TailnetStatus::from_daemon(&raw, routes)
}

fn logout_command(settings: &TailscaleGatewaySettings) -> Vec<String> {
    vec![
        "tailscale".into(),
        format!("--socket={}", settings.socket_path),
        "logout".into(),
    ]
}

/// Log a running gateway out, which removes an ephemeral device from the tailnet at once.
/// A failure is only logged: Tailscale removes an offline ephemeral device by itself later.
async fn logout(docker: &Docker, id: &str, settings: &TailscaleGatewaySettings) {
    match exec(docker, id, logout_command(settings), None, settings).await {
        Ok(_) => tracing::info!("Logged Tailscale gateway out"),
        Err(error) => {
            tracing::warn!(error = ?error, "Unable to log Tailscale gateway out; continuing");
        }
    }
}

/// Log a running gateway out, then stop it. The next resume enrolls it again.
///
/// # Errors
///
/// Fails when the gateway belongs to another lab or Docker cannot stop it.
#[instrument(skip(docker))]
pub async fn stop_gateway(
    docker: &Docker,
    id: &str,
    lab_id: &str,
    settings: &TailscaleGatewaySettings,
) -> Result<()> {
    let details = inspect_owned(docker, id, lab_id).await?;
    if details
        .state
        .as_ref()
        .is_some_and(|s| s.running == Some(true))
    {
        logout(docker, id, settings).await;
        match docker
            .stop_container(
                id,
                Some(StopContainerOptions {
                    t: Some(
                        i32::try_from(settings.stop_timeout_secs)
                            .context("tailscale.stop_timeout_secs is too large")?,
                    ),
                    ..Default::default()
                }),
            )
            .await
        {
            // 304: the gateway had stopped meanwhile.
            Ok(())
            | Err(DockerError::DockerResponseServerError {
                status_code: 304, ..
            }) => {}
            Err(error) => return Err(error).context("Unable to stop Tailscale gateway"),
        }
    }
    Ok(())
}

/// Cleanup uses exact names plus ownership labels, including partially created labs.
///
/// # Errors
///
/// Fails when the gateway or its volume belongs to another lab, or Docker cannot remove them.
#[instrument(skip(docker, settings))]
pub async fn remove_gateway(
    docker: &Docker,
    lab_id: &str,
    settings: &TailscaleGatewaySettings,
) -> Result<()> {
    let name = gateway_name(lab_id);
    match docker
        .inspect_container(&name, None::<InspectContainerOptions>)
        .await
    {
        Ok(details) => {
            ensure!(
                details
                    .config
                    .as_ref()
                    .and_then(|c| c.labels.as_ref())
                    .is_some_and(|l| owned(l, lab_id)),
                "Refusing to remove an unowned Tailscale gateway"
            );
            let id = details
                .id
                .context("Tailscale gateway missing container ID")?;
            if details
                .state
                .as_ref()
                .is_some_and(|s| s.running == Some(true))
            {
                logout(docker, &id, settings).await;
            }
            match docker
                .remove_container(
                    &id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
            {
                Ok(()) => {}
                Err(error) if is_missing(&error) => {}
                Err(error) => return Err(error).context("Unable to remove Tailscale gateway"),
            }
        }
        Err(error) if is_missing(&error) => {}
        Err(error) => return Err(error).context("Unable to inspect Tailscale gateway for cleanup"),
    }
    let volume = volume_name(lab_id);
    match docker.inspect_volume(&volume).await {
        Ok(details) => {
            ensure!(
                owned(&details.labels, lab_id),
                "Refusing to remove an unowned Tailscale identity volume"
            );
            match docker
                .remove_volume(&volume, None::<RemoveVolumeOptions>)
                .await
            {
                Ok(()) => {}
                Err(error) if is_missing(&error) => {}
                Err(error) => {
                    return Err(error).context("Unable to remove Tailscale identity volume");
                }
            }
        }
        Err(error) if is_missing(&error) => {}
        Err(error) => return Err(error).context("Unable to inspect Tailscale identity volume"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{Ipam, IpamConfig, NetworkCreateRequest};
    use bollard::query_parameters::{InspectNetworkOptions, LogsOptions};
    use shared::util::load_config;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Mutex, PoisonError};
    use std::thread;
    use tokio::runtime::Runtime;

    /// A scripted Docker API: answers each expected request in turn, and fails the test on any
    /// request beyond them.
    struct MockDocker {
        handle: thread::JoinHandle<()>,
        done: Sender<()>,
    }

    impl MockDocker {
        /// Check that every scripted request arrived and no other did.
        fn finish(self) {
            self.done.send(()).unwrap();
            self.handle.join().unwrap();
        }
    }

    fn test_settings() -> Result<TailscaleGatewaySettings> {
        match std::env::var("SHERPA_TEST_SERVER_CONFIG") {
            Ok(path) => Ok(load_config(&path)?.tailscale),
            Err(_) => Ok(TailscaleGatewaySettings::default()),
        }
    }

    #[test]
    fn tailnet_settings_004_the_configured_image_socket_and_enrollment_timeout_reach_the_gateway() {
        // Configuration must be passed as an argument, including shell metacharacters.
        let settings = TailscaleGatewaySettings {
            image: "registry.example/tailscale:custom".into(),
            socket_path: "/run/custom/daemon;literal.sock".into(),
            enrollment_timeout_secs: 120,
            ..Default::default()
        };
        let config = GatewayConfig {
            lab_id: "test1234".into(),
            network: "management".into(),
            ipv4: "192.0.2.3".into(),
            ipv6: None,
            routes: vec!["192.0.2.0/24".into()],
            refused: Vec::new(),
        };
        let body = container_config(&config, &settings);
        assert_eq!(body.image.as_deref(), Some(settings.image.as_str()));
        let command = body.cmd.unwrap();
        assert_eq!(command[2], settings.socket_path);
        assert!(command[3..].contains(&config.routes[0]));
        assert!(!command[0].contains(&settings.socket_path));
        let enrollment = enrollment_command(&config.routes, &settings);
        assert_eq!(enrollment[4], format!("--socket={}", settings.socket_path));
        assert_eq!(enrollment[5], config.routes.join(","));
        assert_eq!(enrollment[6], "120s");
        assert!(!enrollment[2].contains(&settings.socket_path));
    }

    #[tokio::test]
    async fn tailnet_settings_005_a_gateway_command_past_the_command_timeout_fails_as_timed_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mock = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut headers = Vec::new();
            let mut byte = [0];
            while !headers.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
            }
            thread::sleep(Duration::from_millis(1300));
            let body = r#"{"message":"delayed response"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        let docker = Docker::connect_with_http(
            &format!("http://{address}"),
            5,
            bollard::API_DEFAULT_VERSION,
        )
        .unwrap();
        let settings = TailscaleGatewaySettings {
            exec_timeout_secs: 1,
            ..Default::default()
        };
        let result = exec(&docker, "gateway-id", vec!["test".into()], None, &settings).await;
        mock.join().unwrap();
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Tailscale command timed out")
        );
    }

    const RUNNING_GATEWAY: &str = r#"{"Id":"exact-id","State":{"Running":true},"Config":{"Labels":{"sh.erpa.tailscale.lab":"test1234","sh.erpa.role":"tailnet-gateway"}}}"#;

    #[tokio::test]
    async fn tailnet_settings_006_the_stop_timeout_is_the_grace_period_docker_is_given() {
        let (docker, mock) = mock_docker(vec![
            ("/containers/exact-id/json", "200 OK", RUNNING_GATEWAY),
            (
                "/containers/exact-id/exec",
                "500 Internal Server Error",
                r#"{"message":"exec unavailable"}"#,
            ),
            ("/containers/exact-id/stop?t=17", "204 No Content", ""),
        ]);
        let settings = TailscaleGatewaySettings {
            stop_timeout_secs: 17,
            ..Default::default()
        };
        let result = stop_gateway(&docker, "exact-id", "test1234", &settings).await;
        mock.finish();
        result.unwrap();
    }

    #[tokio::test]
    async fn tailnet_suspend_004_a_gateway_whose_logout_fails_is_still_stopped() {
        let (docker, mock) = mock_docker(vec![
            ("/containers/exact-id/json", "200 OK", RUNNING_GATEWAY),
            (
                "/containers/exact-id/exec",
                "500 Internal Server Error",
                r#"{"message":"exec unavailable"}"#,
            ),
            ("/containers/exact-id/stop", "204 No Content", ""),
        ]);
        let result = stop_gateway(
            &docker,
            "exact-id",
            "test1234",
            &TailscaleGatewaySettings::default(),
        )
        .await;
        mock.finish();
        result.unwrap();
    }

    #[tokio::test]
    async fn tailnet_removal_002_a_running_gateway_is_logged_out_and_removed_even_when_logout_fails()
     {
        let (docker, mock) = mock_docker(vec![
            (
                "/containers/sherpa-tailnet-test1234/json",
                "200 OK",
                RUNNING_GATEWAY,
            ),
            (
                "/containers/exact-id/exec",
                "500 Internal Server Error",
                r#"{"message":"exec unavailable"}"#,
            ),
            ("/containers/exact-id?", "204 No Content", ""),
            (
                "/volumes/sherpa-tailnet-state-test1234",
                "404 Not Found",
                r#"{"message":"missing"}"#,
            ),
        ]);
        remove_gateway(&docker, "test1234", &TailscaleGatewaySettings::default())
            .await
            .unwrap();
        mock.finish();
    }

    fn mock_docker(
        responses: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (Docker, MockDocker) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (done, finished) = channel();
        let handle = thread::spawn(move || {
            for (path, status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(
                    request.lines().next().unwrap().contains(path),
                    "unexpected request: {request}"
                );
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            finished.recv().unwrap();
            listener.set_nonblocking(true).unwrap();
            if let Ok((mut stream, _)) = listener.accept() {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut line = Vec::new();
                let mut byte = [0];
                while !line.ends_with(b"\r\n") && stream.read_exact(&mut byte).is_ok() {
                    line.push(byte[0]);
                }
                panic!(
                    "unexpected request: {}",
                    String::from_utf8_lossy(&line).trim_end()
                );
            }
        });
        let docker = Docker::connect_with_http(
            &format!("http://{address}"),
            5,
            bollard::API_DEFAULT_VERSION,
        )
        .unwrap();
        (docker, MockDocker { handle, done })
    }

    #[tokio::test]
    async fn tailnet_removal_003_removing_the_gateway_of_a_lab_that_has_none_succeeds() {
        let (docker, mock) = mock_docker(vec![
            (
                "/containers/sherpa-tailnet-test1234/json",
                "404 Not Found",
                r#"{"message":"missing"}"#,
            ),
            (
                "/volumes/sherpa-tailnet-state-test1234",
                "404 Not Found",
                r#"{"message":"missing"}"#,
            ),
        ]);
        remove_gateway(&docker, "test1234", &TailscaleGatewaySettings::default())
            .await
            .unwrap();
        mock.finish();
    }

    #[tokio::test]
    async fn tailnet_removal_004_a_gateway_with_another_labs_labels_is_refused_and_nothing_removed()
    {
        let (docker, mock) = mock_docker(vec![(
            "/containers/sherpa-tailnet-test1234/json",
            "200 OK",
            r#"{"Id":"foreign","Config":{"Labels":{"sh.erpa.tailscale.lab":"another","sh.erpa.role":"tailnet-gateway"}}}"#,
        )]);
        assert!(
            remove_gateway(&docker, "test1234", &TailscaleGatewaySettings::default())
                .await
                .unwrap_err()
                .to_string()
                .contains("unowned")
        );
        mock.finish();
    }

    #[tokio::test]
    async fn tailnet_suspend_005_a_stopped_gateway_is_neither_logged_out_nor_stopped_again() {
        let (docker, mock) = mock_docker(vec![(
            "/containers/exact-id/json",
            "200 OK",
            r#"{"Id":"exact-id","State":{"Running":false},"Config":{"Labels":{"sh.erpa.tailscale.lab":"test1234","sh.erpa.role":"tailnet-gateway"}}}"#,
        )]);
        stop_gateway(
            &docker,
            "exact-id",
            "test1234",
            &TailscaleGatewaySettings::default(),
        )
        .await
        .unwrap();
        mock.finish();
    }

    #[tokio::test]
    async fn tailnet_removal_005_the_identity_volume_stays_when_the_gateway_cannot_be_removed() {
        let (docker, mock) = mock_docker(vec![
            (
                "/containers/sherpa-tailnet-test1234/json",
                "200 OK",
                r#"{"Id":"exact-id","Config":{"Labels":{"sh.erpa.tailscale.lab":"test1234","sh.erpa.role":"tailnet-gateway"}}}"#,
            ),
            (
                "/containers/exact-id?",
                "500 Internal Server Error",
                r#"{"message":"daemon failure"}"#,
            ),
        ]);
        assert!(
            remove_gateway(&docker, "test1234", &TailscaleGatewaySettings::default())
                .await
                .is_err()
        );
        mock.finish();
    }

    #[tokio::test]
    async fn tailnet_status_006_only_the_gateways_own_status_is_read_on_a_large_tailnet() {
        let settings = TailscaleGatewaySettings {
            socket_path: "/run/overridden/status.sock".into(),
            ..Default::default()
        };
        let own_output = r#"{"BackendState":"Running","TailscaleIPs":["100.64.0.1"],"Self":{"AllowedIPs":["192.0.2.0/24"]}}"#;
        let capabilities = [r#""https://tailscale.com/cap/file-sharing""#; 10].join(",");
        let peers = (0..3000)
            .map(|index| {
                format!(r#""nodekey:{index:064x}":{{"HostName":"device-{index}","DNSName":"device-{index}.example.ts.net.","OS":"linux","Online":true,"Capabilities":[{capabilities}],"AllowedIPs":["100.64.0.2/32"]}}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        let full_output = format!(
            r#"{},"Peer":{{{peers}}}}}"#,
            own_output.trim_end().strip_suffix('}').unwrap()
        );
        assert!(full_output.len() > 1024 * 1024);
        TailnetStatus::from_daemon(&full_output, vec!["192.0.2.0/24".into()]).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let mock = thread::spawn(move || {
            let mut own_only = false;
            while !server_stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("Unable to accept mock Docker request: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut headers = Vec::new();
                let mut byte = [0];
                while !headers.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    headers.push(byte[0]);
                }
                let headers = String::from_utf8(headers).unwrap();
                let length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                let request = headers.lines().next().unwrap();
                let response = if request.contains("/containers/gateway-id/json") {
                    r#"{"Id":"gateway-id","State":{"Running":true},"Config":{"Labels":{"sh.erpa.tailscale.lab":"test1234","sh.erpa.role":"tailnet-gateway"}}}"#
                } else if request.contains("/containers/gateway-id/exec") {
                    let body = String::from_utf8(body).unwrap();
                    own_only = body.contains(r#""--peers=false""#)
                        && body.contains("--socket=/run/overridden/status.sock");
                    r#"{"Id":"status-id"}"#
                } else if request.contains("/exec/status-id/start") {
                    stream
                        .write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\n")
                        .unwrap();
                    let output = if own_only { own_output } else { &full_output };
                    let mut frame = vec![1, 0, 0, 0];
                    frame.extend_from_slice(&u32::try_from(output.len()).unwrap().to_be_bytes());
                    frame.extend_from_slice(output.as_bytes());
                    // The original bug closes the stream upon reaching the response limit.
                    let _ = stream.write_all(&frame);
                    continue;
                } else if request.contains("/exec/status-id/json") {
                    r#"{"Running":false,"ExitCode":0}"#
                } else {
                    panic!("Unexpected mock Docker request: {request}");
                };
                let body = response;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let docker = Docker::connect_with_http(
            &format!("http://{address}"),
            5,
            bollard::API_DEFAULT_VERSION,
        )
        .unwrap();
        let result = gateway_status(
            &docker,
            "gateway-id",
            "test1234",
            vec!["192.0.2.0/24".into()],
            &settings,
        )
        .await;
        stop.store(true, Ordering::SeqCst);
        mock.join().unwrap();
        let status = result.expect("a large tailnet must not prevent gateway status checks");
        assert_eq!(status.state, TailnetState::Connected);
        assert_eq!(status.warnings, [] as [std::string::String; 0]);
        assert_eq!(status.tailscale_ips, vec!["100.64.0.1"]);
    }

    fn live_config() -> GatewayConfig {
        GatewayConfig {
            lab_id: format!("tntest{}", std::process::id()),
            network: std::env::var("SHERPA_TEST_TAILSCALE_NETWORK")
                .expect("set a disposable Docker network"),
            ipv4: std::env::var("SHERPA_TEST_TAILSCALE_IPV4")
                .expect("set an unused gateway address"),
            ipv6: std::env::var("SHERPA_TEST_TAILSCALE_IPV6").ok(),
            routes: std::env::var("SHERPA_TEST_TAILSCALE_ROUTES")
                .expect("set comma-separated management routes")
                .split(',')
                .map(str::to_owned)
                .collect(),
            refused: Vec::new(),
        }
    }

    /// A gateway created on the live test network, not yet enrolled.
    struct LiveGateway {
        docker: Docker,
        settings: TailscaleGatewaySettings,
        config: GatewayConfig,
        key: TailnetAuthKey,
        id: String,
    }

    impl LiveGateway {
        async fn create() -> Result<Self> {
            let config = live_config();
            let settings = test_settings()?;
            let docker = Docker::connect_with_local_defaults()?;
            let key = TailnetAuthKey::new(std::env::var("SHERPA_TEST_TAILSCALE_AUTH_KEY")?);
            let id = match create_gateway(&docker, &config, &settings).await {
                Ok(id) => id,
                Err(error) => {
                    remove_gateway(&docker, &config.lab_id, &settings).await?;
                    return Err(error);
                }
            };
            Ok(Self {
                docker,
                settings,
                config,
                key,
                id,
            })
        }

        async fn enroll(&self) -> Result<TailnetStatus> {
            enroll_gateway(
                &self.docker,
                &self.id,
                &self.config.lab_id,
                &self.config.routes,
                &self.key,
                &self.settings,
            )
            .await
        }

        /// Remove the gateway and its volume, then report `result`.
        async fn remove(self, result: Result<()>) -> Result<()> {
            let cleanup = remove_gateway(&self.docker, &self.config.lab_id, &self.settings).await;
            result?;
            cleanup
        }
    }

    /// The live tailnet tests share one gateway address on one network, so they take turns.
    static LIVE_TAILNET: Mutex<()> = Mutex::new(());

    /// Run `test` on a fresh gateway on the live test network, one live test at a time.
    fn run_live_tailnet(test: impl AsyncFnOnce(&LiveGateway) -> Result<()>) -> Result<()> {
        let _turn = LIVE_TAILNET.lock().unwrap_or_else(PoisonError::into_inner);
        Runtime::new()?.block_on(async {
            let gateway = LiveGateway::create().await?;
            let result = test(&gateway).await;
            gateway.remove(result).await
        })
    }

    #[test]
    #[ignore = "requires a disposable Docker network, Linux TUN, and a test tailnet auth key"]
    fn tailnet_join_003_a_gateway_enrolled_with_a_valid_key_reports_connected() -> Result<()> {
        run_live_tailnet(async |gateway| {
            let joined = gateway.enroll().await?;
            ensure!(
                joined.state == TailnetState::Connected,
                "gateway did not connect"
            );
            Ok(())
        })
    }

    #[test]
    #[ignore = "requires a disposable Docker network, Linux TUN, and a test tailnet auth key"]
    fn tailnet_suspend_003_a_stopped_gateway_reports_stopped_and_enrolled_again_connected()
    -> Result<()> {
        run_live_tailnet(async |gateway| {
            gateway.enroll().await?;
            stop_gateway(
                &gateway.docker,
                &gateway.id,
                &gateway.config.lab_id,
                &gateway.settings,
            )
            .await?;
            let stopped = gateway_status(
                &gateway.docker,
                &gateway.id,
                &gateway.config.lab_id,
                gateway.config.routes.clone(),
                &gateway.settings,
            )
            .await?;
            ensure!(
                stopped.state == TailnetState::Stopped,
                "gateway did not stop"
            );
            let resumed = gateway.enroll().await?;
            ensure!(
                resumed.state == TailnetState::Connected,
                "gateway did not reconnect"
            );
            Ok(())
        })
    }

    #[test]
    fn tailnet_join_006_the_gateway_is_unprivileged_with_tun_and_only_the_management_network() {
        let config = GatewayConfig {
            lab_id: "12345678".into(),
            network: "management".into(),
            ipv4: "192.0.2.3".into(),
            ipv6: None,
            routes: vec!["192.0.2.0/24".into()],
            refused: Vec::new(),
        };
        let body = container_config(&config, &TailscaleGatewaySettings::default());
        let host = body.host_config.unwrap();
        assert_eq!(host.privileged, Some(false));
        assert!(host.network_mode.is_none());
        assert_eq!(host.devices.unwrap().len(), 1);
        assert_eq!(host.cap_add.unwrap(), vec!["NET_ADMIN", "NET_RAW"]);
        let networks = body.networking_config.unwrap().endpoints_config.unwrap();
        assert_eq!(networks.len(), 1);
        assert!(networks.contains_key("management"));
        assert!(owned(&body.labels.unwrap(), "12345678"));
        assert!(!owned(&labels("12345678"), "abcdefgh"));
    }
    /// A started gateway, with its daemon up, on a dual-stack Docker network of its own.
    struct SmokeGateway {
        docker: Docker,
        settings: TailscaleGatewaySettings,
        config: GatewayConfig,
        network_id: String,
        id: String,
    }

    impl SmokeGateway {
        /// Start a gateway for the lab `tnsmoke<index><pid>`; `index` keeps the networks of
        /// tests running at the same time apart.
        async fn start(index: u16) -> Result<Self> {
            let docker = Docker::connect_with_local_defaults()?;
            let settings = test_settings()?;
            let lab_id = format!("tnsmoke{index}{}", std::process::id());
            let network = format!("sherpa-{lab_id}");
            let ipv6_base = format!("fdf{index:x}:{:x}::", std::process::id() % 65536);
            let created = docker
                .create_network(NetworkCreateRequest {
                    name: network.clone(),
                    driver: Some("bridge".into()),
                    enable_ipv6: Some(true),
                    ipam: Some(Ipam {
                        config: Some(vec![IpamConfig {
                            subnet: Some(format!("{ipv6_base}/64")),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .await?;
            let config =
                match smoke_config(&docker, &lab_id, &network, &created.id, &ipv6_base).await {
                    Ok(config) => config,
                    Err(error) => {
                        docker.remove_network(&created.id).await?;
                        return Err(error);
                    }
                };
            let mut gateway = Self {
                docker,
                settings,
                config,
                network_id: created.id,
                id: String::new(),
            };
            if let Err(error) = gateway.create_and_start().await {
                return Err(gateway.remove(Err(error)).await.unwrap_err());
            }
            Ok(gateway)
        }

        async fn create_and_start(&mut self) -> Result<()> {
            self.id = create_gateway(&self.docker, &self.config, &self.settings).await?;
            self.docker
                .start_container(&self.id, None::<StartContainerOptions>)
                .await?;
            if let Err(error) = wait_for_socket(&self.docker, &self.id, &self.settings).await {
                print_gateway_logs(&self.docker, &self.id).await?;
                return Err(error);
            }
            Ok(())
        }

        async fn exec(&self, cmd: &[&str], stdin: Option<&str>) -> Result<String> {
            exec(
                &self.docker,
                &self.id,
                cmd.iter().map(|&part| part.to_owned()).collect(),
                stdin,
                &self.settings,
            )
            .await
        }

        /// Check that the forwarding chain accepts the gateway's routes and drops the rest.
        async fn check_forwarding_rules(&self) -> Result<()> {
            for (tool, route) in [
                ("iptables-nft", &self.config.routes[0]),
                ("ip6tables-nft", &self.config.routes[1]),
            ] {
                let rules = self.mangle_rules(tool, "SHERPA-TAILNET").await?;
                ensure!(
                    rules.contains(route.as_str()) && rules.contains("DROP"),
                    "missing {tool} forwarding restriction: {rules}"
                );
            }
            Ok(())
        }

        /// The rules of `chain` in `tool`'s mangle table in the gateway.
        async fn mangle_rules(&self, tool: &str, chain: &str) -> Result<String> {
            self.exec(&[tool, "-t", "mangle", "-S", chain], None).await
        }

        /// Remove the gateway, its volume and its network, then report `result`.
        async fn remove(self, result: Result<()>) -> Result<()> {
            let cleanup = remove_gateway(&self.docker, &self.config.lab_id, &self.settings).await;
            let network_cleanup = self.docker.remove_network(&self.network_id).await;
            result?;
            cleanup?;
            network_cleanup?;
            Ok(())
        }
    }

    /// The gateway configuration for a test network whose IPv6 /64 starts `ipv6_base`: the
    /// third address of its IPv4 subnet and of the /64, advertising both, and refusing the
    /// first and second addresses of each.
    async fn smoke_config(
        docker: &Docker,
        lab_id: &str,
        network: &str,
        network_id: &str,
        ipv6_base: &str,
    ) -> Result<GatewayConfig> {
        let details = docker
            .inspect_network(network_id, None::<InspectNetworkOptions>)
            .await?;
        let subnet = details
            .ipam
            .and_then(|i| i.config)
            .and_then(|c| {
                c.into_iter()
                    .find(|c| c.subnet.as_ref().is_some_and(|s| !s.contains(':')))
            })
            .context("test network missing IPAM")?;
        let gateway = subnet.gateway.context("test network missing gateway")?;
        let (prefix, _) = gateway
            .rsplit_once('.')
            .context("test network must use IPv4")?;
        Ok(GatewayConfig {
            lab_id: lab_id.to_string(),
            network: network.to_string(),
            ipv4: format!("{prefix}.3"),
            ipv6: Some(format!("{ipv6_base}3")),
            routes: vec![
                subnet.subnet.context("test network missing subnet")?,
                format!("{ipv6_base}/64"),
            ],
            refused: vec![
                format!("{prefix}.1").parse()?,
                format!("{prefix}.2").parse()?,
                format!("{ipv6_base}1").parse()?,
                format!("{ipv6_base}2").parse()?,
            ],
        })
    }

    #[tokio::test]
    #[ignore = "requires Linux Docker and TUN; creates and removes a disposable network"]
    async fn tailnet_routes_002_the_forwarding_rules_accept_the_routes_and_drop_the_rest()
    -> Result<()> {
        let gateway = SmokeGateway::start(0).await?;
        let result = gateway.check_forwarding_rules().await;
        gateway.remove(result).await
    }

    #[tokio::test]
    #[ignore = "requires Linux Docker and TUN; creates and removes a disposable network"]
    async fn tailnet_routes_006_traffic_from_the_tailnet_to_the_server_and_the_router_is_dropped_before_the_routes_are_accepted()
    -> Result<()> {
        let gateway = SmokeGateway::start(3).await?;
        let result = async {
            for (tool, host_prefix, route, refused) in [
                (
                    "iptables-nft",
                    "/32",
                    &gateway.config.routes[0],
                    &gateway.config.refused[..2],
                ),
                (
                    "ip6tables-nft",
                    "/128",
                    &gateway.config.routes[1],
                    &gateway.config.refused[2..],
                ),
            ] {
                let rules = gateway.mangle_rules(tool, "SHERPA-TAILNET").await?;
                let position = |rule: String| rules.lines().position(|line| line.ends_with(&rule));
                let accepted = position(format!("-d {route} -j RETURN"))
                    .with_context(|| format!("{tool} does not return {route}: {rules}"))?;
                for address in refused {
                    let dropped = position(format!("-d {address}{host_prefix} -j DROP"))
                        .with_context(|| format!("{tool} does not drop {address}: {rules}"))?;
                    ensure!(
                        dropped < accepted,
                        "{tool} returns {route} before it drops {address}: {rules}"
                    );
                }
            }
            Ok(())
        }
        .await;
        gateway.remove(result).await
    }

    #[tokio::test]
    #[ignore = "requires Linux Docker and TUN; creates and removes a disposable network"]
    async fn tailnet_routes_007_only_replies_are_forwarded_from_the_lab_to_the_tailnet()
    -> Result<()> {
        let gateway = SmokeGateway::start(4).await?;
        let result = async {
            for tool in ["iptables-nft", "ip6tables-nft"] {
                let forward = gateway.mangle_rules(tool, "FORWARD").await?;
                ensure!(
                    forward.lines().any(|line| line == "-A FORWARD -o tailscale0 -j SHERPA-TAILNET-REPLIES"),
                    "{tool} does not filter what leaves for the tailnet: {forward}"
                );
                let rules = gateway.mangle_rules(tool, "SHERPA-TAILNET-REPLIES").await?;
                let rules: Vec<&str> = rules.lines().filter(|line| line.starts_with("-A ")).collect();
                ensure!(
                    rules
                        == [
                            "-A SHERPA-TAILNET-REPLIES -m conntrack --ctstate RELATED,ESTABLISHED -j RETURN",
                            "-A SHERPA-TAILNET-REPLIES -j DROP",
                        ],
                    "{tool} forwards more than replies to the tailnet: {rules:?}"
                );
            }
            Ok(())
        }
        .await;
        gateway.remove(result).await
    }

    #[tokio::test]
    #[ignore = "requires Linux Docker and TUN; creates and removes a disposable network"]
    async fn tailnet_key_009_standard_input_reaches_a_file_in_memory_only_run_gone_after_the_command()
    -> Result<()> {
        let gateway = SmokeGateway::start(1).await?;
        let result = async {
            // A fixture, not a real credential: nothing here contacts Tailscale.
            let secret = "fixture-enrollment-secret";
            gateway
                .exec(&["grep", "-q", "^[^ ]* /run tmpfs ", "/proc/mounts"], None)
                .await
                .context("/run is not a tmpfs")?;
            gateway
                .exec(
                    &[
                        "sh",
                        "-ec",
                        r#"umask 077; cat > /run/test-key; test "$(cat /run/test-key)" = "$1"; rm /run/test-key"#,
                        "check",
                        secret,
                    ],
                    Some(secret),
                )
                .await
                .context("standard input did not reach the file in /run")?;
            // The enrollment command, pointed at a socket no daemon listens on, fails at once.
            let absent = TailscaleGatewaySettings {
                socket_path: "/run/sherpa-test-absent.sock".into(),
                ..gateway.settings.clone()
            };
            let _ = exec(
                &gateway.docker,
                &gateway.id,
                enrollment_command(&gateway.config.routes, &absent),
                Some(secret),
                &gateway.settings,
            )
            .await;
            gateway
                .exec(&["test", "!", "-e", "/run/enrollment-key"], None)
                .await
                .context("the enrollment key file outlived its command")?;
            Ok(())
        }
        .await;
        gateway.remove(result).await
    }

    #[tokio::test]
    #[ignore = "requires Linux Docker and TUN; creates and removes a disposable network"]
    async fn tailnet_suspend_010_a_restarted_gateway_has_its_daemon_and_forwarding_rules()
    -> Result<()> {
        let gateway = SmokeGateway::start(2).await?;
        let result = async {
            stop_gateway(
                &gateway.docker,
                &gateway.id,
                &gateway.config.lab_id,
                &gateway.settings,
            )
            .await?;
            gateway
                .docker
                .start_container(&gateway.id, None::<StartContainerOptions>)
                .await?;
            wait_for_socket(&gateway.docker, &gateway.id, &gateway.settings).await?;
            gateway.check_forwarding_rules().await
        }
        .await;
        gateway.remove(result).await
    }

    /// Print the gateway's last log lines, to show why its daemon did not start.
    async fn print_gateway_logs(docker: &Docker, id: &str) -> Result<()> {
        let mut logs = docker.logs(
            id,
            Some(LogsOptions {
                stdout: true,
                stderr: true,
                tail: "30".into(),
                ..Default::default()
            }),
        );
        while let Some(line) = logs.try_next().await? {
            eprintln!("{line}");
        }
        Ok(())
    }
}
