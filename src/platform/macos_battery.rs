use serde_json::Value;

use crate::models::{BatteryState, BatteryUnit, ChargingStatus};

fn normalize_mac(addr: &str) -> String {
    addr.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn parse_percent_value(val: &Value) -> Option<i32> {
    match val {
        Value::Number(n) => n.as_i64().map(|v| v as i32).filter(|v| (0..=100).contains(v)),
        Value::String(s) => {
            let trimmed = s.trim().trim_end_matches('%').trim();
            trimmed.parse::<i32>().ok().filter(|v| (0..=100).contains(v))
        }
        _ => None,
    }
}

fn build_battery_state(
    left: Option<i32>,
    right: Option<i32>,
    case_pct: Option<i32>,
    single: Option<i32>,
    is_over_ear: bool,
) -> Option<BatteryState> {
    if left.is_none() && right.is_none() && case_pct.is_none() && single.is_none() {
        return None;
    }

    if is_over_ear || (left.is_none() && right.is_none() && single.is_some()) {
        let lvl = single.or(left).or(right)?;
        return Some(BatteryState {
            level: Some(lvl),
            charging_status: ChargingStatus::NotCharging,
            left: None,
            right: None,
            case_battery: None,
        });
    }

    let min_bud = [left, right].into_iter().flatten().min().or(single);
    Some(BatteryState {
        level: min_bud,
        charging_status: ChargingStatus::NotCharging,
        left: left.map(|l| BatteryUnit::new(l, ChargingStatus::NotCharging)),
        right: right.map(|l| BatteryUnit::new(l, ChargingStatus::NotCharging)),
        case_battery: case_pct.map(|l| BatteryUnit::new(l, ChargingStatus::NotCharging)),
    })
}

fn extract_from_device_map(
    map: &serde_json::Map<String, Value>,
    is_over_ear: bool,
) -> Option<BatteryState> {
    let left = map.get("device_batteryLevelLeft").and_then(parse_percent_value);
    let right = map.get("device_batteryLevelRight").and_then(parse_percent_value);
    let case_pct = map.get("device_batteryLevelCase").and_then(parse_percent_value);
    let single = map
        .get("device_batteryLevelMain")
        .or_else(|| map.get("device_batteryLevel"))
        .and_then(parse_percent_value);

    build_battery_state(left, right, case_pct, single, is_over_ear)
}

fn find_device_in_list(list: &[Value], target_mac: &str, is_over_ear: bool) -> Option<BatteryState> {
    for entry in list {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        for (_device_name, props) in obj {
            let Some(props_map) = props.as_object() else {
                continue;
            };
            let Some(addr_val) = props_map.get("device_address").and_then(|v| v.as_str()) else {
                continue;
            };
            if normalize_mac(addr_val) == target_mac {
                return extract_from_device_map(props_map, is_over_ear);
            }
        }
    }
    None
}

/// Parses the JSON output of `system_profiler SPBluetoothDataType -json` for a specific device MAC.
pub fn parse_system_profiler_bluetooth_battery(
    json: &str,
    address: &str,
    is_over_ear: bool,
) -> Option<BatteryState> {
    let target_mac = normalize_mac(address);
    if target_mac.is_empty() {
        return None;
    }

    let root: Value = serde_json::from_str(json).ok()?;
    let items = root.get("SPBluetoothDataType")?.as_array()?;

    for section in items {
        let Some(obj) = section.as_object() else {
            continue;
        };
        for key in ["device_connected", "device_not_connected"] {
            if let Some(list) = obj.get(key).and_then(|v| v.as_array()) {
                if let Some(found) = find_device_in_list(list, &target_mac, is_over_ear) {
                    return Some(found);
                }
            }
        }
    }

    None
}

/// Invokes `system_profiler SPBluetoothDataType -json` on macOS to read Apple battery levels.
#[cfg(target_os = "macos")]
pub fn read_system_profiler_apple_battery(address: &str, is_over_ear: bool) -> Option<BatteryState> {
    let output = std::process::Command::new("/usr/sbin/system_profiler")
        .args(["SPBluetoothDataType", "-json"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_system_profiler_bluetooth_battery(&stdout, address, is_over_ear)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dual_bud_and_case_airpods_pro() {
        let json = r#"{
            "SPBluetoothDataType": [
                {
                    "device_connected": [
                        {
                            "Jesse's AirPods Pro": {
                                "device_address": "AA:BB:CC:DD:EE:FF",
                                "device_batteryLevelLeft": "92%",
                                "device_batteryLevelRight": "88%",
                                "device_batteryLevelCase": "65%"
                            }
                        }
                    ]
                }
            ]
        }"#;

        let batt = parse_system_profiler_bluetooth_battery(json, "aa:bb:cc:dd:ee:ff", false)
            .expect("should parse AirPods Pro battery");
        assert_eq!(
            batt.left,
            Some(BatteryUnit::new(92, ChargingStatus::NotCharging))
        );
        assert_eq!(
            batt.right,
            Some(BatteryUnit::new(88, ChargingStatus::NotCharging))
        );
        assert_eq!(
            batt.case_battery,
            Some(BatteryUnit::new(65, ChargingStatus::NotCharging))
        );
        assert_eq!(batt.level, Some(88));
    }

    #[test]
    fn parses_airpods_max_single_battery_with_hyphen_address() {
        let json = r#"{
            "SPBluetoothDataType": [
                {
                    "device_connected": [
                        {
                            "AirPods Max": {
                                "device_address": "11-22-33-44-55-66",
                                "device_batteryLevelMain": "76%"
                            }
                        }
                    ]
                }
            ]
        }"#;

        let batt = parse_system_profiler_bluetooth_battery(json, "11:22:33:44:55:66", true)
            .expect("should parse AirPods Max battery");
        assert_eq!(batt.level, Some(76));
        assert_eq!(batt.left, None);
        assert_eq!(batt.right, None);
        assert_eq!(batt.case_battery, None);
    }

    #[test]
    fn parses_missing_right_bud_in_not_connected_or_partial() {
        let json = r#"{
            "SPBluetoothDataType": [
                {
                    "device_not_connected": [
                        {
                            "AirPods 3": {
                                "device_address": "AA-11-BB-22-CC-33",
                                "device_batteryLevelLeft": "70%",
                                "device_batteryLevelCase": "100%"
                            }
                        }
                    ]
                }
            ]
        }"#;

        let batt = parse_system_profiler_bluetooth_battery(json, "AA:11:BB:22:CC:33", false)
            .expect("should parse partial AirPods battery");
        assert_eq!(
            batt.left,
            Some(BatteryUnit::new(70, ChargingStatus::NotCharging))
        );
        assert_eq!(batt.right, None);
        assert_eq!(
            batt.case_battery,
            Some(BatteryUnit::new(100, ChargingStatus::NotCharging))
        );
        assert_eq!(batt.level, Some(70));
    }

    #[test]
    fn rejects_invalid_or_missing_json() {
        assert_eq!(
            parse_system_profiler_bluetooth_battery("not json", "AA:BB:CC:DD:EE:FF", false),
            None
        );
        let empty = r#"{"SPBluetoothDataType": [{"device_connected": []}]}"#;
        assert_eq!(
            parse_system_profiler_bluetooth_battery(empty, "AA:BB:CC:DD:EE:FF", false),
            None
        );
    }
}
