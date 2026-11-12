//! Сторона компьютера: что Mac или Windows видит по USB.
//!
//! macOS сама MTP не умеет: файлы смотрят через ADB (раздел «Файлы») или сторонние программы вроде OpenMTP.
//! В Windows планшет виден в Проводнике как MTP-устройство (класс WPD).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::adb::{Adb, Res, Tag, command};
use crate::tools::home_dir;

// ---------------------------------------------------------------- macOS

/// Производители Android-устройств по USB vendor id. Samsung делает и SSD, поэтому совпадение только
/// по производителю считается «похоже на Android», а точное — по серийнику из adb.
pub const ANDROID_VENDORS: [(&str, &str); 12] = [
    ("04e8", "Samsung"),
    ("18d1", "Google"),
    ("22b8", "Motorola"),
    ("12d1", "Huawei"),
    ("2717", "Xiaomi"),
    ("2a70", "OnePlus"),
    ("0bb4", "HTC"),
    ("1004", "LG"),
    ("0fce", "Sony"),
    ("19d2", "ZTE"),
    ("22d9", "OPPO / realme"),
    ("2d95", "vivo"),
];

static ANDROID_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)android|galaxy|pixel|\bmtp\b|\badb\b|SM-[A-Z]\d").unwrap());
static HEX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*0x([0-9a-fA-F]{1,4})").unwrap());
static PAREN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((.+)\)").unwrap());

pub const MTP_APPS: [&str; 3] = ["OpenMTP.app", "Android File Transfer.app", "MacDroid.app"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbDevice {
    pub name: String,
    pub vendor: String,
    pub vendor_id: String,
    pub product_id: String,
    pub serial: String,
    pub speed: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    /// Это устройство из списка adb.
    Adb,
    /// Похоже на Android.
    Likely,
}

/// "0x04e8" или "0x04e8  (Samsung Electronics Co., Ltd.)" -> "04e8"
fn hex_id(value: &str) -> String {
    HEX_RE.captures(value).map(|c| format!("{:0>4}", c[1].to_lowercase())).unwrap_or_default()
}

fn field<'a>(n: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter().find_map(|k| n.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty())).unwrap_or("")
}

/// Обходит дерево system_profiler: новый формат (SPUSBHostDataType) и старый (SPUSBDataType).
pub fn parse_usb(items: &Value) -> Vec<UsbDevice> {
    fn walk(nodes: &Value, found: &mut Vec<UsbDevice>) {
        let Some(nodes) = nodes.as_array() else { return };
        for n in nodes.iter().filter(|n| n.is_object()) {
            let vid = field(n, &["USBDeviceKeyVendorID", "vendor_id"]);
            if !vid.is_empty() {
                let mut vendor = field(n, &["USBDeviceKeyVendorName", "manufacturer"]).to_string();
                if vendor.is_empty() {
                    vendor = PAREN_RE.captures(vid).map(|c| c[1].to_string()).unwrap_or_default();
                }
                found.push(UsbDevice {
                    name: field(n, &["_name"]).into(),
                    vendor,
                    vendor_id: hex_id(vid),
                    product_id: hex_id(field(n, &["USBDeviceKeyProductID", "product_id"])),
                    serial: field(n, &["USBDeviceKeySerialNumber", "serial_num"]).into(),
                    speed: field(n, &["USBDeviceKeyLinkSpeed", "device_speed"]).into(),
                });
            }
            if let Some(children) = n.get("_items") {
                walk(children, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(items, &mut found);
    found
}

/// USB-устройства, которые видит Mac; None, если system_profiler не ответил.
pub fn usb_devices(a: &Adb) -> Res<Option<Vec<UsbDevice>>> {
    for data_type in ["SPUSBHostDataType", "SPUSBDataType"] {
        let r = a.run(&["system_profiler".into(), data_type.into(), "-json".into()], 40.0)?;
        let Ok(json) = serde_json::from_str::<Value>(&r.out) else { continue };
        if let Some(items) = json.get(data_type).filter(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
            return Ok(Some(parse_usb(items)));
        }
    }
    Ok(None)
}

pub fn android_match(dev: &UsbDevice, adb_serials: &HashSet<String>) -> Option<Match> {
    if !dev.serial.is_empty() && adb_serials.contains(&dev.serial) {
        return Some(Match::Adb);
    }
    if ANDROID_VENDORS.iter().any(|(id, _)| *id == dev.vendor_id) || ANDROID_NAME_RE.is_match(&dev.name) {
        return Some(Match::Likely);
    }
    None
}

pub fn mtp_apps() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for folder in [PathBuf::from("/Applications"), home_dir().join("Applications")] {
        for name in MTP_APPS {
            if folder.join(name).is_dir() {
                found.push(folder.join(name));
            }
        }
    }
    found
}

// ---------------------------------------------------------------- Windows

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Portable {
    pub name: String,
    pub status: String,
    pub problem: String,
    pub instance_id: String,
    pub in_explorer: bool,
    pub children: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PnpDevice {
    pub status: String,
    pub class: String,
    pub name: String,
    pub problem: String,
}

/// ConvertTo-Json отдаёт объект вместо массива, если элемент один.
fn as_list(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::Null => vec![],
        other => vec![other],
    }
}

fn s(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub fn parse_portable(json: &str) -> Vec<Portable> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    as_list(v)
        .iter()
        .map(|d| Portable {
            name: s(d, "Name"),
            status: s(d, "Status"),
            problem: s(d, "Problem"),
            instance_id: s(d, "InstanceId"),
            in_explorer: d.get("InExplorer").and_then(Value::as_bool).unwrap_or(false),
            children: as_list(d.get("Children").cloned().unwrap_or(Value::Null))
                .iter()
                .filter_map(|c| c.as_str().map(String::from))
                .collect(),
        })
        .collect()
}

pub fn parse_pnp(json: &str) -> Vec<PnpDevice> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    as_list(v)
        .iter()
        .map(|d| PnpDevice {
            status: s(d, "Status"),
            class: s(d, "Class"),
            name: s(d, "FriendlyName"),
            problem: s(d, "Problem"),
        })
        .collect()
}

const PS_PREFIX: &str = "$ErrorActionPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; ";

fn powershell(a: &Adb, script: &str, timeout: f64) -> Res<String> {
    let argv = ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", &format!("{PS_PREFIX}{script}")]
        .map(String::from);
    Ok(a.run(&argv, timeout)?.out)
}

/// MTP/WPD-устройства Windows и то, что Проводник видит внутри них.
pub fn windows_portable(a: &Adb) -> Res<Vec<Portable>> {
    let script = "$wpd=@(Get-PnpDevice -PresentOnly -Class WPD); $shell=New-Object -ComObject Shell.Application; \
        $pc=@($shell.NameSpace(17).Items()); \
        $r=@(foreach($d in $wpd){ $item=$pc | Where-Object { $_.Name -eq $d.FriendlyName } | Select-Object -First 1; \
        $names=@(); if($item){ $names=@($item.GetFolder.Items() | ForEach-Object { $_.Name }) }; \
        [pscustomobject]@{ Name=$d.FriendlyName; Status=\"$($d.Status)\"; Problem=\"$($d.Problem)\"; \
        InstanceId=$d.InstanceId; InExplorer=[bool]$item; Children=$names } }); \
        ConvertTo-Json -InputObject $r -Depth 3 -Compress";
    Ok(parse_portable(&powershell(a, script, 60.0)?))
}

pub fn windows_usb(a: &Adb) -> Res<Vec<PnpDevice>> {
    let script = "$d=@(Get-PnpDevice -PresentOnly | Where-Object { $_.Class -in 'WPD','AndroidUsbDeviceClass' -or \
        ($_.Class -eq 'USB' -and $_.FriendlyName -match 'Composite|Android|MTP|SAMSUNG|Mobile') } | \
        ForEach-Object { [pscustomobject]@{ Status=\"$($_.Status)\"; Class=$_.Class; FriendlyName=$_.FriendlyName; \
        Problem=\"$($_.Problem)\" } }); ConvertTo-Json -InputObject $d -Compress";
    Ok(parse_pnp(&powershell(a, script, 60.0)?))
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Перезапуск MTP-драйвера: pnputil от администратора (Windows покажет запрос UAC).
pub fn restart_mtp_driver(a: &Adb) -> Res<()> {
    let devs = windows_portable(a)?;
    if devs.is_empty() {
        a.log("MTP-устройств для перезапуска нет.", Tag::Warn);
        return Ok(());
    }
    let lines: Vec<String> =
        devs.iter().map(|d| format!("pnputil /restart-device {}", ps_quote(&d.instance_id))).collect();
    let lines = lines.join("; ");
    a.log(format!("> {lines}"), Tag::Head);
    let script = format!(
        "try {{ Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -Wait -ArgumentList @('-NoProfile','-Command',{}); 'ok' }} catch {{ 'fail' }}",
        ps_quote(&lines)
    );
    if powershell(a, &script, 120.0)?.trim() == "ok" {
        a.log("Драйвер перезапущен. Подождите пару секунд и нажмите F5 в Проводнике.", Tag::Ok);
    } else {
        a.log("Запрос администратора отклонён или не удался.", Tag::Err);
    }
    Ok(())
}

pub fn open_explorer() -> std::io::Result<()> {
    command("explorer.exe").arg("shell:MyComputerFolder").spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mac_new_format() {
        let items = json!([{"_name": "USB 3.1 Bus", "_items": [
            {"_name": "Galaxy Tab A9", "USBDeviceKeyVendorID": "0x04e8", "USBDeviceKeyProductID": "0x6860",
             "USBDeviceKeyVendorName": "SAMSUNG", "USBDeviceKeySerialNumber": "R9XW30ABCDE",
             "USBDeviceKeyLinkSpeed": "480 Mb/s"},
            {"_name": "USB3.1 Hub", "USBDeviceKeyVendorID": "0x2109", "_items": [
                {"_name": "2.4G Mouse", "USBDeviceKeyVendorID": "0x1ea7", "USBDeviceKeyProductID": "0x0066"}]}
        ]}]);
        let devs = parse_usb(&items);
        assert_eq!(
            devs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["Galaxy Tab A9", "USB3.1 Hub", "2.4G Mouse"]
        );
        assert_eq!(devs[0].vendor_id, "04e8");
        let serials: HashSet<String> = ["R9XW30ABCDE".to_string()].into();
        assert_eq!(android_match(&devs[0], &serials), Some(Match::Adb));
        assert_eq!(android_match(&devs[0], &HashSet::new()), Some(Match::Likely));
        assert_eq!(android_match(&devs[2], &HashSet::new()), None);
    }

    #[test]
    fn mac_old_format() {
        let items = json!([{"_name": "USB31Bus", "_items": [
            {"_name": "SAMSUNG_Android", "vendor_id": "0x04e8  (Samsung Electronics Co., Ltd.)",
             "product_id": "0x6860", "serial_num": "R9XW30ABCDE", "device_speed": "high_speed"}]}]);
        let devs = parse_usb(&items);
        assert_eq!(devs[0].vendor, "Samsung Electronics Co., Ltd.");
        assert_eq!(devs[0].product_id, "6860");
        assert_eq!(devs[0].serial, "R9XW30ABCDE");
    }

    #[test]
    fn windows_json() {
        let one = r#"{"Name":"Galaxy Tab A9","Status":"OK","Problem":"CM_PROB_NONE","InstanceId":"USB\\VID_04E8&PID_6860\\R9X","InExplorer":true,"Children":"Internal storage"}"#;
        let p = parse_portable(one);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].children, vec!["Internal storage".to_string()]);
        assert!(p[0].in_explorer);
        assert!(parse_portable("").is_empty());
        let pnp = parse_pnp(r#"[{"Status":"OK","Class":"WPD","FriendlyName":"Galaxy","Problem":"CM_PROB_NONE"}]"#);
        assert_eq!(pnp[0].class, "WPD");
    }

    #[test]
    fn quotes_for_powershell() {
        assert_eq!(ps_quote("a'b"), "'a''b'");
    }
}
