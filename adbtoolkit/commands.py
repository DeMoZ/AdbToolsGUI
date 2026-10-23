"""Список команд — источник правды для интерфейса (как массив $script:Commands в Windows-версии).

У команды есть:
- run(adb, args) — выполняется в рабочем потоке, окно при этом не зависает;
- prepare(ui) — необязательно, выполняется в окне до запуска: диалоги выбора файлов и вопросы.
  Возвращает args для run или None, если пользователь отказался (причина уже в журнале);
- action(ui) — вместо run: действие только в окне (например, открыть просмотр файлов).
"""
from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Protocol

from . import macos
from .core import (
    DIM, ERR, HEAD, IS_MAC, OK, WARN, Adb, InstallJob, drop_plan, find_bundletool, find_java, install_aab,
    install_apk, mark_server_down, obb_package, send_obb, start_server,
)


class Ui(Protocol):
    bundletool: str | None

    def log(self, text: str, tag: str = ...) -> None: ...
    def ask_string(self, title: str, prompt: str, initial: str = "") -> str | None: ...
    def ask_files(self, title: str, filetypes: list[tuple[str, str]], multiple: bool = False) -> list[str]: ...
    def ask_directory(self, title: str) -> str | None: ...
    def open_file_browser(self) -> None: ...


@dataclass
class Command:
    group: str
    title: str
    desc: str
    cmd: str
    run: Callable[[Adb, Any], None] | None = None
    prepare: Callable[[Ui], Any] | None = None
    action: Callable[[Ui], None] | None = None
    confirm: str | None = None
    needs_device: bool = True
    mac_only: bool = False


# ---------------------------------------------------------------- подготовка установки

def ask_obb_package(ui: Ui, files: list[str]) -> str | None:
    """Имя пакета для OBB, у которых оно не читается из имени файла. '' — спрашивать не нужно, None — отказ."""
    if all(obb_package(f) for f in files):
        return ""
    pkg = ui.ask_string("Пакет для OBB",
                        "Имя OBB не похоже на main.<версия>.<пакет>.obb. Введите имя пакета приложения:")
    if not pkg or not pkg.strip():
        ui.log("Отменено.", DIM)
        return None
    return pkg.strip()


def aab_tools(ui: Ui) -> tuple[str, str] | None:
    java = find_java()
    if not java:
        ui.log("Java не найдена: bundletool работает на Java. Установите JDK 17+ (на Mac: brew install --cask "
               "temurin) и перезапустите утилиту.", ERR)
        return None
    bundletool = ui.bundletool if ui.bundletool and Path(ui.bundletool).is_file() else find_bundletool()
    if not bundletool:
        ui.log("bundletool.jar не найден рядом с утилитой (на Mac: brew install bundletool, или скачайте с "
               "github.com/google/bundletool/releases). Укажите файл.", WARN)
        files = ui.ask_files("Укажите bundletool.jar", [("bundletool", "*.jar")])
        if not files:
            return None
        bundletool = files[0]
    ui.bundletool = bundletool
    return java, bundletool


def prepare_install(ui: Ui, paths: list[str]) -> InstallJob | None:
    plan = drop_plan(paths)
    for p in plan.ignored:
        ui.log(f"Пропущено (не APK, OBB и не AAB): {p}", WARN)
    if plan.error:
        ui.log(plan.error, ERR)
        return None
    job = InstallJob(plan)
    if plan.obbs:
        fallback = ask_obb_package(ui, plan.obbs)
        if fallback is None:
            return None
        job.obb_fallback = fallback
    if plan.aabs:
        tools = aab_tools(ui)
        if not tools:
            return None
        job.java, job.bundletool = tools
    return job


APK = [("APK", "*.apk")]
OBB = [("OBB", "*.obb")]
AAB = [("AAB", "*.aab")]
ANY_PACKAGE = [("APK, OBB, AAB", "*.apk *.obb *.aab")]


# ---------------------------------------------------------------- команды

def auto_diagnose(a: Adb, _: Any) -> None:
    a.require(device=False)
    devs = a.refresh_devices()
    dev = next((d for d in devs if d.serial == a.serial), None)
    if not dev:
        a.log("[ADB] Устройств не видно. Проверьте кабель (нужен data-кабель) и порт; на планшете должна быть "
              "включена отладка по USB.", ERR)
    elif dev.state == "unauthorized":
        a.log("[ADB] Планшет не дал разрешение. Разблокируйте экран и нажмите «Разрешить» в окне отладки по USB.", ERR)
    elif dev.state != "device":
        a.log(f"[ADB] Состояние: {dev.state}. Попробуйте «Перезапустить ADB-сервер» и переподключить кабель.", ERR)
    else:
        a.log("[ADB] Устройство отвечает.", OK)
        if "isKeyguardShowing=true" in a.shell("dumpsys window | grep -E 'isKeyguardShowing'").out:
            a.log("[Экран] Планшет заблокирован: пока он заблокирован, Android не отдаёт файлы. Разблокируйте его.", ERR)
        else:
            a.log("[Экран] Планшет разблокирован.", OK)
        usb = a.adb(["shell", "getprop", "sys.usb.config"], quiet=True)
        a.log(f"[USB] Текущий режим: {usb.out}")
        if "mtp" not in usb.out:
            a.log("[USB] MTP не включён, включаю.", WARN)
            a.adb(["shell", "svc", "usb", "setFunctions", "mtp"])
            a.sleep(4)
            usb = a.adb(["shell", "getprop", "sys.usb.config"], quiet=True)
            a.log(f"[USB] Режим после переключения: {usb.out}")
        else:
            a.log("[USB] MTP включён.", OK)
    if IS_MAC:
        mac_usb_check(a, devs, dev)


def mac_usb_check(a: Adb, devs: list, dev: Any) -> None:
    usb = macos.usb_devices()
    if usb is None:
        a.log("[Mac] Не удалось получить список USB-устройств (system_profiler).", WARN)
        return
    serials = {d.serial for d in devs}
    android = [(u, macos.android_match(u, serials)) for u in usb]
    android = [(u, m) for u, m in android if m]
    if not android:
        a.log("[Mac] Mac не видит планшет по USB. Нужен кабель с передачей данных (не только зарядка); попробуйте "
              "другой порт или переходник.", ERR)
    for u, m in android:
        sure = "" if m == "adb" else " (похоже на Android)"
        a.log(f"[Mac] По USB видно: {u.name} — {u.vendor or u.vendor_id}, {u.speed or 'скорость неизвестна'}{sure}", OK)
    if android and not dev:
        a.log("[Mac] Mac видит устройство, а ADB нет: включите на планшете «Отладку по USB» в параметрах "
              "разработчика и разрешите этот компьютер.", WARN)
    a.log("[Mac] macOS сама не работает с MTP: файлы планшета открывайте кнопкой «Файлы планшета…» (через ADB) "
          "или в OpenMTP.", DIM)


def restart_server(a: Adb, _: Any) -> None:
    a.adb(["kill-server"], device=False)
    mark_server_down()
    a.log("> adb start-server", HEAD)
    start_server(a.path)
    a.log("ADB-сервер запущен.", OK)


def device_info(a: Adb, _: Any) -> None:
    for prop in ("ro.product.manufacturer", "ro.product.model", "ro.build.version.release", "ro.serialno"):
        a.log(f"{prop} = {a.adb(['shell', 'getprop', prop], quiet=True).out}")
    a.log(a.shell("dumpsys battery | grep -E '^  (level|status|AC powered|USB powered):'").out)


def mac_usb_list(a: Adb, _: Any) -> None:
    usb = macos.usb_devices()
    if usb is None:
        a.log("Не удалось получить список USB-устройств (system_profiler).", ERR)
        return
    serials = set()
    if a.path:
        serials = {d.serial for d in a.devices()}
    if not usb:
        a.log("USB-устройств не найдено.", WARN)
    for u in usb:
        m = macos.android_match(u, serials)
        mark = {"adb": "  ← устройство из adb", "likely": "  ← похоже на Android"}.get(m, "")
        a.log(f"{u.name} — {u.vendor or '?'} [{u.vendor_id}:{u.product_id}] {u.speed}{mark}", OK if m else DIM)
    if usb and not any(macos.android_match(u, serials) for u in usb):
        a.log("Среди них нет Android-устройства: проверьте кабель (нужна передача данных), порт и переходник.", WARN)


def open_mtp_app(ui: Ui) -> None:
    apps = macos.mtp_apps()
    if not apps:
        ui.log("MTP-программ не найдено. Бесплатная: OpenMTP (brew install --cask openmtp или openmtp.ganeshrvel.com). "
               "Пока она открыта, переключите планшет в «Передачу файлов (MTP)».", WARN)
        return
    ui.log(f"> open -a '{apps[0]}'", HEAD)
    subprocess.Popen(["open", "-a", str(apps[0])])
    ui.log("Если программа не видит планшет, включите «Передача файлов (MTP)» и разблокируйте экран.", DIM)


def prepare_pull(ui: Ui) -> tuple[str, str] | None:
    remote = ui.ask_string("Скачать с планшета", "Путь на планшете (файл или папка):", "/sdcard/DCIM")
    if not remote or not remote.strip():
        ui.log("Отменено.", DIM)
        return None
    local = ui.ask_directory("Куда сохранить")
    if not local:
        return None
    return remote.strip(), local


def prepare_one(title: str, filetypes: list[tuple[str, str]]) -> Callable[[Ui], str | None]:
    def prepare(ui: Ui) -> str | None:
        files = ui.ask_files(title, filetypes)
        return files[0] if files else None
    return prepare


def prepare_apk_obb(ui: Ui) -> tuple[str, list[str], str] | None:
    apk = ui.ask_files("APK для установки", APK)
    if not apk:
        return None
    obbs = ui.ask_files("OBB-файлы (можно несколько)", OBB, multiple=True)
    if not obbs:
        return None
    fallback = ask_obb_package(ui, obbs)
    if fallback is None:
        return None
    return apk[0], obbs, fallback


def run_apk_obb(a: Adb, args: tuple[str, list[str], str]) -> None:
    apk, obbs, fallback = args
    if install_apk(a, apk):
        send_obb(a, obbs, fallback)


def prepare_aab(ui: Ui) -> tuple[str, str, str] | None:
    aab = ui.ask_files("AAB для установки", AAB)
    if not aab:
        return None
    tools = aab_tools(ui)
    if not tools:
        return None
    return aab[0], *tools


def shell_cmd(*args: str) -> Callable[[Adb, Any], None]:
    def run(a: Adb, _: Any) -> None:
        a.adb(["shell", *args])
    return run


GROUP_AUTO = "Быстрое решение"
GROUP_ADB = "Подключение ADB"
GROUP_USB = "Режим USB на планшете"
GROUP_SCREEN = "Экран и питание"
GROUP_INFO = "Информация о планшете"
GROUP_MAC = "Сторона macOS"
GROUP_FILES = "Файлы через ADB"
GROUP_INSTALL = "Установка приложений"


def _usb_mode(mode: str, wait: float = 0) -> Callable[[Adb, Any], None]:
    def run(a: Adb, _: Any) -> None:
        a.adb(["shell", "svc", "usb", "setFunctions", *([mode] if mode else [])])
        if wait:
            a.sleep(wait)
            a.adb(["shell", "getprop", "sys.usb.config"])
    return run


def _usb_state(a: Adb, _: Any) -> None:
    a.adb(["shell", "getprop", "sys.usb.config"])
    a.adb(["shell", "getprop", "sys.usb.state"])


def _lock_state(a: Adb, _: Any) -> None:
    a.adb(["shell", "dumpsys window | grep -E 'isKeyguardShowing|mDreamingLockscreen'"])
    a.adb(["shell", "dumpsys power | grep mWakefulness="])


COMMANDS: list[Command] = [
    Command(GROUP_AUTO, "Авто-диагностика",
            "Проверяет цепочку: ADB, разрешение на планшете, блокировку экрана, USB-режим и что Mac видит по USB. "
            "Если режим не MTP, включает его.",
            "несколько проверок + svc usb setFunctions mtp",
            run=auto_diagnose, needs_device=False),

    Command(GROUP_ADB, "Список устройств",
            "Показывает устройства и их статус: device (всё хорошо), unauthorized (нет разрешения на планшете), "
            "offline (потеряна связь).",
            "adb devices -l",
            run=lambda a, _: a.adb(["devices", "-l"], device=False), needs_device=False),
    Command(GROUP_ADB, "Перезапустить ADB-сервер",
            "Лечит зависшее подключение и статус offline: останавливает и заново запускает фоновый сервис adb "
            "на компьютере.",
            "adb kill-server && adb start-server",
            run=restart_server, needs_device=False),
    Command(GROUP_ADB, "Переподключить устройство",
            "Заставляет планшет заново согласовать связь с ADB. Помогает, если статус offline или окно разрешения "
            "не появилось.",
            "adb -s <серийник> reconnect",
            run=lambda a, _: a.adb(["reconnect"])),
    Command(GROUP_ADB, "Ждать устройство (30 с)",
            "Ждёт, пока планшет станет доступен. Удобно запустить и в это время нажать «Разрешить» на экране.",
            "adb -s <серийник> wait-for-device",
            run=lambda a, _: a.adb(["wait-for-device"], timeout=30)),

    Command(GROUP_USB, "Текущий режим USB",
            "Показывает, какие USB-функции включены. Если в списке есть mtp, передача файлов работает; "
            "sec_charging значит только зарядка.",
            "adb shell getprop sys.usb.config",
            run=_usb_state),
    Command(GROUP_USB, "Передача файлов (MTP)",
            "Включает режим передачи файлов, даже если выбор в шторке не сработал. Нужен для OpenMTP на Mac "
            "и Проводника в Windows.",
            "adb shell svc usb setFunctions mtp",
            run=_usb_mode("mtp", wait=3)),
    Command(GROUP_USB, "Фото (PTP)",
            "Режим передачи изображений: планшет виден как камера (например, в «Захвате изображений» на Mac).",
            "adb shell svc usb setFunctions ptp",
            run=_usb_mode("ptp")),
    Command(GROUP_USB, "USB-модем (RNDIS)",
            "Раздаёт интернет планшета на компьютер по кабелю. Передача файлов при этом отключается. "
            "macOS RNDIS без сторонних драйверов не поддерживает.",
            "adb shell svc usb setFunctions rndis",
            run=_usb_mode("rndis")),
    Command(GROUP_USB, "Только зарядка",
            "Отключает передачу данных, оставляет зарядку.",
            "adb shell svc usb setFunctions",
            run=_usb_mode("")),
    Command(GROUP_USB, "MTP при разблокировке",
            "Просит Android включать передачу файлов каждый раз, когда вы разблокируете экран с подключённым кабелем.",
            "adb shell svc usb setScreenUnlockedFunctions mtp",
            run=shell_cmd("svc", "usb", "setScreenUnlockedFunctions", "mtp")),

    Command(GROUP_SCREEN, "Разбудить экран",
            "Включает погасший экран. Это не снимает блокировку: PIN или рисунок всё равно вводятся на планшете.",
            "adb shell input keyevent KEYCODE_WAKEUP",
            run=shell_cmd("input", "keyevent", "KEYCODE_WAKEUP")),
    Command(GROUP_SCREEN, "Проверить блокировку",
            "Показывает, заблокирован ли планшет. При isKeyguardShowing=true файлы по USB недоступны.",
            "adb shell dumpsys window | grep isKeyguardShowing",
            run=_lock_state),
    Command(GROUP_SCREEN, "Не гасить экран при USB",
            "Пока кабель подключён, экран не будет гаснуть и блокироваться по таймеру. Удобно при долгом копировании.",
            "adb shell svc power stayon usb",
            run=shell_cmd("svc", "power", "stayon", "usb")),
    Command(GROUP_SCREEN, "Вернуть автоотключение",
            "Отменяет предыдущую команду: экран снова гаснет по обычному таймеру.",
            "adb shell svc power stayon false",
            run=shell_cmd("svc", "power", "stayon", "false")),
    Command(GROUP_SCREEN, "Перезагрузить планшет",
            "Обычная перезагрузка. Помогает, когда ничего другое не лечит USB.",
            "adb reboot",
            run=lambda a, _: a.adb(["reboot"]), confirm="Перезагрузить выбранное устройство?"),

    Command(GROUP_INFO, "Сведения об устройстве",
            "Модель, версия Android, серийный номер, заряд батареи.",
            "adb shell getprop ro.product.model / ro.build.version.release; dumpsys battery",
            run=device_info),
    Command(GROUP_INFO, "Свободное место",
            "Сколько занято и свободно во внутренней памяти.",
            "adb shell df -h /sdcard",
            run=shell_cmd("df", "-h", "/sdcard")),
    Command(GROUP_INFO, "Содержимое /sdcard",
            "Список папок внутренней памяти напрямую через ADB. Если здесь файлы есть, а в MTP-программе пусто, "
            "проблема в MTP.",
            "adb shell ls -l /sdcard",
            run=shell_cmd("ls", "-l", "/sdcard")),

    Command(GROUP_MAC, "USB-устройства",
            "Что Mac видит по USB. Если планшета нет и здесь, дело в кабеле, порте или переходнике, а не в ADB.",
            "system_profiler SPUSBHostDataType",
            run=mac_usb_list, needs_device=False, mac_only=True),
    Command(GROUP_MAC, "Открыть MTP-программу",
            "Открывает OpenMTP или Android File Transfer, если они установлены; иначе подсказывает, где взять.",
            "open -a OpenMTP",
            action=open_mtp_app, needs_device=False, mac_only=True),

    Command(GROUP_FILES, "Файлы планшета…",
            "Окно с папками планшета через ADB: просмотр, скачивание и отправка файлов. Работает без MTP.",
            "adb shell ls -la; adb pull; adb push",
            action=lambda ui: ui.open_file_browser()),
    Command(GROUP_FILES, "Скачать с планшета…",
            "Копирует файл или папку с планшета на компьютер по указанному пути.",
            "adb pull <путь на планшете> <папка на компьютере>",
            prepare=prepare_pull,
            run=lambda a, args: a.adb(["pull", args[0], args[1]], timeout=900)),
    Command(GROUP_FILES, "Отправить на планшет…",
            "Копирует выбранный файл с компьютера в папку Download на планшете.",
            "adb push <файл> /sdcard/Download/",
            prepare=prepare_one("Файл для отправки на планшет", []),
            run=lambda a, path: a.adb(["push", path, "/sdcard/Download/"], timeout=900)),

    Command(GROUP_INSTALL, "Установить APK…",
            "Устанавливает выбранный APK на планшет. Если приложение уже есть, обновляет его с сохранением данных.",
            "adb install -r -t <файл.apk>",
            prepare=prepare_one("APK для установки", APK),
            run=lambda a, path: install_apk(a, path)),
    Command(GROUP_INSTALL, "Установить APK + OBB…",
            "Ставит APK и копирует файлы OBB (кэш игры) в /sdcard/Android/obb/<пакет>/. Пакет берётся из имени OBB: "
            "main.<версия>.<пакет>.obb.",
            "adb install -r -t <apk>; adb push <obb> /sdcard/Android/obb/<пакет>/",
            prepare=prepare_apk_obb, run=run_apk_obb),
    Command(GROUP_INSTALL, "Установить AAB…",
            "Собирает из AAB APK под этот планшет и ставит их через bundletool. Нужны Java и bundletool "
            "(brew install bundletool или bundletool.jar рядом с утилитой).",
            "bundletool build-apks --connected-device; bundletool install-apks",
            prepare=prepare_aab, run=lambda a, args: install_aab(a, *args)),
]


def visible_commands() -> list[Command]:
    return [c for c in COMMANDS if IS_MAC or not c.mac_only]
