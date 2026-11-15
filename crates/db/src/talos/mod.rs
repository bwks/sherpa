//! What the server works out for a lab's Talos clusters at `up`: each cluster's Kubernetes API
//! endpoint, shared address and load balancer addresses. With the manifest's side of the
//! cluster in the lab's definition and the members' management addresses, they make the
//! cluster's plan.

mod plan;

pub use plan::{list_talos_cluster_plans, store_talos_cluster_plans};
