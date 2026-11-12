# ADB Toolkit

Утилита для диагностики и «лечения» USB-подключения Android-устройств через ADB: разрешение отладки,
режим USB (MTP / PTP / зарядка), блокировка экрана, сведения об устройстве, файлы планшета и установка
APK / OBB / AAB перетаскиванием в окно.

Одна программа для macOS и Windows, написана на Rust ([egui](https://github.com/emilk/egui)). Светлая и тёмная
тема переключаются вместе с системой.

## Что внутри

- **Диагностика** — одна кнопка проверяет всю цепочку: ADB видит устройство, планшет разрешил отладку, экран
  разблокирован, включён режим передачи файлов (если нет — включает), компьютер видит планшет по USB. Результат
  показывается чек-листом.
- **Подключение ADB, Режим USB, Экран и питание, Об устройстве** — отдельные команды; у каждой видно, что именно
  выполняется (`adb shell svc usb setFunctions mtp` и т. п.).
- **Этот Mac / Этот компьютер** — что система видит по USB: `system_profiler` на Mac, MTP-устройства и драйверы
  в Windows (с перезапуском драйвера MTP).
- **Файлы** — папки планшета через ADB: просмотр, скачивание, отправка (можно перетащить файлы в окно). Работает
  без MTP, поэтому на Mac это основной способ добраться до файлов.
- **Установка приложений** — перетащите APK, APK + OBB или AAB: утилита сама решит, что с ними делать.
- **Журнал** снизу: цветной вывод всех команд, «Прервать» для долгих операций и поле для своей команды `adb`
  (стрелки ↑ / ↓ листают историю).

## Скачать

Готовые сборки — на странице Releases:

- `ADB-Toolkit-macOS-<версия>.dmg` — для Mac (Apple Silicon и Intel, macOS 11+);
- `ADB-Toolkit-Windows-<версия>.zip` — для Windows 10 / 11: распакуйте и запустите `ADB Toolkit.exe`;
- `AdbToolkit-Windows-PowerShell-<версия>.zip` — прежняя Windows-версия на PowerShell (запасной вариант).

Приложение не подписано сертификатом разработчика:

- **macOS** при первом запуске его заблокирует. Откройте «Системные настройки → Конфиденциальность и
  безопасность» и внизу нажмите «Всё равно открыть». Либо снимите карантин:

  ```bash
  xattr -dr com.apple.quarantine "/Applications/ADB Toolkit.app"
  ```

- **Windows** может показать SmartScreen: «Подробнее» → «Выполнить в любом случае».

## Что нужно

`adb`. Утилита ищет его в папке `platform-tools` рядом с программой, в `PATH` и в обычных местах установки
(`/opt/homebrew/bin`, `~/Library/Android/sdk/platform-tools`, `%LOCALAPPDATA%\Android\Sdk\platform-tools`,
`C:\platform-tools`). Путь можно указать и вручную в «Настройках».

На Mac проще всего через Homebrew:

```bash
brew install android-platform-tools
```

В Windows скачайте [SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools) и положите
папку `platform-tools` рядом с `ADB Toolkit.exe`.

Для установки AAB дополнительно нужны Java 17+ и bundletool (или `bundletool*.jar` рядом с программой):

```bash
brew install --cask temurin
```

```bash
brew install bundletool
```

## Чем Mac отличается от Windows

- macOS сама не умеет MTP: в Finder планшет не появится. Файлы открываются в разделе **«Файлы»** (через ADB) или
  в OpenMTP (`brew install --cask openmtp`).
- Раздел **«Этот Mac»** показывает, что Mac видит по USB. Если планшета нет даже там, дело в кабеле, порте или
  переходнике, а не в ADB.
- В Windows раздел **«Этот компьютер»** показывает, что видит Проводник, и умеет перезапустить драйвер MTP.

## Сборка из исходников

Нужен Rust (через [rustup](https://rustup.rs)).

```bash
cargo run
```

Проверки:

```bash
cargo test
```

```bash
cargo run -- --selftest
```

`--selftest` без окна показывает найденные adb / Java / bundletool, устройства и прогоняет основные команды
(авто-диагностика при этом может включить на планшете режим MTP). Проверка файлов на подключённом устройстве
(отправить, найти, скачать обратно, удалить):

```bash
cargo test -- --ignored
```

Сборка `.app` и `.dmg` для Mac (Apple Silicon + Intel):

```bash
scripts/build-macos.sh
```

Результат в `dist/`. GitHub Actions ([`build.yml`](.github/workflows/build.yml)) собирает обе версии на каждый PR и
push в `main`; на тег `v*` создаётся релиз с `.dmg` для Mac и `.zip` для Windows.

Значок перерисовывается скриптом `scripts/make-icon.sh` (нужна macOS).

## Устройство кода

| Файл | Что делает |
| --- | --- |
| `src/adb.rs` | запуск adb и других программ: таймаут, «Прервать», журнал, события для окна |
| `src/commands.rs` | список команд — источник правды для интерфейса |
| `src/devices.rs`, `src/files.rs` | разбор `adb devices -l` и `ls -la` |
| `src/install.rs` | APK / OBB / AAB: что ставить и как |
| `src/host.rs` | что видит компьютер: `system_profiler` (Mac), PnP и Проводник (Windows) |
| `src/tools.rs` | поиск adb, Java и bundletool |
| `src/ui/` | окно: тема, боковое меню, разделы, файлы, журнал |
