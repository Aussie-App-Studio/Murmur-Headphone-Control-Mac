use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use crate::models::{BatteryState, BatteryUnit, ChargingStatus};
use crate::platform::macos_battery::read_system_profiler_apple_battery;
use crate::protocol::{classify_device, BluetoothTransport, BondedDevice, ProtocolError};

type MurmurDeviceCallback = extern "C" fn(
    ctx: *mut c_void,
    name_utf8: *const c_char,
    addr_utf8: *const c_char,
    is_connected: c_int,
    class_of_device: u32,
);

extern "C" {
    fn murmur_macos_is_dark_mode() -> c_int;
    fn murmur_macos_enumerate_paired_devices(cb: MurmurDeviceCallback, ctx: *mut c_void);
    fn murmur_macos_read_apple_battery(
        addr_utf8: *const c_char,
        out_left: *mut c_int,
        out_right: *mut c_int,
        out_case: *mut c_int,
        out_single: *mut c_int,
    ) -> c_int;

    fn murmur_rfcomm_connect(
        addr_utf8: *const c_char,
        uuid_utf8: *const c_char,
        err_buf: *mut c_char,
        err_len: usize,
    ) -> *mut c_void;
    fn murmur_rfcomm_read(handle: *mut c_void, buf: *mut u8, cap: usize, timeout_ms: u32) -> isize;
    fn murmur_rfcomm_write(handle: *mut c_void, buf: *const u8, len: usize) -> c_int;
    fn murmur_rfcomm_is_connected(handle: *mut c_void) -> c_int;
    fn murmur_rfcomm_disconnect(handle: *mut c_void);
    fn murmur_rfcomm_release(handle: *mut c_void);

    fn murmur_l2cap_connect(
        addr_utf8: *const c_char,
        psm: u16,
        err_buf: *mut c_char,
        err_len: usize,
    ) -> *mut c_void;
    fn murmur_l2cap_read(handle: *mut c_void, buf: *mut u8, cap: usize, timeout_ms: u32) -> isize;
    fn murmur_l2cap_write(handle: *mut c_void, buf: *const u8, len: usize) -> c_int;
    fn murmur_l2cap_is_connected(handle: *mut c_void) -> c_int;
    fn murmur_l2cap_disconnect(handle: *mut c_void);
    fn murmur_l2cap_release(handle: *mut c_void);
}

struct OwnedHandle {
    ptr: NonNull<c_void>,
    disconnect_fn: unsafe extern "C" fn(*mut c_void),
    release_fn: unsafe extern "C" fn(*mut c_void),
}

unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            (self.disconnect_fn)(self.ptr.as_ptr());
            (self.release_fn)(self.ptr.as_ptr());
        }
    }
}

pub fn is_system_dark_theme() -> bool {
    unsafe { murmur_macos_is_dark_mode() != 0 }
}

extern "C" fn collect_paired_device(
    ctx: *mut c_void,
    name_utf8: *const c_char,
    addr_utf8: *const c_char,
    is_connected: c_int,
    class_of_device: u32,
) {
    if ctx.is_null() || addr_utf8.is_null() {
        return;
    }
    let out = unsafe { &mut *(ctx as *mut Vec<BondedDevice>) };
    let addr = unsafe { CStr::from_ptr(addr_utf8) }.to_string_lossy();
    let name = if name_utf8.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(name_utf8) }
            .to_string_lossy()
            .into_owned()
    };

    if let Some(classified) = classify_device(&name, &addr, is_connected != 0, class_of_device) {
        out.push(classified);
    }
}

pub fn enumerate_paired_devices(preferred_address: Option<&str>) -> Vec<BondedDevice> {
    let mut result: Vec<BondedDevice> = Vec::new();
    let ctx = (&mut result as *mut Vec<BondedDevice>).cast::<c_void>();
    unsafe {
        murmur_macos_enumerate_paired_devices(collect_paired_device, ctx);
    }

    let pref_upper = preferred_address.map(|s| s.to_uppercase());
    result.sort_by(|a, b| {
        let a_pref = pref_upper
            .as_ref()
            .is_some_and(|p| a.address.eq_ignore_ascii_case(p));
        let b_pref = pref_upper
            .as_ref()
            .is_some_and(|p| b.address.eq_ignore_ascii_case(p));
        b.is_connected
            .cmp(&a.is_connected)
            .then(b_pref.cmp(&a_pref))
            .then(a.name.cmp(&b.name))
    });
    result
}

pub fn read_macos_apple_battery(address: &str, is_over_ear: bool) -> Option<BatteryState> {
    if let Ok(c_addr) = CString::new(address) {
        let mut left: c_int = -1;
        let mut right: c_int = -1;
        let mut case_pct: c_int = -1;
        let mut single: c_int = -1;
        let ok = unsafe {
            murmur_macos_read_apple_battery(
                c_addr.as_ptr(),
                &mut left,
                &mut right,
                &mut case_pct,
                &mut single,
            )
        };
        if ok != 0 {
            let opt = |v: c_int| if (0..=100).contains(&v) { Some(v) } else { None };
            let l = opt(left);
            let r = opt(right);
            let c = opt(case_pct);
            let s = opt(single);

            if is_over_ear || (l.is_none() && r.is_none() && s.is_some()) {
                if let Some(lvl) = s.or(l).or(r) {
                    return Some(BatteryState {
                        level: Some(lvl),
                        charging_status: ChargingStatus::NotCharging,
                        left: None,
                        right: None,
                        case_battery: None,
                    });
                }
            } else if l.is_some() || r.is_some() || c.is_some() {
                let min_bud = [l, r].into_iter().flatten().min().or(s);
                return Some(BatteryState {
                    level: min_bud,
                    charging_status: ChargingStatus::NotCharging,
                    left: l.map(|v| BatteryUnit::new(v, ChargingStatus::NotCharging)),
                    right: r.map(|v| BatteryUnit::new(v, ChargingStatus::NotCharging)),
                    case_battery: c.map(|v| BatteryUnit::new(v, ChargingStatus::NotCharging)),
                });
            }
        }
    }

    read_system_profiler_apple_battery(address, is_over_ear)
}

fn cstr_err(buf: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

pub struct MacRfcommTransport {
    handle: Mutex<Option<Arc<OwnedHandle>>>,
}

impl Default for MacRfcommTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl MacRfcommTransport {
    pub fn new() -> Self {
        Self {
            handle: Mutex::new(None),
        }
    }
}

impl Drop for MacRfcommTransport {
    fn drop(&mut self) {
        self.close();
    }
}

impl BluetoothTransport for MacRfcommTransport {
    fn is_connected(&self) -> bool {
        let Some(h) = self.handle.lock().unwrap().clone() else {
            return false;
        };
        unsafe { murmur_rfcomm_is_connected(h.ptr.as_ptr()) != 0 }
    }

    fn connect(&self, address: &str, service_uuid: &str) -> Result<(), ProtocolError> {
        self.close();
        let c_addr = CString::new(address)
            .map_err(|_| ProtocolError::Io("Invalid Bluetooth address".into()))?;
        let c_uuid = CString::new(service_uuid)
            .map_err(|_| ProtocolError::Io("Invalid service UUID".into()))?;
        let mut err_buf = vec![0 as c_char; 256];

        let raw = unsafe {
            murmur_rfcomm_connect(
                c_addr.as_ptr(),
                c_uuid.as_ptr(),
                err_buf.as_mut_ptr(),
                err_buf.len(),
            )
        };
        let Some(nn) = NonNull::new(raw) else {
            return Err(ProtocolError::Io(cstr_err(&err_buf)));
        };
        *self.handle.lock().unwrap() = Some(Arc::new(OwnedHandle {
            ptr: nn,
            disconnect_fn: murmur_rfcomm_disconnect,
            release_fn: murmur_rfcomm_release,
        }));
        Ok(())
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        let Some(h) = self.handle.lock().unwrap().clone() else {
            return Err(ProtocolError::NotConnected);
        };
        let n = unsafe { murmur_rfcomm_read(h.ptr.as_ptr(), buf.as_mut_ptr(), buf.len(), 2000) };
        if n > 0 {
            Ok(n as usize)
        } else if n == 0 {
            Err(ProtocolError::NotConnected)
        } else if n == -2 {
            Err(ProtocolError::Timeout("RFCOMM read timeout".into()))
        } else {
            Err(ProtocolError::Io("RFCOMM read failed".into()))
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), ProtocolError> {
        let Some(h) = self.handle.lock().unwrap().clone() else {
            return Err(ProtocolError::NotConnected);
        };
        let rc = unsafe { murmur_rfcomm_write(h.ptr.as_ptr(), bytes.as_ptr(), bytes.len()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(ProtocolError::Io("RFCOMM write failed".into()))
        }
    }

    fn close(&self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            unsafe {
                murmur_rfcomm_disconnect(h.ptr.as_ptr());
            }
        }
    }
}

pub struct MacL2capTransport {
    psm: u16,
    handle: Mutex<Option<Arc<OwnedHandle>>>,
}

impl MacL2capTransport {
    pub fn new(psm: u16) -> Self {
        Self {
            psm,
            handle: Mutex::new(None),
        }
    }
}

impl Drop for MacL2capTransport {
    fn drop(&mut self) {
        self.close();
    }
}

impl BluetoothTransport for MacL2capTransport {
    fn is_connected(&self) -> bool {
        let Some(h) = self.handle.lock().unwrap().clone() else {
            return false;
        };
        unsafe { murmur_l2cap_is_connected(h.ptr.as_ptr()) != 0 }
    }

    fn connect(&self, address: &str, _service_uuid: &str) -> Result<(), ProtocolError> {
        self.close();
        let c_addr = CString::new(address)
            .map_err(|_| ProtocolError::Io("Invalid Bluetooth address".into()))?;
        let mut err_buf = vec![0 as c_char; 256];

        let raw = unsafe {
            murmur_l2cap_connect(
                c_addr.as_ptr(),
                self.psm,
                err_buf.as_mut_ptr(),
                err_buf.len(),
            )
        };
        let Some(nn) = NonNull::new(raw) else {
            return Err(ProtocolError::Io(cstr_err(&err_buf)));
        };
        *self.handle.lock().unwrap() = Some(Arc::new(OwnedHandle {
            ptr: nn,
            disconnect_fn: murmur_l2cap_disconnect,
            release_fn: murmur_l2cap_release,
        }));
        Ok(())
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        let Some(h) = self.handle.lock().unwrap().clone() else {
            return Err(ProtocolError::NotConnected);
        };
        let n = unsafe { murmur_l2cap_read(h.ptr.as_ptr(), buf.as_mut_ptr(), buf.len(), 2000) };
        if n > 0 {
            Ok(n as usize)
        } else if n == 0 {
            Err(ProtocolError::NotConnected)
        } else if n == -2 {
            Err(ProtocolError::Timeout("L2CAP read timeout".into()))
        } else {
            Err(ProtocolError::Io("L2CAP read failed".into()))
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), ProtocolError> {
        let Some(h) = self.handle.lock().unwrap().clone() else {
            return Err(ProtocolError::NotConnected);
        };
        let rc = unsafe { murmur_l2cap_write(h.ptr.as_ptr(), bytes.as_ptr(), bytes.len()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(ProtocolError::Io("L2CAP write failed".into()))
        }
    }

    fn close(&self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            unsafe {
                murmur_l2cap_disconnect(h.ptr.as_ptr());
            }
        }
    }
}
