use anyhow::Result;
use db::{
    DbError, LabDetails, LabNetworks, create_lab, create_user, get_lab,
    get_used_ipv6_management_networks, get_used_management_networks, list_labs,
    update_lab_phase_from,
};
use shared::data::LabPhase;

use crate::helper::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn create_lab_stores_its_description_and_where_it_was_started() -> Result<()> {
    let db = setup_db().await?;
    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let details = LabDetails {
        description: Some("Practice BGP.\nTwo routers.".to_string()),
        client_host: Some("dev01".to_string()),
        client_folder: Some("/home/alice/labs/core".to_string()),
    };

    let created = create_lab(
        &db,
        "Core",
        "lab-0003",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &details,
    )
    .await?;
    assert_eq!(created.client_host.as_deref(), Some("dev01"));
    assert_eq!(created.description, details.description);

    let stored = get_lab(&db, "lab-0003").await?;
    assert_eq!(
        stored.description.as_deref(),
        Some("Practice BGP.\nTwo routers.")
    );
    assert_eq!(stored.client_host.as_deref(), Some("dev01"));
    assert_eq!(
        stored.client_folder.as_deref(),
        Some("/home/alice/labs/core")
    );

    // A lab without a description, started from the web UI or the API, has none of them.
    let other = create_lab(
        &db,
        "Web",
        "lab-0004",
        &user,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.2.1", "172.31.2.2"),
        &LabDetails::default(),
    )
    .await?;
    assert!(other.description.is_none());
    assert!(other.client_host.is_none());
    assert!(other.client_folder.is_none());

    // Updating the lab keeps them.
    let updated = update_lab_phase_from(&db, "lab-0003", LabPhase::Creating, LabPhase::Ready)
        .await?
        .unwrap();
    assert_eq!(updated.client_host.as_deref(), Some("dev01"));

    Ok(())
}

#[tokio::test]
async fn test_create_lab_duplicate_lab_id_fails() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;

    // Create first lab
    create_lab(
        &db,
        "Lab One",
        "lab-0003",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    // Try to create lab with same lab_id
    let result = create_lab(
        &db,
        "Lab Two",
        "lab-0003",
        &user,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await;

    let error = result.expect_err("a duplicate lab_id is refused");
    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::LabNotCreated {
            name: "Lab Two".to_owned()
        }),
        "{error:#}"
    );

    Ok(())
}

#[tokio::test]
async fn test_create_lab_duplicate_name_per_user_fails() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;

    // Create first lab
    create_lab(
        &db,
        "My Lab",
        "lab-0004",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    // Try to create lab with same name for same user
    let result = create_lab(
        &db,
        "My Lab",
        "lab-0005",
        &user,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (name, user) combination"
    );
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::LabNotCreated {
            name: "My Lab".to_owned()
        }),
        "{error:#}"
    );

    Ok(())
}

#[tokio::test]
async fn test_create_lab_same_name_different_users_succeeds() -> Result<()> {
    let db = setup_db().await?;

    let user1 = create_user(&db, "eve".to_string(), "TestPass123!", false, vec![]).await?;
    let user2 = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;

    // Both users can have labs with the same name
    let lab1 = create_lab(
        &db,
        "Shared Name",
        "lab-0006",
        &user1,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let lab2 = create_lab(
        &db,
        "Shared Name",
        "lab-0007",
        &user2,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    assert_eq!(lab1.name, "Shared Name");
    assert_eq!(lab2.name, "Shared Name");
    assert_ne!(lab1.user, lab2.user, "Labs should have different owners");

    Ok(())
}

#[tokio::test]
async fn test_create_lab_invalid_lab_id_too_short() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;

    // Try to create lab with lab_id too short
    let result = create_lab(
        &db,
        "Invalid Lab",
        "lab-01",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await;

    let error = result.expect_err("a lab_id of 6 characters is refused");
    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::LabIdLength { length: 6 }),
        "{error:#}"
    );

    Ok(())
}

fn dual_stack_networks(index: u8) -> LabNetworks {
    LabNetworks {
        management_network: format!("172.31.{index}.0/24"),
        gateway_ipv4: format!("172.31.{index}.1"),
        router_ipv4: format!("172.31.{index}.2"),
        management_network_v6: Some(format!("fd00:b00b:0:{index}::/64")),
        loopback_network_v6: Some(format!("fd00:b00c:0:{index}::/64")),
        gateway_ipv6: Some(format!("fd00:b00b:0:{index}::1")),
        router_ipv6: Some(format!("fd00:b00b:0:{index}::2")),
    }
}

#[tokio::test]
async fn lab_addressing_004_a_held_range_is_reported_used_and_cannot_be_held_twice() -> Result<()> {
    let db = setup_db().await?;
    let user = create_user(&db, "mia".to_string(), "TestPass123!", false, vec![]).await?;
    assert_eq!(
        get_used_management_networks(&db).await?.len(),
        0,
        "no lab yet"
    );

    // Labs with no IPv6 range do not collide with each other or with a dual-stack lab.
    for (name, lab_id, index) in [("Lab One", "lab-0101", 1), ("Lab Two", "lab-0102", 2)] {
        let networks = ipv4_lab_networks(
            &format!("172.31.{index}.0/24"),
            &format!("172.31.{index}.1"),
            &format!("172.31.{index}.2"),
        );
        create_lab(&db, name, lab_id, &user, &networks, &LabDetails::default()).await?;
    }
    let networks = dual_stack_networks(3);
    let lab = create_lab(
        &db,
        "Lab Three",
        "lab-0103",
        &user,
        &networks,
        &LabDetails::default(),
    )
    .await?;
    assert_eq!(lab.management_network_v6, networks.management_network_v6);
    assert_eq!(lab.loopback_network_v6, networks.loopback_network_v6);
    assert_eq!(lab.gateway_ipv6, networks.gateway_ipv6);
    assert_eq!(lab.router_ipv6, networks.router_ipv6);

    let in_use = get_used_management_networks(&db).await?;
    assert_eq!(in_use.len(), 3, "{in_use:?}");
    for net in ["172.31.1.0/24", "172.31.2.0/24", "172.31.3.0/24"] {
        assert!(
            in_use.contains(&net.parse()?),
            "{net} should be used: {in_use:?}"
        );
    }
    assert_eq!(
        get_used_ipv6_management_networks(&db)
            .await?
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["fd00:b00b:0:3::/64"]
    );

    // A second lab is refused a held IPv4 range, and a held IPv6 range with a free IPv4 one.
    let held_ipv4 = ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2");
    let held_ipv6 = LabNetworks {
        management_network_v6: networks.management_network_v6.clone(),
        ..dual_stack_networks(4)
    };
    for (lab_id, networks) in [("lab-0104", &held_ipv4), ("lab-0105", &held_ipv6)] {
        let result = create_lab(
            &db,
            "Lab Four",
            lab_id,
            &user,
            networks,
            &LabDetails::default(),
        )
        .await;
        assert!(result.is_err(), "{networks:?} must belong to one lab");
    }
    assert_eq!(
        list_labs(&db).await?.len(),
        3,
        "Rejected labs must not be created"
    );
    Ok(())
}
