SELECT manifest_talos_cluster.position, manifest_talos_cluster.name, manifest_talos_cluster.cni,
    talos_cluster_plan.endpoint, talos_cluster_plan.vip, talos_cluster_plan.load_balancer_first,
    talos_cluster_plan.load_balancer_last, manifest_talos_cluster.oidc, oidc_node.mgmt_ipv4,
    manifest_talos_cluster.vault, vault_node.mgmt_ipv4
FROM talos_cluster_plan
JOIN manifest_talos_cluster ON manifest_talos_cluster.lab = talos_cluster_plan.lab
    AND manifest_talos_cluster.position = talos_cluster_plan.cluster
LEFT JOIN node AS oidc_node ON oidc_node.lab = manifest_talos_cluster.lab
    AND oidc_node.name = manifest_talos_cluster.oidc
LEFT JOIN node AS vault_node ON vault_node.lab = manifest_talos_cluster.lab
    AND vault_node.name = manifest_talos_cluster.vault
WHERE talos_cluster_plan.lab = ?1
ORDER BY manifest_talos_cluster.position
