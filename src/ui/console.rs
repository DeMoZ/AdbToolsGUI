//! Нижняя панель: что выполняется сейчас, цветной журнал и поле для своей команды adb.

use eframe::egui::{self, Align, CornerRadius, Frame, Key, Layout, Margin, RichText, Stroke, text::LayoutJob};
use egui_phosphor::regular as icon;

use super::theme::{mono, palette, semibold};
use super::{App, widgets};

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let frame = Frame::new().fill(p.console).stroke(Stroke::new(1.0, p.card_stroke)).inner_margin(Margin {
        left: 20,
        right: 16,
        top: 8,
        bottom: 10,
    });
    let panel = egui::Panel::bottom("console").frame(frame).show_separator_line(false);
    let panel = if app.console_open {
        panel.resizable(true).default_size(270.0).size_range(170.0..=700.0)
    } else {
        panel.resizable(false)
    };
    panel.show(ui, |ui| {
        header(app, ui);
        if app.console_open {
            ui.add_space(4.0);
            // поле команды снизу, как в терминале; журнал занимает всё остальное
            egui::Panel::bottom("console-input")
                .frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 0 }))
                .resizable(false)
                .show_separator_line(false)
                .show(ui, |ui| input(app, ui));
            body(app, ui);
        }
    });
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        let toggle = if app.console_open { icon::CARET_DOWN } else { icon::CARET_UP };
        if widgets::icon_button(
            ui,
            true,
            toggle,
            if app.console_open { "Свернуть журнал" } else { "Развернуть журнал" },
        )
        .clicked()
        {
            app.console_open = !app.console_open;
        }
        ui.label(RichText::new("Журнал").font(semibold(14.0)).color(p.text));
        let running = app.running.as_ref().map(|r| (r.title.clone(), r.started.elapsed().as_secs()));
        if let Some((title, secs)) = running {
            ui.add_space(10.0);
            Frame::new()
                .fill(p.accent_soft)
                .corner_radius(CornerRadius::same(255))
                .inner_margin(Margin { left: 8, right: 4, top: 2, bottom: 2 })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add(egui::Spinner::new().size(13.0).color(p.accent));
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{title} · {}:{:02}", secs / 60, secs % 60))
                                .size(12.5)
                                .color(p.accent),
                        )
                        .truncate(),
                    );
                    let stop =
                        egui::Button::new(RichText::new(format!("{} Прервать", icon::STOP)).size(12.5).color(p.err))
                            .frame_when_inactive(false)
                            .corner_radius(CornerRadius::same(255));
                    if ui.add(stop).clicked() {
                        app.cancel();
                    }
                });
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(ui, !app.log.is_empty(), icon::TRASH, "Очистить журнал").clicked() {
                app.log.clear();
            }
            if widgets::icon_button(ui, !app.log.is_empty(), icon::COPY, "Копировать всё").clicked() {
                let all: Vec<&str> = app.log.iter().map(|(t, _)| t.as_str()).collect();
                ui.ctx().copy_text(all.join("\n"));
            }
        });
    });
}

fn body(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        let mut job = LayoutJob::default();
        for (i, (text, tag)) in app.log.iter().enumerate() {
            let fmt = egui::TextFormat {
                font_id: mono(12.5),
                color: p.tag(*tag),
                line_height: Some(18.0),
                ..Default::default()
            };
            job.append(text, 0.0, fmt.clone());
            if i + 1 < app.log.len() {
                job.append("\n", 0.0, fmt);
            }
        }
        job.wrap.max_width = ui.available_width();
        ui.add(egui::Label::new(job).selectable(true).wrap());
    });
}

fn input(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let busy = app.busy();
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.card_stroke))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin { left: 12, right: 4, top: 3, bottom: 3 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} adb", icon::TERMINAL)).font(mono(13.0)).color(p.accent));
                let run_w = 120.0;
                let edit = egui::TextEdit::singleline(&mut app.custom)
                    .font(mono(13.0))
                    .frame(egui::Frame::NONE)
                    .hint_text("shell getprop ro.build.version.release")
                    .desired_width(ui.available_width() - run_w);
                let r = ui.add(edit);
                if r.has_focus() {
                    history(app, ui);
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let run = widgets::primary(
                        ui,
                        !busy && !app.custom.trim().is_empty(),
                        format!("{} Выполнить", icon::PLAY),
                    );
                    if run.clicked() || enter {
                        app.run_custom();
                        r.request_focus();
                    }
                });
            });
        });
}

/// Стрелки вверх / вниз листают введённые раньше команды.
fn history(app: &mut App, ui: &egui::Ui) {
    if app.history.is_empty() {
        return;
    }
    let (up, down) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
    if up {
        let pos = app.history_pos.map_or(app.history.len() - 1, |p| p.saturating_sub(1));
        app.history_pos = Some(pos);
        app.custom = app.history[pos].clone();
    } else if down {
        match app.history_pos {
            Some(p) if p + 1 < app.history.len() => {
                app.history_pos = Some(p + 1);
                app.custom = app.history[p + 1].clone();
            }
            _ => {
                app.history_pos = None;
                app.custom.clear();
            }
        }
    }
}
