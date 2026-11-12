# AdbToolsGUI

Графическая утилита для диагностики и «лечения» USB-подключения Android-устройств через ADB.
Отвечай пользователю по-русски.

## Откуда проект

Всё началось с реальной проблемы: Samsung Galaxy Tab A9 (SM-X115, Android 16) был подключён к Windows кабелем,
в Проводнике виден, но со статусом «Подключение» и пустой. Разобранная причина и решение лежат в основе утилиты:

1. `adb devices` показывал `unauthorized`: планшет не дал компьютеру разрешение. Лечится разблокировкой экрана и
   кнопкой «Разрешить» в окне отладки по USB.
2. Даже после выбора «Передача файлов» в шторке `sys.usb.config` оставался `sec_charging,adb`, то есть MTP не был
   включён. Помогло `adb shell svc usb setFunctions mtp`, режим стал `mtp,adb`, и файлы появились.
3. Пока планшет заблокирован, Android не отдаёт файлы по MTP.

## Что уже есть

Одна программа на **Rust + egui (eframe 0.36)** для macOS и Windows; описание для пользователя в [README.md](README.md).
Прежние версии: Python + Tkinter удалена, Windows-версия на PowerShell лежит в
[legacy/windows-powershell/](legacy/windows-powershell) и пока ещё выкладывается в релиз запасным вариантом.

Код в `src/`:
- `adb.rs` — запуск программ (`Adb::run`: таймаут, «Прервать» убивает процесс, опрос раз в 25 мс), `start_server`,
  события для окна (`Event`, `Sink`): журнал, список устройств, шаги диагностики (`Step`), содержимое папки.
- `commands.rs` — список `COMMANDS`, источник правды (группы `Group`, у команды icon / title / desc / cmd / kind,
  `confirm`, `needs_device`, `platform`). `Kind::Job(fn(&mut Adb))` выполняется в рабочем потоке; остальные виды
  (`PickInstall`, `PullByPath`, `PushToDownload`, `OpenMtpApp`, `OpenExplorer`) сначала собирают данные в окне.
- `devices.rs` (`adb devices -l`), `files.rs` (`ls -la`, пути, `list_folder`), `install.rs` (`drop_plan`,
  `install_planned`: AAB через bundletool, APK через `adb install -r -t`, OBB в `/sdcard/Android/obb/<пакет>/`),
  `host.rs` (`system_profiler` на Mac; `Get-PnpDevice` / Проводник / `pnputil` через `powershell.exe` в Windows),
  `tools.rs` (поиск adb / Java / bundletool, `app_dir`), `selftest.rs`.
- `ui/` — окно: `mod.rs` (состояние, очередь событий, запуск работ, диалоги), `pages.rs` (разделы, диагностика с
  чек-листом, зона перетаскивания, настройки), `files_view.rs` (файлы планшета), `console.rs` (журнал и своя
  команда), `theme.rs` (палитры, шрифты), `widgets.rs`.

Интерфейс: слева меню разделов и карточка устройства (статус, выбор, если устройств несколько); в центре раздел;
снизу журнал с «Прервать» и полем `adb …`. Файлы, брошенные в окно, ставятся (или отправляются в открытую папку
в разделе «Файлы»). На Mac заголовок окна прозрачный, окно перетаскивается за верхнюю полосу (`title_drag_area`).

- Работы идут в отдельном потоке по одной (`App::submit`); фоновый опрос `adb devices` раз в 3 с — отдельный
  экземпляр `Adb`, пропускается, пока занят `job_lock`.
- Диалоги выбора файлов — `rfd` (системные), вызываются только в потоке окна; вопросы с вводом — модальные окна egui
  (`Prompt`).
- Шрифт — Inter без символов Private Use Area (`assets/fonts/README.md`): в этой области значки Phosphor
  (`egui-phosphor`), и символы Inter рисовались бы вместо них. Phosphor нельзя ставить перед Inter: он гасит строчную
  латиницу. Моноширинный шрифт берётся из системы.
- `egui-phosphor 0.14` требует egui 0.36, поэтому eframe не обновлён до 0.37.
- Проверки: `cargo test`, `cargo run -- --selftest`, на устройстве `cargo test -- --ignored`;
  `cargo clippy --all-targets -- -D warnings` и `cargo fmt --check` (CI их требует). Проверка, что код собирается под
  Windows, с Mac: `cargo check --target x86_64-pc-windows-msvc`.
- Сборка Mac: `scripts/build-macos.sh` (arm64, `.app` с подписью ad hoc, `.dmg` и `…-macos-arm64.zip` для обновления).
  CI (`.github/workflows/build.yml`): macOS и Windows, на тег `v*` — релиз с `.dmg`, архивами для обновления и `.zip`
  с PowerShell-версией. Значок: `scripts/make-icon.sh` → `assets/`.
- Приложение из Finder получает короткий `PATH` без Homebrew, поэтому пути к adb / Java / bundletool перечислены явно.
  `/usr/bin/java` на Mac — заглушка без JDK, Java ищется через `/usr/libexec/java_home`.
- Windows-часть (PnP, Проводник, `CREATE_NO_WINDOW`, значок через `build.rs` + `winresource`) только собирается в CI,
  на настоящей Windows не проверялась.
- Донаты (`donations.rs`, `ui/extras.rs`): файл `adbtoolkit.json` из `DeMoZ/donations` (raw.githubusercontent →
  jsDelivr → кэш → копия `assets/donations.json` внутри программы), формат schema 1 как в EnhanceCapCut / UniversalKM.
  Ссылки меняются в репозитории donations, не здесь; копию в `assets/` держать такой же.
- Обновления (`update.rs`): схема Windows-версии EnhanceCapCut (`AppUpdates`) на обеих системах — GitHub API
  `releases/latest`, архив `…-macos-arm64.zip` (`ADB Toolkit.app`, упакован `ditto`) или `…-windows-x64.zip`
  (`ADB Toolkit.exe`), проверка SHA-256 из поля `digest`, замена скриптом (bash / PowerShell + robocopy) после выхода,
  перезапуск; при неудаче новая копия стартует с `--update-failed`. Sparkle не используется (Swift-фреймворк).
  В сборке для разработки (`cargo run`) обновление выключено. Сеть — системный `curl` (`net.rs`).
- Mac: только Apple Silicon, сборку для Intel не выпускаем.
- Релиз: `version` в `Cargo.toml` + `docs/releases/v<версия>.md` + тег `v<версия>` (CI сверяет тег с версией).
- Тексты интерфейса на русском.

## Технические грабли (уже наступили, не повторяй)

- **`adb start-server` зависает при перехвате вывода.** Демон наследует каналы stdout/stderr, и чтение вывода
  не завершается. Запускай сервер с перенаправлением в null (`adb::start_server` с `Stdio::null()`, в PowerShell-версии через
  `cmd /c ""adb" start-server >nul 2>&1"`) и только потом выполняй остальные команды adb.
- **Команды с `|` и `grep` на устройстве передавай одним аргументом** в `adb shell "..."`, внутри одинарные кавычки:
  `adb shell "dumpsys window | grep -E 'isKeyguardShowing'"`. Иначе конвейер выполнится на компьютере.
- `dumpsys battery | grep -E 'level|status'` захватывает ещё и строки лога. Используй якорь
  `grep -E '^  (level|status|AC powered|USB powered):'`.
- На этом планшете нет `svc usb resetUsbGadget`. Есть `svc usb setFunctions [mtp|ptp|rndis|midi]` (пусто =
  зарядка), `getFunctions` и `setScreenUnlockedFunctions`.
- Блокировка экрана: `dumpsys window | grep isKeyguardShowing` (true = заблокирован). Бодрствование:
  `dumpsys power | grep mWakefulness=`.
- Не держи длинные команды (`pull`/`push`) в потоке интерфейса: интерфейс не должен зависать, у команд нужен
  таймаут и возможность прервать.
- В PowerShell-версии после любой правки файла `legacy/windows-powershell/AdbToolkit.ps1` нужен UTF-8 **с BOM**, иначе русский текст
  ломается в Windows PowerShell 5.1.
- Серийник для `-s <serial>` берётся из первой колонки `adb devices -l`, а не из `model:`. Была ошибка:
  регулярное выражение для модели затирало `$Matches`, и вместо серийника подставлялась модель.

## Как проверять

- Команды всегда проверяй на реальном устройстве или в режиме самотеста, а не только «по коду».
- Без подключённого устройства состояние планшета (разрешение, режим USB, блокировка) не проверить: прямо
  говори, что не проверено.

## Git

Репозиторий на GitHub: `git@github.com:DeMoZ/AdbToolsGUI.git`. На Windows-машине SSH-ключа для GitHub нет,
использовался HTTPS (`https://github.com/DeMoZ/AdbToolsGUI.git`) с входом через Git Credential Manager.
Основная ветка: `main`.
