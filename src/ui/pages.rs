//! Содержимое разделов: списки команд, диагностика, установка и настройки.

use eframe::egui::{self, Align, CornerRadius, Layout, Margin, RichText, Sense, Stroke, ThemePreference, vec2};
use egui_phosphor::regular as icon;

use super::theme::{Palette, mono, palette, semibold};
use super::widgets::{self, command_row, full_card, separator};
use super::{App, files_view};
use crate::adb::{Step, Tag};
use crate::commands::{self, ANY_PACKAGE, Command, Group};
use crate::tools::{IS_MAC, IS_WIN, find_adb, find_bundletool, find_java};

pub fn group(app: &mut App, ui: &mut egui::Ui, g: Group) {
    match g {
        Group::Diagnose => diagnose(app, ui),
        Group::Install => {
            drop_zone(app, ui);
            ui.add_space(14.0);
            command_list(app, ui, g);
        }
        Group::Files => {
            files_view::show(app, ui);
            ui.add_space(14.0);
            command_list(app, ui, g);
        }
        _ => command_list(app, ui, g),
    }
}

fn command_list(app: &mut App, ui: &mut egui::Ui, g: Group) {
    let p = palette(ui);
    let list: Vec<&'static Command> = commands::visible(g).collect();
    let mut clicked = None;
    full_card(ui, |ui| {
        for (i, c) in list.iter().enumerate() {
            if i > 0 {
                separator(ui);
            }
            let button = if c.title.ends_with('…') { "Выбрать…" } else { "Выполнить" };
            if command_row(ui, p, c.icon, c.title.trim_end_matches('…'), c.desc, c.cmd, button, !app.busy(), c.danger)
            {
                clicked = Some(*c);
            }
        }
    });
    if let Some(c) = clicked {
        let c: &'static Command = commands::by_title(c.title).unwrap();
        app.on_command(c, false);
    }
}

// ---------------------------------------------------------------- диагностика

const STEPS: [(Step, &str, &str, &str); 4] = [
    (
        Step::Adb,
        icon::PLUGS_CONNECTED,
        "ADB видит устройство",
        "Кабель с передачей данных, отладка по USB и разрешение этого компьютера",
    ),
    (Step::Screen, icon::LOCK_KEY_OPEN, "Экран разблокирован", "Пока планшет заблокирован, Android не отдаёт файлы"),
    (
        Step::Usb,
        icon::USB,
        "Режим передачи файлов",
        "В sys.usb.config должен быть mtp; если нет — утилита включит его сама",
    ),
    (Step::Host, icon::LAPTOP, "Компьютер видит планшет", "Что видит система по USB"),
];

fn host_step_desc() -> &'static str {
    if IS_WIN {
        "Проводник видит MTP-устройство и его содержимое"
    } else if IS_MAC {
        "Mac видит планшет по USB (system_profiler)"
    } else {
        "Что видит система по USB"
    }
}

fn diagnose(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let c = commands::by_title("Авто-диагностика").unwrap();
    let busy = app.busy();
    let running_diag = app.running.as_ref().is_some_and(|r| r.title == c.title);

    // главная карточка
    let hero = egui::Frame::new()
        .fill(p.accent_soft)
        .stroke(Stroke::new(1.0, p.accent.gamma_multiply(0.25)))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(24));
    let mut start = false;
    hero.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::icon_tile(ui, icon::STETHOSCOPE, p.accent_text, p.accent, 56.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new("Авто-диагностика").font(semibold(20.0)).color(p.text));
                ui.label(RichText::new(c.desc).color(p.muted));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let text = if running_diag {
                        format!("{}  Проверяю…", icon::HOURGLASS_MEDIUM)
                    } else {
                        format!("{}  Запустить проверку", icon::PLAY)
                    };
                    if widgets::primary(ui, !busy, text).clicked() {
                        start = true;
                    }
                    if running_diag {
                        ui.add(egui::Spinner::new().size(18.0).color(p.accent));
                    }
                });
            });
        });
    });
    if start {
        app.on_command(c, false);
    }

    // чек-лист
    ui.add_space(14.0);
    full_card(ui, |ui| {
        ui.label(RichText::new("Что проверяется").font(semibold(15.0)).color(p.text));
        ui.add_space(6.0);
        for (i, (step, step_icon, title, desc)) in STEPS.iter().enumerate() {
            if i > 0 {
                separator(ui);
            }
            let results = &app.checks[step.index()];
            let desc = if *step == Step::Host { host_step_desc() } else { desc };
            step_row(ui, p, step_icon, title, desc, results, running_diag, app.diag_ran);
        }
    });

    // советы
    ui.add_space(14.0);
    full_card(ui, |ui| {
        ui.label(RichText::new("Частые причины").font(semibold(15.0)).color(p.text));
        ui.add_space(6.0);
        let tips = [
            (
                icon::HAND_TAP,
                "Статус unauthorized",
                "Планшет не дал разрешение: разблокируйте экран и нажмите «Разрешить» в окне отладки по USB.",
            ),
            (
                icon::USB,
                "Файлов не видно, хотя выбрана «Передача файлов»",
                "Режим мог остаться sec_charging,adb. Включите MTP в разделе «Режим USB» — помогает svc usb setFunctions mtp.",
            ),
            (
                icon::LOCK,
                "Устройство видно, но пустое",
                "Пока планшет заблокирован, Android не отдаёт файлы по MTP. Разблокируйте экран.",
            ),
        ];
        for (i, (ic, title, text)) in tips.iter().enumerate() {
            if i > 0 {
                ui.add_space(8.0);
            }
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(*ic).size(18.0).color(p.accent));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(*title).font(semibold(13.5)).color(p.text));
                    ui.label(RichText::new(*text).size(13.0).color(p.muted));
                });
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn step_row(
    ui: &mut egui::Ui,
    p: &Palette,
    step_icon: &str,
    title: &str,
    desc: &str,
    results: &[(Tag, String)],
    running: bool,
    ran: bool,
) {
    let worst = results.iter().map(|(t, _)| *t).max_by_key(|t| match t {
        Tag::Err => 3,
        Tag::Warn => 2,
        Tag::Ok => 1,
        _ => 0,
    });
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (fg, bg) = match worst {
            Some(t) => (p.tag(t), p.tag_soft(t)),
            None => (p.faint, p.code),
        };
        widgets::icon_tile(ui, step_icon, fg, bg, 34.0);
        let w = ui.available_width() - 40.0;
        ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(w);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(title).font(semibold(14.0)).color(p.text));
            if results.is_empty() {
                let text = if running {
                    "Проверяется…"
                } else if ran {
                    "Не проверялось: сначала нужно, чтобы ADB увидел устройство."
                } else {
                    desc
                };
                ui.label(RichText::new(text).size(13.0).color(p.muted));
            } else {
                for (tag, text) in results {
                    let text = text.split_once("] ").map(|(_, t)| t).unwrap_or(text);
                    ui.label(RichText::new(text).size(13.0).color(if *tag == Tag::Ok { p.muted } else { p.tag(*tag) }));
                }
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let (sym, color) = match worst {
                Some(Tag::Ok) => (icon::CHECK_CIRCLE, p.ok),
                Some(Tag::Warn) => (icon::WARNING_CIRCLE, p.warn),
                Some(Tag::Err) => (icon::X_CIRCLE, p.err),
                _ if running => ("", p.faint),
                _ => (icon::CIRCLE_DASHED, p.faint),
            };
            if sym.is_empty() {
                ui.add(egui::Spinner::new().size(18.0));
            } else {
                ui.label(RichText::new(sym).size(22.0).color(color));
            }
        });
    });
}

// ---------------------------------------------------------------- установка

fn drop_zone(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let height = 170.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let hot = hovering || resp.hovered();
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(16), if hot { p.accent_soft } else { p.card });
    dashed_rect(painter, rect.shrink(1.0), 16.0, if hot { p.accent } else { p.faint });
    let c = rect.center();
    painter.text(
        c - vec2(0.0, 36.0),
        egui::Align2::CENTER_CENTER,
        icon::DOWNLOAD,
        egui::FontId::proportional(38.0),
        p.accent,
    );
    painter.text(
        c + vec2(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        "Перетащите сюда APK, OBB или AAB",
        semibold(17.0),
        p.text,
    );
    painter.text(
        c + vec2(0.0, 36.0),
        egui::Align2::CENTER_CENTER,
        "или нажмите, чтобы выбрать файлы. APK ставится, OBB копируются после установки, AAB — через bundletool",
        egui::FontId::proportional(13.0),
        p.muted,
    );
    if resp.clicked() && !app.busy() {
        let files = app.pick_files("APK, OBB или AAB", ANY_PACKAGE, true);
        if !files.is_empty() {
            app.install_files(files, true);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
}

/// Пунктирная рамка: прямые участки и скруглённые углы.
fn dashed_rect(painter: &egui::Painter, rect: egui::Rect, r: f32, color: egui::Color32) {
    let stroke = Stroke::new(1.5, color);
    let mut pts = Vec::new();
    let corners = [
        (rect.right_top() + vec2(-r, r), -90.0_f32),
        (rect.right_bottom() + vec2(-r, -r), 0.0),
        (rect.left_bottom() + vec2(r, -r), 90.0),
        (rect.left_top() + vec2(r, r), 180.0),
    ];
    pts.push(rect.left_top() + vec2(r, 0.0));
    for (center, start) in corners {
        for k in 0..=8 {
            let a = (start + 90.0 * k as f32 / 8.0).to_radians();
            pts.push(center + vec2(a.cos(), a.sin()) * r);
        }
    }
    pts.push(rect.left_top() + vec2(r, 0.0));
    painter.extend(egui::Shape::dashed_line(&pts, stroke, 7.0, 5.0));
}

// ---------------------------------------------------------------- настройки

pub fn settings(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    full_card(ui, |ui| {
        ui.label(RichText::new("adb").font(semibold(15.0)).color(p.text));
        ui.label(
            RichText::new(
                "Путь к adb. Ищется в platform-tools рядом с программой, в PATH и в обычных местах установки.",
            )
            .size(13.0)
            .color(p.muted),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let edit = egui::TextEdit::singleline(&mut app.adb_edit)
                .font(mono(13.0))
                .margin(Margin::symmetric(10, 7))
                .desired_width(ui.available_width() - 250.0);
            let r = ui.add(edit);
            if r.lost_focus() {
                let path = app.adb_edit.clone();
                app.set_adb_path(path);
            }
            if widgets::secondary(ui, true, format!("{} Обзор…", icon::FOLDER_OPEN), false).clicked() {
                let name = if IS_WIN { "adb.exe" } else { "adb" };
                let mut dlg = rfd::FileDialog::new().set_title("Укажите adb");
                if IS_WIN {
                    dlg = dlg.add_filter("adb", &["exe"]);
                }
                if let Some(path) = dlg.pick_file() {
                    let path = path.to_string_lossy().to_string();
                    if !path.ends_with(name) {
                        app.log(format!("Похоже, это не {name}: {path}"), Tag::Warn);
                    }
                    app.adb_edit = path.clone();
                    app.set_adb_path(path);
                }
            }
            if widgets::secondary(ui, true, format!("{} Найти", icon::MAGNIFYING_GLASS), false).clicked() {
                match find_adb() {
                    Some(path) => {
                        app.adb_edit = path.clone();
                        app.set_adb_path(path);
                    }
                    None => app.log("adb не найден ни в одном из обычных мест.", Tag::Err),
                }
            }
        });
        separator(ui);
        ui.label(RichText::new("Для установки AAB").font(semibold(15.0)).color(p.text));
        // поиск Java запускает /usr/libexec/java_home: делаем его один раз, а не каждый кадр
        let (java, found_bt) = app.tools_cache.get_or_insert_with(|| (find_java(), find_bundletool())).clone();
        let bt = app.bundletool.clone().or(found_bt);
        tool_line(
            ui,
            p,
            "Java",
            java.as_deref(),
            if IS_MAC { "brew install --cask temurin" } else { "установите JDK 17+" },
        );
        tool_line(
            ui,
            p,
            "bundletool",
            bt.as_deref(),
            if IS_MAC {
                "brew install bundletool"
            } else {
                "положите bundletool.jar рядом с программой"
            },
        );
        ui.horizontal(|ui| {
            if widgets::secondary(ui, true, format!("{} Проверить снова", icon::ARROWS_CLOCKWISE), false).clicked()
            {
                app.tools_cache = None;
            }
            if widgets::secondary(ui, true, format!("{} Указать bundletool.jar…", icon::FILE_ARROW_UP), false).clicked()
                && let Some(f) = rfd::FileDialog::new().add_filter("bundletool", &["jar"]).pick_file()
            {
                app.bundletool = Some(f.to_string_lossy().into());
            }
        });
    });
    ui.add_space(14.0);
    full_card(ui, |ui| {
        ui.label(RichText::new("Поведение и вид").font(semibold(15.0)).color(p.text));
        ui.add_space(4.0);
        ui.checkbox(&mut app.auto_refresh, "Обновлять список устройств каждые 3 секунды");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Тема").color(p.muted));
            ui.add_space(8.0);
            for (pref, label, ic) in [
                (ThemePreference::System, "Как в системе", icon::CIRCLE_HALF),
                (ThemePreference::Light, "Светлая", icon::SUN),
                (ThemePreference::Dark, "Тёмная", icon::MOON),
            ] {
                let selected = app.theme == pref;
                let text = RichText::new(format!("{ic}  {label}")).color(if selected { p.accent } else { p.text });
                let b = egui::Button::new(text).selected(selected).min_size(vec2(0.0, 30.0));
                if ui.add(b).clicked() {
                    app.theme = pref;
                    app.settings.theme = match pref {
                        ThemePreference::Light => "light",
                        ThemePreference::Dark => "dark",
                        ThemePreference::System => "system",
                    }
                    .into();
                    app.settings.save();
                }
            }
        });
    });
    ui.add_space(14.0);
    full_card(ui, |ui| super::extras::settings_card(app, ui));
    ui.add_space(14.0);
    full_card(ui, |ui| {
        ui.label(RichText::new("О программе").font(semibold(15.0)).color(p.text));
        ui.label(
            RichText::new(format!(
                "ADB Toolkit {} · Rust + egui. Проверка без окна: adb-toolkit --selftest",
                env!("CARGO_PKG_VERSION")
            ))
            .size(13.0)
            .color(p.muted),
        );
    });
}

fn tool_line(ui: &mut egui::Ui, p: &Palette, name: &str, path: Option<&str>, hint: &str) {
    ui.horizontal(|ui| {
        let (sym, color) = if path.is_some() { (icon::CHECK_CIRCLE, p.ok) } else { (icon::X_CIRCLE, p.faint) };
        ui.label(RichText::new(sym).size(16.0).color(color));
        ui.label(RichText::new(name).font(semibold(13.5)).color(p.text));
        match path {
            Some(path) => ui.add(egui::Label::new(RichText::new(path).font(mono(12.0)).color(p.muted)).truncate()),
            None => ui.label(RichText::new(format!("не найдено — {hint}")).size(13.0).color(p.muted)),
        };
    });
}
