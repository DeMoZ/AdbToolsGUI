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

Рабочая Windows-версия: [AdbToolkit.ps1](AdbToolkit.ps1) (PowerShell 5.1 + WinForms) и запуск через
[AdbToolkit.bat](AdbToolkit.bat). Проверка без окна: `powershell -STA -File AdbToolkit.ps1 -SelfTest`.

Интерфейс:
- сверху путь к `adb.exe` и выбор подключённого устройства, цветной статус (device / unauthorized / offline),
  автообновление раз в 3 секунды;
- слева кнопки команд по группам: у каждой название, краткое описание и сама команда серым шрифтом;
- в начале раздела «Установка приложений» область для перетаскивания APK / OBB / AAB (`Get-DropPlan` решает по расширениям: AAB через bundletool,
  APK через `adb install`, OBB копируются после установки; AAB вместе с APK не принимаются);
- справа цветной журнал вывода, «Очистить» и «Копировать всё»;
- снизу поле для своей команды `adb` к выбранному устройству.

Список команд лежит в массиве `$script:Commands` (поля Group, Title, Desc, Cmd, Run, необязательный Confirm).
Это источник правды для переноса: группы «Быстрое решение» (авто-диагностика), «Подключение ADB», «Режим USB на
планшете», «Экран и питание», «Информация о планшете», «Файлы через ADB», «Установка приложений» (APK: `adb install -r -t`; APK + OBB: установка и
`adb push` в `/sdcard/Android/obb/<пакет>/`, пакет берётся из имени `main.<версия>.<пакет>.obb`; AAB: через `bundletool`
`build-apks --connected-device` + `install-apks`, нужны Java и `bundletool.jar` рядом со скриптом).

## Версия для macOS (Python + Tkinter)

Стек выбран: Python 3.10+ и Tkinter, перетаскивание через `tkinterdnd2` (без него зона установки работает по
щелчку). Код в [adbtoolkit/](adbtoolkit), описание для пользователя в [README.md](README.md).

- `core.py` — поиск adb / Java / bundletool, запуск программ (`Adb.run`: таймаут, прерывание), разбор
  `adb devices -l` и `ls -la`, установка APK / OBB / AAB (`drop_plan`, `install_planned`). Без Tk.
- `commands.py` — список команд `COMMANDS`, источник правды (как `$script:Commands`). У команды `run(adb, args)`
  выполняется в рабочем потоке; диалоги (выбор файлов, вопросы) — только в `prepare(ui)` до запуска, в потоке их
  вызывать нельзя. `action(ui)` — действие только в окне.
- `macos.py` — `system_profiler SPUSBHostDataType` (старые macOS: `SPUSBDataType`) и поиск OpenMTP.
- `gui.py` — окно; `filebrowser.py` — «Файлы планшета» через `adb shell ls -la` + pull / push.
- Журнал и список устройств идут из потоков в окно через очередь. Фоновый опрос `adb devices` раз в 3 с идёт отдельным
  экземпляром `Adb` и пропускается, пока выполняется команда (`job_lock`).
- Проверки: `python3 -m unittest discover -s tests -t .` и `python3 -m adbtoolkit --selftest`.
- Сборка: `scripts/build-macos.sh` (PyInstaller → `dist/ADB Toolkit.app` и `.dmg`), в CI —
  `.github/workflows/build-macos.yml` на `macos-latest` (arm64). Приложение не подписано: в README инструкция
  для Gatekeeper.
- Приложение из Finder получает короткий `PATH` без Homebrew, поэтому пути к adb / Java / bundletool перечислены явно.
  `/usr/bin/java` на Mac — заглушка без JDK, Java ищется через `/usr/libexec/java_home`.
- Блок «Сторона Windows» в Python-версию не перенесён: на Windows пока работает `AdbToolkit.ps1`. Остальной код
  кроссплатформенный (пути adb для Windows, `CREATE_NO_WINDOW`), но на Windows не проверялся.
- Тексты интерфейса на русском; при правке команд меняйте их в обеих версиях, если команда общая.

## Технические грабли (уже наступили, не повторяй)

- **`adb start-server` зависает при перехвате вывода.** Демон наследует каналы stdout/stderr, и чтение вывода
  не завершается. Запускай сервер с перенаправлением в null (в Python-версии `start_server` с выводом в `DEVNULL`, в Windows-версии через
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
- В PowerShell-версии после любой правки файла `AdbToolkit.ps1` нужен UTF-8 **с BOM**, иначе русский текст
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
