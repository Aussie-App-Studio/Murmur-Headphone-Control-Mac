pub mod airpods_aap;
pub mod airpods_ble;
pub mod bose_bmap;
pub mod galaxy_buds;
pub mod generic;
pub mod mdr_arq;
pub mod mdr_frame;
#[cfg(test)]
pub mod simulated;
pub mod sony_v1;
pub mod sony_v2;
pub mod soundcore;
pub mod transport;

#[allow(unused_imports)]
pub use transport::{classify_device, BluetoothTransport, BondedDevice, ProtocolError};
