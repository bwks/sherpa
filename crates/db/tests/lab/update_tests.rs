use anyhow::Result;
use db::{
    LabDetails, create_lab, create_user, get_lab, update_lab_phase_from,
    update_lab_tailscale_container,
};
use shared::data::LabPhase;

use crate::helper::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn tailnet_inventory_survives_lab_phase_updates() -> Result<()> {
    let database = setup_db().await?;
    let user = create_user(
        &database,
        "tailnet-owner".into(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    let lab = create_lab(
        &database,
        "tailnet",
        "tailnet1",
        &user,
        &ipv4_lab_networks(
            "192.0.2.0/24",
            "198.51.100.0/24",
            "198.51.100.1",
            "198.51.100.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    assert!(lab.tailscale_container_id.is_none());
    let lab_id = lab.id.clone().unwrap();
    update_lab_tailscale_container(&database, &lab_id, Some("exact-docker-id")).await?;
    update_lab_phase_from(&database, "tailnet1", LabPhase::Creating, LabPhase::Ready).await?;
    let loaded = get_lab(&database, "tailnet1").await?;
    assert_eq!(
        loaded.tailscale_container_id.as_deref(),
        Some("exact-docker-id")
    );
    Ok(())
}
