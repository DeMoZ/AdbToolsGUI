//! Запуск adb и других программ: таймаут, прерывание, журнал. Всё без окна.
//!
//! Методы `Adb` вызываются из рабочего потока. События (журнал, список устройств, шаги диагностики)
//! уходят через `Sink`: окно кладёт их в очередь, самотест печатает в консоль.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::devices::{Device, parse_devices};
use crate::files::Listing;

/// Вид строки журнала: окно раскрашивает их по теме.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Text,
    Head,
    Err,
    Warn,
    Ok,
    Dim,
}

/// Шаги авто-диагностики: окно показывает их чек-листом.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Adb,
    Screen,
    Usb,
    Host,
}

impl Step {
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug)]
pub enum Event {
    Log(String, Tag),
    /// Список устройств после команды (например, авто-диагностики).
    Devices(Vec<Device>),
    /// Результат фонового опроса `adb devices`; None — опрос пропущен или не удался.
    Polled(Option<Vec<Device>>),
    Check(Step, Tag, String),
    /// Содержимое папки планшета для окна файлов.
    Folder(Listing),
    Done,
    /// Свежий файл донатов; None — не загрузился.
    Donations(Option<crate::donations::Info>),
    UpdateChecked(Result<Option<crate::update::Release>, String>),
    /// Распакованная новая версия или ошибка загрузки.
    UpdateDownloaded(Result<std::path::PathBuf, String>),
}

#[derive(Clone)]
pub struct Sink(Arc<dyn Fn(Event) + Send + Sync>);

impl Sink {
    pub fn new(f: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Sink(Arc::new(f))
    }

    pub fn console() -> Self {
        Sink::new(|e| {
            if let Event::Log(text, _) = e {
                println!("{text}")
            }
        })
    }

    pub fn send(&self, e: Event) {
        (self.0)(e)
    }

    pub fn log(&self, text: impl Into<String>, tag: Tag) {
        self.send(Event::Log(text.into(), tag))
    }
}

/// Команду нельзя продолжать: прервал пользователь или причина уже в журнале.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    Cancelled,
    Abort,
}

pub type Res<T> = Result<T, Stop>;

#[derive(Debug, Default, Clone)]
pub struct Outcome {
    pub out: String,
    pub err: String,
    pub code: i32,
    pub timed_out: bool,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.code == 0 && !self.timed_out
    }
}

// ---------------------------------------------------------------- сервер adb

static SERVER_UP: AtomicBool = AtomicBool::new(false);
static SERVER_LOCK: Mutex<()> = Mutex::new(());

/// В Windows без этого флага у каждого вызова adb мелькает консольное окно.
pub fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// `adb start-server` оставляет демона, который наследует каналы вывода: если их читать, чтение не кончится.
/// Поэтому сервер запускается с выводом в null, а остальные команды adb идут уже к работающему серверу.
pub fn start_server(adb_path: &str) {
    let _guard = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut child) =
        command(adb_path).arg("start-server").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    SERVER_UP.store(true, Ordering::SeqCst);
}

pub fn mark_server_down() {
    SERVER_UP.store(false, Ordering::SeqCst);
}

pub fn server_up() -> bool {
    SERVER_UP.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------- Adb

#[derive(Clone)]
pub struct Adb {
    pub sink: Sink,
    pub path: Option<String>,
    pub serial: Option<String>,
    cancel: Arc<AtomicBool>,
}

fn reader(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim_end().to_string()
}

pub fn join_args<S: AsRef<str>>(args: &[S]) -> String {
    args.iter()
        .map(|a| shlex::try_quote(a.as_ref()).map(|q| q.into_owned()).unwrap_or_else(|_| a.as_ref().to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

impl Adb {
    pub fn new(sink: Sink) -> Self {
        Adb { sink, path: None, serial: None, cancel: Arc::new(AtomicBool::new(false)) }
    }

    pub fn log(&self, text: impl Into<String>, tag: Tag) {
        self.sink.log(text, tag)
    }

    pub fn check(&self, step: Step, tag: Tag, text: impl Into<String>) {
        let text = text.into();
        self.sink.send(Event::Check(step, tag, text.clone()));
        self.log(text, tag);
    }

    // --- прерывание

    pub fn reset_cancel(&self) {
        self.cancel.store(false, Ordering::SeqCst)
    }

    /// Вызывается из окна: текущий процесс будет убит при следующей проверке (раз в 25 мс).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst)
    }

    pub fn check_cancel(&self) -> Res<()> {
        if self.cancel.load(Ordering::SeqCst) { Err(Stop::Cancelled) } else { Ok(()) }
    }

    pub fn sleep(&self, seconds: f64) -> Res<()> {
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < end {
            self.check_cancel()?;
            thread::sleep(Duration::from_millis(50));
        }
        self.check_cancel()
    }

    // --- запуск

    /// Запускает программу и ждёт её не дольше `timeout` секунд. «Прервать» убивает процесс.
    pub fn run(&self, argv: &[String], timeout: f64) -> Res<Outcome> {
        self.check_cancel()?;
        let mut child = match command(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return Ok(Outcome { err: e.to_string(), code: -1, ..Default::default() }),
        };
        let out_rx = reader(child.stdout.take().expect("stdout"));
        let err_rx = reader(child.stderr.take().expect("stderr"));
        let start = Instant::now();
        let mut timed_out = false;
        let code = loop {
            if self.cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Stop::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status.code().unwrap_or(-1),
                Ok(None) => {}
                Err(_) => break -1,
            }
            if start.elapsed().as_secs_f64() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break -1;
            }
            thread::sleep(Duration::from_millis(25));
        };
        // Вывод может держать дочерний процесс (например, демон adb): долго его не ждём.
        let wait = Duration::from_millis(if timed_out { 1500 } else { 3000 });
        let out = out_rx.recv_timeout(wait).map(text).unwrap_or_default();
        let err = err_rx.recv_timeout(wait).map(text).unwrap_or_default();
        self.check_cancel()?;
        Ok(Outcome { out, err, code, timed_out })
    }

    pub fn report(&self, r: &Outcome, timeout: f64) {
        if !r.out.is_empty() {
            self.log(r.out.clone(), Tag::Text);
        }
        if !r.err.is_empty() {
            self.log(r.err.clone(), if r.code == 0 { Tag::Dim } else { Tag::Err });
        }
        if r.timed_out {
            self.log(format!("Команда не завершилась за {timeout} с и была прервана."), Tag::Warn);
        }
    }

    /// Прерывает команду, если нет adb или (при `device`) не выбрано устройство.
    pub fn require(&self, device: bool) -> Res<()> {
        if self.path.is_none() {
            self.log("adb не найден. Укажите путь к нему в настройках.", Tag::Err);
            return Err(Stop::Abort);
        }
        if device && self.serial.is_none() {
            self.log("Сначала выберите устройство.", Tag::Warn);
            return Err(Stop::Abort);
        }
        Ok(())
    }

    pub fn adb_path(&self) -> &str {
        self.path.as_deref().unwrap_or("adb")
    }

    pub fn adb_opts(&self, args: &[&str], device: bool, quiet: bool, timeout: f64) -> Res<Outcome> {
        self.require(device)?;
        let mut full: Vec<String> = Vec::new();
        if device {
            full.push("-s".into());
            full.push(self.serial.clone().unwrap_or_default());
        }
        full.extend(args.iter().map(|s| s.to_string()));
        if !server_up() {
            start_server(self.adb_path());
        }
        if !quiet {
            self.log(format!("> adb {}", join_args(&full)), Tag::Head);
        }
        let mut argv = vec![self.adb_path().to_string()];
        argv.extend(full);
        let r = self.run(&argv, timeout)?;
        if !quiet {
            self.report(&r, timeout);
            if r.err.contains("unauthorized") {
                self.log("Подсказка: разблокируйте планшет и разрешите отладку по USB в появившемся окне.", Tag::Warn);
            }
        }
        Ok(r)
    }

    /// Команда к выбранному устройству с выводом в журнал.
    pub fn adb(&self, args: &[&str]) -> Res<Outcome> {
        self.adb_opts(args, true, false, 30.0)
    }

    pub fn adb_quiet(&self, args: &[&str]) -> Res<Outcome> {
        self.adb_opts(args, true, true, 30.0)
    }

    /// Команда с `|` и grep выполняется на устройстве, только если передать её одной строкой.
    pub fn shell(&self, cmd: &str) -> Res<Outcome> {
        self.adb_opts(&["shell", cmd], true, true, 30.0)
    }

    /// Запускает внешнюю программу и пишет её вывод в журнал.
    pub fn logged(&self, argv: &[String], timeout: f64) -> Res<Outcome> {
        let name = std::path::Path::new(&argv[0])
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| argv[0].clone());
        self.log(format!("> {name} {}", join_args(&argv[1..])), Tag::Head);
        let r = self.run(argv, timeout)?;
        self.report(&r, timeout);
        Ok(r)
    }

    pub fn devices(&self) -> Res<Vec<Device>> {
        Ok(parse_devices(&self.adb_opts(&["devices", "-l"], false, true, 15.0)?.out))
    }

    /// Обновляет список устройств и, если выбранного больше нет, берёт первое.
    pub fn refresh_devices(&mut self) -> Res<Vec<Device>> {
        let devs = self.devices()?;
        if let Some(first) = devs.first()
            && !devs.iter().any(|d| Some(&d.serial) == self.serial.as_ref())
        {
            self.serial = Some(first.serial.clone());
        }
        self.sink.send(Event::Devices(devs.clone()));
        Ok(devs)
    }
}
