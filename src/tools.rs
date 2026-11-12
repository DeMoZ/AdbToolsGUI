//! Поиск adb, Java и bundletool.
//!
//! Приложение, запущенное из Finder, получает короткий PATH без Homebrew, поэтому пути перечислены явно.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::adb::command;

pub const IS_MAC: bool = cfg!(target_os = "macos");
pub const IS_WIN: bool = cfg!(windows);

pub fn home_dir() -> PathBuf {
    env::var_os(if IS_WIN { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

/// Папка, рядом с которой ищем platform-tools и bundletool: папка с .app / .exe
/// (при `cargo run` — корень репозитория).
pub fn app_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    }
    let exe = env::current_exe().ok().and_then(|p| p.canonicalize().ok()).unwrap_or_default();
    // .../ADB Toolkit.app/Contents/MacOS/adb-toolkit -> папка, где лежит .app
    let bundle = exe.ancestors().nth(3);
    if IS_MAC
        && bundle.is_some_and(|b| b.extension().is_some_and(|e| e == "app"))
        && let Some(dir) = exe.ancestors().nth(4)
    {
        return dir.to_path_buf();
    }
    exe.parent().map(Path::to_path_buf).unwrap_or_default()
}

fn exe_name(name: &str) -> String {
    if IS_WIN { format!("{name}.exe") } else { name.to_string() }
}

pub fn which(name: &str) -> Option<PathBuf> {
    let exe = exe_name(name);
    env::split_paths(&env::var_os("PATH")?).map(|d| d.join(&exe)).find(|p| p.is_file())
}

pub fn adb_candidates() -> Vec<PathBuf> {
    let exe = exe_name("adb");
    let mut found = vec![app_dir().join("platform-tools").join(&exe)];
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(v) = env::var_os(var) {
            found.push(PathBuf::from(v).join("platform-tools").join(&exe));
        }
    }
    if IS_WIN {
        found.push(PathBuf::from(r"C:\platform-tools\adb.exe"));
        if let Some(v) = env::var_os("LOCALAPPDATA") {
            found.push(PathBuf::from(v).join(r"Android\Sdk\platform-tools").join(&exe));
        }
    } else {
        let home = home_dir();
        found.push("/opt/homebrew/bin/adb".into());
        found.push("/usr/local/bin/adb".into());
        found.push(home.join("Library/Android/sdk/platform-tools/adb"));
        found.push(home.join("Android/Sdk/platform-tools/adb"));
    }
    found
}

pub fn find_adb() -> Option<String> {
    let c = adb_candidates();
    if c[0].is_file() {
        return Some(c[0].to_string_lossy().into());
    }
    which("adb").or_else(|| c[1..].iter().find(|p| p.is_file()).cloned()).map(|p| p.to_string_lossy().into())
}

pub fn find_java() -> Option<String> {
    let exe = exe_name("java");
    if let Some(home) = env::var_os("JAVA_HOME") {
        let p = PathBuf::from(home).join("bin").join(&exe);
        if p.is_file() {
            return Some(p.to_string_lossy().into());
        }
    }
    if IS_MAC {
        // /usr/bin/java на Mac есть всегда, но без JDK это заглушка, которая только показывает окно.
        if let Ok(out) = command("/usr/libexec/java_home").stderr(Stdio::null()).output() {
            let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let p = PathBuf::from(&home).join("bin/java");
            if out.status.success() && !home.is_empty() && p.is_file() {
                return Some(p.to_string_lossy().into());
            }
        }
        return ["/opt/homebrew/opt/openjdk/bin/java", "/usr/local/opt/openjdk/bin/java"]
            .into_iter()
            .find(|p| Path::new(p).is_file())
            .map(String::from);
    }
    which("java").map(|p| p.to_string_lossy().into())
}

fn newest_jar(dir: &Path) -> Option<PathBuf> {
    let mut jars: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            name.starts_with("bundletool") && name.ends_with(".jar")
        })
        .collect();
    jars.sort();
    jars.pop()
}

pub fn find_bundletool() -> Option<String> {
    let base = app_dir();
    let dirs = [
        base.clone(),
        base.join("tools"),
        base.join("platform-tools"),
        // brew install bundletool
        PathBuf::from("/opt/homebrew/opt/bundletool/libexec"),
        PathBuf::from("/usr/local/opt/bundletool/libexec"),
    ];
    dirs.iter().find_map(|d| newest_jar(d)).map(|p| p.to_string_lossy().into())
}

/// Имя файла из пути Mac или Windows.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
