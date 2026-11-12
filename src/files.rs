//! Файлы на планшете: разбор `ls -la` (toybox) и пути вида `/sdcard/DCIM/`.

use std::sync::LazyLock;

use regex::Regex;

use crate::adb::{Adb, Res};

pub const START: &str = "/sdcard/";

static LS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([dlcbps-])[rwxsStT-]{9}\S*\s+\d+\s+\S+\s+\S+\s+(\d+)\s+(\d{4}-\d\d-\d\d \d\d:\d\d)\s(.+)$").unwrap()
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub date: String,
}

impl RemoteEntry {
    /// Папка или ссылка (в /sdcard ссылки почти всегда ведут на папки).
    pub fn is_folder(&self) -> bool {
        self.is_dir || self.is_link
    }
}

pub fn parse_ls(text: &str) -> Vec<RemoteEntry> {
    let mut entries: Vec<RemoteEntry> = text
        .lines()
        .filter_map(|line| {
            let m = LS_RE.captures(line)?;
            let kind = &m[1];
            let mut name = m[4].to_string();
            if kind == "l"
                && let Some((n, _)) = name.split_once(" -> ")
            {
                name = n.to_string();
            }
            if name == "." || name == ".." {
                return None;
            }
            Some(RemoteEntry {
                name,
                is_dir: kind == "d",
                is_link: kind == "l",
                size: m[2].parse().unwrap_or(0),
                date: m[3].to_string(),
            })
        })
        .collect();
    entries.sort_by_key(|e| (!e.is_folder(), e.name.to_lowercase()));
    entries
}

pub fn remote_join(folder: &str, name: &str) -> String {
    format!("{}/{}", folder.trim_end_matches('/'), name)
}

pub fn remote_parent(folder: &str) -> String {
    let folder = folder.trim_end_matches('/');
    match folder.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/"),
        None => "/".into(),
    }
}

/// Хлебные крошки: ("sdcard", "/sdcard/"), ("DCIM", "/sdcard/DCIM/").
pub fn breadcrumbs(folder: &str) -> Vec<(String, String)> {
    let mut path = String::from("/");
    let mut out = vec![("/".to_string(), "/".to_string())];
    for part in folder.split('/').filter(|p| !p.is_empty()) {
        path.push_str(part);
        path.push('/');
        out.push((part.to_string(), path.clone()));
    }
    out
}

pub fn human_size(size: u64) -> String {
    let mut value = size as f64;
    for unit in ["Б", "КБ", "МБ", "ГБ"] {
        if value < 1024.0 || unit == "ГБ" {
            return if unit == "Б" { format!("{value:.0} {unit}") } else { format!("{value:.1} {unit}") };
        }
        value /= 1024.0;
    }
    size.to_string()
}

pub fn quote(s: &str) -> String {
    shlex::try_quote(s).map(|q| q.into_owned()).unwrap_or_else(|_| s.to_string())
}

/// Читает папку. Путь со «/» в конце, чтобы ls зашёл внутрь ссылок вроде /sdcard.
/// Папка и её содержимое или (папка, текст ошибки).
pub type Listing = Result<(String, Vec<RemoteEntry>), (String, String)>;

pub fn list_folder(a: &Adb, folder: &str) -> Res<Listing> {
    let folder = format!("{}/", folder.trim_end_matches('/'));
    let r = a.adb_opts(&["shell", &format!("ls -la {}", quote(&folder))], true, true, 20.0)?;
    let entries = parse_ls(&r.out);
    if entries.is_empty() && (r.code != 0 || !r.err.is_empty()) {
        let msg = if !r.err.is_empty() {
            r.err
        } else if !r.out.is_empty() {
            r.out
        } else {
            "не удалось прочитать папку".into()
        };
        return Ok(Err((folder, msg.trim().to_string())));
    }
    Ok(Ok((folder, entries)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toybox_listing() {
        let text = "total 52\n\
            drwxrws---  2 u0_a123 media_rw 3452 2024-05-01 10:22 .\n\
            drwxrws---  9 root    media_rw 3452 2024-05-01 10:22 ..\n\
            drwxrws--x  5 root    media_rw 3452 2024-04-01 09:00 Android\n\
            -rw-rw----  1 u0_a123 media_rw 1048576 2024-05-02 11:30 My file.mp4\n\
            lrw-r--r--  1 root    root       21 2024-01-01 00:00 sdcard -> /storage/self/primary\n\
            ls: ./secret: Permission denied\n";
        let e = parse_ls(text);
        assert_eq!(e.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Android", "sdcard", "My file.mp4"]);
        assert!(e[0].is_dir);
        assert!(e[1].is_link);
        assert_eq!(e[2].size, 1048576);
    }

    #[test]
    fn paths() {
        assert_eq!(remote_join("/sdcard/", "DCIM"), "/sdcard/DCIM");
        assert_eq!(remote_parent("/sdcard/DCIM/"), "/sdcard/");
        assert_eq!(remote_parent("/sdcard/"), "/");
        assert_eq!(
            breadcrumbs("/sdcard/DCIM/"),
            vec![
                ("/".into(), "/".into()),
                ("sdcard".into(), "/sdcard/".into()),
                ("DCIM".into(), "/sdcard/DCIM/".into())
            ]
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 Б");
        assert_eq!(human_size(1048576), "1.0 МБ");
    }
}

/// Проверка на подключённом устройстве: `cargo test -- --ignored`.
/// Отправляет временный файл в /sdcard/Download, находит его через ls, скачивает обратно и удаляет.
#[cfg(test)]
mod device_tests {
    use super::*;
    use crate::adb::{Adb, Sink};

    #[test]
    #[ignore]
    fn push_list_pull() {
        let mut a = Adb::new(Sink::console());
        a.path = crate::tools::find_adb();
        let dev =
            a.devices().unwrap().into_iter().find(|d| d.state == "device").expect("нужно подключённое устройство");
        a.serial = Some(dev.serial);
        let dir = std::env::temp_dir().join(format!("adbtoolkit_dev_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = format!("adbtoolkit test {}.txt", std::process::id());
        let local = dir.join(&name);
        std::fs::write(&local, "проверка ADB Toolkit\n").unwrap();
        let remote = remote_join("/sdcard/Download/", &name);

        assert!(a.adb(&["push", &local.to_string_lossy(), "/sdcard/Download/"]).unwrap().ok());
        let (_, entries) = list_folder(&a, "/sdcard/Download").unwrap().unwrap();
        let e = entries.iter().find(|e| e.name == name).expect("файл виден в ls");
        assert_eq!(e.size, "проверка ADB Toolkit\n".len() as u64);

        let back = dir.join("back");
        std::fs::create_dir_all(&back).unwrap();
        assert!(a.adb(&["pull", &remote, &back.to_string_lossy()]).unwrap().ok());
        assert_eq!(std::fs::read_to_string(back.join(&name)).unwrap(), "проверка ADB Toolkit\n");

        assert!(a.adb(&["shell", "rm", &quote(&remote)]).unwrap().ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
