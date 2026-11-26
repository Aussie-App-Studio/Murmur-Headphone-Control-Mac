use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, Notify};

use crate::catalog::{resolve_device_profile, HeadphoneFormFactor};
use crate::models::{
    AudioCodec, BatteryState, BatteryUnit, ChargingStatus, EarDetectionStatus, EarPlacement,
    HeadphoneCapabilities, HeadphoneFinish, HeadphoneState, MdrProtocolVersion, NoiseControlMode,
    NoiseControlState,
};
use crate::protocol::transport::{BluetoothTransport, HeadphoneSession, ProtocolError, AIRPODS_UUID};

pub const L2CAP_PSM_AAP: u16 = 0x1001;

pub const OPCODE_BATTERY: u16 = 0x0004;
pub const OPCODE_EAR_DETECTION: u16 = 0x0006;
pub const OPCODE_SETTINGS: u16 = 0x0009;
pub const OPCODE_ENABLE_NOTIFICATIONS: u16 = 0x000F;
pub const OPCODE_EXTENDED_FEATURES: u16 = 0x004D;

pub const COMPONENT_SINGLE: u8 = 0x01;
pub const COMPONENT_RIGHT: u8 = 0x02;
pub const COMPONENT_LEFT: u8 = 0x04;
pub const COMPONENT_CASE: u8 = 0x08;

pub const SETTING_IN_EAR_DETECTION: u8 = 0x0A;
pub const SETTING_LISTENING_MODE: u8 = 0x0D;
pub const SETTING_CONVERSATIONAL_AWARENESS: u8 = 0x28;

pub fn handshake_packet() -> [u8; 16] {
    [
        0x00, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]
}

pub fn enable_notifications_packet() -> [u8; 10] {
    [0x04, 0x00, 0x04, 0x00, 0x0F, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]
}

pub fn enable_extended_features_packet() -> [u8; 14] {
    [
        0x04, 0x00, 0x04, 0x00, 0x4D, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]
}

pub fn setting_packet(setting_id: u8, value_byte: u8) -> [u8; 11] {
    [
        0x04, 0x00, 0x04, 0x00, 0x09, 0x00, setting_id, value_byte, 0x00, 0x00, 0x00,
    ]
}

pub fn set_listening_mode(mode: NoiseControlMode) -> [u8; 11] {
    let mode_byte = match mode {
        NoiseControlMode::Off => 0x01,
        NoiseControlMode::NoiseCancelling => 0x02,
        NoiseControlMode::AmbientSound => 0x03,
        NoiseControlMode::Adaptive => 0x04,
    };
    setting_packet(SETTING_LISTENING_MODE, mode_byte)
}

pub fn set_conversational_awareness(enabled: bool) -> [u8; 11] {
    setting_packet(
        SETTING_CONVERSATIONAL_AWARENESS,
        if enabled { 0x01 } else { 0x02 },
    )
}

pub fn set_in_ear_detection(enabled: bool) -> [u8; 11] {
    setting_packet(SETTING_IN_EAR_DETECTION, if enabled { 0x01 } else { 0x02 })
}

pub fn read_opcode(packet: &[u8]) -> Option<u16> {
    if packet.len() < 6 {
        return None;
    }
    Some(u16::from_le_bytes([packet[4], packet[5]]))
}

pub fn apply_packet(state: &HeadphoneState, packet: &[u8]) -> HeadphoneState {
    if packet.len() < 6 || packet[0] != 0x04 || packet[2] != 0x04 {
        return state.clone();
    }

    match read_opcode(packet) {
        Some(OPCODE_BATTERY) => apply_battery_packet(state, packet),
        Some(OPCODE_SETTINGS) => apply_settings_packet(state, packet),
        Some(OPCODE_EAR_DETECTION) => apply_ear_detection_packet(state, packet),
        _ => state.clone(),
    }
}

fn apply_battery_packet(state: &HeadphoneState, packet: &[u8]) -> HeadphoneState {
    if packet.len() < 7 {
        return state.clone();
    }
    let count = packet[6] as usize;

    let mut single_level = state.battery.level;
    let mut single_charging = state.battery.charging_status;
    let mut left = state.battery.left.clone();
    let mut right = state.battery.right.clone();
    let mut case_battery = state.battery.case_battery.clone();

    for i in 0..count {
        let off = 7 + i * 5;
        if off + 4 >= packet.len() {
            break;
        }
        let comp_type = packet[off];
        let lvl = packet[off + 2] as i32;
        let status_byte = packet[off + 3];

        let chg = match status_byte {
            0x01 => ChargingStatus::Charging,
            _ => ChargingStatus::NotCharging,
        };
        let unit = if status_byte != 0x04 && (0..=100).contains(&lvl) {
            Some(BatteryUnit::new(lvl, chg))
        } else {
            None
        };

        match comp_type {
            COMPONENT_SINGLE => {
                single_level = unit.as_ref().map(|u| u.level);
                single_charging = unit
                    .as_ref()
                    .map(|u| u.charging_status)
                    .unwrap_or(ChargingStatus::NotCharging);
            }
            COMPONENT_RIGHT => right = unit,
            COMPONENT_LEFT => left = unit,
            COMPONENT_CASE => case_battery = unit,
            _ => {}
        }
    }

    let mut combined = BatteryState {
        level: single_level,
        charging_status: single_charging,
        left,
        right,
        case_battery,
    };
    if combined.is_dual_battery() {
        combined.level = combined.effective_level();
    }

    let mut next = state.clone();
    next.battery = combined;
    next
}

fn apply_settings_packet(state: &HeadphoneState, packet: &[u8]) -> HeadphoneState {
    if packet.len() < 8 {
        return state.clone();
    }
    let setting_id = packet[6];
    let val_byte = packet[7];

    let mut next = state.clone();
    match setting_id {
        SETTING_LISTENING_MODE => {
            next.noise_control.mode = match val_byte {
                0x01 => NoiseControlMode::Off,
                0x02 => NoiseControlMode::NoiseCancelling,
                0x03 => NoiseControlMode::AmbientSound,
                0x04 => NoiseControlMode::Adaptive,
                _ => state.noise_control.mode,
            };
        }
        SETTING_CONVERSATIONAL_AWARENESS => {
            next.speak_to_chat = val_byte == 0x01;
        }
        SETTING_IN_EAR_DETECTION => {
            next.wearing_detection = val_byte == 0x01;
        }
        _ => {}
    }
    next
}

fn apply_ear_detection_packet(state: &HeadphoneState, packet: &[u8]) -> HeadphoneState {
    if packet.len() < 8 {
        return state.clone();
    }
    let mut next = state.clone();
    next.ear_detection = Some(EarDetectionStatus {
        left: EarPlacement::from_byte(packet[6]),
        right: EarPlacement::from_byte(packet[7]),
    });
    next
}

#[derive(Default)]
struct SyncProgress {
    saw_battery: bool,
    saw_listening_mode: bool,
    saw_awareness: bool,
    saw_in_ear: bool,
}

impl SyncProgress {
    fn is_complete(&self) -> bool {
        self.saw_battery && self.saw_listening_mode && self.saw_awareness && self.saw_in_ear
    }
}

pub struct AirPodsAapSession {
    transport: Arc<dyn BluetoothTransport>,
    closed: AtomicBool,
    write_lock: tokio::sync::Mutex<()>,
    state: Mutex<HeadphoneState>,
    sync_progress: Mutex<SyncProgress>,
    sync_notify: Arc<Notify>,
    state_tx: broadcast::Sender<HeadphoneState>,
    disconnect_tx: broadcast::Sender<()>,
    reader_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl AirPodsAapSession {
    pub fn new(transport: Arc<dyn BluetoothTransport>) -> Arc<Self> {
        let (state_tx, _) = broadcast::channel(32);
        let (disconnect_tx, _) = broadcast::channel(4);
        Arc::new(Self {
            transport,
            closed: AtomicBool::new(false),
            write_lock: tokio::sync::Mutex::new(()),
            state: Mutex::new(HeadphoneState::default()),
            sync_progress: Mutex::new(SyncProgress::default()),
            sync_notify: Arc::new(Notify::new()),
            state_tx,
            disconnect_tx,
            reader_handle: Mutex::new(None),
        })
    }

    fn stop_reader(&self) {
        self.transport.close();
        if let Some(h) = self.reader_handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    fn start_reader(self: &Arc<Self>) {
        let session = Arc::clone(self);
        let handle = std::thread::Builder::new()
            .name("murmur-airpods-aap-rx".into())
            .spawn(move || {
                let mut buf = [0u8; 1024];
                while !session.closed.load(Ordering::SeqCst) && session.transport.is_connected() {
                    match session.transport.read(&mut buf) {
                        Ok(0) | Err(ProtocolError::NotConnected) => break,
                        Ok(n) => session.handle_incoming_packet(&buf[..n]),
                        Err(ProtocolError::Timeout(_)) => continue,
                        Err(e) => {
                            log::debug!("AirPods AAP reader ended: {e}");
                            break;
                        }
                    }
                }
                session.sync_notify.notify_waiters();
                if !session.closed.load(Ordering::SeqCst) {
                    let _ = session.disconnect_tx.send(());
                }
            });
        if let Ok(h) = handle {
            *self.reader_handle.lock().unwrap() = Some(h);
        }
    }

    fn handle_incoming_packet(&self, packet: &[u8]) {
        self.update_local_state(|curr| apply_packet(curr, packet));

        if packet.len() >= 6 && packet[0] == 0x04 && packet[2] == 0x04 {
            let mut progress = self.sync_progress.lock().unwrap();
            match read_opcode(packet) {
                Some(OPCODE_BATTERY) => progress.saw_battery = true,
                Some(OPCODE_SETTINGS) if packet.len() >= 8 => match packet[6] {
                    SETTING_LISTENING_MODE => progress.saw_listening_mode = true,
                    SETTING_CONVERSATIONAL_AWARENESS => progress.saw_awareness = true,
                    SETTING_IN_EAR_DETECTION => progress.saw_in_ear = true,
                    _ => {}
                },
                _ => {}
            }
            if progress.is_complete() {
                self.sync_notify.notify_waiters();
            }
        }
    }

    fn update_local_state<F: FnOnce(&HeadphoneState) -> HeadphoneState>(&self, transform: F) {
        let updated = {
            let mut st = self.state.lock().unwrap();
            let next = transform(&st);
            if next != *st {
                *st = next.clone();
                Some(next)
            } else {
                None
            }
        };
        if let Some(next) = updated {
            let _ = self.state_tx.send(next);
        }
    }

    async fn write_command(&self, bytes: &[u8]) -> Result<(), ProtocolError> {
        if self.closed.load(Ordering::SeqCst) || !self.transport.is_connected() {
            return Err(ProtocolError::NotConnected);
        }
        let _guard = self.write_lock.lock().await;
        let transport = Arc::clone(&self.transport);
        let payload = bytes.to_vec();
        tokio::task::spawn_blocking(move || transport.write(&payload))
            .await
            .map_err(|e| ProtocolError::Io(e.to_string()))?
    }
}

#[async_trait::async_trait]
impl HeadphoneSession for Arc<AirPodsAapSession> {
    fn protocol_version(&self) -> MdrProtocolVersion {
        MdrProtocolVersion::AirPodsAap
    }

    fn is_connected(&self) -> bool {
        !self.closed.load(Ordering::SeqCst) && self.transport.is_connected()
    }

    fn subscribe_state(&self) -> broadcast::Receiver<HeadphoneState> {
        self.state_tx.subscribe()
    }

    fn subscribe_disconnect(&self) -> broadcast::Receiver<()> {
        self.disconnect_tx.subscribe()
    }

    async fn connect_and_sync(
        &self,
        address: &str,
        device_name: &str,
    ) -> Result<HeadphoneState, ProtocolError> {
        self.stop_reader();
        self.closed.store(false, Ordering::SeqCst);

        let resolved_name = if device_name.trim().is_empty() {
            "AirPods".to_string()
        } else {
            device_name.trim().to_string()
        };
        let is_over_ear =
            resolve_device_profile(&resolved_name, false).form_factor == HeadphoneFormFactor::OverEar;

        {
            let mut st = self.state.lock().unwrap();
            *st = HeadphoneState {
                model_name: resolved_name,
                firmware_version: "AirPods AAP (L2CAP)".to_string(),
                detected_finish: HeadphoneFinish::WarmWhite,
                bluetooth_address: address.to_string(),
                protocol_version: MdrProtocolVersion::AirPodsAap,
                codec: AudioCodec::Aac,
                custom_capabilities: Some(HeadphoneCapabilities::airpods_aap(is_over_ear)),
                ..Default::default()
            };
        }
        *self.sync_progress.lock().unwrap() = SyncProgress::default();

        let transport = Arc::clone(&self.transport);
        let addr = address.to_string();
        tokio::task::spawn_blocking(move || transport.connect(&addr, AIRPODS_UUID))
            .await
            .map_err(|e| ProtocolError::Io(e.to_string()))??;

        self.start_reader();

        if let Err(e) = self.write_command(&handshake_packet()).await {
            self.stop_reader();
            return Err(e);
        }
        if let Err(e) = self.write_command(&enable_notifications_packet()).await {
            self.stop_reader();
            return Err(e);
        }
        if let Err(e) = self.write_command(&enable_extended_features_packet()).await {
            self.stop_reader();
            return Err(e);
        }

        let already_done = self.sync_progress.lock().unwrap().is_complete();
        if !already_done {
            let _ = tokio::time::timeout(
                Duration::from_millis(800),
                self.sync_notify.notified(),
            )
            .await;
        }

        Ok(self.state.lock().unwrap().clone())
    }

    async fn poll_battery(&self) -> Result<(), ProtocolError> {
        // AirPods push unsolicited 0x04 battery frames while AAP notifications are enabled.
        Ok(())
    }

    async fn set_noise_control(&self, state: &NoiseControlState) -> Result<(), ProtocolError> {
        self.write_command(&set_listening_mode(state.mode)).await?;
        self.update_local_state(|curr| {
            let mut next = curr.clone();
            next.noise_control.mode = state.mode;
            next
        });
        Ok(())
    }

    async fn set_speak_to_chat(&self, enabled: bool) -> Result<(), ProtocolError> {
        self.write_command(&set_conversational_awareness(enabled))
            .await?;
        self.update_local_state(|curr| {
            let mut next = curr.clone();
            next.speak_to_chat = enabled;
            next
        });
        Ok(())
    }

    async fn set_wearing_detection(&self, enabled: bool) -> Result<(), ProtocolError> {
        self.write_command(&set_in_ear_detection(enabled)).await?;
        self.update_local_state(|curr| {
            let mut next = curr.clone();
            next.wearing_detection = enabled;
            next
        });
        Ok(())
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.sync_notify.notify_waiters();
        self.stop_reader();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Condvar;

    struct SimulatedAirPodsTransport {
        connected: AtomicBool,
        rx_queue: Mutex<VecDeque<Vec<u8>>>,
        rx_cv: Condvar,
        state: Mutex<SimAirPodsState>,
    }

    struct SimAirPodsState {
        notifications_enabled: bool,
        listening_mode: u8,
        conversational_awareness: u8,
        in_ear_detection: u8,
    }

    impl SimulatedAirPodsTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                connected: AtomicBool::new(false),
                rx_queue: Mutex::new(VecDeque::new()),
                rx_cv: Condvar::new(),
                state: Mutex::new(SimAirPodsState {
                    notifications_enabled: false,
                    listening_mode: 0x02,
                    conversational_awareness: 0x01,
                    in_ear_detection: 0x01,
                }),
            })
        }

        fn push_rx(&self, packet: Vec<u8>) {
            let mut q = self.rx_queue.lock().unwrap();
            q.push_back(packet);
            self.rx_cv.notify_all();
        }
    }

    impl BluetoothTransport for SimulatedAirPodsTransport {
        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }

        fn connect(&self, _address: &str, _service_uuid: &str) -> Result<(), ProtocolError> {
            self.rx_queue.lock().unwrap().clear();
            let mut st = self.state.lock().unwrap();
            st.notifications_enabled = false;
            st.listening_mode = 0x02;
            st.conversational_awareness = 0x01;
            st.in_ear_detection = 0x01;
            self.connected.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn read(&self, buf: &mut [u8]) -> Result<usize, ProtocolError> {
            let mut q = self.rx_queue.lock().unwrap();
            while q.is_empty() {
                if !self.connected.load(Ordering::SeqCst) {
                    return Err(ProtocolError::NotConnected);
                }
                q = self.rx_cv.wait(q).unwrap();
            }
            let pkt = q.pop_front().unwrap();
            if pkt.is_empty() && !self.connected.load(Ordering::SeqCst) {
                return Err(ProtocolError::NotConnected);
            }
            let n = pkt.len().min(buf.len());
            buf[..n].copy_from_slice(&pkt[..n]);
            Ok(n)
        }

        fn write(&self, bytes: &[u8]) -> Result<(), ProtocolError> {
            if !self.connected.load(Ordering::SeqCst) {
                return Err(ProtocolError::NotConnected);
            }
            if bytes.len() < 6 {
                return Ok(());
            }
            if bytes[0] == 0x00 && bytes[4] == 0x01 && bytes[5] == 0x00 {
                return Ok(());
            }
            if bytes[0] != 0x04 || bytes[2] != 0x04 {
                return Ok(());
            }
            match read_opcode(bytes) {
                Some(OPCODE_ENABLE_NOTIFICATIONS) => {
                    let mut st = self.state.lock().unwrap();
                    if !st.notifications_enabled {
                        st.notifications_enabled = true;
                        let lm = st.listening_mode;
                        let ca = st.conversational_awareness;
                        let ie = st.in_ear_detection;
                        drop(st);
                        self.push_rx(vec![
                            0x04, 0x00, 0x04, 0x00, 0x04, 0x00, 0x03, COMPONENT_LEFT, 0x01, 91,
                            0x02, 0x01, COMPONENT_RIGHT, 0x01, 87, 0x01, 0x01, COMPONENT_CASE,
                            0x01, 64, 0x02, 0x01,
                        ]);
                        self.push_rx(vec![0x04, 0x00, 0x04, 0x00, 0x06, 0x00, 0x00, 0x01]);
                        self.push_rx(setting_packet(SETTING_LISTENING_MODE, lm).to_vec());
                        self.push_rx(setting_packet(SETTING_CONVERSATIONAL_AWARENESS, ca).to_vec());
                        self.push_rx(setting_packet(SETTING_IN_EAR_DETECTION, ie).to_vec());
                    }
                }
                Some(OPCODE_SETTINGS) if bytes.len() >= 8 => {
                    let id = bytes[6];
                    let val = bytes[7];
                    let mut st = self.state.lock().unwrap();
                    match id {
                        SETTING_LISTENING_MODE => st.listening_mode = val,
                        SETTING_CONVERSATIONAL_AWARENESS => st.conversational_awareness = val,
                        SETTING_IN_EAR_DETECTION => st.in_ear_detection = val,
                        _ => {}
                    }
                    drop(st);
                    self.push_rx(setting_packet(id, val).to_vec());
                }
                _ => {}
            }
            Ok(())
        }

        fn close(&self) {
            self.connected.store(false, Ordering::SeqCst);
            self.rx_queue.lock().unwrap().push_back(Vec::new());
            self.rx_cv.notify_all();
        }
    }

    #[tokio::test]
    async fn full_airpods_aap_sync_and_commands() {
        let transport = SimulatedAirPodsTransport::new();
        let session = AirPodsAapSession::new(transport);
        let synced = session
            .connect_and_sync("AA:BB:CC:DD:EE:FF", "AirPods Pro 2")
            .await
            .unwrap();

        assert_eq!(synced.protocol_version, MdrProtocolVersion::AirPodsAap);
        assert_eq!(synced.battery.left, Some(BatteryUnit::new(91, ChargingStatus::NotCharging)));
        assert_eq!(synced.battery.right, Some(BatteryUnit::new(87, ChargingStatus::Charging)));
        assert_eq!(synced.battery.case_battery, Some(BatteryUnit::new(64, ChargingStatus::NotCharging)));
        assert_eq!(synced.noise_control.mode, NoiseControlMode::NoiseCancelling);
        assert!(synced.speak_to_chat);
        assert!(synced.wearing_detection);
        assert_eq!(
            synced.ear_detection,
            Some(EarDetectionStatus {
                left: EarPlacement::InEar,
                right: EarPlacement::OutOfEar,
            })
        );
        assert!(synced.capabilities().supports_adaptive_audio);
        assert!(synced.capabilities().supports_noise_control);

        let mut rx = session.subscribe_state();
        session
            .set_noise_control(&NoiseControlState {
                mode: NoiseControlMode::Adaptive,
                ambient_level: 10,
                focus_on_voice: false,
            })
            .await
            .unwrap();
        let updated = rx.recv().await.unwrap();
        assert_eq!(updated.noise_control.mode, NoiseControlMode::Adaptive);

        session.set_speak_to_chat(false).await.unwrap();
        let updated = rx.recv().await.unwrap();
        assert!(!updated.speak_to_chat);

        session.set_wearing_detection(false).await.unwrap();
        let updated = rx.recv().await.unwrap();
        assert!(!updated.wearing_detection);

        session.close();
        assert!(!session.is_connected());
    }

    #[test]
    fn parses_airpods_max_single_battery_and_absent_bud() {
        let base = HeadphoneState::default();
        let single_pkt = [
            0x04, 0x00, 0x04, 0x00, 0x04, 0x00, 0x01, COMPONENT_SINGLE, 0x01, 78, 0x01, 0x01,
        ];
        let st = apply_packet(&base, &single_pkt);
        assert_eq!(st.battery.level, Some(78));
        assert_eq!(st.battery.charging_status, ChargingStatus::Charging);

        let dual_pkt = [
            0x04, 0x00, 0x04, 0x00, 0x04, 0x00, 0x02, COMPONENT_LEFT, 0x01, 0, 0x04, 0x01,
            COMPONENT_RIGHT, 0x01, 63, 0x02, 0x01,
        ];
        let st2 = apply_packet(&base, &dual_pkt);
        assert_eq!(st2.battery.left, None);
        assert_eq!(
            st2.battery.right,
            Some(BatteryUnit::new(63, ChargingStatus::NotCharging))
        );
        assert_eq!(st2.battery.level, Some(63));
    }
}
