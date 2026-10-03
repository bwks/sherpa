//! Per-lab Tailscale gateway. All host interaction is through the Docker API.
use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
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

use shared::data::{
    TAILNET_DOCS_URL, TailnetAuthKey, TailnetState, TailnetStatus, TailscaleGatewaySettings,
};

const LAB_LABEL: &str = "sh.erpa.tailscale.lab";
const ROLE_LABEL: &str = "sh.erpa.role";

#[derive(Debug)]
pub struct GatewayConfig {
    pub lab_id: String,
    pub network: String,
    pub ipv4: String,
    pub ipv6: Option<String>,
    pub routes: Vec<String>,
}

#[instrument(level = "debug")]
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
#[instrument(skip(labels), level = "debug")]
pub fn is_gateway(labels: &HashMap<String, String>) -> bool {
    labels
        .get(ROLE_LABEL)
        .is_some_and(|value| value == "tailnet-gateway")
}

fn owned(labels: &HashMap<String, String>, lab_id: &str) -> bool {
    labels.get(LAB_LABEL).is_some_and(|value| value == lab_id) && is_gateway(labels)
}

fn is_missing(error: &DockerError) -> bool {
    matches!(
        error,
        DockerError::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
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
        // Rules live in mangle/FORWARD, before Tailscale's filter/FORWARD accepts.
        // Install them before starting the daemon, on every container start.
        entrypoint: Some(vec!["sh".into(), "-ec".into()]),
        cmd: Some(vec![
            "socket=\"$1\"; shift; mkdir -p \"$(dirname \"$socket\")\"; chmod 0700 /var/lib/tailscale; for tool in iptables-nft ip6tables-nft; do $tool -t mangle -N SHERPA-TAILNET; done; for route in \"$@\"; do case $route in *:*) tool=ip6tables-nft;; *) tool=iptables-nft;; esac; $tool -t mangle -A SHERPA-TAILNET -d \"$route\" -j RETURN; done; for tool in iptables-nft ip6tables-nft; do $tool -t mangle -A SHERPA-TAILNET -j DROP; $tool -t mangle -I FORWARD 1 -i tailscale0 -j SHERPA-TAILNET; done; exec tailscaled --state=/var/lib/tailscale/tailscaled.state --socket=\"$socket\"".into(),
            "gateway".into(),
            settings.socket_path.clone(),
        ].into_iter().chain(config.routes.iter().cloned()).collect()),
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
            tmpfs: Some(HashMap::from([("/run".into(), "rw,noexec,nosuid,size=16m,mode=0700".into())])),
            ..Default::default()
        }),
        networking_config: Some(NetworkingConfig {
            endpoints_config: Some(HashMap::from([(config.network.clone(), EndpointSettings {
                ipam_config: Some(EndpointIpamConfig {
                    ipv4_address: Some(config.ipv4.clone()),
                    ipv6_address: config.ipv6.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            })])),
        }),
        ..Default::default()
    }
}

/// Create an identity volume and gateway, but do not enroll until its ID is persisted.
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
                    return Ok::<(), anyhow::Error>(());
                }
                sleep(Duration::from_millis(250)).await;
            }
        },
    )
    .await
    .context("Tailscale daemon did not become ready")?
}

fn enrollment_command(config: &GatewayConfig, settings: &TailscaleGatewaySettings) -> Vec<String> {
    let script = "umask 077; trap 'rm -f /run/enrollment-key' EXIT; cat > /run/enrollment-key; tailscale \"$1\" up --auth-key=file:/run/enrollment-key --timeout=\"$3\" --accept-dns=false --accept-routes=false --snat-subnet-routes=true --stateful-filtering=true --advertise-routes=\"$2\"";
    vec![
        "sh".into(),
        "-c".into(),
        script.into(),
        "enroll".into(),
        format!("--socket={}", settings.socket_path),
        config.routes.join(","),
        format!("{}s", settings.enrollment_timeout_secs),
    ]
}

#[instrument(skip(docker, key), fields(lab_id = %config.lab_id))]
pub async fn enroll_gateway(
    docker: &Docker,
    id: &str,
    config: &GatewayConfig,
    key: &TailnetAuthKey,
    settings: &TailscaleGatewaySettings,
) -> Result<TailnetStatus> {
    inspect_owned(docker, id, &config.lab_id).await?;
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
        enrollment_command(config, settings),
        Some(key.expose()),
        settings,
    )
    .await
    .context("Tailscale enrollment failed")?;
    wait_connected(docker, id, &config.lab_id, config.routes.clone(), settings).await
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
                TailnetState::NeedsAuthentication => bail!("Tailscale gateway requires reauthentication or device approval; see {}#recovery", TAILNET_DOCS_URL),
                _ => sleep(Duration::from_millis(500)).await,
            }
        }
    }).await.context("Tailscale gateway did not connect")?
}

#[instrument(skip(docker), level = "debug")]
pub async fn gateway_status(
    docker: &Docker,
    id: &str,
    lab_id: &str,
    routes: Vec<String>,
    settings: &TailscaleGatewaySettings,
) -> Result<TailnetStatus> {
    let details = inspect_owned(docker, id, lab_id).await?;
    if !details
        .state
        .as_ref()
        .is_some_and(|s| s.running == Some(true))
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

#[instrument(skip(docker))]
pub async fn resume_gateway(
    docker: &Docker,
    id: &str,
    lab_id: &str,
    routes: Vec<String>,
    settings: &TailscaleGatewaySettings,
) -> Result<TailnetStatus> {
    let details = inspect_owned(docker, id, lab_id).await?;
    if !details
        .state
        .as_ref()
        .is_some_and(|s| s.running == Some(true))
    {
        docker
            .start_container(id, None::<StartContainerOptions>)
            .await
            .context("Unable to resume Tailscale gateway")?;
        wait_for_socket(docker, id, settings).await?;
    }
    wait_connected(docker, id, lab_id, routes, settings).await
}

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
            Ok(()) => {}
            Err(DockerError::DockerResponseServerError {
                status_code: 304, ..
            }) => {}
            Err(error) => return Err(error).context("Unable to stop Tailscale gateway"),
        }
    }
    Ok(())
}

/// Cleanup uses exact names plus ownership labels, including partially created labs.
#[instrument(skip(docker))]
pub async fn remove_gateway(docker: &Docker, lab_id: &str) -> Result<()> {
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
    use std::thread;

    fn test_settings() -> Result<TailscaleGatewaySettings> {
        match std::env::var("SHERPA_TEST_SERVER_CONFIG") {
            Ok(path) => Ok(load_config(&path)?.tailscale),
            Err(_) => Ok(TailscaleGatewaySettings::default()),
        }
    }

    #[test]
    fn gateway_configuration_uses_operator_image_and_socket() {
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
        };
        let body = container_config(&config, &settings);
        assert_eq!(body.image.as_deref(), Some(settings.image.as_str()));
        let command = body.cmd.unwrap();
        assert_eq!(command[2], settings.socket_path);
        assert_eq!(command[3], config.routes[0]);
        assert!(!command[0].contains(&settings.socket_path));
        let enrollment = enrollment_command(&config, &settings);
        assert_eq!(enrollment[4], format!("--socket={}", settings.socket_path));
        assert_eq!(enrollment[5], config.routes.join(","));
        assert_eq!(enrollment[6], "120s");
        assert!(!enrollment[2].contains(&settings.socket_path));
    }

    #[tokio::test]
    async fn gateway_exec_honors_operator_timeout() {
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

    #[tokio::test]
    async fn gateway_stop_uses_operator_grace_period() {
        let (docker, mock) = mock_docker(vec![
            (
                "/containers/exact-id/json",
                "200 OK",
                r#"{"Id":"exact-id","State":{"Running":true},"Config":{"Labels":{"sh.erpa.tailscale.lab":"test1234","sh.erpa.role":"tailnet-gateway"}}}"#,
            ),
            ("/containers/exact-id/stop?t=17", "204 No Content", ""),
        ]);
        let settings = TailscaleGatewaySettings {
            stop_timeout_secs: 17,
            ..Default::default()
        };
        let result = stop_gateway(&docker, "exact-id", "test1234", &settings).await;
        mock.join().unwrap();
        result.unwrap();
    }

    fn mock_docker(
        responses: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (Docker, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
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
        });
        let docker = Docker::connect_with_http(
            &format!("http://{address}"),
            5,
            bollard::API_DEFAULT_VERSION,
        )
        .unwrap();
        (docker, handle)
    }

    #[tokio::test]
    async fn cleanup_of_absent_resources_is_idempotent() {
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
        remove_gateway(&docker, "test1234").await.unwrap();
        mock.join().unwrap();
    }

    #[tokio::test]
    async fn cleanup_refuses_other_labs_gateway() {
        let (docker, mock) = mock_docker(vec![(
            "/containers/sherpa-tailnet-test1234/json",
            "200 OK",
            r#"{"Id":"foreign","Config":{"Labels":{"sh.erpa.tailscale.lab":"another","sh.erpa.role":"tailnet-gateway"}}}"#,
        )]);
        assert!(
            remove_gateway(&docker, "test1234")
                .await
                .unwrap_err()
                .to_string()
                .contains("unowned")
        );
        mock.join().unwrap();
    }

    #[tokio::test]
    async fn stopped_gateway_is_not_stopped_again() {
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
        mock.join().unwrap();
    }

    #[tokio::test]
    async fn cleanup_preserves_identity_if_container_removal_fails() {
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
        assert!(remove_gateway(&docker, "test1234").await.is_err());
        mock.join().unwrap();
    }

    #[tokio::test]
    async fn gateway_status_succeeds_without_loading_large_peer_inventory() {
        let settings = TailscaleGatewaySettings {
            socket_path: "/run/overridden/status.sock".into(),
            ..Default::default()
        };
        let own_output = "{\"BackendState\":\"Running\",\"TailscaleIPs\":[\"100.64.0.1\"],\"Self\":{\"AllowedIPs\":[\"192.0.2.0/24\"]}}\n";
        let capabilities = ["\"https://tailscale.com/cap/file-sharing\""; 10].join(",");
        let peers = (0..3000)
            .map(|index| {
                format!(r#""nodekey:{index:064x}":{{"HostName":"device-{index}","DNSName":"device-{index}.example.ts.net.","OS":"linux","Online":true,"Capabilities":[{capabilities}],"AllowedIPs":["100.64.0.2/32"]}}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        let full_output = format!(
            "{},\"Peer\":{{{peers}}}}}\n",
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
                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                let request = headers.lines().next().unwrap();
                let response = if request.contains("/containers/gateway-id/json") {
                    r#"{"Id":"gateway-id","State":{"Running":true},"Config":{"Labels":{"sh.erpa.tailscale.lab":"test1234","sh.erpa.role":"tailnet-gateway"}}}"#
                } else if request.contains("/containers/gateway-id/exec") {
                    let body = String::from_utf8(body).unwrap();
                    own_only = body.contains("\"--peers=false\"")
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
        assert!(status.warnings.is_empty());
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
        }
    }

    #[tokio::test]
    #[ignore = "requires a disposable Docker network, Linux TUN, and a test tailnet auth key"]
    async fn live_tailnet_enrollment_stop_resume_cleanup() -> Result<()> {
        let config = live_config();
        let settings = test_settings()?;
        let docker = Docker::connect_with_local_defaults()?;
        let key = TailnetAuthKey::new(std::env::var("SHERPA_TEST_TAILSCALE_AUTH_KEY")?);
        let result: Result<()> = async {
            let id = create_gateway(&docker, &config, &settings).await?;
            let joined = enroll_gateway(&docker, &id, &config, &key, &settings).await?;
            ensure!(
                joined.state == TailnetState::Connected,
                "gateway did not connect"
            );
            stop_gateway(&docker, &id, &config.lab_id, &settings).await?;
            stop_gateway(&docker, &id, &config.lab_id, &settings).await?;
            let stopped = gateway_status(
                &docker,
                &id,
                &config.lab_id,
                config.routes.clone(),
                &settings,
            )
            .await?;
            ensure!(
                stopped.state == TailnetState::Stopped,
                "gateway did not stop"
            );
            let resumed = resume_gateway(
                &docker,
                &id,
                &config.lab_id,
                config.routes.clone(),
                &settings,
            )
            .await?;
            ensure!(
                joined.tailscale_ips == resumed.tailscale_ips,
                "gateway identity changed"
            );
            Ok(())
        }
        .await;
        let cleanup = remove_gateway(&docker, &config.lab_id).await;
        result?;
        cleanup?;
        remove_gateway(&docker, &config.lab_id).await?;
        Ok(())
    }

    #[test]
    fn gateway_is_scoped_and_credentials_are_not_in_container_configuration() {
        let config = GatewayConfig {
            lab_id: "12345678".into(),
            network: "management".into(),
            ipv4: "192.0.2.3".into(),
            ipv6: None,
            routes: vec!["192.0.2.0/24".into()],
        };
        let body = container_config(&config, &TailscaleGatewaySettings::default());
        assert_eq!(
            body.env,
            Some(vec!["TS_DEBUG_FIREWALL_MODE=nftables".into()])
        );
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
    #[tokio::test]
    #[ignore = "requires Linux Docker and TUN; creates and removes a disposable network"]
    async fn live_gateway_daemon_smoke() -> Result<()> {
        let docker = Docker::connect_with_local_defaults()?;
        let settings = test_settings()?;
        let lab_id = format!("tnsmoke{}", std::process::id());
        let network = format!("sherpa-{lab_id}");
        let ipv6_prefix = format!("fdff:{:x}::/64", std::process::id() % 65536);
        let ipv6_address = format!("fdff:{:x}::3", std::process::id() % 65536);
        let created = docker
            .create_network(NetworkCreateRequest {
                name: network.clone(),
                driver: Some("bridge".into()),
                enable_ipv6: Some(true),
                ipam: Some(Ipam {
                    config: Some(vec![IpamConfig {
                        subnet: Some(ipv6_prefix.clone()),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await?;
        let result: Result<()> = async {
            let details = docker
                .inspect_network(&created.id, None::<InspectNetworkOptions>)
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
            let config = GatewayConfig {
                lab_id: lab_id.clone(),
                network: network.clone(),
                ipv4: format!("{prefix}.3"),
                ipv6: Some(ipv6_address),
                routes: vec![
                    subnet.subnet.context("test network missing subnet")?,
                    ipv6_prefix,
                ],
            };
            let id = create_gateway(&docker, &config, &settings).await?;
            docker
                .start_container(&id, None::<StartContainerOptions>)
                .await?;
            if let Err(error) = wait_for_socket(&docker, &id, &settings).await {
                let mut logs = docker.logs(
                    &id,
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
                return Err(error);
            }
            let rules = exec(
                &docker,
                &id,
                vec![
                    "iptables-nft".into(),
                    "-t".into(),
                    "mangle".into(),
                    "-S".into(),
                    "SHERPA-TAILNET".into(),
                ],
                None,
                &settings,
            )
            .await?;
            ensure!(
                rules.contains(&config.routes[0]) && rules.contains("DROP"),
                "missing forwarding restriction"
            );
            let ipv6_rules = exec(
                &docker,
                &id,
                vec![
                    "ip6tables-nft".into(),
                    "-t".into(),
                    "mangle".into(),
                    "-S".into(),
                    "SHERPA-TAILNET".into(),
                ],
                None,
                &settings,
            )
            .await?;
            ensure!(
                ipv6_rules.contains(&config.routes[1]) && ipv6_rules.contains("DROP"),
                "missing IPv6 forwarding restriction"
            );
            // Exercise secret stdin/file delivery without transmitting a real credential.
            let secret = "fixture-enrollment-secret";
            exec(
                &docker,
                &id,
                vec![
                    "sh".into(),
                    "-ec".into(),
                    "umask 077; cat > /run/test-key; test -s /run/test-key; rm /run/test-key"
                        .into(),
                ],
                Some(secret),
                &settings,
            )
            .await?;
            stop_gateway(&docker, &id, &lab_id, &settings).await?;
            docker
                .start_container(&id, None::<StartContainerOptions>)
                .await?;
            wait_for_socket(&docker, &id, &settings).await?;
            stop_gateway(&docker, &id, &lab_id, &settings).await?;
            Ok(())
        }
        .await;
        let cleanup = remove_gateway(&docker, &lab_id).await;
        let network_cleanup = docker.remove_network(&created.id).await;
        result?;
        cleanup?;
        network_cleanup?;
        Ok(())
    }
}
