"""Окно «Файлы планшета»: папки устройства через `adb shell ls`, скачивание и отправка файлов.

macOS не умеет MTP, поэтому это основной способ добраться до файлов планшета с Mac.
Все обращения к adb идут через очередь команд главного окна (App.submit).
"""
from __future__ import annotations

import shlex
import tkinter as tk
from tkinter import ttk
from typing import TYPE_CHECKING

from .core import ERR, HEAD, OK, Adb, RemoteEntry, file_name, parse_ls, remote_join, remote_parent

if TYPE_CHECKING:
    from .gui import App

START = "/sdcard/"


def human_size(size: int) -> str:
    value = float(size)
    for unit in ("Б", "КБ", "МБ", "ГБ"):
        if value < 1024 or unit == "ГБ":
            return f"{value:.0f} {unit}" if unit == "Б" else f"{value:.1f} {unit}"
        value /= 1024
    return str(size)


def list_folder(a: Adb, folder: str) -> tuple[str, list[RemoteEntry] | None, str]:
    # Путь со «/» в конце, чтобы ls зашёл внутрь ссылок вроде /sdcard.
    folder = folder.rstrip("/") + "/"
    r = a.shell(f"ls -la {shlex.quote(folder)}", timeout=20)
    entries = parse_ls(r.out)
    if not entries and (r.code != 0 or r.err):
        return folder, None, (r.err or r.out or "не удалось прочитать папку").strip()
    return folder, entries, ""


class DeviceBrowser(tk.Toplevel):
    def __init__(self, app: App) -> None:
        super().__init__(app.root)
        self.app = app
        self.title("Файлы планшета")
        self.geometry("760x560")
        self.minsize(520, 320)
        self.folder = START
        self.entries: dict[str, RemoteEntry] = {}

        bar = ttk.Frame(self, padding=(10, 8, 10, 4))
        bar.pack(side="top", fill="x")
        self.up_btn = ttk.Button(bar, text="↑ Вверх", command=self.go_up)
        self.up_btn.pack(side="left")
        self.path_var = tk.StringVar(value=self.folder)
        path = ttk.Entry(bar, textvariable=self.path_var)
        path.pack(side="left", fill="x", expand=True, padx=6)
        path.bind("<Return>", lambda e: self.load(self.path_var.get().strip() or START))
        self.reload_btn = ttk.Button(bar, text="Обновить", command=lambda: self.load(self.folder))
        self.reload_btn.pack(side="left")

        actions = ttk.Frame(self, padding=(10, 4, 10, 10))
        actions.pack(side="bottom", fill="x")
        self.pull_btn = ttk.Button(actions, text="Скачать выбранное…", command=self.pull)
        self.pull_btn.pack(side="left")
        self.push_btn = ttk.Button(actions, text="Отправить сюда…", command=self.push)
        self.push_btn.pack(side="left", padx=(6, 0))
        self.status = ttk.Label(actions, text="", foreground="#787878")
        self.status.pack(side="left", padx=(12, 0))

        frame = ttk.Frame(self, padding=(10, 0))
        frame.pack(side="top", fill="both", expand=True)
        self.tree = ttk.Treeview(frame, columns=("size", "date"), selectmode="extended")
        self.tree.heading("#0", text="Имя", anchor="w")
        self.tree.heading("size", text="Размер", anchor="e")
        self.tree.heading("date", text="Изменён", anchor="w")
        self.tree.column("#0", width=380, stretch=True)
        self.tree.column("size", width=100, anchor="e", stretch=False)
        self.tree.column("date", width=150, stretch=False)
        scroll = ttk.Scrollbar(frame, orient="vertical", command=self.tree.yview)
        self.tree.configure(yscrollcommand=scroll.set)
        scroll.pack(side="right", fill="y")
        self.tree.pack(side="left", fill="both", expand=True)
        self.tree.bind("<Double-1>", self._on_open)
        self.tree.bind("<Return>", self._on_open)

        self.set_busy(app.busy)
        self.load(self.folder)

    def set_busy(self, busy: bool) -> None:
        state = "disabled" if busy else "normal"
        for b in (self.up_btn, self.reload_btn, self.pull_btn, self.push_btn):
            b.configure(state=state)

    # ---------------------------------------------------------------- навигация

    def load(self, folder: str) -> None:
        if not self.app.require_device():
            return
        self.status.configure(text="Читаю папку…")
        self.app.submit(lambda a: list_folder(a, folder), self._show, blank_line=False)

    def _show(self, result: tuple[str, list[RemoteEntry] | None, str] | None) -> None:
        if not self.winfo_exists():
            return
        if result is None:
            self.status.configure(text="")
            return
        folder, entries, error = result
        if entries is None:
            self.status.configure(text="")
            self.app.log(f"{folder}: {error}", ERR)
            self.path_var.set(self.folder)
            return
        self.folder = folder
        self.path_var.set(folder)
        self.tree.delete(*self.tree.get_children())
        self.entries = {}
        for e in entries:
            icon = "📁 " if e.is_dir or e.is_link else "📄 "
            size = "" if e.is_dir or e.is_link else human_size(e.size)
            iid = self.tree.insert("", "end", text=icon + e.name, values=(size, e.date))
            self.entries[iid] = e
        self.status.configure(text=f"{len(entries)} элементов")

    def _on_open(self, _: tk.Event) -> None:
        sel = self.tree.selection()
        if len(sel) == 1:
            e = self.entries.get(sel[0])
            if e and (e.is_dir or e.is_link):
                self.load(remote_join(self.folder, e.name))

    def go_up(self) -> None:
        if self.folder.rstrip("/"):
            self.load(remote_parent(self.folder))

    # ---------------------------------------------------------------- копирование

    def pull(self) -> None:
        selected = [self.entries[i] for i in self.tree.selection() if i in self.entries]
        if not selected:
            self.app.log("Выберите файлы или папки в окне «Файлы планшета».", ERR)
            return
        local = self.app.ask_directory("Куда сохранить")
        if not local:
            return
        remotes = [remote_join(self.folder, e.name) for e in selected]
        self.app.log("=== Скачать с планшета ===", HEAD)

        def run(a: Adb) -> None:
            for r in remotes:
                if a.adb(["pull", r, local], timeout=1800).ok:
                    a.log(f"Скачано: {file_name(r)}", OK)
        self.app.submit(run)

    def push(self) -> None:
        files = self.app.ask_files("Файлы для отправки на планшет", [], multiple=True)
        if not files:
            return
        folder = self.folder
        self.app.log("=== Отправить на планшет ===", HEAD)

        def run(a: Adb) -> None:
            for f in files:
                if a.adb(["push", f, folder], timeout=1800).ok:
                    a.log(f"Отправлено: {file_name(f)}", OK)
        self.app.submit(run, lambda _: self.winfo_exists() and self.load(self.folder))
