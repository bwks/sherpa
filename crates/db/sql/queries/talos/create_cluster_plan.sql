INSERT INTO talos_cluster_plan (lab, cluster, endpoint, vip, load_balancer_first,
    load_balancer_last)
SELECT lab, position, ?3, ?4, ?5, ?6
FROM manifest_talos_cluster
WHERE lab = ?1 AND name = ?2
