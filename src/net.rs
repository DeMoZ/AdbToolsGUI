//! Сеть через системный `curl` (есть в macOS и в Windows 10+): без своей реализации TLS в программе.
//! Нужна только для донатов и обновлений; adb работает без сети.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::adb::command;

fn curl() -> std::process::Command {
    let mut c = command(if cfg!(windows) { "curl.exe" } else { "/usr/bin/curl" });
    // GitHub API без User-Agent отвечает отказом.
    c.args(["-fsSL", "--proto", "=https", "-A", "ADB-Toolkit"]).stdin(Stdio::null());
    c
}

/// Скачивает небольшой ответ (JSON) целиком.
pub fn get(url: &str, timeout_secs: u32) -> Result<Vec<u8>, String> {
    let out = curl()
        .args(["--max-time", &timeout_secs.to_string(), url])
        .output()
        .map_err(|e| format!("не удалось запустить curl: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("curl завершился с кодом {:?}", out.status.code())
        } else {
            err
        })
    }
}

/// Скачивает файл на диск.
pub fn download(url: &str, to: &Path) -> Result<(), String> {
    let out = curl()
        .args(["--max-time", "900", "-o"])
        .arg(to)
        .arg(url)
        .output()
        .map_err(|e| format!("не удалось запустить curl: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Открывает страницу в браузере.
pub fn open_url(url: &str) -> std::io::Result<()> {
    if cfg!(windows) {
        command("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn().map(|_| ())
    } else if cfg!(target_os = "macos") {
        command("open").arg(url).spawn().map(|_| ())
    } else {
        command("xdg-open").arg(url).spawn().map(|_| ())
    }
}

fn base_dir(var_win: &str, mac: &str, linux: &str) -> PathBuf {
    let home = crate::tools::home_dir();
    let dir = if cfg!(windows) {
        std::env::var_os(var_win).map(PathBuf::from).unwrap_or_else(|| home.join("AppData"))
    } else if cfg!(target_os = "macos") {
        home.join(mac)
    } else {
        home.join(linux)
    };
    dir.join("ADB Toolkit")
}

/// Настройки: `~/Library/Application Support/ADB Toolkit`, `%APPDATA%\ADB Toolkit`.
pub fn config_dir() -> PathBuf {
    base_dir("APPDATA", "Library/Application Support", ".config")
}

/// Кэш: `~/Library/Caches/ADB Toolkit`, `%LOCALAPPDATA%\ADB Toolkit`.
pub fn cache_dir() -> PathBuf {
    base_dir("LOCALAPPDATA", "Library/Caches", ".cache")
}
