//! Раздел «Поддержать» и обновления программы: проверка, окно с предложением, загрузка и перезапуск.

use std::path::PathBuf;
use std::thread;

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke};
use egui_phosphor::regular as icon;

use super::theme::{palette, semibold};
use super::{App, widgets};
use crate::adb::{Event, Tag};
use crate::donations;
use crate::settings::now;
use crate::update::{self, Release, format_version};

// ---------------------------------------------------------------- донаты

pub struct DonateState {
    pub info: donations::Info,
    pub loading: bool,
    /// Свежий файл не загрузился, показана сохранённая или встроенная копия.
    pub offline: bool,
}

impl Default for DonateState {
    fn default() -> Self {
        DonateState { info: donations::cached(), loading: false, offline: false }
    }
}

/// При каждом открытии раздела файл загружается заново; до ответа видна последняя копия.
pub fn refresh_donations(app: &mut App) {
    if app.donate.loading {
        return;
    }
    app.donate.loading = true;
    let sink = app.sink.clone();
    thread::spawn(move || sink.send(Event::Donations(donations::download())));
}

pub fn donations_loaded(app: &mut App, info: Option<donations::Info>) {
    app.donate.loading = false;
    app.donate.offline = info.is_none();
    if let Some(info) = info {
        app.donate.info = info;
    }
}

pub fn donate_page(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let info = app.donate.info.clone();
    if let Some(message) = &info.message {
        Frame::new().fill(p.pink_soft).corner_radius(CornerRadius::same(16)).inner_margin(Margin::same(22)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::icon_tile(ui, icon::HEART, Color32::WHITE, p.pink, 48.0);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Спасибо, что пользуетесь!").font(semibold(18.0)).color(p.text));
                            if app.donate.loading {
                                ui.add(egui::Spinner::new().size(16.0).color(p.pink));
                            }
                        });
                        ui.label(RichText::new(message).size(14.0).color(p.muted));
                    });
                });
            },
        );
        ui.add_space(14.0);
    }
    for m in &info.methods {
        let card = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.card_stroke))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(16));
        let r = card
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::icon_tile(ui, method_icon(&m.id), p.pink, p.pink_soft, 38.0);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.label(RichText::new(&m.title).font(semibold(15.0)).color(p.text));
                        if let Some(d) = &m.description {
                            ui.label(RichText::new(d).size(13.0).color(p.muted));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(icon::ARROW_SQUARE_OUT).size(18.0).color(p.pink));
                    });
                });
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&m.url);
        if r.clicked()
            && let Err(e) = crate::net::open_url(&m.url)
        {
            app.log(format!("Не удалось открыть браузер: {e}"), Tag::Err);
        }
        ui.add_space(10.0);
    }
    if app.donate.offline {
        ui.label(
            RichText::new("Свежий список способов загрузить не удалось: показана сохранённая копия.")
                .size(12.5)
                .color(p.faint),
        );
    }
}

fn method_icon(id: &str) -> &'static str {
    match id {
        "cloudtips" => icon::CREDIT_CARD,
        "tribute" => icon::GLOBE_HEMISPHERE_WEST,
        _ => icon::HAND_HEART,
    }
}

// ---------------------------------------------------------------- обновления

#[derive(Default)]
pub struct UpdateState {
    pub checking: bool,
    manual: bool,
    pub available: Option<Release>,
    pub downloading: bool,
    /// Итог последней проверки для раздела «Настройки».
    pub status: Option<(Tag, String)>,
    /// Показать окно с предложением обновиться.
    pub offer: bool,
}

/// Проверка при запуске: не чаще раза в сутки и только в собранной программе.
pub fn check_on_start(app: &mut App) {
    if app.settings.auto_update
        && update::install_target().is_some()
        && now() - app.settings.last_update_check >= 86_400
    {
        check(app, false);
    }
}

pub fn check(app: &mut App, manual: bool) {
    if app.updates.checking || app.updates.downloading {
        return;
    }
    app.updates.checking = true;
    app.updates.manual = manual;
    let sink = app.sink.clone();
    thread::spawn(move || sink.send(Event::UpdateChecked(update::latest())));
}

pub fn checked(app: &mut App, result: Result<Option<Release>, String>) {
    app.updates.checking = false;
    let manual = app.updates.manual;
    let current = update::current();
    match result {
        Ok(found) => {
            app.settings.last_update_check = now();
            app.settings.save();
            match found.filter(|r| r.version > current) {
                Some(r) => {
                    let v = format_version(r.version);
                    app.updates.status = Some((Tag::Head, format!("Доступна версия {v}.")));
                    app.updates.offer = manual || app.settings.skipped_version != v;
                    app.updates.available = Some(r);
                }
                None => {
                    app.updates.available = None;
                    app.updates.status =
                        Some((Tag::Ok, format!("Установлена последняя версия {}.", format_version(current))));
                }
            }
        }
        Err(e) => {
            app.updates.status = Some((Tag::Err, format!("Не удалось проверить обновления: {e}")));
            if manual {
                app.log(format!("Не удалось проверить обновления: {e}"), Tag::Err);
            }
        }
    }
}

fn install(app: &mut App, release: Release) {
    if app.busy() {
        app.log("Дождитесь окончания текущей операции, потом обновите программу.", Tag::Warn);
        return;
    }
    app.updates.downloading = true;
    app.updates.offer = false;
    app.log(format!("Загружаю ADB Toolkit {}…", format_version(release.version)), Tag::Head);
    let sink = app.sink.clone();
    thread::spawn(move || sink.send(Event::UpdateDownloaded(update::download(&release))));
}

pub fn downloaded(app: &mut App, ctx: &egui::Context, result: Result<PathBuf, String>) {
    app.updates.downloading = false;
    let target = update::install_target();
    match (result, target) {
        (Ok(source), Some(target)) => match update::start_install(&source, &target) {
            Ok(()) => {
                app.log("Обновление загружено, перезапускаю программу.", Tag::Ok);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => fail(app, format!("не удалось запустить установку: {e}")),
        },
        (Err(e), _) => fail(app, e),
        (Ok(_), None) => fail(app, "не найдено, где установлена программа".into()),
    }
}

fn fail(app: &mut App, e: String) {
    app.log(format!("Обновление не установлено: {e}"), Tag::Err);
    app.updates.status = Some((Tag::Err, format!("Обновление не установлено: {e}")));
}

/// Карточка в боковом меню, когда есть новая версия.
pub fn sidebar_banner(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let Some(r) = &app.updates.available else { return };
    let v = format_version(r.version);
    let downloading = app.updates.downloading;
    let resp = Frame::new()
        .fill(p.accent_soft)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if downloading {
                    ui.add(egui::Spinner::new().size(15.0).color(p.accent));
                } else {
                    ui.label(RichText::new(icon::ARROW_CIRCLE_UP).size(18.0).color(p.accent));
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let title = if downloading {
                        "Загружаю обновление…".to_string()
                    } else {
                        format!("Версия {v}")
                    };
                    ui.label(RichText::new(title).font(semibold(13.0)).color(p.accent));
                    if !downloading {
                        ui.label(RichText::new("Нажмите, чтобы обновить").size(11.5).color(p.muted));
                    }
                });
            });
        })
        .response
        .interact(Sense::click());
    if resp.clicked() && !downloading {
        app.updates.offer = true;
    }
    ui.add_space(8.0);
}

/// Карточка «Обновления» в настройках.
pub fn settings_card(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    ui.label(RichText::new("Обновления").font(semibold(15.0)).color(p.text));
    let target = update::install_target();
    ui.label(
        RichText::new(format!(
            "Установлена версия {}. Новые версии берутся из релизов github.com/{}.",
            env!("CARGO_PKG_VERSION"),
            update::REPO
        ))
        .size(13.0)
        .color(p.muted),
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let enabled = target.is_some() && !app.updates.checking && !app.updates.downloading;
        if widgets::secondary(ui, enabled, format!("{} Проверить обновления", icon::ARROW_CIRCLE_UP), false).clicked()
        {
            check(app, true);
        }
        if app.updates.checking {
            ui.add(egui::Spinner::new().size(16.0));
        }
        if let Some(r) = app.updates.available.clone()
            && widgets::primary(ui, !app.updates.downloading, format!("Установить {}", format_version(r.version)))
                .clicked()
        {
            app.updates.offer = true;
        }
    });
    if target.is_none() {
        ui.label(
            RichText::new("Обновление работает только в собранной программе (из .dmg или .zip релиза).")
                .size(12.5)
                .color(p.faint),
        );
    }
    if let Some((tag, text)) = &app.updates.status {
        ui.label(RichText::new(text).size(13.0).color(if *tag == Tag::Head { p.accent } else { p.tag(*tag) }));
    }
    let mut auto = app.settings.auto_update;
    if ui.checkbox(&mut auto, "Проверять обновления автоматически (раз в сутки)").changed()
    {
        app.settings.auto_update = auto;
        app.settings.save();
    }
}

/// Окно «Доступна версия …»: обновить, позже или пропустить.
pub fn offer_modal(app: &mut App, ctx: &egui::Context) {
    if !app.updates.offer {
        return;
    }
    let Some(release) = app.updates.available.clone() else {
        app.updates.offer = false;
        return;
    };
    let v = format_version(release.version);
    let target = update::install_target();
    let writable = target.as_deref().is_some_and(update::can_write);
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("update-offer"))
        .frame(Frame::popup(&ctx.global_style()).inner_margin(Margin::same(22)).corner_radius(CornerRadius::same(14)));
    let resp = modal.show(ctx, |ui| {
        let p = palette(ui);
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            widgets::icon_tile(ui, icon::ARROW_CIRCLE_UP, p.accent, p.accent_soft, 36.0);
            ui.label(RichText::new(format!("Доступна версия {v}")).font(semibold(17.0)).color(p.text));
        });
        ui.add_space(4.0);
        ui.label(RichText::new(format!("У вас {}.", env!("CARGO_PKG_VERSION"))).color(p.muted));
        if !release.notes.is_empty() {
            ui.add_space(6.0);
            Frame::new().fill(p.code).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10)).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        ui.label(RichText::new(&release.notes).size(13.0).color(p.text));
                    });
                },
            );
        }
        if !writable {
            ui.add_space(6.0);
            let place = target.as_ref().map(|t| t.display().to_string()).unwrap_or_default();
            ui.label(
                RichText::new(format!(
                    "Эта копия не может обновить себя в {place}: скачайте новую версию со страницы релиза."
                ))
                .size(13.0)
                .color(p.warn),
            );
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let main = if writable {
                "Обновить и перезапустить"
            } else {
                "Открыть страницу релиза"
            };
            if widgets::primary(ui, true, main).clicked() {
                action = Some(if writable { 1 } else { 2 });
            }
            if widgets::secondary(ui, true, "Позже", false).clicked() {
                action = Some(0);
            }
            if widgets::secondary(ui, true, "Пропустить версию", false).clicked() {
                action = Some(3);
            }
        });
    });
    if resp.should_close() && action.is_none() {
        action = Some(0);
    }
    match action {
        Some(1) => install(app, release),
        Some(2) => {
            app.updates.offer = false;
            let _ = crate::net::open_url(&release.page_url);
        }
        Some(3) => {
            app.updates.offer = false;
            app.settings.skipped_version = v;
            app.settings.save();
        }
        Some(_) => app.updates.offer = false,
        None => {}
    }
}
