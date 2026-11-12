//! Небольшие элементы интерфейса: карточки, кнопки, бейджи, пункты меню.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Response, RichText, Sense, Stroke, Ui, vec2,
};

use super::theme::{CARD_RADIUS, Palette, RADIUS, palette, semibold};

pub fn card(ui: &Ui) -> Frame {
    let p = palette(ui);
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.card_stroke))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(18))
}

/// Карточка во всю ширину.
pub fn full_card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    card(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Заливная кнопка основного действия.
pub fn primary(ui: &mut Ui, enabled: bool, text: impl Into<String>) -> Response {
    let p = palette(ui);
    let text = RichText::new(text.into()).font(semibold(14.0)).color(p.accent_text);
    let fill = if enabled { p.accent } else { p.accent.gamma_multiply(0.45) };
    ui.add_enabled(enabled, egui::Button::new(text).fill(fill).stroke(Stroke::NONE).min_size(vec2(0.0, 34.0)))
}

/// Обычная кнопка; `danger` — красная надпись.
pub fn secondary(ui: &mut Ui, enabled: bool, text: impl Into<String>, danger: bool) -> Response {
    let p = palette(ui);
    let mut t = RichText::new(text.into()).font(semibold(13.5));
    if danger {
        t = t.color(p.err);
    }
    ui.add_enabled(enabled, egui::Button::new(t).min_size(vec2(0.0, 30.0)))
}

/// Кнопка-значок без рамки.
pub fn icon_button(ui: &mut Ui, enabled: bool, icon: &str, tip: &str) -> Response {
    let p = palette(ui);
    let r = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(icon).size(16.0).color(p.muted))
            .frame_when_inactive(false)
            .min_size(vec2(30.0, 30.0)),
    );
    r.on_hover_text(tip)
}

/// Бейдж: точка и текст на мягком фоне.
pub fn pill(ui: &mut Ui, text: &str, color: Color32, soft: Color32) -> Response {
    Frame::new()
        .fill(soft)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.5, color);
            ui.label(RichText::new(text).size(12.0).color(color).font(semibold(12.0)));
        })
        .response
}

/// Квадратная плашка со значком.
pub fn icon_tile(ui: &mut Ui, icon: &str, fg: Color32, bg: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS + 1), bg);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, egui::FontId::proportional(size * 0.5), fg);
}

/// Пункт бокового меню.
pub fn nav_item(ui: &mut Ui, icon: &str, label: &str, selected: bool, badge: Option<Color32>) -> Response {
    let p = palette(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let bg = if selected {
        p.accent_soft
    } else if resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), bg);
    let color = if selected { p.accent } else { p.text };
    let painter = ui.painter();
    painter.text(
        rect.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        if selected { p.accent } else { p.muted },
    );
    let font = if selected { semibold(13.5) } else { egui::FontId::proportional(13.5) };
    painter.text(rect.left_center() + vec2(38.0, 0.0), Align2::LEFT_CENTER, label, font, color);
    if let Some(c) = badge {
        painter.circle_filled(rect.right_center() - vec2(14.0, 0.0), 4.0, c);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn section_label(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    ui.add_space(4.0);
    ui.label(RichText::new(text.to_uppercase()).size(11.0).color(p.faint).font(semibold(11.0)));
}

/// Текст команды серым моноширинным шрифтом на плашке.
pub fn code_chip(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    Frame::new().fill(p.code).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(7, 3)).show(
        ui,
        |ui| {
            ui.add(egui::Label::new(RichText::new(text).font(super::theme::mono(11.5)).color(p.muted)).truncate());
        },
    );
}

/// Ряд в карточке-списке: значок, заголовок, описание, команда и кнопка справа.
/// Возвращает true, если нажата кнопка.
#[allow(clippy::too_many_arguments)]
pub fn command_row(
    ui: &mut Ui,
    p: &Palette,
    icon: &str,
    title: &str,
    desc: &str,
    cmd: &str,
    button: &str,
    enabled: bool,
    danger: bool,
) -> bool {
    let mut clicked = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (fg, bg) = if danger { (p.err, p.err_soft) } else { (p.accent, p.accent_soft) };
        icon_tile(ui, icon, fg, bg, 38.0);
        let button_w = 128.0;
        let text_w = (ui.available_width() - button_w - 14.0).max(120.0);
        ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.set_width(text_w);
            ui.label(RichText::new(title).font(semibold(14.5)).color(p.text));
            ui.label(RichText::new(desc).size(13.0).color(p.muted));
            ui.add_space(2.0);
            code_chip(ui, cmd);
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.add_space(0.0);
            if secondary(ui, enabled, button, danger).clicked() {
                clicked = true;
            }
        });
    });
    clicked
}

pub fn separator(ui: &mut Ui) {
    let p = palette(ui);
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.card_stroke);
    ui.add_space(6.0);
}

/// Пункт меню своего цвета (например, розовый «Поддержать»).
pub fn nav_item_tinted(
    ui: &mut Ui,
    icon: &str,
    label: &str,
    selected: bool,
    color: Color32,
    soft: Color32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let bg = if selected || resp.hovered() { soft } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), bg);
    let painter = ui.painter();
    painter.text(
        rect.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        color,
    );
    painter.text(rect.left_center() + vec2(38.0, 0.0), Align2::LEFT_CENTER, label, semibold(13.5), color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
