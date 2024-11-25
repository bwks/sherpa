use crate::topology::Device;

// Check duplicate device definition
pub fn check_duplicate_device(devices: Vec<Device>) -> bool {
    let mut devs: Vec<String> = vec![];

    for device in devices {
        if devs.contains(&device.name) {
            println!("Device {} defined more than once in manifest", device.name);
            return true;
        } else {
            devs.push(device.name)
        }
    }
    false
}
