//! Оформление: палитры для светлой и тёмной темы (следуют системе), шрифты, скругления и отступы.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle,
    Theme, Visuals,
};

use crate::adb::Tag;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub card_stroke: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub accent_soft: Color32,
    pub ok: Color32,
    pub ok_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub err: Color32,
    pub err_soft: Color32,
    pub pink: Color32,
    pub pink_soft: Color32,
    pub console: Color32,
    pub code: Color32,
    pub input: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Palette = Palette {
    bg: hex(0xF5F6F8),
    sidebar: hex(0xECEEF2),
    card: hex(0xFFFFFF),
    card_stroke: hex(0xE3E6EB),
    hover: hex(0xF1F3F6),
    text: hex(0x1B1E24),
    muted: hex(0x5F6673),
    faint: hex(0x8D94A0),
    accent: hex(0x2F6BFF),
    accent_text: hex(0xFFFFFF),
    accent_soft: hex(0xE6EEFF),
    ok: hex(0x188A4E),
    ok_soft: hex(0xE2F5EA),
    warn: hex(0xB76E00),
    warn_soft: hex(0xFFF3DC),
    err: hex(0xD03030),
    err_soft: hex(0xFDE7E7),
    pink: hex(0xD9346F),
    pink_soft: hex(0xFCE8F0),
    console: hex(0xFBFBFC),
    code: hex(0xF0F2F5),
    input: hex(0xFFFFFF),
};

pub const DARK: Palette = Palette {
    bg: hex(0x15161A),
    sidebar: hex(0x1C1D22),
    card: hex(0x22242A),
    card_stroke: hex(0x30323A),
    hover: hex(0x2A2C33),
    text: hex(0xECEEF2),
    muted: hex(0xA3A9B4),
    faint: hex(0x737A86),
    accent: hex(0x5B8CFF),
    accent_text: hex(0xFFFFFF),
    accent_soft: hex(0x233152),
    ok: hex(0x4ADE80),
    ok_soft: hex(0x16301F),
    warn: hex(0xFBBF24),
    warn_soft: hex(0x382D12),
    err: hex(0xF87171),
    err_soft: hex(0x3B1C1E),
    pink: hex(0xFF7AA8),
    pink_soft: hex(0x3A1D2A),
    console: hex(0x101114),
    code: hex(0x2B2D34),
    input: hex(0x1A1B20),
};

pub fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode { &DARK } else { &LIGHT }
}

impl Palette {
    pub fn tag(&self, tag: Tag) -> Color32 {
        match tag {
            Tag::Text => self.text,
            Tag::Head => self.accent,
            Tag::Err => self.err,
            Tag::Warn => self.warn,
            Tag::Ok => self.ok,
            Tag::Dim => self.faint,
        }
    }

    pub fn tag_soft(&self, tag: Tag) -> Color32 {
        match tag {
            Tag::Err => self.err_soft,
            Tag::Warn => self.warn_soft,
            Tag::Ok => self.ok_soft,
            Tag::Head => self.accent_soft,
            _ => self.code,
        }
    }
}

pub const RADIUS: u8 = 8;
pub const CARD_RADIUS: u8 = 12;

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Моноширинный шрифт берём из системы: Menlo / SF Mono на Mac, Cascadia / Consolas в Windows.
fn system_mono() -> Option<(Vec<u8>, u32)> {
    let candidates: &[(&str, u32)] = if cfg!(target_os = "macos") {
        &[("/System/Library/Fonts/SFNSMono.ttf", 0), ("/System/Library/Fonts/Menlo.ttc", 0)]
    } else if cfg!(windows) {
        &[(r"C:\Windows\Fonts\CascadiaMono.ttf", 0), (r"C:\Windows\Fonts\consola.ttf", 0)]
    } else {
        &[("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0)]
    };
    candidates.iter().find_map(|(p, i)| std::fs::read(p).ok().map(|b| (b, *i)))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"))),
    );
    // Из Inter убраны символы Private Use Area (assets/fonts/README.md): там значки Phosphor.
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let mut bold: Vec<String> = fonts.families[&FontFamily::Proportional].clone();
    bold[0] = "inter-semibold".into();
    fonts.families.insert(FontFamily::Name("semibold".into()), bold);
    if let Some((bytes, index)) = system_mono() {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert("system-mono".into(), Arc::new(data));
        let mono = fonts.families.entry(FontFamily::Monospace).or_default();
        mono.insert(0, "system-mono".into());
        // значки и в моноширинном тексте
        mono.push("phosphor".into());
    }
    ctx.set_fonts(fonts);

    for (theme, p) in [(Theme::Light, &LIGHT), (Theme::Dark, &DARK)] {
        ctx.style_mut_of(theme, |style| {
            style.text_styles = [
                (TextStyle::Small, FontId::proportional(12.0)),
                (TextStyle::Body, FontId::proportional(14.0)),
                (TextStyle::Button, FontId::proportional(14.0)),
                (TextStyle::Heading, semibold(24.0)),
                (TextStyle::Monospace, mono(12.5)),
            ]
            .into();
            let s = &mut style.spacing;
            s.item_spacing = egui::vec2(8.0, 8.0);
            s.button_padding = egui::vec2(12.0, 6.0);
            s.interact_size.y = 30.0;
            s.window_margin = Margin::same(20);
            s.menu_margin = Margin::same(6);
            s.combo_width = 220.0;
            s.scroll.bar_width = 8.0;
            s.scroll.floating = true;
            style.visuals = visuals(theme, p);
        });
    }
}

fn visuals(theme: Theme, p: &Palette) -> Visuals {
    let mut v = if theme == Theme::Dark { Visuals::dark() } else { Visuals::light() };
    let r = CornerRadius::same(RADIUS);
    v.override_text_color = None;
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.card_stroke);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow = Shadow {
        offset: [0, 12],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(if theme == Theme::Dark { 120 } else { 40 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(if theme == Theme::Dark { 100 } else { 30 }),
    };
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = p.input;
    v.text_edit_bg_color = Some(p.input);
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.code;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.err;
    v.selection.bg_fill = p.accent.gamma_multiply(if theme == Theme::Dark { 0.45 } else { 0.25 });
    v.selection.stroke = Stroke::new(1.5, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.card_stroke);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.code;
    w.inactive.weak_bg_fill = p.code;
    w.inactive.bg_stroke = Stroke::new(1.0, p.card_stroke);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = r;
    v
}
