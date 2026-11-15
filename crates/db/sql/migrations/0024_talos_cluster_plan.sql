-- What the server works out for each Talos cluster at `up`, beside the manifest's side in
-- `manifest_talos_cluster` and `manifest_talos_member`: the Kubernetes API endpoint, the shared
-- address of a cluster with several control planes, and the load balancer addresses of a Cilium
-- cluster. Members' addresses are their nodes' `mgmt_ipv4`. A row goes with its manifest
-- cluster: storing the lab's definition again replaces the clusters and drops their rows, and
-- destroying the lab removes both.
--
-- Releases up to this one saved the cluster's plan in `clusters/<cluster>/cluster.toml` in the
-- lab's folder. At startup sherpad copies it from there into this table and deletes the file.
CREATE TABLE talos_cluster_plan (
    lab TEXT NOT NULL,
    cluster INTEGER NOT NULL,
    endpoint TEXT NOT NULL,
    vip TEXT,
    load_balancer_first TEXT,
    load_balancer_last TEXT,
    CHECK ((load_balancer_first IS NULL) = (load_balancer_last IS NULL)),
    PRIMARY KEY (lab, cluster),
    FOREIGN KEY (lab, cluster) REFERENCES manifest_talos_cluster(lab, position) ON DELETE CASCADE
) STRICT;
