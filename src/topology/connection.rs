use crate::topology::Device;

use super::Interface;

pub struct Connection {
    device_a: Device,
    interface_a: Interface,
    device_b: Device,
    interface_b: Interface,
}
