"""Проверки на стороне macOS: что Mac видит по USB и какие MTP-программы установлены.

macOS сама MTP не умеет, поэтому замены «Проводника» здесь нет: файлы смотрят через ADB
(окно «Файлы планшета») или сторонние программы вроде OpenMTP.
"""
from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

# Производители Android-устройств по USB vendor id. Samsung делает и SSD, поэтому совпадение только
# по производителю считается «похоже на Android», а точное — по серийнику из adb.
ANDROID_VENDORS = {
    "04e8": "Samsung", "18d1": "Google", "22b8": "Motorola", "12d1": "Huawei", "2717": "Xiaomi",
    "2a70": "OnePlus", "0bb4": "HTC", "1004": "LG", "0fce": "Sony", "19d2": "ZTE", "22d9": "OPPO / realme",
    "2d95": "vivo",
}
ANDROID_NAME_RE = re.compile(r"android|galaxy|pixel|\bmtp\b|\badb\b|SM-[A-Z]\d", re.IGNORECASE)

MTP_APPS = ("OpenMTP.app", "Android File Transfer.app", "MacDroid.app")


@dataclass(frozen=True)
class UsbDevice:
    name: str
    vendor: str
    vendor_id: str
    product_id: str
    serial: str
    speed: str


def _hex_id(value: str) -> str:
    # "0x04e8" или "0x04e8  (Samsung Electronics Co., Ltd.)" -> "04e8"
    m = re.match(r"\s*0x([0-9a-fA-F]{1,4})", value or "")
    return m.group(1).lower().zfill(4) if m else ""


def parse_usb(items: list) -> list[UsbDevice]:
    """Обходит дерево system_profiler: новый формат (SPUSBHostDataType) и старый (SPUSBDataType)."""
    found: list[UsbDevice] = []

    def walk(nodes: list) -> None:
        for n in nodes or []:
            if not isinstance(n, dict):
                continue
            vid = n.get("USBDeviceKeyVendorID") or n.get("vendor_id") or ""
            if vid:
                vendor = n.get("USBDeviceKeyVendorName") or n.get("manufacturer") or ""
                if not vendor:
                    m = re.search(r"\((.+)\)", vid)
                    vendor = m.group(1) if m else ""
                found.append(UsbDevice(
                    name=n.get("_name", ""),
                    vendor=vendor,
                    vendor_id=_hex_id(vid),
                    product_id=_hex_id(n.get("USBDeviceKeyProductID") or n.get("product_id") or ""),
                    serial=n.get("USBDeviceKeySerialNumber") or n.get("serial_num") or "",
                    speed=n.get("USBDeviceKeyLinkSpeed") or n.get("device_speed") or "",
                ))
            walk(n.get("_items", []))

    walk(items)
    return found


def usb_devices() -> list[UsbDevice] | None:
    """USB-устройства, которые видит Mac; None, если system_profiler не ответил."""
    for data_type in ("SPUSBHostDataType", "SPUSBDataType"):
        try:
            r = subprocess.run(["system_profiler", data_type, "-json"], capture_output=True, text=True, timeout=40)
            items = json.loads(r.stdout or "{}").get(data_type)
        except (OSError, subprocess.TimeoutExpired, ValueError):
            continue
        if items:
            return parse_usb(items)
    return None


def android_match(dev: UsbDevice, adb_serials: set[str]) -> str | None:
    """'adb' — это устройство из списка adb, 'likely' — похоже на Android, None — не похоже."""
    if dev.serial and dev.serial in adb_serials:
        return "adb"
    if dev.vendor_id in ANDROID_VENDORS or ANDROID_NAME_RE.search(dev.name):
        return "likely"
    return None


def mtp_apps() -> list[Path]:
    found = []
    for folder in (Path("/Applications"), Path.home() / "Applications"):
        for name in MTP_APPS:
            if (folder / name).is_dir():
                found.append(folder / name)
    return found
