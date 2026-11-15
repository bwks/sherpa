DELETE FROM talos_cluster_plan
WHERE lab = ?1
    AND cluster IN (SELECT position FROM manifest_talos_cluster WHERE lab = ?1 AND name = ?2)
