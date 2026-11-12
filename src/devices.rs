//! Разбор `adb devices -l` и текст статуса устройства.

use std::sync::LazyLock;

use regex::Regex;

use crate::adb::Tag;

static DEVICE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(\S+)\s+(device|unauthorized|offline|recovery|sideload|bootloader|authorizing|connecting|no permissions\S*)\s*(.*)$",
    )
    .unwrap()
});
static MODEL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"model:(\S+)").unwrap());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    pub state: String,
    pub model: String,
}

impl Device {
    #[cfg(test)]
    pub fn new(serial: &str, state: &str, model: &str) -> Self {
        Device { serial: serial.into(), state: state.into(), model: model.into() }
    }

    pub fn title(&self) -> String {
        if self.model.is_empty() { "Android-устройство".into() } else { self.model.clone() }
    }

    /// Подключено по сети (`adb connect`, беспроводная отладка), а не кабелем.
    pub fn is_wireless(&self) -> bool {
        self.serial.contains(':') || self.serial.contains("._adb-tls-connect")
    }
}

/// Разбирает вывод `adb devices -l`. Серийник берётся из первой колонки, а не из `model:`.
pub fn parse_devices(text: &str) -> Vec<Device> {
    text.lines()
        .filter_map(|line| {
            let m = DEVICE_RE.captures(line)?;
            let model = MODEL_RE.captures(&m[3]).map(|c| c[1].replace('_', " ")).unwrap_or_default();
            Some(Device { serial: m[1].to_string(), state: m[2].to_string(), model })
        })
        .collect()
}

/// Короткий статус для бейджа и подробное пояснение.
pub fn state_text(state: &str) -> (&'static str, &'static str, Tag) {
    match state {
        "device" => ("Подключено", "Устройство отвечает, команды можно выполнять.", Tag::Ok),
        "unauthorized" => {
            ("Нет разрешения", "Разблокируйте экран планшета и нажмите «Разрешить» в окне отладки по USB.", Tag::Warn)
        }
        "offline" => ("Offline", "Переподключите кабель или перезапустите ADB-сервер.", Tag::Err),
        "authorizing" => ("Авторизация", "Идёт авторизация, подождите.", Tag::Warn),
        "recovery" => ("Recovery", "Устройство в режиме восстановления.", Tag::Warn),
        "sideload" => ("Sideload", "Устройство в режиме sideload.", Tag::Warn),
        "bootloader" => ("Bootloader", "Устройство в загрузчике.", Tag::Warn),
        _ => ("Нет доступа", "У системы нет прав на USB-устройство.", Tag::Warn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_comes_from_first_column() {
        let text = "* daemon started successfully\n\
            List of devices attached\n\
            R9XW30ABCDE            device usb:1-1 product:gta9wifixx model:SM_X115 device:gta9wifi transport_id:1\n\
            emulator-5554          unauthorized transport_id:2\n\
            0123456789ABCDEF       no permissions (missing udev rules?); see [http://developer.android.com]\n";
        assert_eq!(
            parse_devices(text),
            vec![
                Device::new("R9XW30ABCDE", "device", "SM X115"),
                Device::new("emulator-5554", "unauthorized", ""),
                Device::new("0123456789ABCDEF", "no permissions", ""),
            ]
        );
    }

    #[test]
    fn wireless() {
        assert!(Device::new("192.168.1.102:5555", "device", "").is_wireless());
        assert!(!Device::new("R9XW30ABCDE", "device", "").is_wireless());
    }
}
