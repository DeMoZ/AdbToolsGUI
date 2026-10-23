"""Проверка без окна на реальном компьютере: какие программы найдены и как отвечают основные команды."""
from __future__ import annotations

from . import commands
from .core import Abort, Adb, IS_MAC, app_dir, find_adb, find_bundletool, find_java

SELFTEST_COMMANDS = ("Авто-диагностика", "Текущий режим USB", "Проверить блокировку", "Сведения об устройстве",
                     "USB-устройства", "Перезапустить ADB-сервер")


def run() -> int:
    print(f"app dir: {app_dir()}")
    print(f"java: {find_java()}; bundletool: {find_bundletool()}")
    a = Adb()
    a.path = find_adb()
    print(f"adb: {a.path}")
    if not a.path:
        return 1
    devices = a.devices()
    for d in devices:
        print(f"  {d.serial:<24} {d.state:<14} {d.model}")
    if not devices:
        print("  (устройств нет: команды для устройства пропустятся)")
    a.serial = devices[0].serial if devices else None
    by_title = {c.title: c for c in commands.COMMANDS}
    for title in SELFTEST_COMMANDS:
        c = by_title[title]
        if c.mac_only and not IS_MAC:
            continue
        print(f"=== {title} ===")
        try:
            c.run(a, None)
        except Abort:
            pass
    return 0
