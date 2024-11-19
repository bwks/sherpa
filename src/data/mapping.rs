// Device name to ip address mapping
pub struct DeviceIp {
    pub name: String,
    pub ip_address: String,
}

// Data used to clone disk for VM creation
pub struct CloneDisk {
    pub src: String,
    pub dst: String,
}
