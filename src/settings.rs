//! Настройки между запусками: тема, автопроверка обновлений, пропущенная версия.
//! Файл `settings.json` в `~/Library/Application Support/ADB Toolkit` или `%APPDATA%\ADB Toolkit`.

use serde_json::{Value, json};

use crate::net::config_dir;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// "system", "light" или "dark".
    pub theme: String,
    pub auto_update: bool,
    /// Время последней проверки обновлений (секунды Unix).
    pub last_update_check: u64,
    /// Версия, которую пользователь попросил не предлагать.
    pub skipped_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { theme: "system".into(), auto_update: true, last_update_check: 0, skipped_version: String::new() }
    }
}

impl Settings {
    pub fn from_json(text: &str) -> Self {
        let d = Settings::default();
        let Ok(v) = serde_json::from_str::<Value>(text) else { return d };
        Settings {
            theme: v.get("theme").and_then(Value::as_str).unwrap_or(&d.theme).to_string(),
            auto_update: v.get("autoUpdate").and_then(Value::as_bool).unwrap_or(d.auto_update),
            last_update_check: v.get("lastUpdateCheck").and_then(Value::as_u64).unwrap_or(0),
            skipped_version: v.get("skippedVersion").and_then(Value::as_str).unwrap_or_default().to_string(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&json!({
            "theme": self.theme,
            "autoUpdate": self.auto_update,
            "lastUpdateCheck": self.last_update_check,
            "skippedVersion": self.skipped_version,
        }))
        .unwrap_or_default()
    }

    pub fn load() -> Self {
        std::fs::read_to_string(config_dir().join("settings.json")).map(|t| Settings::from_json(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("settings.json"), self.to_json());
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let s = Settings {
            theme: "dark".into(),
            auto_update: false,
            last_update_check: 5,
            skipped_version: "0.4.0".into(),
        };
        assert_eq!(Settings::from_json(&s.to_json()), s);
        assert_eq!(Settings::from_json("garbage"), Settings::default());
        assert!(Settings::from_json(r#"{"theme":"light"}"#).auto_update);
    }
}
