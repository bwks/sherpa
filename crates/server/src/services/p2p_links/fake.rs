//! A host for tests: interfaces, redirects, impairments and carriers are maps, and the failures
//! are chosen by the test.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::{Future, ready};

use anyhow::{Result, bail};

use super::host::{EndDevice, LinkHost};
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
    /// Interfaces whose redirect cannot be read.
    pub(crate) failing_filter_reads: HashSet<String>,
    /// How many changes were made: redirects attached, impairments set, veths made.
    pub(crate) writes: Cell<u32>,
    /// The interfaces whose carrier is down. A new interface has carrier.
    pub(crate) carrier_down: RefCell<HashSet<String>>,
    /// Each carrier change, in order.
    pub(crate) carrier_writes: RefCell<Vec<(EndDevice, bool)>>,
    /// Interfaces whose carrier cannot be read or set.
    pub(crate) failing_carrier: HashSet<String>,
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
            // A new interface has no filter and no qdisc, and has carrier.
            self.redirects.borrow_mut().remove(*name);
            self.carrier_down.borrow_mut().remove(*name);
        }
    }

    /// Remove each interface in `names`, as a shut-off VM's taps go.
    pub(crate) fn remove(&self, names: &[&str]) {
        for name in names {
            self.interfaces.borrow_mut().remove(*name);
            self.redirects.borrow_mut().remove(*name);
            self.carrier_down.borrow_mut().remove(*name);
        }
    }

    /// Whether interface `name` exists and has carrier.
    pub(crate) fn has_carrier(&self, name: &str) -> bool {
        self.index(name).is_some() && !self.carrier_down.borrow().contains(name)
    }

    /// Take, or give back, the carrier of interface `name`, as a user's `ip link set` would.
    pub(crate) fn set_carrier_by_hand(&self, name: &str, up: bool) {
        if up {
            self.carrier_down.borrow_mut().remove(name);
        } else {
            self.carrier_down.borrow_mut().insert(name.to_string());
        }
    }

    /// The carrier changes made, in order, and forget them.
    pub(crate) fn take_carrier_writes(&self) -> Vec<(EndDevice, bool)> {
        self.carrier_writes.take()
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

    /// The name of the interface with index `ifindex`.
    fn name_of(&self, ifindex: u32) -> Option<String> {
        self.interfaces
            .borrow()
            .iter()
            .find(|(_, index)| **index == ifindex)
            .map(|(name, _)| name.clone())
    }

    fn read_redirect(&self, ifindex: u32) -> Result<bool> {
        let Some(name) = self.name_of(ifindex) else {
            bail!("no interface has index {ifindex}");
        };
        if self.failing_filter_reads.contains(&name) {
            bail!("netlink refused the filter dump of {name}");
        }
        Ok(self.redirect(&name).is_some())
    }

    fn wrote(&self) {
        self.writes.set(self.writes.get() + 1);
    }

    fn set_netem(&self, ifindex: u32, impairment: &LinkImpairment) -> Result<()> {
        self.wrote();
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

    fn read_carrier(&self, end: &EndDevice) -> Result<Option<bool>> {
        let iface = iface_of(end);
        if self.failing_carrier.contains(iface) {
            bail!("cannot read the carrier of {iface}");
        }
        Ok(self.index(iface).map(|_| self.has_carrier(iface)))
    }

    fn write_carrier(&self, end: &EndDevice, up: bool) -> Result<()> {
        let iface = iface_of(end);
        if self.failing_carrier.contains(iface) {
            bail!("cannot set the carrier of {iface}");
        }
        if self.index(iface).is_none() {
            bail!("no interface {iface}");
        }
        self.carrier_writes.borrow_mut().push((end.clone(), up));
        self.set_carrier_by_hand(iface, up);
        Ok(())
    }

    fn make_veth(&self, veth: &ContainerVeth) -> Result<()> {
        self.wrote();
        if self.failing_veths.contains(&veth.host_veth) {
            bail!("netlink refused {}", veth.host_veth);
        }
        self.make(&[veth.host_veth.as_str()]);
        Ok(())
    }
}

/// The host interface of end `end`.
fn iface_of(end: &EndDevice) -> &str {
    match end {
        EndDevice::Veth(iface) | EndDevice::Tap { tap: iface, .. } => iface,
    }
}

impl LinkHost for FakeHost {
    fn ifindex(&self, iface: &str) -> impl Future<Output = Result<Option<u32>>> {
        ready(self.look_up(iface))
    }

    fn has_redirect(&self, ifindex: u32) -> impl Future<Output = Result<bool>> {
        ready(self.read_redirect(ifindex))
    }

    fn attach_redirect(&self, iface: &str, peer_ifindex: u32) -> Result<()> {
        self.wrote();
        if self.failing_redirects.contains(iface) {
            // The real attach detaches the old program before it loads the new one, so the
            // end is left bare.
            self.redirects.borrow_mut().remove(iface);
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

    fn carrier(&self, end: &EndDevice) -> impl Future<Output = Result<Option<bool>>> {
        ready(self.read_carrier(end))
    }

    fn set_carrier(&self, end: &EndDevice, up: bool) -> impl Future<Output = Result<()>> {
        ready(self.write_carrier(end, up))
    }
}
