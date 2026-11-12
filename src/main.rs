//! ADB Toolkit: диагностика и «лечение» USB-подключения Android-устройств через ADB.
//! Запуск без окна: `adb-toolkit --selftest`.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod adb;
mod commands;
mod devices;
mod donations;
mod files;
mod host;
mod install;
mod net;
mod selftest;
mod settings;
mod tools;
mod ui;
mod update;

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--selftest") {
        attach_console();
        std::process::exit(selftest::run());
    }
    ui::run()
}

/// Сборка для Windows без консоли: самотест печатает в консоль, из которой его запустили.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
