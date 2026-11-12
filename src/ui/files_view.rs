//! Раздел «Файлы»: папки планшета через `adb shell ls -la`, скачивание и отправка файлов.
//!
//! macOS не умеет MTP, поэтому это основной способ добраться до файлов планшета с Mac.
//! Все обращения к adb идут через общую очередь окна (`App::submit`).

use std::collections::BTreeSet;

use eframe::egui::{self, Align, Layout, Margin, RichText, Sense, vec2};
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;

use super::theme::{mono, palette, semibold};
use super::{After, App, widgets};
use crate::adb::Tag;
use crate::files::{Listing, RemoteEntry, START, breadcrumbs, human_size, remote_join, remote_parent};

pub struct FilesState {
    pub folder: String,
    pub path_edit: String,
    pub editing_path: bool,
    pub entries: Vec<RemoteEntry>,
    pub selected: BTreeSet<usize>,
    pub anchor: Option<usize>,
    pub loading: bool,
    pub loaded_serial: Option<String>,
    pub error: Option<String>,
}

impl Default for FilesState {
    fn default() -> Self {
        FilesState {
            folder: START.into(),
            path_edit: START.into(),
            editing_path: false,
            entries: vec![],
            selected: BTreeSet::new(),
            anchor: None,
            loading: false,
            loaded_serial: None,
            error: None,
        }
    }
}

impl FilesState {
    pub fn loaded(&mut self, result: Listing, log: &mut Vec<(String, Tag)>) {
        self.loading = false;
        match result {
            Ok((folder, entries)) => {
                self.folder = folder.clone();
                self.path_edit = folder;
                self.entries = entries;
                self.selected.clear();
                self.anchor = None;
                self.error = None;
            }
            Err((folder, error)) => {
                log.push((format!("{folder}: {error}"), Tag::Err));
                self.error = Some(error);
                self.path_edit = self.folder.clone();
            }
        }
    }
}

fn file_icon(e: &RemoteEntry) -> &'static str {
    if e.is_link {
        return icon::FOLDER_SIMPLE_DASHED;
    }
    if e.is_dir {
        return icon::FOLDER_SIMPLE;
    }
    let ext = e.name.rsplit_once('.').map(|(_, x)| x.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" => icon::IMAGE,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "3gp" => icon::FILM_STRIP,
        "mp3" | "m4a" | "ogg" | "wav" | "flac" | "aac" | "opus" => icon::MUSIC_NOTES,
        "apk" | "aab" | "obb" => icon::ANDROID_LOGO,
        "zip" | "rar" | "7z" | "tar" | "gz" => icon::FILE_ZIP,
        "pdf" => icon::FILE_PDF,
        "txt" | "log" | "md" | "json" | "xml" | "csv" => icon::FILE_TEXT,
        _ => icon::FILE,
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let busy = app.busy();
    let has_device = app.serial.is_some() && app.adb_path.is_some();
    let mut go: Option<String> = None;

    widgets::card(ui).inner_margin(Margin::same(0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // --- панель: назад, путь (крошки), обновить, действия
        egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 10, bottom: 10 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let can_up = !app.files.folder.trim_end_matches('/').is_empty();
                if widgets::icon_button(ui, !busy && has_device && can_up, icon::ARROW_UP, "Вверх").clicked() {
                    go = Some(remote_parent(&app.files.folder));
                }
                if widgets::icon_button(ui, !busy && has_device, icon::HOUSE, "Внутренняя память (/sdcard)").clicked()
                {
                    go = Some(START.into());
                }
                if widgets::icon_button(ui, !busy && has_device, icon::ARROWS_CLOCKWISE, "Обновить").clicked() {
                    go = Some(app.files.folder.clone());
                }
                ui.add_space(6.0);
                let actions_w = 300.0;
                let path_w = (ui.available_width() - actions_w).max(160.0);
                ui.allocate_ui_with_layout(vec2(path_w, 30.0), Layout::left_to_right(Align::Center), |ui| {
                    if app.files.editing_path {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut app.files.path_edit)
                                .font(mono(13.0))
                                .margin(Margin::symmetric(8, 5))
                                .desired_width(path_w - 8.0),
                        );
                        r.request_focus();
                        if r.lost_focus() {
                            app.files.editing_path = false;
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                let path = app.files.path_edit.trim();
                                go = Some(if path.is_empty() { START.into() } else { path.to_string() });
                            } else {
                                app.files.path_edit = app.files.folder.clone();
                            }
                        }
                    } else {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let crumbs = breadcrumbs(&app.files.folder);
                        let n = crumbs.len();
                        for (i, (name, path)) in crumbs.into_iter().enumerate() {
                            if i > 0 {
                                ui.label(RichText::new(icon::CARET_RIGHT).size(11.0).color(p.faint));
                            }
                            let last = i + 1 == n;
                            let label = if name == "/" { icon::HARD_DRIVE.to_string() } else { name };
                            let text = RichText::new(label).size(13.5).color(if last { p.text } else { p.muted });
                            let text = if last { text.font(semibold(13.5)) } else { text };
                            let b = egui::Button::new(text).frame_when_inactive(false).min_size(vec2(0.0, 26.0));
                            if ui.add_enabled(!busy && has_device, b).clicked() {
                                go = Some(path);
                            }
                        }
                        let edit = ui.add(
                            egui::Button::new(RichText::new(icon::PENCIL_SIMPLE).size(13.0).color(p.faint))
                                .frame_when_inactive(false),
                        );
                        if edit.on_hover_text("Ввести путь").clicked() {
                            app.files.editing_path = true;
                        }
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let n = app.files.selected.len();
                    if widgets::secondary(
                        ui,
                        !busy && has_device,
                        format!("{} Отправить сюда…", icon::UPLOAD_SIMPLE),
                        false,
                    )
                    .clicked()
                    {
                        let files = app.pick_files("Файлы для отправки на планшет", &[], true);
                        if !files.is_empty() {
                            let folder = app.files.folder.clone();
                            app.push_files(files, folder, After::ReloadFiles);
                        }
                    }
                    let label = if n > 0 {
                        format!("{} Скачать ({n})…", icon::DOWNLOAD_SIMPLE)
                    } else {
                        format!("{} Скачать…", icon::DOWNLOAD_SIMPLE)
                    };
                    if widgets::primary(ui, !busy && has_device && n > 0, label).clicked() {
                        download_selected(app);
                    }
                });
            });
        });
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, p.card_stroke);

        // --- таблица
        egui::Frame::new().inner_margin(Margin { left: 8, right: 8, top: 4, bottom: 8 }).show(ui, |ui| {
            if !has_device {
                empty(
                    ui,
                    icon::DEVICE_TABLET,
                    "Устройство не подключено",
                    "Подключите планшет и разрешите отладку по USB.",
                );
                return;
            }
            if app.files.loading && app.files.entries.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| ui.add(egui::Spinner::new().size(22.0)));
                ui.add_space(30.0);
                return;
            }
            if app.files.entries.is_empty() {
                let text = app.files.error.clone().unwrap_or_else(|| "Папка пуста.".into());
                empty(ui, icon::FOLDER_DASHED, "Здесь ничего нет", &text);
                return;
            }
            table(app, ui, &mut go);
        });
    });

    if let Some(folder) = go {
        app.files.editing_path = false;
        app.load_folder(folder);
    }
}

fn empty(ui: &mut egui::Ui, ic: &str, title: &str, text: &str) {
    let p = palette(ui);
    ui.add_space(26.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(ic).size(34.0).color(p.faint));
        ui.label(RichText::new(title).font(semibold(15.0)).color(p.text));
        ui.label(RichText::new(text).size(13.0).color(p.muted));
    });
    ui.add_space(26.0);
}

fn table(app: &mut App, ui: &mut egui::Ui, go: &mut Option<String>) {
    let p = palette(ui);
    let row_h = 30.0;
    let height = (app.files.entries.len() as f32 * row_h + 34.0).min(420.0);
    let mods = ui.input(|i| i.modifiers);
    let mut click: Option<usize> = None;
    let mut open: Option<usize> = None;
    let busy = app.busy();
    TableBuilder::new(ui)
        .striped(false)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::remainder().at_least(200.0).clip(true))
        .column(Column::exact(110.0))
        .column(Column::exact(150.0))
        .min_scrolled_height(height)
        .max_scroll_height(height)
        .header(28.0, |mut h| {
            for (title, right) in [("Имя", false), ("Размер", true), ("Изменён", false)] {
                h.col(|ui| {
                    let t = RichText::new(title).size(12.0).font(semibold(12.0)).color(p.faint);
                    if right {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(12.0);
                            ui.label(t)
                        });
                    } else {
                        ui.add_space(8.0);
                        ui.label(t);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, app.files.entries.len(), |mut row| {
                let i = row.index();
                let e = &app.files.entries[i];
                row.set_selected(app.files.selected.contains(&i));
                row.col(|ui| {
                    ui.add_space(8.0);
                    let color = if e.is_folder() { p.accent } else { p.muted };
                    ui.label(RichText::new(file_icon(e)).size(17.0).color(color));
                    ui.add(
                        egui::Label::new(RichText::new(&e.name).size(13.5).color(p.text)).truncate().selectable(false),
                    );
                });
                row.col(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        let size = if e.is_folder() { "—".to_string() } else { human_size(e.size) };
                        ui.label(RichText::new(size).size(13.0).color(p.muted));
                    });
                });
                row.col(|ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&e.date).size(13.0).color(p.muted));
                });
                let r = row.response();
                if r.double_clicked() {
                    open = Some(i);
                } else if r.clicked() {
                    click = Some(i);
                }
            });
        });

    if let Some(i) = click {
        let f = &mut app.files;
        if mods.shift {
            let a = f.anchor.unwrap_or(i);
            f.selected = (a.min(i)..=a.max(i)).collect();
        } else if mods.command {
            if !f.selected.remove(&i) {
                f.selected.insert(i);
            }
            f.anchor = Some(i);
        } else {
            f.selected = [i].into();
            f.anchor = Some(i);
        }
    }
    if let Some(i) = open {
        let e = &app.files.entries[i];
        if e.is_folder() && !busy {
            *go = Some(remote_join(&app.files.folder, &e.name));
        }
    }
}

fn download_selected(app: &mut App) {
    let remotes: Vec<String> = app
        .files
        .selected
        .iter()
        .filter_map(|i| app.files.entries.get(*i))
        .map(|e| remote_join(&app.files.folder, &e.name))
        .collect();
    if remotes.is_empty() {
        app.log("Выберите файлы или папки.", Tag::Warn);
        return;
    }
    if let Some(local) = app.pick_folder("Куда сохранить") {
        app.pull_paths(remotes, local);
    }
}
