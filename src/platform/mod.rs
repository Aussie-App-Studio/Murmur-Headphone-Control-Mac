#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod macos_battery;

#[cfg(target_os = "macos")]
pub mod macos_bt;

#[cfg(target_os = "macos")]
pub use macos_bt::{
    enumerate_paired_devices, is_system_dark_theme, read_macos_apple_battery, MacL2capTransport,
    MacRfcommTransport,
};

#[cfg(not(target_os = "macos"))]
mod stub {
    use crate::models::BatteryState;
    use crate::protocol::{BluetoothTransport, BondedDevice, ProtocolError};

    pub fn enumerate_paired_devices(_preferred_address: Option<&str>) -> Vec<BondedDevice> {
        Vec::new()
    }

    pub fn is_system_dark_theme() -> bool {
        true
    }

    pub fn read_macos_apple_battery(_address: &str, _is_over_ear: bool) -> Option<BatteryState> {
        None
    }

    pub struct MacRfcommTransport;
    impl MacRfcommTransport {
        pub fn new() -> Self {
            Self
        }
    }

    impl BluetoothTransport for MacRfcommTransport {
        fn is_connected(&self) -> bool {
            false
        }
        fn connect(&self, _address: &str, _service_uuid: &str) -> Result<(), ProtocolError> {
            Err(ProtocolError::Io(
                "macOS IOBluetooth RFCOMM is only available on macOS".into(),
            ))
        }
        fn read(&self, _buf: &mut [u8]) -> Result<usize, ProtocolError> {
            Err(ProtocolError::NotConnected)
        }
        fn write(&self, _bytes: &[u8]) -> Result<(), ProtocolError> {
            Err(ProtocolError::NotConnected)
        }
        fn close(&self) {}
    }

    pub struct MacL2capTransport;
    impl MacL2capTransport {
        pub fn new(_psm: u16) -> Self {
            Self
        }
    }

    impl BluetoothTransport for MacL2capTransport {
        fn is_connected(&self) -> bool {
            false
        }
        fn connect(&self, _address: &str, _service_uuid: &str) -> Result<(), ProtocolError> {
            Err(ProtocolError::Io(
                "macOS IOBluetooth L2CAP is only available on macOS".into(),
            ))
        }
        fn read(&self, _buf: &mut [u8]) -> Result<usize, ProtocolError> {
            Err(ProtocolError::NotConnected)
        }
        fn write(&self, _bytes: &[u8]) -> Result<(), ProtocolError> {
            Err(ProtocolError::NotConnected)
        }
        fn close(&self) {}
    }
}

#[cfg(not(target_os = "macos"))]
#[allow(unused_imports)]
pub use stub::{
    enumerate_paired_devices, is_system_dark_theme, read_macos_apple_battery, MacL2capTransport,
    MacRfcommTransport,
};
