"""Окно ADB Toolkit на Tkinter.

Команды выполняются в рабочем потоке: журнал и список устройств приходят в окно через очередь,
которую окно разбирает раз в 50 мс (Tk нельзя трогать из других потоков).
"""
from __future__ import annotations

import queue
import shlex
import threading
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, font as tkfont, messagebox, simpledialog, ttk
from typing import Any, Callable

from . import commands
from .core import (
    DIM, ERR, HEAD, IS_MAC, IS_WIN, OK, TEXT, WARN, Abort, Adb, Cancelled, Device, find_adb, mark_server_down,
    state_text,
)
from .filebrowser import DeviceBrowser

try:
    from tkinterdnd2 import DND_FILES, TkinterDnD
except ImportError:  # без tkinterdnd2 область установки работает только по щелчку
    TkinterDnD = None

LOG_COLORS = {TEXT: "#dcdcdc", HEAD: "#6ec8ff", ERR: "#ff7878", WARN: "#ffc85a", OK: "#82dc82", DIM: "#969696"}
# на светлом фоне окна берём более тёмные оттенки
STATE_COLORS = {OK: "#148228", WARN: "#be6e00", ERR: "#be1e1e"}
GROUP_BG, GROUP_FG = "#e4ecf7", "#1e3c6e"
ZONE_BG, ZONE_HOT = "#f4f8fd", "#cde4ff"

NO_DEVICE_COMMANDS = {"devices", "kill-server", "start-server", "version", "help", "connect", "disconnect", "pair",
                      "mdns"}


class App:
    def __init__(self) -> None:
        self.root = TkinterDnD.Tk() if TkinterDnD else tk.Tk()
        self.root.title("ADB Toolkit")
        self.root.geometry("1180x780")
        self.root.minsize(900, 520)

        self.queue: queue.Queue = queue.Queue()
        self.adb = Adb(self.log)
        self.adb.on_devices = lambda devs: self.queue.put(("devices", devs, True))
        self.poller = Adb(self.log)  # отдельный экземпляр для фонового опроса устройств, без журнала
        self.job_lock = threading.Lock()
        self.adb_path: str | None = None
        self.serial: str | None = None
        self.devices: list[Device] = []
        self.dev_sig: str | None = None
        self.busy = False
        self.polling = False
        self.bundletool: str | None = None
        self.browser: DeviceBrowser | None = None
        self.command_buttons: list[ttk.Button] = []

        self.mono = tkfont.nametofont("TkFixedFont").copy()
        self.mono.configure(size=12 if IS_MAC else 10)
        self.mono_small = self.mono.copy()
        self.mono_small.configure(size=10 if IS_MAC else 8)
        self.bold = tkfont.nametofont("TkDefaultFont").copy()
        self.bold.configure(weight="bold")

        self._build_top()
        self._build_bottom()
        self._build_middle()

        self.root.after(50, self._pump)
        self.root.after(100, self._start)
        self.root.protocol("WM_DELETE_WINDOW", self._close)

    # ---------------------------------------------------------------- построение окна

    def _build_top(self) -> None:
        top = ttk.Frame(self.root, padding=(10, 8, 10, 4))
        top.pack(side="top", fill="x")
        top.columnconfigure(1, weight=1)

        ttk.Label(top, text="adb:").grid(row=0, column=0, sticky="w", padx=(0, 6))
        self.adb_var = tk.StringVar()
        entry = ttk.Entry(top, textvariable=self.adb_var)
        entry.grid(row=0, column=1, columnspan=4, sticky="ew")
        entry.bind("<Return>", lambda e: self._set_adb_path(self.adb_var.get().strip()))
        entry.bind("<FocusOut>", lambda e: self._set_adb_path(self.adb_var.get().strip()))
        ttk.Button(top, text="Обзор…", command=self._browse_adb).grid(row=0, column=5, padx=(6, 0))

        ttk.Label(top, text="Устройство:").grid(row=1, column=0, sticky="w", padx=(0, 6), pady=(8, 0))
        row = ttk.Frame(top)
        row.grid(row=1, column=1, columnspan=5, sticky="ew", pady=(8, 0))
        self.dev_combo = ttk.Combobox(row, state="readonly", width=48)
        self.dev_combo.pack(side="left")
        self.dev_combo.bind("<<ComboboxSelected>>", self._on_device_selected)
        self.refresh_btn = ttk.Button(row, text="Обновить", command=lambda: self.poll(force=True))
        self.refresh_btn.pack(side="left", padx=(6, 0))
        self.auto_var = tk.BooleanVar(value=True)
        ttk.Checkbutton(row, text="Автообновление", variable=self.auto_var).pack(side="left", padx=(10, 0))
        self.state_label = tk.Label(row, text="", font=self.bold, anchor="w")
        self.state_label.pack(side="left", padx=(12, 0), fill="x", expand=True)

    def _build_bottom(self) -> None:
        bottom = ttk.Frame(self.root, padding=(10, 4, 10, 10))
        bottom.pack(side="bottom", fill="x")
        ttk.Label(bottom, text="adb", font=self.mono).pack(side="left", padx=(0, 6))
        self.custom_var = tk.StringVar()
        entry = ttk.Entry(bottom, textvariable=self.custom_var, font=self.mono)
        entry.pack(side="left", fill="x", expand=True)
        entry.bind("<Return>", lambda e: self._run_custom())
        self.custom_btn = ttk.Button(bottom, text="Выполнить", command=self._run_custom)
        self.custom_btn.pack(side="left", padx=(6, 0))
        self.cancel_btn = ttk.Button(bottom, text="Прервать", command=self._cancel, state="disabled")
        self.cancel_btn.pack(side="left", padx=(6, 0))

    def _build_middle(self) -> None:
        paned = ttk.PanedWindow(self.root, orient="horizontal")
        paned.pack(side="top", fill="both", expand=True, padx=10)

        # --- слева: команды в прокручиваемой области
        left = ttk.Frame(paned)
        canvas = tk.Canvas(left, highlightthickness=0, borderwidth=0)
        scroll = ttk.Scrollbar(left, orient="vertical", command=canvas.yview)
        canvas.configure(yscrollcommand=scroll.set)
        scroll.pack(side="right", fill="y")
        canvas.pack(side="left", fill="both", expand=True)
        inner = ttk.Frame(canvas, padding=(0, 0, 8, 8))
        window = canvas.create_window((0, 0), window=inner, anchor="nw")
        canvas.bind("<Configure>", lambda e: canvas.itemconfigure(window, width=e.width))
        self._bind_wheel(canvas)
        paned.add(left, weight=1)

        inner.columnconfigure(1, weight=1)
        wrapped: list[tk.Widget] = []
        row = 0
        last_group = None
        for c in commands.visible_commands():
            if c.group != last_group:
                last_group = c.group
                tk.Label(inner, text="  " + c.group, font=self.bold, bg=GROUP_BG, fg=GROUP_FG, anchor="w",
                         pady=4).grid(row=row, column=0, columnspan=2, sticky="ew", pady=(10 if row else 4, 6))
                row += 1
                if c.group == commands.GROUP_INSTALL:
                    self.zone = self._build_drop_zone(inner)
                    self.zone.grid(row=row, column=0, columnspan=2, sticky="ew", pady=(0, 8))
                    wrapped.append(self.zone)
                    row += 1
            b = ttk.Button(inner, text=c.title, width=24, command=lambda c=c: self.on_command(c))
            b.grid(row=row, column=0, rowspan=2, sticky="nw", pady=(0, 10))
            self.command_buttons.append(b)
            desc = ttk.Label(inner, text=c.desc, justify="left")
            desc.grid(row=row, column=1, sticky="ew", padx=(10, 0))
            cmd = ttk.Label(inner, text=c.cmd, font=self.mono_small, foreground="#787878")
            cmd.grid(row=row + 1, column=1, sticky="nw", padx=(10, 0), pady=(0, 10))
            wrapped += [desc, cmd]
            row += 2

        def rewrap(event: tk.Event) -> None:
            # описание стоит справа от кнопки: переносим по оставшейся ширине
            width = max(event.width - self.command_buttons[0].winfo_width() - 30, 160)
            for w in wrapped:
                w.configure(wraplength=width if w is not self.zone else event.width - 20)
        inner.bind("<Configure>", lambda e: (canvas.configure(scrollregion=canvas.bbox("all")), rewrap(e)))

        # --- справа: журнал
        right = ttk.Frame(paned)
        head = ttk.Frame(right)
        head.pack(side="top", fill="x", pady=(0, 4))
        ttk.Button(head, text="Очистить", command=self._clear_log).pack(side="left")
        ttk.Button(head, text="Копировать всё", command=self._copy_log).pack(side="left", padx=(6, 0))
        frame = ttk.Frame(right)
        frame.pack(side="top", fill="both", expand=True)
        self.out = tk.Text(frame, wrap="word", bg="#1e1e1e", fg=LOG_COLORS[TEXT], insertbackground="#dcdcdc",
                           font=self.mono, borderwidth=0, highlightthickness=0, padx=8, pady=6, state="disabled")
        out_scroll = ttk.Scrollbar(frame, orient="vertical", command=self.out.yview)
        self.out.configure(yscrollcommand=out_scroll.set)
        out_scroll.pack(side="right", fill="y")
        self.out.pack(side="left", fill="both", expand=True)
        for tag, color in LOG_COLORS.items():
            self.out.tag_configure(tag, foreground=color)
        paned.add(right, weight=1)
        # командам нужно больше места, чем журналу
        self.root.after(200, lambda: paned.sashpos(0, int(paned.winfo_width() * 0.55)))

    def _build_drop_zone(self, parent: tk.Widget) -> tk.Label:
        text = "Перетащите сюда APK, OBB или AAB: утилита сама поймёт, как их установить\n(или нажмите и выберите файлы)"
        if not TkinterDnD:
            text = "Нажмите и выберите APK, OBB или AAB: утилита сама поймёт, как их установить\n" \
                   "(для перетаскивания установите пакет tkinterdnd2)"
        zone = tk.Label(parent, text=text, bg=ZONE_BG, fg=GROUP_FG, relief="solid", borderwidth=1, height=4,
                        cursor="hand2", justify="center")
        zone.bind("<Button-1>", lambda e: self._pick_install_files())
        if TkinterDnD:
            zone.drop_target_register(DND_FILES)
            zone.dnd_bind("<<DropEnter>>", lambda e: (zone.configure(bg=ZONE_HOT), e.action)[1])
            zone.dnd_bind("<<DropLeave>>", lambda e: (zone.configure(bg=ZONE_BG), e.action)[1])
            zone.dnd_bind("<<Drop>>", self._on_drop)
        return zone

    def _bind_wheel(self, canvas: tk.Canvas) -> None:
        def wheel(event: tk.Event) -> None:
            if event.num == 4:
                step = -1
            elif event.num == 5:
                step = 1
            elif IS_MAC:
                step = -event.delta
            else:
                step = -int(event.delta / 120)
            canvas.yview_scroll(step, "units")

        def enter(_: tk.Event) -> None:
            canvas.bind_all("<MouseWheel>", wheel)
            canvas.bind_all("<Button-4>", wheel)
            canvas.bind_all("<Button-5>", wheel)

        def leave(_: tk.Event) -> None:
            for seq in ("<MouseWheel>", "<Button-4>", "<Button-5>"):
                canvas.unbind_all(seq)

        canvas.bind("<Enter>", enter)
        canvas.bind("<Leave>", leave)

    # ---------------------------------------------------------------- журнал и очередь

    def log(self, text: str, tag: str = TEXT) -> None:
        """Можно вызывать из любого потока."""
        self.queue.put(("log", text, tag))

    def _write(self, text: str, tag: str) -> None:
        self.out.configure(state="normal")
        self.out.insert("end", text + "\n", tag)
        self.out.configure(state="disabled")
        self.out.see("end")

    def _pump(self) -> None:
        try:
            while True:
                msg = self.queue.get_nowait()
                kind = msg[0]
                if kind == "log":
                    self._write(msg[1], msg[2])
                elif kind == "devices":
                    self._show_devices(msg[1], msg[2])
                elif kind == "polled":
                    self.polling = False
                    if msg[1] is not None:
                        self._show_devices(msg[1], msg[2])
                elif kind == "done":
                    self._job_done(msg[1], msg[2], msg[3])
        except queue.Empty:
            pass
        self.root.after(50, self._pump)

    def _clear_log(self) -> None:
        self.out.configure(state="normal")
        self.out.delete("1.0", "end")
        self.out.configure(state="disabled")

    def _copy_log(self) -> None:
        text = self.out.get("1.0", "end-1c")
        if text:
            self.root.clipboard_clear()
            self.root.clipboard_append(text)

    # ---------------------------------------------------------------- устройства

    def _start(self) -> None:
        path = find_adb()
        if path:
            self.adb_var.set(path)
            self._set_adb_path(path)
        else:
            self.log("adb не найден. Установите его (brew install android-platform-tools) или укажите путь сверху.",
                     ERR)
            self._update_state()
        self.log("Выберите устройство и нажмите нужную кнопку. Начните с «Авто-диагностики».", DIM)
        self.log("")
        self.root.after(3000, self._tick)

    def _tick(self) -> None:
        if self.auto_var.get():
            self.poll()
        self.root.after(3000, self._tick)

    def _set_adb_path(self, path: str) -> None:
        if path == (self.adb_path or ""):
            return
        if path and not Path(path).is_file():
            self.log(f"Файл не найден: {path}", ERR)
            return
        self.adb_path = path or None
        self.poller.path = self.adb_path
        mark_server_down()
        self.dev_sig = None
        self.poll(force=True)

    def _browse_adb(self) -> None:
        path = filedialog.askopenfilename(parent=self.root, title="Укажите adb",
                                          filetypes=[("adb", "adb.exe" if IS_WIN else "adb"), ("Все файлы", "*")])
        if path:
            self.adb_var.set(path)
            self._set_adb_path(path)

    def poll(self, force: bool = False) -> None:
        """Опрос `adb devices` в фоне. Пока выполняется команда, опрос пропускается."""
        if not self.adb_path:
            self._update_state()
            return
        if self.polling or self.busy:
            return
        self.polling = True

        def work() -> None:
            devs = None
            if self.job_lock.acquire(blocking=False):
                try:
                    devs = self.poller.devices()
                except Exception:
                    devs = None
                finally:
                    self.job_lock.release()
            self.queue.put(("polled", devs, force))
        threading.Thread(target=work, daemon=True).start()

    def _show_devices(self, devs: list[Device], force: bool) -> None:
        sig = ";".join(f"{d.serial}|{d.state}" for d in devs)
        if not force and sig == self.dev_sig:
            return
        self.dev_sig = sig
        self.devices = devs
        self.dev_combo["values"] = [f"{d.model or 'устройство'}  ({d.serial})  [{d.state}]" for d in devs]
        idx = next((i for i, d in enumerate(devs) if d.serial == self.serial), 0 if devs else -1)
        if idx >= 0:
            self.dev_combo.current(idx)
            self.serial = devs[idx].serial
        else:
            self.dev_combo.set("")
            self.serial = None
        self._update_state()

    def _on_device_selected(self, _: tk.Event) -> None:
        i = self.dev_combo.current()
        if 0 <= i < len(self.devices):
            self.serial = self.devices[i].serial
        self._update_state()

    def _update_state(self) -> None:
        dev = next((d for d in self.devices if d.serial == self.serial), None)
        if not self.adb_path:
            text, tag = "adb не найден: укажите путь", ERR
        elif not dev:
            text, tag = "ADB не видит устройств", ERR
        else:
            text, tag = state_text(dev.state)
        self.state_label.configure(text=text, fg=STATE_COLORS.get(tag, STATE_COLORS[WARN]))

    # ---------------------------------------------------------------- выполнение команд

    def submit(self, fn: Callable[[Adb], Any], on_done: Callable[[Any], None] | None = None,
               blank_line: bool = True) -> bool:
        """Запускает fn(adb) в рабочем потоке. on_done(результат) вызывается в окне."""
        if self.busy:
            self.log("Дождитесь окончания текущей операции.", WARN)
            return False
        self.adb.path = self.adb_path
        self.adb.serial = self.serial
        self.adb.reset_cancel()
        self._set_busy(True)

        def work() -> None:
            result = None
            with self.job_lock:
                try:
                    result = fn(self.adb)
                except Cancelled:
                    self.log("Прервано.", WARN)
                except Abort:
                    pass
                except Exception as e:  # ошибка одной команды не должна ронять окно
                    self.log(f"Ошибка: {e}", ERR)
            self.queue.put(("done", on_done, result, blank_line))
        threading.Thread(target=work, daemon=True).start()
        return True

    def _job_done(self, on_done: Callable[[Any], None] | None, result: Any, blank_line: bool) -> None:
        if blank_line:
            self._write("", TEXT)
        self._set_busy(False)
        if self.adb.serial and self.adb.serial != self.serial:
            self.serial = self.adb.serial
            self._update_state()
        if on_done:
            on_done(result)
        self.poll()

    def _set_busy(self, value: bool) -> None:
        self.busy = value
        state = "disabled" if value else "normal"
        for b in self.command_buttons:
            b.configure(state=state)
        self.custom_btn.configure(state=state)
        self.refresh_btn.configure(state=state)
        self.cancel_btn.configure(state="normal" if value else "disabled")
        self.root.configure(cursor="watch" if value else "")
        if self.browser and self.browser.winfo_exists():
            self.browser.set_busy(value)

    def _cancel(self) -> None:
        if self.busy:
            self.adb.cancel()

    def require_device(self) -> bool:
        if not self.adb_path:
            self.log("adb не найден. Укажите путь к нему сверху.", ERR)
            return False
        if not self.serial:
            self.log("Сначала выберите устройство в списке сверху.", WARN)
            return False
        return True

    def on_command(self, c: commands.Command) -> None:
        if self.busy:
            return
        if c.confirm and not messagebox.askyesno(c.title, c.confirm, icon="warning", parent=self.root):
            return
        self.log(f"=== {c.title} ===", HEAD)
        if c.needs_device and not self.require_device():
            self.log("")
            return
        if c.action:
            c.action(self)
            self.log("")
            return
        args = None
        if c.prepare:
            args = c.prepare(self)
            if args is None:
                self.log("")
                return
        self.submit(lambda a: c.run(a, args))

    def _run_custom(self) -> None:
        line = self.custom_var.get().strip()
        if not line or self.busy:
            return
        try:
            tokens = shlex.split(line)
        except ValueError as e:
            self.log(f"Не разобрать команду: {e}", ERR)
            return
        if tokens and tokens[0] == "adb":
            tokens = tokens[1:]
        if not tokens:
            return
        device = tokens[0] not in NO_DEVICE_COMMANDS
        self.submit(lambda a: a.adb(tokens, device=device))

    # ---------------------------------------------------------------- установка перетаскиванием

    def _on_drop(self, event: Any) -> Any:
        self.zone.configure(bg=ZONE_BG)
        paths = list(self.root.tk.splitlist(event.data))
        # запускаем после выхода из обработчика, чтобы Finder не ждал окончания перетаскивания
        self.root.after(50, lambda: self.install(paths))
        return event.action

    def _pick_install_files(self) -> None:
        if self.busy:
            return
        files = self.ask_files("APK, OBB или AAB", commands.ANY_PACKAGE, multiple=True)
        if files:
            self.install(files)

    def install(self, paths: list[str]) -> None:
        if self.busy:
            self.log("Дождитесь окончания текущей операции.", WARN)
            return
        self.log("=== Установка файлов ===", HEAD)
        if not self.require_device():
            self.log("")
            return
        job = commands.prepare_install(self, paths)
        if job is None:
            self.log("")
            return
        self.submit(lambda a: commands.install_planned(a, job))

    # ---------------------------------------------------------------- диалоги для команд (commands.Ui)

    def ask_string(self, title: str, prompt: str, initial: str = "") -> str | None:
        return simpledialog.askstring(title, prompt, initialvalue=initial, parent=self.root)

    def ask_files(self, title: str, filetypes: list[tuple[str, str]], multiple: bool = False) -> list[str]:
        types = list(filetypes) + [("Все файлы", "*")]
        if multiple:
            files = list(filedialog.askopenfilenames(parent=self.root, title=title, filetypes=types))
        else:
            one = filedialog.askopenfilename(parent=self.root, title=title, filetypes=types)
            files = [one] if one else []
        if not files:
            self.log("Отменено.", DIM)
        return files

    def ask_directory(self, title: str) -> str | None:
        path = filedialog.askdirectory(parent=self.root, title=title, mustexist=True)
        if not path:
            self.log("Отменено.", DIM)
        return path or None

    def open_file_browser(self) -> None:
        if self.browser and self.browser.winfo_exists():
            self.browser.lift()
            return
        self.browser = DeviceBrowser(self)

    # ----------------------------------------------------------------

    def _close(self) -> None:
        if self.busy:
            self.adb.cancel()
        self.root.destroy()

    def run(self) -> None:
        self.root.mainloop()


def main() -> None:
    App().run()
