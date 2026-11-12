//! Список команд — источник правды для интерфейса (как массив `$script:Commands` в Windows-версии на PowerShell).
//!
//! `Kind::Job` выполняется в рабочем потоке, окно при этом не зависает. Остальные виды сначала собирают
//! данные в окне (выбор файлов, вопросы) и уже потом запускают работу в потоке.

use std::collections::HashSet;

use egui_phosphor::regular as icon;

use crate::adb::{Adb, Res, Step, Tag, mark_server_down, start_server};
use crate::devices::Device;
use crate::host::{self, Match};
use crate::tools::{IS_MAC, IS_WIN};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Diagnose,
    Connection,
    Usb,
    Screen,
    Info,
    Host,
    Files,
    Install,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::Diagnose,
        Group::Connection,
        Group::Usb,
        Group::Screen,
        Group::Info,
        Group::Host,
        Group::Files,
        Group::Install,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Group::Diagnose => "Диагностика",
            Group::Connection => "Подключение ADB",
            Group::Usb => "Режим USB",
            Group::Screen => "Экран и питание",
            Group::Info => "Об устройстве",
            Group::Host if IS_WIN => "Этот компьютер",
            Group::Host => "Этот Mac",
            Group::Files => "Файлы",
            Group::Install => "Установка приложений",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Group::Diagnose => "Проверка всей цепочки за один шаг: ADB, разрешение, экран, режим USB и сам компьютер.",
            Group::Connection => "Когда устройство не видно, висит в offline или не появляется окно разрешения.",
            Group::Usb => "Что планшет отдаёт по кабелю. Для файлов нужен MTP, даже если в шторке выбрано другое.",
            Group::Screen => "Пока экран заблокирован, Android не отдаёт файлы по USB.",
            Group::Info => "Модель, версия Android, батарея и память.",
            Group::Host if IS_WIN => "Что видят Windows и Проводник: MTP-устройства и их драйверы.",
            Group::Host => "Что Mac видит по USB. macOS сама не умеет MTP, поэтому файлы открываются через ADB.",
            Group::Files => "Папки планшета через ADB: просмотр, скачивание и отправка. Работает без MTP.",
            Group::Install => "APK, APK + OBB и AAB. Перетащите файлы в окно — утилита сама поймёт, что с ними делать.",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Group::Diagnose => icon::STETHOSCOPE,
            Group::Connection => icon::PLUGS_CONNECTED,
            Group::Usb => icon::USB,
            Group::Screen => icon::DEVICE_TABLET_SPEAKER,
            Group::Info => icon::INFO,
            Group::Host if IS_WIN => icon::WINDOWS_LOGO,
            Group::Host => icon::LAPTOP,
            Group::Files => icon::FOLDER_OPEN,
            Group::Install => icon::PACKAGE,
        }
    }
}

pub type Filters = &'static [(&'static str, &'static [&'static str])];
pub const APK: Filters = &[("APK", &["apk"])];
pub const APK_OBB: Filters = &[("APK и OBB", &["apk", "obb"])];
pub const AAB: Filters = &[("AAB", &["aab"])];
pub const ANY_PACKAGE: Filters = &[("APK, OBB, AAB", &["apk", "obb", "aab"])];

#[derive(Clone, Copy)]
pub enum Kind {
    Job(fn(&mut Adb) -> Res<()>),
    /// Выбрать файлы и поставить их как при перетаскивании.
    PickInstall(&'static str, Filters),
    PullByPath,
    PushToDownload,
    OpenMtpApp,
    OpenExplorer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    All,
    Mac,
    Windows,
}

#[derive(Clone, Copy)]
pub struct Command {
    pub group: Group,
    pub icon: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    pub cmd: &'static str,
    pub kind: Kind,
    pub confirm: Option<&'static str>,
    pub needs_device: bool,
    pub platform: Platform,
    /// Опасное действие: кнопка красная.
    pub danger: bool,
}

const fn cmd(
    group: Group,
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
    cmd: &'static str,
    kind: Kind,
) -> Command {
    Command {
        group,
        icon,
        title,
        desc,
        cmd,
        kind,
        confirm: None,
        needs_device: true,
        platform: Platform::All,
        danger: false,
    }
}

impl Command {
    const fn no_device(mut self) -> Self {
        self.needs_device = false;
        self
    }
    const fn only(mut self, p: Platform) -> Self {
        self.platform = p;
        self
    }
    const fn confirm(mut self, text: &'static str) -> Self {
        self.confirm = Some(text);
        self.danger = true;
        self
    }

    pub fn visible(&self) -> bool {
        match self.platform {
            Platform::All => true,
            Platform::Mac => IS_MAC,
            Platform::Windows => IS_WIN,
        }
    }
}

// ---------------------------------------------------------------- команды

pub fn auto_diagnose(a: &mut Adb) -> Res<()> {
    a.require(false)?;
    let devs = a.refresh_devices()?;
    let dev = devs.iter().find(|d| Some(&d.serial) == a.serial.as_ref()).cloned();
    match &dev {
        None => a.check(
            Step::Adb,
            Tag::Err,
            "[ADB] Устройств не видно. Проверьте кабель (нужен data-кабель) и порт; на планшете должна быть включена отладка по USB.",
        ),
        Some(d) if d.state == "unauthorized" => a.check(
            Step::Adb,
            Tag::Err,
            "[ADB] Планшет не дал разрешение. Разблокируйте экран и нажмите «Разрешить» в окне отладки по USB.",
        ),
        Some(d) if d.state != "device" => a.check(
            Step::Adb,
            Tag::Err,
            format!("[ADB] Состояние: {}. Попробуйте «Перезапустить ADB-сервер» и переподключить кабель.", d.state),
        ),
        Some(d) => {
            a.check(Step::Adb, Tag::Ok, format!("[ADB] {} отвечает.", d.title()));
            if a.shell("dumpsys window | grep -E 'isKeyguardShowing'")?.out.contains("isKeyguardShowing=true") {
                a.check(
                    Step::Screen,
                    Tag::Err,
                    "[Экран] Планшет заблокирован: пока он заблокирован, Android не отдаёт файлы. Разблокируйте его.",
                );
            } else {
                a.check(Step::Screen, Tag::Ok, "[Экран] Планшет разблокирован.");
            }
            let mut usb = a.adb_quiet(&["shell", "getprop", "sys.usb.config"])?;
            a.log(format!("[USB] Текущий режим: {}", usb.out), Tag::Text);
            if !usb.out.contains("mtp") {
                a.log("[USB] MTP не включён, включаю.", Tag::Warn);
                a.adb(&["shell", "svc", "usb", "setFunctions", "mtp"])?;
                a.sleep(4.0)?;
                usb = a.adb_quiet(&["shell", "getprop", "sys.usb.config"])?;
                let tag = if usb.out.contains("mtp") { Tag::Ok } else { Tag::Err };
                a.check(Step::Usb, tag, format!("[USB] Режим после переключения: {}", usb.out));
            } else {
                a.check(Step::Usb, Tag::Ok, format!("[USB] MTP включён ({}).", usb.out));
            }
            if d.is_wireless() {
                a.log(
                    "[USB] Устройство подключено по сети: режим USB влияет только на передачу файлов по кабелю.",
                    Tag::Dim,
                );
            }
        }
    }
    if IS_MAC {
        mac_usb_check(a, &devs, dev.as_ref())?;
    } else if IS_WIN {
        windows_check(a)?;
    }
    Ok(())
}

fn mac_usb_check(a: &Adb, devs: &[Device], dev: Option<&Device>) -> Res<()> {
    let Some(usb) = host::usb_devices(a)? else {
        a.check(Step::Host, Tag::Warn, "[Mac] Не удалось получить список USB-устройств (system_profiler).");
        return Ok(());
    };
    let serials: HashSet<String> = devs.iter().map(|d| d.serial.clone()).collect();
    let android: Vec<_> = usb.iter().filter_map(|u| host::android_match(u, &serials).map(|m| (u, m))).collect();
    if android.is_empty() {
        let wireless = dev.is_some_and(Device::is_wireless);
        a.check(
            Step::Host,
            if wireless { Tag::Warn } else { Tag::Err },
            if wireless {
                "[Mac] По USB планшета не видно: он подключён по сети. Для передачи файлов по кабелю подключите кабель с передачей данных."
            } else {
                "[Mac] Mac не видит планшет по USB. Нужен кабель с передачей данных (не только зарядка); попробуйте другой порт или переходник."
            },
        );
    }
    for (u, m) in &android {
        let sure = if *m == Match::Adb { "" } else { " (похоже на Android)" };
        let vendor = if u.vendor.is_empty() { &u.vendor_id } else { &u.vendor };
        let speed = if u.speed.is_empty() { "скорость неизвестна" } else { &u.speed };
        a.check(Step::Host, Tag::Ok, format!("[Mac] По USB видно: {} — {vendor}, {speed}{sure}", u.name));
    }
    if !android.is_empty() && dev.is_none() {
        a.log(
            "[Mac] Mac видит устройство, а ADB нет: включите на планшете «Отладку по USB» в параметрах разработчика и разрешите этот компьютер.",
            Tag::Warn,
        );
    }
    a.log(
        "[Mac] macOS сама не работает с MTP: файлы планшета открывайте в разделе «Файлы» (через ADB) или в OpenMTP.",
        Tag::Dim,
    );
    Ok(())
}

fn windows_check(a: &Adb) -> Res<()> {
    let w = host::windows_portable(a)?;
    if w.is_empty() {
        a.check(
            Step::Host,
            Tag::Err,
            "[Windows] MTP-устройств нет: Проводник планшет не видит. Включите «Передачу файлов» и разблокируйте экран.",
        );
    }
    for d in &w {
        let tag = if d.status == "OK" && !d.children.is_empty() { Tag::Ok } else { Tag::Warn };
        let inside = if d.children.is_empty() {
            "пусто (доступ не выдан)".to_string()
        } else {
            d.children.join(", ")
        };
        a.check(Step::Host, tag, format!("[Windows] {}: статус {}, в Проводнике: {inside}", d.name, d.status));
    }
    Ok(())
}

fn restart_server(a: &mut Adb) -> Res<()> {
    a.adb_opts(&["kill-server"], false, false, 30.0)?;
    mark_server_down();
    a.log("> adb start-server", Tag::Head);
    start_server(a.adb_path());
    a.log("ADB-сервер запущен.", Tag::Ok);
    Ok(())
}

fn device_info(a: &mut Adb) -> Res<()> {
    for prop in ["ro.product.manufacturer", "ro.product.model", "ro.build.version.release", "ro.serialno"] {
        let v = a.adb_quiet(&["shell", "getprop", prop])?.out;
        a.log(format!("{prop} = {v}"), Tag::Text);
    }
    let battery = a.shell("dumpsys battery | grep -E '^  (level|status|AC powered|USB powered):'")?.out;
    a.log(battery, Tag::Text);
    Ok(())
}

fn mac_usb_list(a: &mut Adb) -> Res<()> {
    let Some(usb) = host::usb_devices(a)? else {
        a.log("Не удалось получить список USB-устройств (system_profiler).", Tag::Err);
        return Ok(());
    };
    let serials: HashSet<String> =
        if a.path.is_some() { a.devices()?.into_iter().map(|d| d.serial).collect() } else { HashSet::new() };
    if usb.is_empty() {
        a.log("USB-устройств не найдено.", Tag::Warn);
    }
    for u in &usb {
        let m = host::android_match(u, &serials);
        let mark = match m {
            Some(Match::Adb) => "  ← устройство из adb",
            Some(Match::Likely) => "  ← похоже на Android",
            None => "",
        };
        let vendor = if u.vendor.is_empty() { "?" } else { &u.vendor };
        a.log(
            format!("{} — {vendor} [{}:{}] {}{mark}", u.name, u.vendor_id, u.product_id, u.speed),
            if m.is_some() { Tag::Ok } else { Tag::Dim },
        );
    }
    if !usb.is_empty() && !usb.iter().any(|u| host::android_match(u, &serials).is_some()) {
        a.log(
            "Среди них нет Android-устройства: проверьте кабель (нужна передача данных), порт и переходник.",
            Tag::Warn,
        );
    }
    Ok(())
}

fn win_explorer(a: &mut Adb) -> Res<()> {
    let w = host::windows_portable(a)?;
    if w.is_empty() {
        a.log("MTP-устройств в Windows не найдено.", Tag::Warn);
    }
    for d in &w {
        a.log(
            format!("{}: статус {}, проблема {}, в Проводнике: {}", d.name, d.status, d.problem, d.in_explorer),
            Tag::Text,
        );
        if d.children.is_empty() {
            a.log("  (внутри пусто)", Tag::Warn);
        }
        for c in &d.children {
            a.log(format!("  {c}"), Tag::Ok);
        }
    }
    Ok(())
}

fn win_usb(a: &mut Adb) -> Res<()> {
    let devs = host::windows_usb(a)?;
    if devs.is_empty() {
        a.log("Подходящих устройств нет.", Tag::Warn);
    }
    for d in &devs {
        a.log(
            format!("{:<8} {:<22} {} ({})", d.status, d.class, d.name, d.problem),
            if d.status == "OK" { Tag::Ok } else { Tag::Err },
        );
    }
    Ok(())
}

fn win_restart_mtp(a: &mut Adb) -> Res<()> {
    host::restart_mtp_driver(a)
}

fn usb_state(a: &mut Adb) -> Res<()> {
    a.adb(&["shell", "getprop", "sys.usb.config"])?;
    a.adb(&["shell", "getprop", "sys.usb.state"])?;
    Ok(())
}

fn usb_mode(a: &Adb, mode: &str, wait: f64) -> Res<()> {
    let mut args = vec!["shell", "svc", "usb", "setFunctions"];
    if !mode.is_empty() {
        args.push(mode);
    }
    a.adb(&args)?;
    if wait > 0.0 {
        a.sleep(wait)?;
        a.adb(&["shell", "getprop", "sys.usb.config"])?;
    }
    Ok(())
}

fn lock_state(a: &mut Adb) -> Res<()> {
    a.adb(&["shell", "dumpsys window | grep -E 'isKeyguardShowing|mDreamingLockscreen'"])?;
    a.adb(&["shell", "dumpsys power | grep mWakefulness="])?;
    Ok(())
}

fn ok(_: crate::adb::Outcome) {}

pub const COMMANDS: &[Command] = &[
    cmd(Group::Diagnose, icon::STETHOSCOPE, "Авто-диагностика",
        "Проверяет цепочку: ADB, разрешение на планшете, блокировку экрана, режим USB и что компьютер видит по USB. \
         Если режим не MTP, включает его.",
        "несколько проверок + svc usb setFunctions mtp",
        Kind::Job(auto_diagnose)).no_device(),

    cmd(Group::Connection, icon::LIST_BULLETS, "Список устройств",
        "Устройства и их статус: device — всё хорошо, unauthorized — нет разрешения на планшете, offline — потеряна связь.",
        "adb devices -l",
        Kind::Job(|a| a.adb_opts(&["devices", "-l"], false, false, 30.0).map(ok))).no_device(),
    cmd(Group::Connection, icon::ARROWS_CLOCKWISE, "Перезапустить ADB-сервер",
        "Лечит зависшее подключение и статус offline: останавливает и заново запускает фоновый сервис adb на компьютере.",
        "adb kill-server && adb start-server",
        Kind::Job(restart_server)).no_device(),
    cmd(Group::Connection, icon::PLUGS, "Переподключить устройство",
        "Заставляет планшет заново согласовать связь с ADB. Помогает, если статус offline или окно разрешения не появилось.",
        "adb -s <серийник> reconnect",
        Kind::Job(|a| a.adb(&["reconnect"]).map(ok))),
    cmd(Group::Connection, icon::HOURGLASS_MEDIUM, "Ждать устройство (30 с)",
        "Ждёт, пока планшет станет доступен. Удобно запустить и в это время нажать «Разрешить» на экране.",
        "adb -s <серийник> wait-for-device",
        Kind::Job(|a| a.adb_opts(&["wait-for-device"], true, false, 30.0).map(ok))),

    cmd(Group::Usb, icon::MAGNIFYING_GLASS, "Текущий режим USB",
        "Какие USB-функции включены. Если в списке есть mtp, передача файлов работает; sec_charging — только зарядка.",
        "adb shell getprop sys.usb.config",
        Kind::Job(usb_state)),
    cmd(Group::Usb, icon::FOLDERS, "Передача файлов (MTP)",
        "Включает режим передачи файлов, даже если выбор в шторке не сработал. Нужен для Проводника в Windows и OpenMTP на Mac.",
        "adb shell svc usb setFunctions mtp",
        Kind::Job(|a| usb_mode(a, "mtp", 3.0))),
    cmd(Group::Usb, icon::CAMERA, "Фото (PTP)",
        "Режим передачи изображений: планшет виден как камера (например, в «Захвате изображений» на Mac).",
        "adb shell svc usb setFunctions ptp",
        Kind::Job(|a| usb_mode(a, "ptp", 0.0))),
    cmd(Group::Usb, icon::GLOBE, "USB-модем (RNDIS)",
        "Раздаёт интернет планшета на компьютер по кабелю. Передача файлов при этом отключается. macOS RNDIS без сторонних драйверов не поддерживает.",
        "adb shell svc usb setFunctions rndis",
        Kind::Job(|a| usb_mode(a, "rndis", 0.0))),
    cmd(Group::Usb, icon::BATTERY_CHARGING, "Только зарядка",
        "Отключает передачу данных, оставляет зарядку.",
        "adb shell svc usb setFunctions",
        Kind::Job(|a| usb_mode(a, "", 0.0))),
    cmd(Group::Usb, icon::LOCK_KEY_OPEN, "MTP при разблокировке",
        "Просит Android включать передачу файлов каждый раз, когда вы разблокируете экран с подключённым кабелем.",
        "adb shell svc usb setScreenUnlockedFunctions mtp",
        Kind::Job(|a| a.adb(&["shell", "svc", "usb", "setScreenUnlockedFunctions", "mtp"]).map(ok))),

    cmd(Group::Screen, icon::SUN, "Разбудить экран",
        "Включает погасший экран. Блокировку это не снимает: PIN или рисунок всё равно вводятся на планшете.",
        "adb shell input keyevent KEYCODE_WAKEUP",
        Kind::Job(|a| a.adb(&["shell", "input", "keyevent", "KEYCODE_WAKEUP"]).map(ok))),
    cmd(Group::Screen, icon::LOCK, "Проверить блокировку",
        "Заблокирован ли планшет. При isKeyguardShowing=true файлы по USB недоступны.",
        "adb shell dumpsys window | grep isKeyguardShowing",
        Kind::Job(lock_state)),
    cmd(Group::Screen, icon::EYE, "Не гасить экран при USB",
        "Пока кабель подключён, экран не гаснет и не блокируется по таймеру. Удобно при долгом копировании.",
        "adb shell svc power stayon usb",
        Kind::Job(|a| a.adb(&["shell", "svc", "power", "stayon", "usb"]).map(ok))),
    cmd(Group::Screen, icon::MOON, "Вернуть автоотключение",
        "Отменяет предыдущую команду: экран снова гаснет по обычному таймеру.",
        "adb shell svc power stayon false",
        Kind::Job(|a| a.adb(&["shell", "svc", "power", "stayon", "false"]).map(ok))),
    cmd(Group::Screen, icon::POWER, "Перезагрузить планшет",
        "Обычная перезагрузка. Помогает, когда ничто другое не лечит USB.",
        "adb reboot",
        Kind::Job(|a| a.adb(&["reboot"]).map(ok))).confirm("Перезагрузить выбранное устройство?"),

    cmd(Group::Info, icon::IDENTIFICATION_CARD, "Сведения об устройстве",
        "Производитель, модель, версия Android, серийный номер, заряд батареи.",
        "adb shell getprop ro.product.model …; dumpsys battery",
        Kind::Job(device_info)),
    cmd(Group::Info, icon::HARD_DRIVES, "Свободное место",
        "Сколько занято и свободно во внутренней памяти.",
        "adb shell df -h /sdcard",
        Kind::Job(|a| a.adb(&["shell", "df", "-h", "/sdcard"]).map(ok))),
    cmd(Group::Info, icon::TREE_STRUCTURE, "Содержимое /sdcard",
        "Папки внутренней памяти напрямую через ADB. Если здесь файлы есть, а в Проводнике или MTP-программе пусто, проблема в MTP.",
        "adb shell ls -l /sdcard",
        Kind::Job(|a| a.adb(&["shell", "ls", "-l", "/sdcard"]).map(ok))),

    cmd(Group::Host, icon::USB, "USB-устройства",
        "Что Mac видит по USB. Если планшета нет и здесь, дело в кабеле, порте или переходнике, а не в ADB.",
        "system_profiler SPUSBHostDataType",
        Kind::Job(mac_usb_list)).no_device().only(Platform::Mac),
    cmd(Group::Host, icon::APP_WINDOW, "Открыть MTP-программу",
        "Открывает OpenMTP или Android File Transfer, если они установлены; иначе подсказывает, где взять.",
        "open -a OpenMTP",
        Kind::OpenMtpApp).no_device().only(Platform::Mac),
    cmd(Group::Host, icon::FOLDER_SIMPLE_USER, "Что видит Проводник",
        "MTP-устройства Windows, статус их драйвера и содержимое, которое видит Проводник. Пусто — доступ не выдан.",
        "Get-PnpDevice -Class WPD + Shell.Application",
        Kind::Job(win_explorer)).no_device().only(Platform::Windows),
    cmd(Group::Host, icon::USB, "USB-устройства Android",
        "Подключённые USB-устройства и их драйверы. Ищите строки с кодом проблемы, отличным от CM_PROB_NONE.",
        "Get-PnpDevice -PresentOnly -Class WPD, USB, AndroidUsbDeviceClass",
        Kind::Job(win_usb)).no_device().only(Platform::Windows),
    cmd(Group::Host, icon::ARROW_COUNTER_CLOCKWISE, "Перезапустить драйвер MTP",
        "Отключает и заново включает MTP-устройство в Windows. Потребуется подтвердить запрос администратора (UAC).",
        "pnputil /restart-device <InstanceId>   (от администратора)",
        Kind::Job(win_restart_mtp)).no_device().only(Platform::Windows)
        .confirm("Windows запросит права администратора. Продолжить?"),
    cmd(Group::Host, icon::DESKTOP, "Открыть «Этот компьютер»",
        "Открывает Проводник на списке дисков и устройств, где виден планшет.",
        "explorer shell:MyComputerFolder",
        Kind::OpenExplorer).no_device().only(Platform::Windows),

    cmd(Group::Files, icon::DOWNLOAD_SIMPLE, "Скачать по пути…",
        "Копирует файл или папку с планшета на компьютер по указанному пути.",
        "adb pull <путь на планшете> <папка на компьютере>",
        Kind::PullByPath),
    cmd(Group::Files, icon::UPLOAD_SIMPLE, "Отправить в Download…",
        "Копирует выбранные файлы с компьютера в папку Download на планшете.",
        "adb push <файл> /sdcard/Download/",
        Kind::PushToDownload),

    cmd(Group::Install, icon::ANDROID_LOGO, "Установить APK…",
        "Ставит выбранный APK. Если приложение уже есть, обновляет его с сохранением данных.",
        "adb install -r -t <файл.apk>",
        Kind::PickInstall("APK для установки", APK)),
    cmd(Group::Install, icon::STACK, "Установить APK + OBB…",
        "Выберите APK и файлы OBB (кэш игры): APK ставится, OBB копируются в /sdcard/Android/obb/<пакет>/. \
         Пакет берётся из имени main.<версия>.<пакет>.obb.",
        "adb install -r -t <apk>; adb push <obb> /sdcard/Android/obb/<пакет>/",
        Kind::PickInstall("APK и OBB", APK_OBB)),
    cmd(Group::Install, icon::CUBE, "Установить AAB…",
        "Собирает из AAB APK под этот планшет и ставит их через bundletool. Нужны Java и bundletool \
         (brew install bundletool или bundletool.jar рядом с утилитой).",
        "bundletool build-apks --connected-device; bundletool install-apks",
        Kind::PickInstall("AAB для установки", AAB)),
];

pub fn visible(group: Group) -> impl Iterator<Item = &'static Command> {
    COMMANDS.iter().filter(move |c| c.group == group && c.visible())
}

pub fn by_title(title: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.title == title)
}
