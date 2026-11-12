//! Установка приложений: APK через `adb install`, OBB в `/sdcard/Android/obb/<пакет>/`, AAB через bundletool.

use std::path::Path;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::adb::{Adb, Res, Tag, server_up, start_server};
use crate::tools::{file_name, home_dir};

static OBB_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:main|patch)\.\d+\.([A-Za-z]\w*(?:\.\w+)+)\.obb$").unwrap());

/// Имя пакета из имени OBB: main.<версия>.<пакет>.obb или patch.<версия>.<пакет>.obb.
pub fn obb_package(path: &str) -> Option<String> {
    OBB_RE.captures(file_name(path)).map(|c| c[1].to_string())
}

#[derive(Clone, Debug, Default)]
pub struct DropPlan {
    pub apks: Vec<String>,
    pub obbs: Vec<String>,
    pub aabs: Vec<String>,
    pub ignored: Vec<String>,
    pub error: Option<&'static str>,
}

fn ext(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Разбирает файлы по расширениям и решает, что с ними делать.
pub fn drop_plan(paths: &[String]) -> DropPlan {
    let mut plan = DropPlan::default();
    for p in paths {
        if Path::new(p).is_dir() {
            plan.ignored.push(p.clone());
            continue;
        }
        match ext(p).as_str() {
            "apk" => plan.apks.push(p.clone()),
            "obb" => plan.obbs.push(p.clone()),
            "aab" => plan.aabs.push(p.clone()),
            _ => plan.ignored.push(p.clone()),
        }
    }
    plan.error = if plan.aabs.len() > 1 {
        Some("Выбрано несколько AAB: ставьте их по одному.")
    } else if !plan.aabs.is_empty() && !plan.apks.is_empty() {
        Some("AAB и APK вместе: непонятно, что ставить. Выберите что-то одно (OBB можно добавить к любому).")
    } else if plan.aabs.is_empty() && plan.apks.is_empty() && plan.obbs.is_empty() {
        Some("Среди файлов нет APK, OBB или AAB.")
    } else {
        None
    };
    plan
}

impl DropPlan {
    /// Нужно спросить пакет: у какого-то OBB он не читается из имени.
    pub fn needs_obb_package(&self) -> bool {
        self.obbs.iter().any(|f| obb_package(f).is_none())
    }
}

pub fn install_hint(text: &str) -> Option<&'static str> {
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["INSTALL_FAILED_UPDATE_INCOMPATIBLE", "INSTALL_FAILED_SHARED_USER_INCOMPATIBLE"]) {
        Some(
            "Подсказка: на планшете стоит это приложение с другой подписью. Удалите его (adb uninstall <пакет>) и повторите.",
        )
    } else if has(&["INSTALL_FAILED_VERSION_DOWNGRADE"]) {
        Some("Подсказка: на планшете версия новее. Удалите её (adb uninstall <пакет>) или ставьте более новую сборку.")
    } else if has(&["INSTALL_FAILED_INSUFFICIENT_STORAGE"]) {
        Some("Подсказка: на планшете не хватает места.")
    } else if has(&[
        "INSTALL_FAILED_OLDER_SDK",
        "INSTALL_FAILED_NO_MATCHING_ABIS",
        "INSTALL_FAILED_DEPRECATED_SDK_VERSION",
    ]) {
        Some("Подсказка: сборка не подходит этому планшету (версия Android или архитектура процессора).")
    } else if has(&["USER_RESTRICTED", "canceled by user", "INSTALL_FAILED_ABORTED"]) {
        Some(
            "Подсказка: установку блокирует планшет. Посмотрите на его экран и подтвердите установку; на Samsung \
             отключите «Auto Blocker» в Настройки → Безопасность и конфиденциальность.",
        )
    } else {
        None
    }
}

fn log_hint(a: &Adb, out: &str, err: &str) {
    if let Some(h) = install_hint(&format!("{out} {err}")) {
        a.log(h, Tag::Warn);
    }
}

pub fn install_apk(a: &Adb, path: &str) -> Res<bool> {
    let r = a.adb_opts(&["install", "-r", "-t", path], true, false, 900.0)?;
    if r.ok() && r.out.contains("Success") {
        a.log("APK установлен.", Tag::Ok);
        return Ok(true);
    }
    a.log("Установить APK не удалось.", Tag::Err);
    log_hint(a, &r.out, &r.err);
    Ok(false)
}

/// Копирует OBB в /sdcard/Android/obb/<пакет>/. Пакет берётся из имени файла, иначе `fallback`.
pub fn send_obb(a: &Adb, files: &[String], fallback: &str) -> Res<bool> {
    for f in files {
        let pkg = obb_package(f).unwrap_or_else(|| fallback.to_string());
        let folder = format!("/sdcard/Android/obb/{pkg}");
        if !a.adb(&["shell", "mkdir", "-p", &folder])?.ok() {
            a.log("Не удалось создать папку для OBB на планшете.", Tag::Err);
            return Ok(false);
        }
        let target = format!("{folder}/{}", file_name(f));
        if !a.adb_opts(&["push", f, &target], true, false, 1800.0)?.ok() {
            a.log(format!("OBB не скопирован: {}", file_name(f)), Tag::Err);
            return Ok(false);
        }
    }
    a.log("OBB скопирован.", Tag::Ok);
    Ok(true)
}

/// AAB ставится через bundletool: он собирает из бандла APK под выбранное устройство и устанавливает их.
pub fn install_aab(a: &Adb, aab: &str, java: &str, bundletool: &str) -> Res<bool> {
    a.require(true)?;
    if !home_dir().join(".android").join("debug.keystore").is_file() {
        a.log(
            "Не найден ~/.android/debug.keystore: bundletool не сможет подписать APK, и установка, скорее всего, не пройдёт.",
            Tag::Warn,
        );
    }
    if !server_up() {
        start_server(a.adb_path());
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let apks = std::env::temp_dir().join(format!("adbtoolkit_{}_{stamp}.apks", std::process::id()));
    let apks_s = apks.to_string_lossy().to_string();
    let common = [format!("--device-id={}", a.serial.clone().unwrap_or_default()), format!("--adb={}", a.adb_path())];
    let result = (|| {
        let mut build: Vec<String> = vec![
            java.into(),
            "-jar".into(),
            bundletool.into(),
            "build-apks".into(),
            format!("--bundle={aab}"),
            format!("--output={apks_s}"),
            "--connected-device".into(),
        ];
        build.extend(common.iter().cloned());
        if !a.logged(&build, 600.0)?.ok() {
            a.log("Не удалось собрать APK из AAB.", Tag::Err);
            return Ok(false);
        }
        let mut install: Vec<String> =
            vec![java.into(), "-jar".into(), bundletool.into(), "install-apks".into(), format!("--apks={apks_s}")];
        install.extend(common.iter().cloned());
        let r = a.logged(&install, 600.0)?;
        if r.ok() {
            a.log("AAB установлен.", Tag::Ok);
            return Ok(true);
        }
        a.log("Установить AAB не удалось.", Tag::Err);
        log_hint(a, &r.out, &r.err);
        Ok(false)
    })();
    let _ = std::fs::remove_file(&apks);
    result
}

/// Всё для установки, собранное заранее в окне (файлы, пакет для OBB, Java и bundletool).
#[derive(Clone, Debug, Default)]
pub struct InstallJob {
    pub plan: DropPlan,
    pub obb_fallback: String,
    pub java: Option<String>,
    pub bundletool: Option<String>,
}

/// AAB или APK ставятся, OBB копируются после установки. Только OBB: приложение уже должно стоять на планшете.
pub fn install_planned(a: &Adb, job: &InstallJob) -> Res<()> {
    let plan = &job.plan;
    for f in &plan.aabs {
        a.log(format!("AAB: {}", file_name(f)), Tag::Head);
        let (Some(java), Some(bt)) = (&job.java, &job.bundletool) else { return Ok(()) };
        if !install_aab(a, f, java, bt)? {
            return Ok(());
        }
    }
    for f in &plan.apks {
        a.log(format!("APK: {}", file_name(f)), Tag::Head);
        if !install_apk(a, f)? {
            return Ok(());
        }
    }
    if !plan.obbs.is_empty() {
        if plan.aabs.is_empty() && plan.apks.is_empty() {
            a.log("APK не передан: копирую только OBB, приложение должно уже стоять на планшете.", Tag::Warn);
        }
        send_obb(a, &plan.obbs, &job.obb_fallback)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_from_name() {
        let cases = [
            ("main.123.com.foo.bar.obb", Some("com.foo.bar")),
            ("patch.5.com.x_y.z1.obb", Some("com.x_y.z1")),
            (r"C:\dl\main.7.org.a.b.obb", Some("org.a.b")),
            ("/Users/me/Downloads/main.7.org.a.b.obb", Some("org.a.b")),
            ("game.obb", None),
            ("main.1.nopkg.obb", None),
        ];
        for (name, expected) in cases {
            assert_eq!(obb_package(name).as_deref(), expected, "{name}");
        }
    }

    fn check(paths: &[&str], apks: usize, obbs: usize, aabs: usize, error: bool) {
        let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        let p = drop_plan(&paths);
        assert_eq!(
            (p.apks.len(), p.obbs.len(), p.aabs.len(), p.error.is_some()),
            (apks, obbs, aabs, error),
            "{paths:?}"
        );
    }

    #[test]
    fn plan_cases() {
        check(&["a.apk", "b.OBB"], 1, 1, 0, false);
        check(&["x.aab", "main.1.a.b.obb"], 0, 1, 1, false);
        check(&["only.obb"], 0, 1, 0, false);
        check(&["x.aab", "y.apk"], 1, 0, 1, true);
        check(&["x.aab", "y.aab"], 0, 0, 2, true);
        check(&["notes.txt"], 0, 0, 0, true);
    }

    #[test]
    fn folder_is_ignored() {
        let dir = std::env::temp_dir().join(format!("adbtoolkit_test_{}", std::process::id()));
        let folder = dir.join("build.apk");
        std::fs::create_dir_all(&folder).unwrap();
        let folder_s = folder.to_string_lossy().to_string();
        let p = drop_plan(&[folder_s.clone(), "a.apk".into()]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(p.ignored, vec![folder_s]);
        assert_eq!(p.apks, vec!["a.apk".to_string()]);
    }

    #[test]
    fn hints() {
        assert!(install_hint("Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: ...]").unwrap().contains("подписью"));
        assert!(install_hint("INSTALL_FAILED_ABORTED: User rejected permissions").unwrap().contains("Auto Blocker"));
        assert!(install_hint("Success").is_none());
    }
}
