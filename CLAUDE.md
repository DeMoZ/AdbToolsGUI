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

## Задача: версия для macOS

Пользователь будет развивать кроссплатформенную версию на Mac. Стек пока не выбран. Обсуждались:
Python + Tkinter (PyInstaller), Electron, нативный Swift. Выбор за пользователем, спроси перед стартом, если он не
сказал.

Что учесть:
- **Блок «Сторона Windows» на Mac не переносится**: `Get-PnpDevice`, `Shell.Application`, `pnputil`, проводник.
  Его можно заменить на проверки macOS (например `system_profiler SPUSBDataType`), но это отдельное решение.
- **macOS не умеет MTP.** Файлы планшета удобнее смотреть через «Файлы через ADB» (`adb pull` / `adb push`) или
  сторонние программы (OpenMTP, Android File Transfer). В кроссплатформенной версии стоит сделать просмотр папок
  планшета через `adb shell ls`.
- Где искать `adb` на Mac: `PATH`, `/opt/homebrew/bin/adb`, `/usr/local/bin/adb`,
  `~/Library/Android/sdk/platform-tools/adb`. Установка: `brew install android-platform-tools`.
- Сборка `.app` / `.dmg`: на Windows её не сделать. Используйте GitHub Actions с раннером `macos-latest`.
  Неподписанное приложение Gatekeeper блокирует, нужны инструкция «открыть через правый клик» или подпись.
- Эту же кодовую базу разумно сделать и под Windows, чтобы поддерживать одну версию.

## Технические грабли (уже наступили, не повторяй)

- **`adb start-server` зависает при перехвате вывода.** Демон наследует каналы stdout/stderr, и чтение вывода
  не завершается. Запускай сервер с перенаправлением в null (в Windows-версии через
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
