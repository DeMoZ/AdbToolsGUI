//! Обновление программы из релизов GitHub, как в Windows-версии EnhanceCapCut (`AppUpdates`), на обеих системах.
//!
//! Последний релиз берётся из GitHub API; в нём ищется архив для своей системы
//! (`…-macos-arm64.zip` с `ADB Toolkit.app` или `…-windows-x64.zip` с `ADB Toolkit.exe`). Скачанный архив сверяется
//! с SHA-256, который GitHub публикует для каждого файла релиза, распаковывается во временную папку, а заменяет
//! программу небольшой скрипт уже после её выхода и запускает новую версию.

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::adb::command;
use crate::net;

pub const REPO: &str = "DeMoZ/AdbToolsGUI";
const WORK_PREFIX: &str = "ADB-Toolkit-update-";

/// Суффикс архива для этой системы.
pub fn asset_suffix() -> &'static str {
    if cfg!(windows) { "-windows-x64.zip" } else { "-macos-arm64.zip" }
}

pub fn latest_api() -> String {
    std::env::var("ADBTOOLKIT_UPDATES_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

pub type Version = (u32, u32, u32);

pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim().trim_start_matches('v');
    let mut parts = s.split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

pub fn current() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

pub fn format_version(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub zip_url: String,
    pub sha256: Option<String>,
    pub page_url: String,
    pub notes: String,
}

/// Ответ GitHub «последний релиз»: тег, архив для этой системы и его SHA-256. None — в релизе нет архива для системы.
pub fn parse_release(json: &str, suffix: &str) -> Option<Release> {
    let v: Value = serde_json::from_str(json).ok()?;
    let version = parse_version(v.get("tag_name")?.as_str()?)?;
    let asset = v
        .get("assets")?
        .as_array()?
        .iter()
        .find(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| n.to_lowercase().ends_with(suffix)))?;
    let zip_url = asset.get("browser_download_url")?.as_str()?.to_string();
    // "sha256:<hex>"; у файлов, загруженных до того, как GitHub начал считать суммы, её нет
    let sha256 =
        asset.get("digest").and_then(Value::as_str).and_then(|d| d.strip_prefix("sha256:")).map(|h| h.to_lowercase());
    Some(Release {
        version,
        zip_url,
        sha256,
        page_url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        notes: release_notes(v.get("body").and_then(Value::as_str).unwrap_or_default()),
    })
}

/// Описание релиза для окна обновления: без раздела «Установка» и разметки заголовков.
fn release_notes(body: &str) -> String {
    let body = body.split("\n## Установка").next().unwrap_or(body);
    body.lines().map(|l| l.trim_start_matches('#').trim_start()).collect::<Vec<_>>().join("\n").trim().to_string()
}

pub fn latest() -> Result<Option<Release>, String> {
    let data = net::get(&latest_api(), 20)?;
    Ok(parse_release(&String::from_utf8_lossy(&data), asset_suffix()))
}

/// Где лежит программа, которую заменит обновление: `ADB Toolkit.app` на Mac, папка с .exe в Windows.
/// None — сборка для разработки (`cargo run`) или программа запущена не из .app.
pub fn install_target() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return None;
    }
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    if cfg!(target_os = "macos") {
        let app = exe.ancestors().nth(3)?;
        return (app.extension()? == "app").then(|| app.to_path_buf());
    }
    exe.parent().map(Path::to_path_buf)
}

/// Можно ли писать туда, где лежит программа (на Mac — в папку с .app).
pub fn can_write(target: &Path) -> bool {
    let folder = if cfg!(target_os = "macos") { target.parent() } else { Some(target) };
    let Some(folder) = folder else { return false };
    let probe = folder.join(format!(".adbtoolkit-update-check-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Скачивает архив, сверяет SHA-256 и распаковывает. Возвращает распакованную программу (.app или папку с .exe).
pub fn download(release: &Release) -> Result<PathBuf, String> {
    let work = std::env::temp_dir().join(format!("{WORK_PREFIX}{}", format_version(release.version)));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let zip = work.join("update.zip");
    net::download(&release.zip_url, &zip)?;
    if let Some(expected) = &release.sha256 {
        let actual = sha256_file(&zip).map_err(|e| e.to_string())?;
        if &actual != expected {
            return Err(format!(
                "скачанный архив не совпадает с опубликованной суммой SHA-256 ({actual} ≠ {expected})"
            ));
        }
    }
    let unpacked = work.join("app");
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let status = if cfg!(windows) {
        command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "Expand-Archive -LiteralPath {} -DestinationPath {} -Force",
                ps_quote(&zip),
                ps_quote(&unpacked)
            ))
            .status()
    } else {
        // ditto сохраняет подпись и атрибуты .app
        command("/usr/bin/ditto").arg("-x").arg("-k").arg(&zip).arg(&unpacked).status()
    };
    if !status.map(|s| s.success()).unwrap_or(false) {
        return Err("не удалось распаковать архив обновления".into());
    }
    let _ = std::fs::remove_file(&zip);
    let found = if cfg!(windows) {
        find(&unpacked, &|p| p.file_name().is_some_and(|n| n == "ADB Toolkit.exe"))
            .and_then(|p| p.parent().map(Path::to_path_buf))
    } else {
        find(&unpacked, &|p| p.extension().is_some_and(|e| e == "app"))
    };
    found.ok_or_else(|| "в архиве обновления нет программы".into())
}

fn find(dir: &Path, pred: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if pred(&p) {
            return Some(p);
        }
        if p.is_dir()
            && p.extension().is_none_or(|e| e != "app")
            && let Some(found) = find(&p, pred)
        {
            return Some(found);
        }
    }
    None
}

fn ps_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "''"))
}

fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

/// Папка загрузки (ADB-Toolkit-update-<версия>), которую скрипт удалит после копирования.
fn work_dir(source: &Path) -> PathBuf {
    source
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(WORK_PREFIX)))
        .unwrap_or(source)
        .to_path_buf()
}

/// Скрипт для Mac: ждёт выхода программы (`pid`), меняет .app на новую и запускает её.
/// Если заменить не удалось, возвращает старую и запускает её с `--update-failed`.
pub fn mac_script(source: &Path, target: &Path, pid: u32) -> String {
    let old = target.with_extension("app.old");
    let (s, t, o, w) = (sh_quote(source), sh_quote(target), sh_quote(&old), sh_quote(&work_dir(source)));
    format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done\n\
         sleep 0.5\n\
         rm -rf {o}\n\
         if mv {t} {o} && mv {s} {t}; then\n\
         \x20 rm -rf {o} {w}\n\
         \x20 xattr -dr com.apple.quarantine {t} 2>/dev/null\n\
         \x20 open {t}\n\
         else\n\
         \x20 [ -e {t} ] || mv {o} {t}\n\
         \x20 open {t} --args --update-failed\n\
         fi\n"
    )
}

/// Скрипт для Windows: ждёт выхода программы, копирует новые файлы поверх и запускает её.
pub fn windows_script(source: &Path, target: &Path, pid: u32) -> String {
    let exe = target.join("ADB Toolkit.exe");
    format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue\r\n\
         Start-Sleep -Milliseconds 500\r\n\
         robocopy {} {} /E /R:10 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null\r\n\
         if ($LASTEXITCODE -ge 8) {{ Start-Process -FilePath {} -ArgumentList '--update-failed'; exit 1 }}\r\n\
         Remove-Item -LiteralPath {} -Recurse -Force -ErrorAction SilentlyContinue\r\n\
         Start-Process -FilePath {}\r\n",
        ps_quote(source),
        ps_quote(target),
        ps_quote(&exe),
        ps_quote(&work_dir(source)),
        ps_quote(&exe),
    )
}

/// Запускает скрипт замены; после этого программа должна выйти.
pub fn start_install(source: &Path, target: &Path) -> Result<(), String> {
    let pid = std::process::id();
    if cfg!(windows) {
        let script = std::env::temp_dir().join("ADB-Toolkit-update.ps1");
        std::fs::write(&script, windows_script(source, target, pid)).map_err(|e| e.to_string())?;
        command("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
            .arg(&script)
            .spawn()
            .map_err(|e| e.to_string())?;
    } else {
        let script = std::env::temp_dir().join("ADB-Toolkit-update.sh");
        std::fs::write(&script, mac_script(source, target, pid)).map_err(|e| e.to_string())?;
        command("/bin/bash").arg(&script).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATEST: &str = r###"{"tag_name":"v0.4.0","html_url":"https://github.com/DeMoZ/AdbToolsGUI/releases/tag/v0.4.0",
        "body":"## Что нового\n- Быстрее\n## Установка\nСкачайте","assets":[
        {"name":"ADB-Toolkit-macOS-v0.4.0.dmg","browser_download_url":"https://x/dmg"},
        {"name":"ADB-Toolkit-0.4.0-macos-arm64.zip","browser_download_url":"https://x/mac.zip","digest":"sha256:ABCDEF"},
        {"name":"ADB-Toolkit-0.4.0-windows-x64.zip","browser_download_url":"https://x/win.zip"}]}"###;

    #[test]
    fn release_for_each_system() {
        let mac = parse_release(LATEST, "-macos-arm64.zip").unwrap();
        assert_eq!(mac.version, (0, 4, 0));
        assert_eq!(mac.zip_url, "https://x/mac.zip");
        assert_eq!(mac.sha256.as_deref(), Some("abcdef"));
        assert_eq!(mac.notes, "Что нового\n- Быстрее");
        let win = parse_release(LATEST, "-windows-x64.zip").unwrap();
        assert_eq!(win.sha256, None);
        assert!(parse_release(LATEST, "-linux.zip").is_none());
        assert!(parse_release("{}", "-windows-x64.zip").is_none());
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.4"), Some((0, 4, 0)));
        assert_eq!(parse_version("x"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn sha256_of_file() {
        let p = std::env::temp_dir().join(format!("adbtoolkit_sha_{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_file(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn scripts_quote_paths() {
        let src = Path::new("/tmp/ADB-Toolkit-update-0.4.0/app/ADB Toolkit.app");
        let s = mac_script(src, Path::new("/Applications/ADB Toolkit.app"), 42);
        assert!(s.contains("kill -0 42"));
        assert!(s.contains("'/Applications/ADB Toolkit.app.old'"));
        assert!(s.contains("rm -rf '/Applications/ADB Toolkit.app.old' '/tmp/ADB-Toolkit-update-0.4.0'"));
        // пути собираются через Path::join, поэтому проверка не зависит от разделителя в системе
        let work = std::env::temp_dir().join("ADB-Toolkit-update-0.4.0");
        let target = std::env::temp_dir().join("Rom's tools");
        let w = windows_script(&work.join("app"), &target, 7);
        assert!(w.contains("Wait-Process -Id 7"));
        assert!(w.contains(&ps_quote(&target.join("ADB Toolkit.exe"))));
        assert!(w.contains("Rom''s tools"));
        assert!(w.contains(&format!("Remove-Item -LiteralPath {}", ps_quote(&work))));
    }
}
