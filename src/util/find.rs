use crate::topology::Device;

/// Get's a device ID by it's name from a slice of topology Device.
pub fn get_dev_id(devices: &[Device], name: &str) -> Option<u8> {
    devices
        .iter()
        .find(|device| device.name == name)
        .map(|device| device.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceModels;

    // Helper function to create a test device
    fn create_device(id: u8, name: &str, model: DeviceModels) -> Device {
        Device {
            id,
            name: name.to_string(),
            device_model: model,
        }
    }

    #[test]
    fn test_get_dev_id_found() {
        let devices = vec![
            create_device(1, "Device A", DeviceModels::CiscoCat8000v),
            create_device(2, "Device B", DeviceModels::AristaVeos),
            create_device(3, "Device C", DeviceModels::NvidiaCumulus),
        ];

        assert_eq!(get_dev_id(&devices, "Device B"), Some(2));
    }

    #[test]
    fn test_get_dev_id_not_found() {
        let devices = vec![
            create_device(1, "Device A", DeviceModels::CiscoCat8000v),
            create_device(2, "Device B", DeviceModels::AristaVeos),
        ];

        assert_eq!(get_dev_id(&devices, "Device C"), None);
    }

    #[test]
    fn test_get_dev_id_empty_list() {
        let devices: Vec<Device> = vec![];

        assert_eq!(get_dev_id(&devices, "Any Device"), None);
    }

    #[test]
    fn test_get_dev_id_first_element() {
        let devices = vec![
            create_device(1, "Device A", DeviceModels::CiscoCat8000v),
            create_device(2, "Device B", DeviceModels::AristaVeos),
        ];

        assert_eq!(get_dev_id(&devices, "Device A"), Some(1));
    }

    #[test]
    fn test_get_dev_id_last_element() {
        let devices = vec![
            create_device(1, "Device A", DeviceModels::CiscoCat8000v),
            create_device(2, "Device B", DeviceModels::AristaVeos),
            create_device(3, "Device C", DeviceModels::NvidiaCumulus),
        ];

        assert_eq!(get_dev_id(&devices, "Device C"), Some(3));
    }

    #[test]
    fn test_get_dev_id_case_sensitive() {
        let devices = vec![
            create_device(1, "Device A", DeviceModels::CiscoCat8000v),
            create_device(2, "Device B", DeviceModels::AristaVeos),
        ];

        assert_eq!(get_dev_id(&devices, "device a"), None);
    }

    #[test]
    fn test_get_dev_id_multiple_matches() {
        let devices = vec![
            create_device(1, "Device A", DeviceModels::CiscoCat8000v),
            create_device(2, "Device B", DeviceModels::AristaVeos),
            create_device(3, "Device A", DeviceModels::NvidiaCumulus),
        ];

        assert_eq!(get_dev_id(&devices, "Device A"), Some(1));
    }
}
