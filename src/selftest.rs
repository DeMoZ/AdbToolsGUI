//! Проверка без окна на реальном компьютере: какие программы найдены и как отвечают основные команды.

use crate::adb::{Adb, Sink};
use crate::commands::{self, Kind};
use crate::tools::{app_dir, find_adb, find_bundletool, find_java};

const SELFTEST: [&str; 8] = [
    "Авто-диагностика",
    "Текущий режим USB",
    "Проверить блокировку",
    "Сведения об устройстве",
    "USB-устройства",
    "Что видит Проводник",
    "USB-устройства Android",
    "Перезапустить ADB-сервер",
];

pub fn run() -> i32 {
    println!("app dir: {}", app_dir().display());
    println!("java: {:?}; bundletool: {:?}", find_java(), find_bundletool());
    let mut a = Adb::new(Sink::console());
    a.path = find_adb();
    println!("adb: {:?}", a.path);
    if a.path.is_none() {
        return 1;
    }
    let devices = a.devices().unwrap_or_default();
    for d in &devices {
        println!("  {:<24} {:<14} {}", d.serial, d.state, d.model);
    }
    if devices.is_empty() {
        println!("  (устройств нет: команды для устройства пропустятся)");
    }
    a.serial = devices.first().map(|d| d.serial.clone());
    for title in SELFTEST {
        let Some(c) = commands::by_title(title) else { continue };
        if !c.visible() {
            continue;
        }
        println!("=== {title} ===");
        if let Kind::Job(run) = c.kind {
            let _ = run(&mut a);
        }
    }
    0
}
