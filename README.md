# ADB Toolkit

Графическая утилита для диагностики и «лечения» USB-подключения Android-устройств через ADB: разрешение отладки,
режим USB (MTP / PTP / зарядка), блокировка экрана, сведения об устройстве, файлы через ADB и установка APK / OBB / AAB
(можно перетащить файлы в окно).

Две версии:

- **macOS (и кроссплатформенная основа)** — Python + Tkinter, папка [`adbtoolkit/`](adbtoolkit).
- **Windows** — PowerShell + WinForms: [`AdbToolkit.ps1`](AdbToolkit.ps1), запуск через [`AdbToolkit.bat`](AdbToolkit.bat).
  В релизах это архив `AdbToolkit-Windows-<версия>.zip`: распакуйте и запустите `AdbToolkit.bat`.

## macOS

### Что нужно

`adb`. Проще всего через Homebrew:

```bash
brew install android-platform-tools
```

Утилита ищет `adb` в папке `platform-tools` рядом с приложением, в `PATH`, в `/opt/homebrew/bin`, `/usr/local/bin`
и `~/Library/Android/sdk/platform-tools`. Путь можно указать и вручную в окне.

Для установки AAB дополнительно нужны Java 17+ и bundletool:

```bash
brew install --cask temurin
```

```bash
brew install bundletool
```

(или положите `bundletool*.jar` рядом с приложением).

### Готовое приложение

Скачайте `ADB-Toolkit-macOS.dmg` со страницы Releases (или из артефактов сборки GitHub Actions) и перетащите
приложение в «Программы». Сборка для Apple Silicon.

Приложение не подписано, поэтому при первом запуске macOS его заблокирует. Откройте «Системные настройки →
Конфиденциальность и безопасность», внизу нажмите «Всё равно открыть» у ADB Toolkit. Либо снимите карантин командой:

```bash
xattr -dr com.apple.quarantine "/Applications/ADB Toolkit.app"
```

### Запуск из исходников

Нужен Python 3.10+ с Tk 8.6 (Python с python.org или `brew install python-tk`; системный `/usr/bin/python3` со старым
Tk 8.5 не подходит). Двойной щелчок по [`AdbToolkit.command`](AdbToolkit.command) сам создаст `.venv` и запустит окно.
Без пакета `tkinterdnd2` всё работает, кроме перетаскивания файлов.

Проверки:

```bash
python3 -m unittest discover -s tests -t .
```

```bash
python3 -m adbtoolkit --selftest
```

`--selftest` без окна показывает найденные adb / Java / bundletool, устройства и прогоняет основные команды.

### Сборка `.app` и `.dmg`

```bash
scripts/build-macos.sh
```

Результат в `dist/`. То же самое делает GitHub Actions ([`build.yml`](.github/workflows/build.yml)) на каждый
PR и push в `main`; на тег `v*` создаётся релиз с `.dmg` для Mac и `.zip` для Windows.

### Чем Mac отличается от Windows

- macOS сама не умеет MTP: в Finder планшет не появится. Файлы открываются кнопкой **«Файлы планшета…»** (просмотр папок,
  скачивание и отправка через ADB) или в OpenMTP (`brew install --cask openmtp`).
- Вместо блока «Сторона Windows» есть **«Сторона macOS»**: что Mac видит по USB (`system_profiler`) и запуск
  MTP-программы. Авто-диагностика тоже проверяет USB: если планшета нет даже там, дело в кабеле, порте или переходнике.
- Долгие команды выполняются в фоне, окно не зависает; кнопка **«Прервать»** останавливает текущую команду.
