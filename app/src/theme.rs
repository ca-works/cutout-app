//! Design tokens, ported from the macOS `Theme.swift`: every colour is tuned
//! separately for light and dark (not simply inverted).

use eframe::egui::{self, epaint::Shadow, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};
use std::sync::Arc;

pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0, 122, 255);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(10, 132, 255);

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Workspace behind the image.
    pub canvas: Color32,
    /// Window chrome (toolbar, menu bar, status bar).
    pub chrome: Color32,
    /// Sidebar / inspector background.
    pub sidebar: Color32,
    /// Raised card surface (option groups, empty state).
    pub card: Color32,
    pub card_stroke: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub tertiary: Color32,
    pub accent: Color32,
    pub wordmark: Color32,
    pub separator: Color32,
}

impl Palette {
    pub fn get(dark: bool) -> Palette {
        if dark {
            Palette {
                dark,
                canvas: Color32::from_gray(27),
                chrome: Color32::from_rgb(38, 38, 40),
                sidebar: Color32::from_rgb(41, 41, 43),
                card: Color32::from_white_alpha(14),
                card_stroke: Color32::from_white_alpha(23),
                hover: Color32::from_white_alpha(23),
                text: Color32::from_gray(236),
                secondary: Color32::from_rgb(160, 160, 166),
                tertiary: Color32::from_rgb(110, 110, 116),
                accent: ACCENT_DARK,
                wordmark: Color32::from_rgb(245, 245, 247),
                separator: Color32::from_white_alpha(26),
            }
        } else {
            Palette {
                dark,
                canvas: Color32::from_gray(237),
                chrome: Color32::from_rgb(236, 235, 235),
                sidebar: Color32::from_rgb(243, 242, 242),
                card: Color32::from_white_alpha(184),
                card_stroke: Color32::from_black_alpha(20),
                hover: Color32::from_black_alpha(15),
                text: Color32::from_gray(28),
                secondary: Color32::from_rgb(110, 110, 116),
                tertiary: Color32::from_rgb(160, 160, 166),
                accent: ACCENT_LIGHT,
                wordmark: Color32::from_rgb(29, 29, 31),
                separator: Color32::from_black_alpha(28),
            }
        }
    }
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

pub fn semibold_family() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Install fonts: Segoe UI on Windows (the system UI font), Lucide icons, and
/// system fallbacks for scripts the default fonts lack.
pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(build_fonts(false));
    // CJK / Indic / Thai / symbol fallbacks are large; add them without delaying start-up.
    #[cfg(windows)]
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            ctx.set_fonts(build_fonts(true));
            ctx.request_repaint();
        });
    }
}

fn build_fonts(with_fallbacks: bool) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let _ = with_fallbacks;
    fonts.font_data.insert("lucide".into(), Arc::new(FontData::from_static(include_bytes!("../assets/lucide.ttf"))));
    fonts.families.insert(icon_family(), vec!["lucide".into()]);

    let load = |paths: &[&str]| -> Option<Vec<u8>> { paths.iter().find_map(|p| std::fs::read(p).ok()) };

    #[cfg(windows)]
    let (regular, semibold, fallbacks): (Option<Vec<u8>>, Option<Vec<u8>>, Vec<&str>) = {
        let root = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let f = |n: &str| format!("{root}\\Fonts\\{n}");
        (
            load(&[&f("segoeui.ttf")]),
            load(&[&f("seguisb.ttf"), &f("segoeuib.ttf")]),
            vec![],
        )
    };
    #[cfg(not(windows))]
    let (regular, semibold, fallbacks): (Option<Vec<u8>>, Option<Vec<u8>>, Vec<&str>) = (
        load(&["/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf"]),
        load(&["/System/Library/Fonts/Supplemental/Arial Bold.ttf"]),
        vec![],
    );
    let _ = fallbacks;

    if let Some(b) = regular {
        fonts.font_data.insert("ui".into(), Arc::new(FontData::from_owned(b)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui".into());
    }
    let sb_name = if let Some(b) = semibold {
        fonts.font_data.insert("ui-semibold".into(), Arc::new(FontData::from_owned(b)));
        "ui-semibold"
    } else {
        "ui"
    };
    let mut sb_family = vec![sb_name.to_string()];
    if let Some(p) = fonts.families.get(&FontFamily::Proportional) {
        sb_family.extend(p.iter().cloned());
    }
    fonts.families.insert(semibold_family(), sb_family);
    #[cfg(windows)]
    if with_fallbacks {
        let root = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        for (i, (file, index)) in [("msyh.ttc", 0u32), ("YuGothR.ttc", 0), ("malgun.ttf", 0), ("Nirmala.ttc", 0), ("leelawui.ttf", 0), ("seguisym.ttf", 0)].into_iter().enumerate() {
            if let Ok(b) = std::fs::read(format!("{root}\\Fonts\\{file}")) {
                let name = format!("fallback-{i}");
                let mut data = FontData::from_owned(b);
                data.index = index;
                fonts.font_data.insert(name.clone(), Arc::new(data));
                for fam in [FontFamily::Proportional, semibold_family()] {
                    fonts.families.entry(fam).or_default().push(name.clone());
                }
            }
        }
    }
    fonts
}

pub fn apply(ctx: &egui::Context) {
    for dark in [false, true] {
        let p = Palette::get(dark);
        let mut v = if dark { Visuals::dark() } else { Visuals::light() };
        v.panel_fill = p.sidebar;
        v.window_fill = if dark { Color32::from_rgb(48, 48, 50) } else { Color32::from_rgb(248, 248, 248) };
        v.extreme_bg_color = if dark { Color32::from_gray(30) } else { Color32::WHITE };
        v.faint_bg_color = p.hover;
        v.override_text_color = Some(p.text);
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.accent.gamma_multiply(if dark { 0.55 } else { 0.35 });
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
        v.window_stroke = Stroke::new(1.0, p.card_stroke);
        v.window_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if dark { 120 } else { 50 }) };
        v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 45 }) };
        let w = &mut v.widgets;
        let r = CornerRadius::same(7);
        for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            s.corner_radius = r;
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        w.inactive.bg_fill = if dark { Color32::from_white_alpha(20) } else { Color32::from_white_alpha(200) };
        w.inactive.weak_bg_fill = if dark { Color32::from_white_alpha(16) } else { Color32::from_white_alpha(170) };
        w.inactive.bg_stroke = Stroke::new(1.0, p.card_stroke);
        w.inactive.fg_stroke = Stroke::new(1.0, p.text);
        w.hovered.bg_fill = if dark { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(240) };
        w.hovered.weak_bg_fill = if dark { Color32::from_white_alpha(30) } else { Color32::from_black_alpha(14) };
        w.hovered.bg_stroke = Stroke::new(1.0, p.card_stroke);
        w.hovered.fg_stroke = Stroke::new(1.0, p.text);
        w.active.bg_fill = if dark { Color32::from_white_alpha(46) } else { Color32::from_black_alpha(26) };
        w.active.weak_bg_fill = w.active.bg_fill;
        w.active.fg_stroke = Stroke::new(1.0, p.text);
        w.open.weak_bg_fill = p.hover;
        ctx.set_visuals_of(if dark { egui::Theme::Dark } else { egui::Theme::Light }, v);
    }
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.slider_width = 150.0;
        s.spacing.interact_size.y = 24.0;
        s.spacing.menu_margin = egui::Margin::symmetric(6, 6);
        s.text_styles.insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(13.5, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(11.5, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Heading, FontId::new(17.0, semibold_family()));
        s.text_styles.insert(TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace));
        s.animation_time = 0.12;
        s.interaction.tooltip_delay = 0.5;
    });
}

pub fn icon(c: char, size: f32) -> egui::RichText {
    egui::RichText::new(c.to_string()).font(FontId::new(size, icon_family()))
}
