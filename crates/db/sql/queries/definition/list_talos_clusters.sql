SELECT position, name, cni, oidc, vault
FROM manifest_talos_cluster
WHERE lab = ?1
ORDER BY position
