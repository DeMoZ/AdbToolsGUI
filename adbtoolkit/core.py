"""ADB, внешние программы и установка приложений: всё, что не зависит от окна.

Методы Adb вызываются из рабочего потока. Журнал пишется через функцию log(text, tag),
которую передаёт окно (из любого потока), а без окна печатается в консоль.
"""
from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

IS_MAC = sys.platform == "darwin"
IS_WIN = os.name == "nt"

# Теги журнала: цвета для них задаёт окно.
TEXT, HEAD, ERR, WARN, OK, DIM = "text", "head", "err", "warn", "ok", "dim"

LogFn = Callable[..., None]


class Cancelled(Exception):
    """Пользователь нажал «Прервать»."""


class Abort(Exception):
    """Команду нельзя продолжать; причина уже записана в журнал."""


def print_log(text: str, tag: str = TEXT) -> None:
    print(text, flush=True)


def _popen_flags() -> dict:
    # В Windows без этого флага у каждого вызова adb мелькает консольное окно.
    if IS_WIN:
        return {"creationflags": subprocess.CREATE_NO_WINDOW}
    return {}


def app_dir() -> Path:
    """Папка, рядом с которой ищем platform-tools и bundletool: папка с .app / .exe или корень репозитория."""
    if getattr(sys, "frozen", False):
        exe = Path(sys.executable).resolve()
        # .../ADB Toolkit.app/Contents/MacOS/ADB Toolkit -> папка, где лежит .app
        if IS_MAC and len(exe.parents) > 3 and exe.parents[2].suffix == ".app":
            return exe.parents[3]
        return exe.parent
    return Path(__file__).resolve().parent.parent


# ---------------------------------------------------------------- поиск программ

def adb_candidates() -> list[Path]:
    exe = "adb.exe" if IS_WIN else "adb"
    found = [app_dir() / "platform-tools" / exe]
    for var in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        if os.environ.get(var):
            found.append(Path(os.environ[var]) / "platform-tools" / exe)
    if IS_WIN:
        found.append(Path(r"C:\platform-tools\adb.exe"))
        if os.environ.get("LOCALAPPDATA"):
            found.append(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk" / "platform-tools" / exe)
    else:
        # Приложение, запущенное из Finder, получает короткий PATH без Homebrew, поэтому пути перечислены явно.
        found += [
            Path("/opt/homebrew/bin/adb"),
            Path("/usr/local/bin/adb"),
            Path.home() / "Library" / "Android" / "sdk" / "platform-tools" / "adb",
            Path.home() / "Android" / "Sdk" / "platform-tools" / "adb",
        ]
    return found


def find_adb() -> str | None:
    candidates = adb_candidates()
    if candidates[0].is_file():
        return str(candidates[0])
    on_path = shutil.which("adb")
    if on_path:
        return on_path
    for p in candidates[1:]:
        if p.is_file():
            return str(p)
    return None


def find_java() -> str | None:
    exe = "java.exe" if IS_WIN else "java"
    if os.environ.get("JAVA_HOME"):
        p = Path(os.environ["JAVA_HOME"]) / "bin" / exe
        if p.is_file():
            return str(p)
    if IS_MAC:
        # /usr/bin/java на Mac есть всегда, но без установленной JDK это заглушка, которая только показывает окно.
        try:
            r = subprocess.run(["/usr/libexec/java_home"], capture_output=True, text=True, timeout=10)
            if r.returncode == 0 and r.stdout.strip():
                p = Path(r.stdout.strip()) / "bin" / "java"
                if p.is_file():
                    return str(p)
        except (OSError, subprocess.TimeoutExpired):
            pass
        for p in ("/opt/homebrew/opt/openjdk/bin/java", "/usr/local/opt/openjdk/bin/java"):
            if Path(p).is_file():
                return p
        return None
    return shutil.which(exe)


def find_bundletool() -> str | None:
    base = app_dir()
    for d in (base, base / "tools", base / "platform-tools"):
        if d.is_dir():
            jars = sorted(d.glob("bundletool*.jar"), reverse=True)
            if jars:
                return str(jars[0])
    # brew install bundletool
    for d in ("/opt/homebrew/opt/bundletool/libexec", "/usr/local/opt/bundletool/libexec"):
        jars = sorted(Path(d).glob("bundletool*.jar"), reverse=True) if Path(d).is_dir() else []
        if jars:
            return str(jars[0])
    return None


# ---------------------------------------------------------------- устройства

DEVICE_RE = re.compile(
    r"^(\S+)\s+(device|unauthorized|offline|recovery|sideload|bootloader|authorizing|connecting|no permissions\S*)\s*(.*)$")


@dataclass(frozen=True)
class Device:
    serial: str
    state: str
    model: str = ""


def parse_devices(text: str) -> list[Device]:
    """Разбирает вывод `adb devices -l`. Серийник берётся из первой колонки, а не из model:."""
    result = []
    for line in text.splitlines():
        m = DEVICE_RE.match(line)
        if not m:
            continue
        serial, state, extra = m.group(1), m.group(2), m.group(3)
        model = re.search(r"model:(\S+)", extra)
        result.append(Device(serial, state, model.group(1).replace("_", " ") if model else ""))
    return result


def state_text(state: str) -> tuple[str, str]:
    return {
        "device": ("готово к работе", OK),
        "unauthorized": ("не разрешено на планшете: разблокируйте экран и нажмите «Разрешить»", WARN),
        "offline": ("offline: переподключите кабель или перезапустите ADB-сервер", ERR),
        "authorizing": ("идёт авторизация, подождите", WARN),
    }.get(state, (state, WARN))


# ---------------------------------------------------------------- запуск программ

@dataclass
class Result:
    out: str = ""
    err: str = ""
    code: int = -1
    timed_out: bool = False

    @property
    def ok(self) -> bool:
        return self.code == 0 and not self.timed_out


_server_lock = threading.Lock()
_server_up = False


def start_server(adb_path: str) -> None:
    """`adb start-server` оставляет демона, который наследует каналы вывода: если их читать, чтение не кончится.

    Поэтому сервер запускается с выводом в /dev/null, а остальные команды adb идут уже к работающему серверу.
    """
    global _server_up
    with _server_lock:
        try:
            subprocess.run([adb_path, "start-server"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=20, **_popen_flags())
        except (OSError, subprocess.TimeoutExpired):
            pass
        _server_up = True


def mark_server_down() -> None:
    global _server_up
    with _server_lock:
        _server_up = False


def server_up() -> bool:
    return _server_up


class Adb:
    def __init__(self, log: LogFn = print_log):
        self.log = log
        self.path: str | None = None
        self.serial: str | None = None
        self.on_devices: Callable[[list[Device]], None] | None = None
        self._proc: subprocess.Popen | None = None
        self._cancel = threading.Event()

    # --- прерывание (вызывается из окна)

    def reset_cancel(self) -> None:
        self._cancel.clear()

    def cancel(self) -> None:
        self._cancel.set()
        p = self._proc
        if p and p.poll() is None:
            try:
                p.kill()
            except OSError:
                pass

    def check_cancel(self) -> None:
        if self._cancel.is_set():
            raise Cancelled()

    def sleep(self, seconds: float) -> None:
        if self._cancel.wait(seconds):
            raise Cancelled()

    # --- запуск

    def run(self, argv: list[str], timeout: float = 30) -> Result:
        """Запускает программу и ждёт её не дольше timeout секунд."""
        self.check_cancel()
        try:
            p = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 text=True, encoding="utf-8", errors="replace", **_popen_flags())
        except OSError as e:
            return Result(err=str(e))
        self._proc = p
        try:
            try:
                out, err = p.communicate(timeout=timeout)
                timed_out = False
            except subprocess.TimeoutExpired:
                p.kill()
                timed_out = True
                try:
                    out, err = p.communicate(timeout=1.5)
                except subprocess.TimeoutExpired:
                    # вывод держит дочерний процесс (например, демон adb): не ждём его
                    out, err = "", ""
        finally:
            self._proc = None
        self.check_cancel()
        return Result((out or "").rstrip(), (err or "").rstrip(), -1 if timed_out else p.returncode, timed_out)

    def report(self, r: Result, timeout: float) -> None:
        if r.out:
            self.log(r.out)
        if r.err:
            self.log(r.err, DIM if r.code == 0 else ERR)
        if r.timed_out:
            self.log(f"Команда не завершилась за {timeout:g} с и была прервана.", WARN)

    def require(self, device: bool = True) -> None:
        """Прерывает команду, если нет adb или (при device=True) не выбрано устройство."""
        if not self.path:
            self.log("adb не найден. Укажите путь к нему сверху.", ERR)
            raise Abort()
        if device and not self.serial:
            self.log("Сначала выберите устройство в списке сверху.", WARN)
            raise Abort()

    def adb(self, args: list[str], device: bool = True, quiet: bool = False, timeout: float = 30) -> Result:
        self.require(device)
        full = ["-s", self.serial] if device else []
        full += args
        if not server_up():
            start_server(self.path)
        if not quiet:
            self.log("> adb " + shlex.join(full), HEAD)
        r = self.run([self.path, *full], timeout)
        if not quiet:
            self.report(r, timeout)
            if "unauthorized" in r.err:
                self.log("Подсказка: разблокируйте планшет и разрешите отладку по USB в появившемся окне.", WARN)
        return r

    def shell(self, command: str, quiet: bool = True, timeout: float = 30) -> Result:
        """Команда с `|` и grep выполняется на устройстве, только если передать её одной строкой."""
        return self.adb(["shell", command], quiet=quiet, timeout=timeout)

    def logged(self, argv: list[str], timeout: float) -> Result:
        """Запускает внешнюю программу и пишет её вывод в журнал."""
        self.log(f"> {Path(argv[0]).name} {shlex.join(argv[1:])}", HEAD)
        r = self.run(argv, timeout)
        self.report(r, timeout)
        return r

    def devices(self) -> list[Device]:
        return parse_devices(self.adb(["devices", "-l"], device=False, quiet=True, timeout=15).out)

    def refresh_devices(self) -> list[Device]:
        """Обновляет список устройств и, если выбранного больше нет, берёт первое."""
        devs = self.devices()
        if devs and self.serial not in {d.serial for d in devs}:
            self.serial = devs[0].serial
        if self.on_devices:
            self.on_devices(devs)
        return devs


# ---------------------------------------------------------------- файлы на планшете

LS_RE = re.compile(r"^([dlcbps-])[rwxsStT-]{9}\S*\s+\d+\s+\S+\s+\S+\s+(\d+)\s+(\d{4}-\d\d-\d\d \d\d:\d\d)\s(.+)$")


@dataclass(frozen=True)
class RemoteEntry:
    name: str
    is_dir: bool
    is_link: bool
    size: int
    date: str


def parse_ls(text: str) -> list[RemoteEntry]:
    """Разбирает вывод `ls -la` (toybox) с планшета."""
    entries = []
    for line in text.splitlines():
        m = LS_RE.match(line)
        if not m:
            continue
        kind, size, date, name = m.groups()
        if kind == "l" and " -> " in name:
            name = name.split(" -> ", 1)[0]
        if name in (".", ".."):
            continue
        entries.append(RemoteEntry(name, kind == "d", kind == "l", int(size), date))
    entries.sort(key=lambda e: (not (e.is_dir or e.is_link), e.name.lower()))
    return entries


def remote_join(folder: str, name: str) -> str:
    return folder.rstrip("/") + "/" + name


def remote_parent(folder: str) -> str:
    folder = folder.rstrip("/")
    parent = folder.rsplit("/", 1)[0]
    return (parent or "") + "/"


# ---------------------------------------------------------------- установка приложений

OBB_RE = re.compile(r"^(?:main|patch)\.\d+\.(?P<pkg>[A-Za-z]\w*(?:\.\w+)+)\.obb$")


def file_name(path: str) -> str:
    # отрезаем и «/», и «\», чтобы одинаково разбирать пути Mac и Windows
    return re.split(r"[\\/]", path)[-1]


def obb_package(path: str) -> str | None:
    """Имя пакета из имени OBB: main.<версия>.<пакет>.obb или patch.<версия>.<пакет>.obb."""
    m = OBB_RE.match(file_name(path))
    return m.group("pkg") if m else None


@dataclass
class DropPlan:
    apks: list[str] = field(default_factory=list)
    obbs: list[str] = field(default_factory=list)
    aabs: list[str] = field(default_factory=list)
    ignored: list[str] = field(default_factory=list)
    error: str | None = None


def drop_plan(paths: list[str]) -> DropPlan:
    """Разбирает файлы по расширениям и решает, что с ними делать."""
    plan = DropPlan()
    for p in paths:
        ext = os.path.splitext(p)[1].lower()
        if os.path.isdir(p):
            plan.ignored.append(p)
        elif ext == ".apk":
            plan.apks.append(p)
        elif ext == ".obb":
            plan.obbs.append(p)
        elif ext == ".aab":
            plan.aabs.append(p)
        else:
            plan.ignored.append(p)
    if len(plan.aabs) > 1:
        plan.error = "Выбрано несколько AAB: ставьте их по одному."
    elif plan.aabs and plan.apks:
        plan.error = "AAB и APK вместе: непонятно, что ставить. Выберите что-то одно (OBB можно добавить к любому)."
    elif not (plan.aabs or plan.apks or plan.obbs):
        plan.error = "Среди файлов нет APK, OBB или AAB."
    return plan


def install_hint(text: str) -> str | None:
    if re.search(r"INSTALL_FAILED_UPDATE_INCOMPATIBLE|INSTALL_FAILED_SHARED_USER_INCOMPATIBLE", text):
        return "Подсказка: на планшете стоит это приложение с другой подписью. Удалите его (adb uninstall <пакет>) и повторите."
    if "INSTALL_FAILED_VERSION_DOWNGRADE" in text:
        return "Подсказка: на планшете версия новее. Удалите её (adb uninstall <пакет>) или ставьте более новую сборку."
    if "INSTALL_FAILED_INSUFFICIENT_STORAGE" in text:
        return "Подсказка: на планшете не хватает места."
    if re.search(r"INSTALL_FAILED_OLDER_SDK|INSTALL_FAILED_NO_MATCHING_ABIS|INSTALL_FAILED_DEPRECATED_SDK_VERSION", text):
        return "Подсказка: сборка не подходит этому планшету (версия Android или архитектура процессора)."
    if re.search(r"USER_RESTRICTED|canceled by user|INSTALL_FAILED_ABORTED", text):
        return ("Подсказка: установку блокирует планшет. Посмотрите на его экран и подтвердите установку; на Samsung "
                "отключите «Auto Blocker» в Настройки → Безопасность и конфиденциальность.")
    return None


def install_apk(a: Adb, path: str) -> bool:
    r = a.adb(["install", "-r", "-t", path], timeout=900)
    if r.ok and "Success" in r.out:
        a.log("APK установлен.", OK)
        return True
    a.log("Установить APK не удалось.", ERR)
    hint = install_hint(f"{r.out} {r.err}")
    if hint:
        a.log(hint, WARN)
    return False


def send_obb(a: Adb, files: list[str], fallback_package: str) -> bool:
    """Копирует OBB в /sdcard/Android/obb/<пакет>/. Пакет берётся из имени файла, иначе fallback_package."""
    for f in files:
        pkg = obb_package(f) or fallback_package
        folder = f"/sdcard/Android/obb/{pkg}"
        if not a.adb(["shell", "mkdir", "-p", folder]).ok:
            a.log("Не удалось создать папку для OBB на планшете.", ERR)
            return False
        if not a.adb(["push", f, f"{folder}/{file_name(f)}"], timeout=1800).ok:
            a.log(f"OBB не скопирован: {file_name(f)}", ERR)
            return False
    a.log("OBB скопирован.", OK)
    return True


def install_aab(a: Adb, aab: str, java: str, bundletool: str) -> bool:
    """AAB ставится через bundletool: он собирает из бандла APK под выбранное устройство и устанавливает их."""
    a.require()
    if not (Path.home() / ".android" / "debug.keystore").is_file():
        a.log("Не найден ~/.android/debug.keystore: bundletool не сможет подписать APK, и установка, скорее всего, "
              "не пройдёт.", WARN)
    if not server_up():
        start_server(a.path)
    apks = Path(tempfile.gettempdir()) / f"adbtoolkit_{uuid.uuid4().hex}.apks"
    common = [f"--device-id={a.serial}", f"--adb={a.path}"]
    try:
        r = a.logged([java, "-jar", bundletool, "build-apks", f"--bundle={aab}", f"--output={apks}",
                      "--connected-device", *common], 600)
        if not r.ok:
            a.log("Не удалось собрать APK из AAB.", ERR)
            return False
        r = a.logged([java, "-jar", bundletool, "install-apks", f"--apks={apks}", *common], 600)
        if r.ok:
            a.log("AAB установлен.", OK)
            return True
        a.log("Установить AAB не удалось.", ERR)
        hint = install_hint(f"{r.out} {r.err}")
        if hint:
            a.log(hint, WARN)
        return False
    finally:
        apks.unlink(missing_ok=True)


@dataclass
class InstallJob:
    """Всё, что нужно для установки, собранное заранее в окне (выбор файлов, пакет для OBB, bundletool)."""
    plan: DropPlan
    obb_fallback: str = ""
    java: str | None = None
    bundletool: str | None = None


def install_planned(a: Adb, job: InstallJob) -> None:
    """AAB или APK ставятся, OBB копируются после установки. Только OBB: приложение уже должно стоять на планшете."""
    plan = job.plan
    for f in plan.aabs:
        a.log(f"AAB: {file_name(f)}", HEAD)
        if not install_aab(a, f, job.java, job.bundletool):
            return
    for f in plan.apks:
        a.log(f"APK: {file_name(f)}", HEAD)
        if not install_apk(a, f):
            return
    if plan.obbs:
        if not (plan.aabs or plan.apks):
            a.log("APK не передан: копирую только OBB, приложение должно уже стоять на планшете.", WARN)
        send_obb(a, plan.obbs, job.obb_fallback)
