//! Окно ADB Toolkit на egui.
//!
//! Команды выполняются в рабочем потоке: журнал и список устройств приходят в окно через канал,
//! окно разбирает его в начале каждого кадра. Фоновый опрос `adb devices` раз в 3 с идёт отдельным
//! экземпляром `Adb` и пропускается, пока выполняется команда (`job_lock`).

mod console;
mod extras;
mod files_view;
mod pages;
mod theme;
mod widgets;

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, ThemePreference, vec2};
use egui_phosphor::regular as icon;

use crate::adb::{Adb, Event, Sink, Stop, Tag, mark_server_down};
use crate::commands::{self, Command, Group, Kind};
use crate::devices::{Device, state_text};
use crate::host;
use crate::install::{InstallJob, drop_plan, install_planned};
use crate::tools::{IS_MAC, IS_WIN, file_name, find_adb, find_bundletool, find_java};

use files_view::FilesState;
use theme::{palette, semibold};
use widgets::{nav_item, pill, section_label};

const MAX_LOG: usize = 5000;
const NO_DEVICE_COMMANDS: [&str; 9] =
    ["devices", "kill-server", "start-server", "version", "help", "connect", "disconnect", "pair", "mdns"];

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ADB Toolkit")
            .with_inner_size([1200.0, 820.0])
            .with_min_inner_size([920.0, 600.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png")).unwrap_or_default(),
            )
            .with_drag_and_drop(true)
            // на Mac боковая панель уходит под прозрачный заголовок окна, как в Finder и «Заметках»
            .with_fullsize_content_view(IS_MAC)
            .with_titlebar_shown(!IS_MAC)
            .with_title_shown(!IS_MAC),
        ..Default::default()
    };
    eframe::run_native("ADB Toolkit", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Group(Group),
    Settings,
    Donate,
}

/// Что сделать в окне после окончания работы.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum After {
    Nothing,
    /// Без пустой строки в журнале (служебные операции вроде чтения папки).
    Silent,
    ReloadFiles,
}

enum Prompt {
    Confirm(&'static Command),
    ObbPackage { job: InstallJob, value: String },
    PullPath { value: String },
}

struct Running {
    title: String,
    started: Instant,
    after: After,
}

pub struct App {
    ctx: egui::Context,
    settings: crate::settings::Settings,
    donate: extras::DonateState,
    updates: extras::UpdateState,
    sink: Sink,
    rx: mpsc::Receiver<Event>,
    adb: Adb,
    poller: Adb,
    job_lock: Arc<Mutex<()>>,

    adb_path: Option<String>,
    adb_edit: String,
    bundletool: Option<String>,
    /// Найденные Java и bundletool для раздела «Настройки».
    tools_cache: Option<(Option<String>, Option<String>)>,
    auto_refresh: bool,
    theme: ThemePreference,

    devices: Vec<Device>,
    dev_sig: Option<String>,
    serial: Option<String>,
    polling: bool,
    last_poll: Instant,

    running: Option<Running>,
    section: Section,
    log: Vec<(String, Tag)>,
    console_open: bool,
    custom: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    prompt: Option<Prompt>,
    checks: [Vec<(Tag, String)>; 4],
    diag_ran: bool,
    files: FilesState,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        theme::install(ctx);
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        let sink = Sink::new(move |e| {
            let _ = tx.send(e);
            repaint.request_repaint();
        });
        let settings = crate::settings::Settings::load();
        let theme = match settings.theme.as_str() {
            "light" => ThemePreference::Light,
            "dark" => ThemePreference::Dark,
            _ => ThemePreference::System,
        };
        let mut app = App {
            ctx: ctx.clone(),
            settings,
            donate: Default::default(),
            updates: Default::default(),
            adb: Adb::new(sink.clone()),
            poller: Adb::new(sink.clone()),
            sink,
            rx,
            job_lock: Arc::new(Mutex::new(())),
            adb_path: None,
            adb_edit: String::new(),
            bundletool: None,
            tools_cache: None,
            auto_refresh: true,
            theme,
            devices: vec![],
            dev_sig: None,
            serial: None,
            polling: false,
            last_poll: Instant::now(),
            running: None,
            section: Section::Group(Group::Diagnose),
            log: vec![],
            console_open: true,
            custom: String::new(),
            history: vec![],
            history_pos: None,
            prompt: None,
            checks: Default::default(),
            diag_ran: false,
            files: FilesState::default(),
        };
        match find_adb() {
            Some(path) => {
                app.adb_edit = path.clone();
                app.set_adb_path(path);
            }
            None => app.log(
                if IS_WIN {
                    "adb не найден. Положите папку platform-tools рядом с программой или укажите путь в настройках."
                } else {
                    "adb не найден. Установите его (brew install android-platform-tools) или укажите путь в настройках."
                },
                Tag::Err,
            ),
        }
        app.log("Выберите устройство и нужное действие. Начните с «Авто-диагностики».", Tag::Dim);
        if std::env::args().any(|a| a == "--update-failed") {
            app.log(
                "Обновление не установлено: не удалось заменить файлы программы. Проверьте обновления ещё раз.",
                Tag::Err,
            );
        }
        extras::check_on_start(&mut app);
        app
    }

    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    pub fn log(&mut self, text: impl Into<String>, tag: Tag) {
        let text = text.into();
        for line in text.split('\n') {
            self.log.push((line.trim_end_matches('\r').to_string(), tag));
        }
        if self.log.len() > MAX_LOG {
            let extra = self.log.len() - MAX_LOG;
            self.log.drain(..extra);
        }
    }

    fn blank(&mut self) {
        if self.log.last().is_some_and(|(t, _)| !t.is_empty()) {
            self.log.push((String::new(), Tag::Text));
        }
    }

    pub fn device(&self) -> Option<&Device> {
        self.devices.iter().find(|d| Some(&d.serial) == self.serial.as_ref())
    }

    // ---------------------------------------------------------------- события

    fn pump(&mut self) {
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::Log(text, tag) => self.log(text, tag),
                Event::Devices(devs) => self.show_devices(devs, true),
                Event::Polled(devs) => {
                    self.polling = false;
                    if let Some(devs) = devs {
                        self.show_devices(devs, false);
                    }
                }
                Event::Check(step, tag, text) => self.checks[step.index()].push((tag, text)),
                Event::Folder(result) => self.files.loaded(result, &mut self.log),
                Event::Done => self.job_done(),
                Event::Donations(info) => extras::donations_loaded(self, info),
                Event::UpdateChecked(result) => extras::checked(self, result),
                Event::UpdateDownloaded(result) => {
                    let ctx = self.ctx.clone();
                    extras::downloaded(self, &ctx, result)
                }
            }
        }
    }

    fn set_adb_path(&mut self, path: String) {
        let path = path.trim().to_string();
        if Some(&path) == self.adb_path.as_ref() {
            return;
        }
        if !path.is_empty() && !std::path::Path::new(&path).is_file() {
            self.log(format!("Файл не найден: {path}"), Tag::Err);
            return;
        }
        self.adb_path = if path.is_empty() { None } else { Some(path) };
        self.poller.path = self.adb_path.clone();
        mark_server_down();
        self.dev_sig = None;
        self.poll(true);
    }

    /// Опрос `adb devices` в фоне. Пока выполняется команда, опрос пропускается.
    fn poll(&mut self, force: bool) {
        self.last_poll = Instant::now();
        if self.adb_path.is_none() || self.polling || self.busy() {
            return;
        }
        self.polling = true;
        let poller = self.poller.clone();
        let lock = self.job_lock.clone();
        let sink = self.sink.clone();
        if force {
            self.dev_sig = None;
        }
        thread::spawn(move || {
            let devs = match lock.try_lock() {
                Ok(_guard) => poller.devices().ok(),
                Err(_) => None,
            };
            sink.send(Event::Polled(devs));
        });
    }

    fn show_devices(&mut self, devs: Vec<Device>, force: bool) {
        let sig: String = devs.iter().map(|d| format!("{}|{};", d.serial, d.state)).collect();
        if !force && Some(&sig) == self.dev_sig.as_ref() {
            return;
        }
        self.dev_sig = Some(sig);
        if !devs.iter().any(|d| Some(&d.serial) == self.serial.as_ref()) {
            self.serial = devs.first().map(|d| d.serial.clone());
        }
        self.devices = devs;
    }

    // ---------------------------------------------------------------- выполнение

    fn submit(
        &mut self,
        title: impl Into<String>,
        after: After,
        f: impl FnOnce(&mut Adb) -> Result<(), Stop> + Send + 'static,
    ) -> bool {
        if self.busy() {
            self.log("Дождитесь окончания текущей операции.", Tag::Warn);
            return false;
        }
        self.adb.path = self.adb_path.clone();
        self.adb.serial = self.serial.clone();
        self.adb.reset_cancel();
        self.running = Some(Running { title: title.into(), started: Instant::now(), after });
        let mut adb = self.adb.clone();
        let lock = self.job_lock.clone();
        thread::spawn(move || {
            {
                let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut adb)));
                match result {
                    Ok(Err(Stop::Cancelled)) => adb.log("Прервано.", Tag::Warn),
                    Ok(_) => {}
                    Err(_) => adb.log("Ошибка: команда завершилась аварийно.", Tag::Err),
                }
            }
            adb.sink.send(Event::Done);
        });
        true
    }

    fn job_done(&mut self) {
        let Some(r) = self.running.take() else { return };
        if r.after != After::Silent {
            self.blank();
        }
        if r.after == After::ReloadFiles {
            let folder = self.files.folder.clone();
            self.load_folder(folder);
        }
        self.poll(false);
    }

    fn cancel(&mut self) {
        if self.busy() {
            self.adb.cancel();
        }
    }

    pub fn require_device(&mut self) -> bool {
        if self.adb_path.is_none() {
            self.log("adb не найден. Укажите путь к нему в настройках.", Tag::Err);
            return false;
        }
        if self.serial.is_none() {
            self.log("Сначала подключите и выберите устройство.", Tag::Warn);
            return false;
        }
        true
    }

    fn on_command(&mut self, c: &'static Command, confirmed: bool) {
        if self.busy() {
            return;
        }
        if c.confirm.is_some() && !confirmed {
            self.prompt = Some(Prompt::Confirm(c));
            return;
        }
        self.log(format!("=== {} ===", c.title), Tag::Head);
        if c.needs_device && !self.require_device() {
            self.blank();
            return;
        }
        match c.kind {
            Kind::Job(f) => {
                if c.group == Group::Diagnose {
                    self.checks = Default::default();
                    self.diag_ran = true;
                }
                self.submit(c.title, After::Nothing, f);
            }
            Kind::PickInstall(title, filters) => {
                let files = self.pick_files(title, filters, true);
                if files.is_empty() {
                    self.blank();
                } else {
                    self.install_files(files, false);
                }
            }
            Kind::PullByPath => self.prompt = Some(Prompt::PullPath { value: "/sdcard/DCIM".into() }),
            Kind::PushToDownload => {
                let files = self.pick_files("Файлы для отправки на планшет", &[], true);
                if files.is_empty() {
                    self.blank();
                } else {
                    self.push_files(files, "/sdcard/Download/".into(), After::Nothing);
                }
            }
            Kind::OpenMtpApp => {
                self.open_mtp_app();
                self.blank();
            }
            Kind::OpenExplorer => {
                self.log("> explorer shell:MyComputerFolder", Tag::Head);
                if let Err(e) = host::open_explorer() {
                    self.log(format!("Не удалось открыть Проводник: {e}"), Tag::Err);
                }
                self.blank();
            }
        }
    }

    fn open_mtp_app(&mut self) {
        let apps = host::mtp_apps();
        let Some(app) = apps.first() else {
            self.log(
                "MTP-программ не найдено. Бесплатная: OpenMTP (brew install --cask openmtp или openmtp.ganeshrvel.com). \
                 Пока она открыта, переключите планшет в «Передачу файлов (MTP)».",
                Tag::Warn,
            );
            return;
        };
        self.log(format!("> open -a '{}'", app.display()), Tag::Head);
        if let Err(e) = crate::adb::command("open").arg("-a").arg(app).spawn() {
            self.log(format!("Не удалось открыть: {e}"), Tag::Err);
        }
        self.log("Если программа не видит планшет, включите «Передача файлов (MTP)» и разблокируйте экран.", Tag::Dim);
    }

    fn run_custom(&mut self) {
        let line = self.custom.trim().to_string();
        if line.is_empty() || self.busy() {
            return;
        }
        let Some(mut tokens) = shlex::split(&line) else {
            self.log("Не разобрать команду: проверьте кавычки.", Tag::Err);
            return;
        };
        if tokens.first().is_some_and(|t| t == "adb") {
            tokens.remove(0);
        }
        if tokens.is_empty() {
            return;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line);
        }
        self.history_pos = None;
        self.custom.clear();
        let device = !NO_DEVICE_COMMANDS.contains(&tokens[0].as_str());
        self.submit(format!("adb {}", tokens.join(" ")), After::Nothing, move |a| {
            let args: Vec<&str> = tokens.iter().map(String::as_str).collect();
            a.adb_opts(&args, device, false, 120.0).map(|_| ())
        });
    }

    // ---------------------------------------------------------------- файлы и установка

    pub fn pick_files(&mut self, title: &str, filters: commands::Filters, multiple: bool) -> Vec<String> {
        let mut dlg = rfd::FileDialog::new().set_title(title);
        for (name, exts) in filters {
            dlg = dlg.add_filter(*name, exts);
        }
        let files: Vec<String> = if multiple {
            dlg.pick_files().unwrap_or_default().into_iter().map(|p| p.to_string_lossy().into()).collect()
        } else {
            dlg.pick_file().map(|p| vec![p.to_string_lossy().into()]).unwrap_or_default()
        };
        if files.is_empty() {
            self.log("Отменено.", Tag::Dim);
        }
        files
    }

    pub fn pick_folder(&mut self, title: &str) -> Option<String> {
        let dir = rfd::FileDialog::new().set_title(title).pick_folder().map(|p| p.to_string_lossy().to_string());
        if dir.is_none() {
            self.log("Отменено.", Tag::Dim);
        }
        dir
    }

    /// Установка выбранных или перетащенных файлов: утилита сама решает, что делать с APK / OBB / AAB.
    pub fn install_files(&mut self, paths: Vec<String>, header: bool) {
        if self.busy() {
            self.log("Дождитесь окончания текущей операции.", Tag::Warn);
            return;
        }
        if header {
            self.log("=== Установка файлов ===", Tag::Head);
        }
        if !self.require_device() {
            self.blank();
            return;
        }
        let plan = drop_plan(&paths);
        for p in &plan.ignored {
            self.log(format!("Пропущено (не APK, OBB и не AAB): {p}"), Tag::Warn);
        }
        if let Some(err) = plan.error {
            self.log(err, Tag::Err);
            self.blank();
            return;
        }
        let job = InstallJob { plan, ..Default::default() };
        if job.plan.needs_obb_package() {
            self.prompt = Some(Prompt::ObbPackage { job, value: String::new() });
            return;
        }
        self.continue_install(job);
    }

    fn continue_install(&mut self, mut job: InstallJob) {
        if !job.plan.aabs.is_empty() {
            let Some(java) = find_java() else {
                self.log(
                    "Java не найдена: bundletool работает на Java. Установите JDK 17+ (на Mac: brew install --cask temurin) и перезапустите утилиту.",
                    Tag::Err,
                );
                self.blank();
                return;
            };
            let mut bt = self.bundletool.clone().filter(|b| std::path::Path::new(b).is_file()).or_else(find_bundletool);
            if bt.is_none() {
                self.log(
                    "bundletool.jar не найден рядом с утилитой (на Mac: brew install bundletool, или скачайте с \
                     github.com/google/bundletool/releases). Укажите файл.",
                    Tag::Warn,
                );
                bt = self.pick_files("Укажите bundletool.jar", &[("bundletool", &["jar"])], false).into_iter().next();
            }
            let Some(bt) = bt else {
                self.blank();
                return;
            };
            self.bundletool = Some(bt.clone());
            job.java = Some(java);
            job.bundletool = Some(bt);
        }
        let names: Vec<&str> =
            job.plan.apks.iter().chain(&job.plan.aabs).chain(&job.plan.obbs).map(|f| file_name(f)).collect();
        let title = format!("Установка: {}", names.join(", "));
        self.submit(title, After::Nothing, move |a| install_planned(a, &job));
    }

    pub fn push_files(&mut self, files: Vec<String>, folder: String, after: After) {
        if !self.require_device() {
            return;
        }
        self.log("=== Отправить на планшет ===", Tag::Head);
        let title = format!("Отправка в {folder}");
        self.submit(title, after, move |a| {
            for f in &files {
                if a.adb_opts(&["push", f, &folder], true, false, 1800.0)?.ok() {
                    a.log(format!("Отправлено: {}", file_name(f)), Tag::Ok);
                }
            }
            Ok(())
        });
    }

    pub fn pull_paths(&mut self, remotes: Vec<String>, local: String) {
        self.log("=== Скачать с планшета ===", Tag::Head);
        let title = format!("Скачивание в {}", file_name(&local));
        self.submit(title, After::Nothing, move |a| {
            for r in &remotes {
                if a.adb_opts(&["pull", r, &local], true, false, 1800.0)?.ok() {
                    a.log(format!("Скачано: {}", file_name(r)), Tag::Ok);
                }
            }
            Ok(())
        });
    }

    pub fn load_folder(&mut self, folder: String) {
        if self.busy() || self.adb_path.is_none() || self.serial.is_none() {
            return;
        }
        self.files.loading = true;
        self.files.loaded_serial = self.serial.clone();
        self.submit("Чтение папки", After::Silent, move |a| {
            let r = crate::files::list_folder(a, &folder)?;
            a.sink.send(Event::Folder(r));
            Ok(())
        });
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<String> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_string_lossy().to_string()).collect());
        if dropped.is_empty() {
            return;
        }
        if self.busy() {
            self.log("Дождитесь окончания текущей операции.", Tag::Warn);
            return;
        }
        if self.section == Section::Group(Group::Files) {
            let folder = self.files.folder.clone();
            self.push_files(dropped, folder, After::ReloadFiles);
        } else {
            self.section = Section::Group(Group::Install);
            self.install_files(dropped, true);
        }
    }

    // ---------------------------------------------------------------- окно

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        ui.add_space(if IS_MAC { 26.0 } else { 14.0 });
        ui.horizontal(|ui| {
            widgets::icon_tile(ui, icon::ANDROID_LOGO, p.accent_text, p.accent, 32.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("ADB Toolkit").font(semibold(16.0)).color(p.text));
                ui.label(RichText::new("USB-помощник для Android").size(11.5).color(p.faint));
            });
        });
        ui.add_space(14.0);
        self.device_card(ui);
        ui.add_space(14.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            section_label(ui, "Разделы");
            ui.add_space(4.0);
            for g in Group::ALL {
                let selected = self.section == Section::Group(g);
                let badge = if g == Group::Diagnose { self.diag_badge(p) } else { None };
                if nav_item(ui, g.icon(), g.title(), selected, badge).clicked() {
                    self.open_section(Section::Group(g));
                }
            }
            ui.add_space(12.0);
            section_label(ui, "Приложение");
            ui.add_space(4.0);
            if nav_item(ui, icon::GEAR_SIX, "Настройки", self.section == Section::Settings, None).clicked() {
                self.section = Section::Settings;
            }
            if widgets::nav_item_tinted(
                ui,
                icon::HEART,
                "Поддержать",
                self.section == Section::Donate,
                p.pink,
                p.pink_soft,
            )
            .clicked()
            {
                self.open_section(Section::Donate);
            }
            ui.add_space(12.0);
            extras::sidebar_banner(self, ui);
        });
    }

    fn open_section(&mut self, s: Section) {
        self.section = s;
        if s == Section::Donate {
            extras::refresh_donations(self);
        }
        if s == Section::Group(Group::Files) && self.files.loaded_serial != self.serial {
            let folder = self.files.folder.clone();
            self.load_folder(folder);
        }
    }

    fn diag_badge(&self, p: &theme::Palette) -> Option<Color32> {
        if !self.diag_ran || self.busy() {
            return None;
        }
        let worst = self.checks.iter().flatten().map(|(t, _)| *t).max_by_key(|t| match t {
            Tag::Err => 3,
            Tag::Warn => 2,
            Tag::Ok => 1,
            _ => 0,
        });
        worst.map(|t| p.tag(t))
    }

    fn device_card(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        widgets::card(ui).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let busy = self.busy();
            let (title, sub, state_tag, state_label, hint) = match (&self.adb_path, self.device()) {
                (None, _) => {
                    ("adb не найден".to_string(), "Укажите путь в настройках".to_string(), Tag::Err, "Нет adb", None)
                }
                (Some(_), None) => (
                    "Нет устройств".to_string(),
                    "Подключите кабель и включите отладку по USB".to_string(),
                    Tag::Dim,
                    "Не подключено",
                    None,
                ),
                (Some(_), Some(d)) => {
                    let (label, hint, tag) = state_text(&d.state);
                    (d.title(), d.serial.clone(), tag, label, if tag == Tag::Ok { None } else { Some(hint) })
                }
            };
            let wireless = self.device().is_some_and(Device::is_wireless);
            ui.horizontal(|ui| {
                let dev_icon = if wireless { icon::WIFI_HIGH } else { icon::DEVICE_TABLET };
                let fg = if state_tag == Tag::Dim { p.faint } else { p.tag(state_tag) };
                let bg = if state_tag == Tag::Dim { p.code } else { p.tag_soft(state_tag) };
                widgets::icon_tile(ui, dev_icon, fg, bg, 36.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(egui::Label::new(RichText::new(title).font(semibold(14.0)).color(p.text)).truncate());
                    ui.add(egui::Label::new(RichText::new(sub).size(11.5).color(p.faint)).truncate());
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let color = if state_tag == Tag::Dim { p.faint } else { p.tag(state_tag) };
                let soft = if state_tag == Tag::Dim { p.code } else { p.tag_soft(state_tag) };
                pill(ui, state_label, color, soft);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.polling {
                        ui.add(egui::Spinner::new().size(14.0));
                    } else if widgets::icon_button(
                        ui,
                        !busy && self.adb_path.is_some(),
                        icon::ARROWS_CLOCKWISE,
                        "Обновить список устройств",
                    )
                    .clicked()
                    {
                        self.poll(true);
                    }
                });
            });
            if let Some(h) = hint {
                ui.add_space(2.0);
                ui.label(RichText::new(h).size(12.0).color(p.muted));
            }
            if self.devices.len() > 1 {
                ui.add_space(4.0);
                let current = self.device().map(|d| format!("{} · {}", d.title(), d.serial)).unwrap_or_default();
                egui::ComboBox::from_id_salt("device").width(ui.available_width()).selected_text(current).show_ui(
                    ui,
                    |ui| {
                        for d in self.devices.clone() {
                            let label = format!("{} · {} [{}]", d.title(), d.serial, d.state);
                            if ui.selectable_label(self.serial.as_ref() == Some(&d.serial), label).clicked() {
                                self.serial = Some(d.serial.clone());
                            }
                        }
                    },
                );
            }
        });
    }

    fn header(&self, ui: &mut egui::Ui, icon: &str, title: &str, subtitle: &str) {
        let p = palette(ui);
        ui.horizontal(|ui| {
            let color = if self.section == Section::Donate { p.pink } else { p.accent };
            ui.label(RichText::new(icon).size(26.0).color(color));
            ui.add_space(2.0);
            ui.label(RichText::new(title).font(semibold(24.0)).color(p.text));
        });
        ui.label(RichText::new(subtitle).size(14.0).color(p.muted));
        ui.add_space(14.0);
    }

    fn drop_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let p = palette(ui);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
        let rect = rect.shrink(12.0);
        painter.rect(
            rect,
            CornerRadius::same(16),
            p.bg.gamma_multiply(0.92),
            Stroke::new(2.0, p.accent),
            egui::StrokeKind::Inside,
        );
        let (icon, text, sub) = if self.section == Section::Group(Group::Files) {
            (
                icon::UPLOAD_SIMPLE,
                "Отпустите, чтобы отправить на планшет".to_string(),
                format!("в папку {}", self.files.folder),
            )
        } else {
            (
                icon::PACKAGE,
                "Отпустите, чтобы установить".to_string(),
                "APK, OBB или AAB — утилита сама поймёт, что делать".to_string(),
            )
        };
        let c = rect.center();
        painter.text(
            c - vec2(0.0, 40.0),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(48.0),
            p.accent,
        );
        painter.text(c + vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, text, semibold(20.0), p.text);
        painter.text(c + vec2(0.0, 42.0), egui::Align2::CENTER_CENTER, sub, egui::FontId::proportional(14.0), p.muted);
    }

    fn prompts(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.prompt.as_mut() else { return };
        let mut close = false;
        let mut accept = false;
        let modal = egui::Modal::new(egui::Id::new("prompt")).frame(
            Frame::popup(&ctx.global_style()).inner_margin(Margin::same(22)).corner_radius(CornerRadius::same(14)),
        );
        let resp = modal.show(ctx, |ui| {
            let p = palette(ui);
            ui.set_width(420.0);
            let (icon_s, title, text, danger) = match prompt {
                Prompt::Confirm(c) => (c.icon, c.title, c.confirm.unwrap_or(""), c.danger),
                Prompt::ObbPackage { .. } => (
                    icon::STACK,
                    "Пакет для OBB",
                    "Имя OBB не похоже на main.<версия>.<пакет>.obb. Введите имя пакета приложения:",
                    false,
                ),
                Prompt::PullPath { .. } => {
                    (icon::DOWNLOAD_SIMPLE, "Скачать с планшета", "Путь на планшете (файл или папка):", false)
                }
            };
            ui.horizontal(|ui| {
                let (fg, bg) = if danger { (p.err, p.err_soft) } else { (p.accent, p.accent_soft) };
                widgets::icon_tile(ui, icon_s, fg, bg, 36.0);
                ui.label(RichText::new(title).font(semibold(17.0)).color(p.text));
            });
            ui.add_space(6.0);
            ui.label(RichText::new(text).color(p.muted));
            let hint = if matches!(prompt, Prompt::PullPath { .. }) { "/sdcard/DCIM" } else { "com.example.game" };
            let value = match prompt {
                Prompt::ObbPackage { value, .. } | Prompt::PullPath { value } => Some(value),
                Prompt::Confirm(_) => None,
            };
            if let Some(value) = value {
                ui.add_space(4.0);
                let r = ui.add(
                    egui::TextEdit::singleline(value)
                        .font(theme::mono(13.0))
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(10, 7))
                        .hint_text(hint),
                );
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    accept = true;
                }
            }
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok_text = match prompt {
                    Prompt::Confirm(_) => "Продолжить",
                    Prompt::ObbPackage { .. } => "Установить",
                    Prompt::PullPath { .. } => "Выбрать папку…",
                };
                let ok = if danger {
                    let t = RichText::new(ok_text).font(semibold(14.0)).color(Color32::WHITE);
                    ui.add(egui::Button::new(t).fill(p.err).stroke(Stroke::NONE).min_size(vec2(0.0, 34.0)))
                } else {
                    widgets::primary(ui, true, ok_text)
                };
                if ok.clicked() {
                    accept = true;
                }
                if widgets::secondary(ui, true, "Отмена", false).clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        if accept {
            let prompt = self.prompt.take().unwrap();
            match prompt {
                Prompt::Confirm(c) => self.on_command(c, true),
                Prompt::ObbPackage { mut job, value } => {
                    if value.trim().is_empty() {
                        self.log("Отменено.", Tag::Dim);
                        self.blank();
                    } else {
                        job.obb_fallback = value.trim().to_string();
                        self.continue_install(job);
                    }
                }
                Prompt::PullPath { value } => {
                    let remote = value.trim().to_string();
                    if remote.is_empty() {
                        self.log("Отменено.", Tag::Dim);
                    } else if let Some(local) = self.pick_folder("Куда сохранить") {
                        self.pull_paths(vec![remote], local);
                        return;
                    }
                    self.blank();
                }
            }
        } else if close {
            if !matches!(self.prompt, Some(Prompt::Confirm(_))) {
                self.log("Отменено.", Tag::Dim);
                self.blank();
            }
            self.prompt = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump();
        if self.auto_refresh && self.last_poll.elapsed() >= Duration::from_secs(3) {
            self.poll(false);
        }
        ctx.request_repaint_after(Duration::from_millis(if self.busy() { 200 } else { 1000 }));
        self.handle_drops(&ctx);

        let p = palette(ui);
        if IS_MAC {
            title_drag_area(ui);
        }
        egui::Panel::left("nav")
            .exact_size(256.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.sidebar).inner_margin(Margin { left: 14, right: 14, top: 8, bottom: 14 }))
            .show(ui, |ui| self.sidebar(ui));

        console::panel(self, ui);

        let central = egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin {
                left: 32,
                right: 32,
                top: if IS_MAC { 30 } else { 24 },
                bottom: 8,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(980.0_f32.min(ui.available_width()));
                    match self.section {
                        Section::Group(g) => {
                            self.header(ui, g.icon(), g.title(), g.subtitle());
                            pages::group(self, ui, g);
                        }
                        Section::Settings => {
                            self.header(
                                ui,
                                icon::GEAR_SIX,
                                "Настройки",
                                "Пути к программам, обновление списка и тема оформления.",
                            );
                            pages::settings(self, ui);
                        }
                        Section::Donate => {
                            self.header(
                                ui,
                                icon::HEART,
                                "Поддержать ADB Toolkit",
                                "Программа бесплатная. Нажмите на способ — откроется страница оплаты в браузере.",
                            );
                            extras::donate_page(self, ui);
                        }
                    }
                    ui.add_space(16.0);
                });
            });
        self.drop_overlay(ui, central.response.rect);
        self.prompts(&ctx);
        if self.prompt.is_none() {
            extras::offer_modal(self, &ctx);
        }
        ctx.set_theme(self.theme);
    }
}

/// Заголовок окна на Mac прозрачный: перетаскивание окна за верхнюю полосу и двойной щелчок — «увеличить».
/// Область создаётся до панелей, поэтому кнопки поверх неё работают как обычно.
fn title_drag_area(ui: &mut egui::Ui) {
    let full = ui.max_rect();
    let rect = egui::Rect::from_min_size(full.min, vec2(full.width(), 28.0));
    let r = ui.interact(rect, egui::Id::new("title-drag"), egui::Sense::click_and_drag());
    if r.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if r.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
}
