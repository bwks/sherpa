//! A host for tests: interfaces, redirects and impairments are maps, and the failures are
//! chosen by the test.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::{Future, ready};

use anyhow::{Result, bail};

use super::host::LinkHost;
use super::veths::ContainerVeth;

use shared::data::LinkImpairment;

/// A host whose interfaces, redirects and impairments are maps, and whose failures are
/// chosen by the test.
#[derive(Default)]
pub(crate) struct FakeHost {
    pub(crate) interfaces: RefCell<HashMap<String, u32>>,
    pub(crate) next_index: Cell<u32>,
    pub(crate) redirects: RefCell<HashMap<String, u32>>,
    pub(crate) netem: RefCell<HashMap<u32, LinkImpairment>>,
    pub(crate) failing_lookups: HashSet<String>,
    pub(crate) failing_redirects: HashSet<String>,
    pub(crate) failing_veths: HashSet<String>,
    /// Interfaces whose netem cannot be changed.
    pub(crate) failing_netem: HashSet<String>,
    pub(crate) no_container: bool,
}

impl FakeHost {
    /// Make (or make again, with a new index) each interface in `names`.
    pub(crate) fn make(&self, names: &[&str]) {
        for name in names {
            let index = self.next_index.get() + 10;
            self.next_index.set(index);
            self.interfaces
                .borrow_mut()
                .insert((*name).to_string(), index);
            // A new interface has no filter and no qdisc.
            self.redirects.borrow_mut().remove(*name);
        }
    }

    /// Remove each interface in `names`, as a shut-off VM's taps go.
    pub(crate) fn remove(&self, names: &[&str]) {
        for name in names {
            self.interfaces.borrow_mut().remove(*name);
            self.redirects.borrow_mut().remove(*name);
        }
    }

    /// The impairment on interface `name`'s egress, if it has one.
    pub(crate) fn netem_on(&self, name: &str) -> Option<LinkImpairment> {
        let index = self.index(name)?;
        self.netem.borrow().get(&index).copied()
    }

    pub(crate) fn index(&self, name: &str) -> Option<u32> {
        self.interfaces.borrow().get(name).copied()
    }

    pub(crate) fn redirect(&self, name: &str) -> Option<u32> {
        self.redirects.borrow().get(name).copied()
    }

    /// Whether the link between `a` and `b` redirects both ways to the current ends.
    pub(crate) fn wired(&self, a: &str, b: &str) -> bool {
        self.index(a).is_some()
            && self.index(b).is_some()
            && self.redirect(a) == self.index(b)
            && self.redirect(b) == self.index(a)
    }
}

impl FakeHost {
    fn look_up(&self, iface: &str) -> Result<Option<u32>> {
        if self.failing_lookups.contains(iface) {
            bail!("netlink refused the lookup of {iface}");
        }
        Ok(self.index(iface))
    }

    fn set_netem(&self, ifindex: u32, impairment: &LinkImpairment) -> Result<()> {
        let refused = self
            .interfaces
            .borrow()
            .iter()
            .any(|(name, index)| *index == ifindex && self.failing_netem.contains(name));
        if refused {
            bail!("the kernel refused the qdisc change on ifindex {ifindex}");
        }
        if impairment.is_clear() {
            self.netem.borrow_mut().remove(&ifindex);
        } else {
            self.netem.borrow_mut().insert(ifindex, *impairment);
        }
        Ok(())
    }

    fn pid_of(&self, container: &str) -> Result<u32> {
        if self.no_container {
            bail!("no such container: {container}");
        }
        Ok(4242)
    }

    fn make_veth(&self, veth: &ContainerVeth) -> Result<()> {
        if self.failing_veths.contains(&veth.host_veth) {
            bail!("netlink refused {}", veth.host_veth);
        }
        self.make(&[veth.host_veth.as_str()]);
        Ok(())
    }
}

impl LinkHost for FakeHost {
    fn ifindex(&self, iface: &str) -> impl Future<Output = Result<Option<u32>>> {
        ready(self.look_up(iface))
    }

    fn attach_redirect(&self, iface: &str, peer_ifindex: u32) -> Result<()> {
        if self.failing_redirects.contains(iface) {
            bail!("failed to attach the eBPF redirect on {iface}");
        }
        self.redirects
            .borrow_mut()
            .insert(iface.to_string(), peer_ifindex);
        Ok(())
    }

    fn sync_netem(
        &self,
        ifindex: u32,
        impairment: &LinkImpairment,
    ) -> impl Future<Output = Result<()>> {
        ready(self.set_netem(ifindex, impairment))
    }

    fn container_pid(&self, container: &str) -> impl Future<Output = Result<u32>> {
        ready(self.pid_of(container))
    }

    fn create_veth(&self, veth: &ContainerVeth, _pid: u32) -> impl Future<Output = Result<()>> {
        ready(self.make_veth(veth))
    }
}
