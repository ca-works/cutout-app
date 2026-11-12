//! Small reusable controls in the CutOut style.

use crate::theme::Palette;
use crate::theme::{self, icon};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

pub fn card_frame(pal: Palette) -> egui::Frame {
    egui::Frame::new().fill(pal.card).stroke(Stroke::new(1.0, pal.card_stroke)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(12))
}

pub fn section_header(ui: &mut Ui, pal: Palette, title: &str) {
    ui.label(egui::RichText::new(title.to_uppercase()).font(FontId::new(10.5, theme::semibold_family())).color(pal.secondary).extra_letter_spacing(0.6));
}

/// Borderless icon button with hover / pressed feedback. Returns the response.
pub fn icon_button(ui: &mut Ui, pal: Palette, glyph: char, tip: &str, enabled: bool, on: bool) -> Response {
    let size = Vec2::new(30.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let p = pal;
    let bg = if on {
        p.accent.gamma_multiply(0.22)
    } else if enabled && resp.is_pointer_button_down_on() {
        p.hover.gamma_multiply(1.8)
    } else if enabled && resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
    let col = if !enabled {
        p.tertiary
    } else if on {
        p.accent
    } else {
        p.text
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::new(16.0, theme::icon_family()), col);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, tip));
    resp.on_hover_text(tip)
}

/// Labelled button with an icon (used for the primary "Remove Background" action).
pub fn prominent_button(ui: &mut Ui, pal: Palette, glyph: char, label: &str, enabled: bool) -> Response {
    let font = FontId::new(13.5, egui::FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::WHITE);
    let size = Vec2::new(galley.size().x + 52.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let p = pal;
    let base = p.accent;
    let fill = if !enabled {
        p.hover
    } else if resp.is_pointer_button_down_on() {
        base.gamma_multiply(0.85)
    } else if resp.hovered() {
        base.gamma_multiply(1.08)
    } else {
        base
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    let fg = if enabled { Color32::WHITE } else { p.tertiary };
    ui.painter().text(egui::pos2(rect.min.x + 17.0, rect.center().y), Align2::CENTER_CENTER, glyph, FontId::new(15.0, theme::icon_family()), fg);
    ui.painter().galley(egui::pos2(rect.min.x + 32.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    resp
}

/// Outlined button with icon and optional label.
pub fn soft_button(ui: &mut Ui, pal: Palette, glyph: Option<char>, label: &str, enabled: bool) -> Response {
    let font = FontId::new(13.5, egui::FontFamily::Proportional);
    let p = pal;
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, p.text);
    let icon_w = if glyph.is_some() { 22.0 } else { 0.0 };
    let w = galley.size().x + icon_w + 20.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if enabled && resp.is_pointer_button_down_on() {
        p.hover.gamma_multiply(1.8)
    } else if enabled && resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(rect, CornerRadius::same(7), bg, Stroke::new(1.0, p.card_stroke), StrokeKind::Inside);
    let fg = if enabled { p.text } else { p.tertiary };
    let mut x = rect.min.x + 10.0;
    if let Some(g) = glyph {
        ui.painter().text(egui::pos2(x + 7.0, rect.center().y), Align2::CENTER_CENTER, g, FontId::new(15.0, theme::icon_family()), fg);
        x += 22.0;
    }
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    resp
}

/// Small push button for tool option panels.
pub fn small_button(ui: &mut Ui, pal: Palette, label: &str, enabled: bool) -> Response {
    soft_button(ui, pal, None, label, enabled)
}

pub fn primary_small(ui: &mut Ui, pal: Palette, label: &str, enabled: bool) -> Response {
    let p = pal;
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::new(13.5, egui::FontFamily::Proportional), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        p.hover
    } else if resp.is_pointer_button_down_on() {
        p.accent.gamma_multiply(0.85)
    } else if resp.hovered() {
        p.accent.gamma_multiply(1.08)
    } else {
        p.accent
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, if enabled { Color32::WHITE } else { p.tertiary });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    resp
}

/// Key-cap chip, e.g. the tool's shortcut in the options header.
pub fn key_cap(ui: &mut Ui, pal: Palette, text: &str) {
    let p = pal;
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::new(12.5, theme::semibold_family()), p.text);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 12.0, 20.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(5), p.hover, Stroke::new(0.5, p.card_stroke), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, p.text);
}

/// A labelled slider with a numeric read-out. Returns (changed, drag_stopped).
pub fn slider_row(ui: &mut Ui, pal: Palette, title: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, fmt: &dyn Fn(f64) -> String, step: Option<f64>) -> (bool, bool) {
    let mut changed = false;
    let mut stopped = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.horizontal(|ui| {
            ui.label(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(fmt(*value)).monospace().color(pal.secondary));
            });
        });
        let w = ui.available_width();
        ui.spacing_mut().slider_width = w - 4.0;
        let mut s = egui::Slider::new(value, range).show_value(false).trailing_fill(true);
        if let Some(st) = step {
            s = s.step_by(st);
        }
        let r = ui.add(s);
        changed = r.changed();
        stopped = r.drag_stopped() || (r.changed() && !r.dragged() && !r.is_pointer_button_down_on());
    });
    (changed, stopped)
}

pub fn pct(v: f64) -> String {
    format!("{}%", (v * 100.0).round() as i64)
}

pub fn px(v: f64) -> String {
    format!("{} px", v.round() as i64)
}

pub fn color_row(ui: &mut Ui, title: &str, rgba: &mut [f32; 4]) {
    ui.horizontal(|ui| {
        if !title.is_empty() {
            ui.label(title);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut c = Color32::from_rgba_unmultiplied((rgba[0] * 255.0).round() as u8, (rgba[1] * 255.0).round() as u8, (rgba[2] * 255.0).round() as u8, (rgba[3] * 255.0).round() as u8);
            if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend).changed() {
                let [r, g, b, a] = c.to_srgba_unmultiplied();
                *rgba = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0];
            }
        });
    });
}

/// Segmented icon picker; returns true when the selection changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, pal: Palette, items: &[(T, char, &str)], value: &mut T) -> bool {
    let mut changed = false;
    let p = pal;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (v, g, tip) in items {
            let sel = *value == *v;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(34.0, 26.0), Sense::click());
            let bg = if sel {
                p.accent.gamma_multiply(0.25)
            } else if resp.hovered() {
                p.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect(rect, CornerRadius::same(6), bg, Stroke::new(1.0, if sel { p.accent } else { p.card_stroke }), StrokeKind::Inside);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, *g, FontId::new(15.0, theme::icon_family()), if sel { p.accent } else { p.text });
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, sel, *tip));
            if resp.on_hover_text(*tip).clicked() && !sel {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

pub fn toggle_chip(ui: &mut Ui, pal: Palette, label: &str, on: &mut bool) -> bool {
    let p = pal;
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::new(13.5, egui::FontFamily::Proportional), p.text);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 22.0, 26.0), Sense::click());
    let bg = if *on { p.accent.gamma_multiply(0.25) } else if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
    ui.painter().rect(rect, CornerRadius::same(7), bg, Stroke::new(1.0, if *on { p.accent } else { p.card_stroke }), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, if *on { p.accent } else { p.text });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CheckBox, true, *on, label));
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}

pub fn secondary(pal: Palette, s: impl Into<String>) -> egui::RichText {
    egui::RichText::new(s.into()).color(pal.secondary)
}

pub fn small(pal: Palette, s: impl Into<String>) -> egui::RichText {
    egui::RichText::new(s.into()).size(11.5).color(pal.secondary)
}

#[allow(dead_code)]
pub fn icon_label(ui: &mut Ui, glyph: char, size: f32, color: Color32) -> Rect {
    ui.label(icon(glyph, size).color(color)).rect
}

/// AI-only tooltip: foreground area avoids panel clipping; placement flips below
/// near the top of the viewport. Uses the current theme and respects reduced motion.
pub fn ai_removal_tooltip(ctx: &egui::Context, button: &Response, reduce_motion: bool) {
    const TEXT: &str = "Slower, but better for complex images";
    let id = button.id.with("ai_tooltip");
    let t = ctx.animate_bool_with_time(id, button.hovered(), if reduce_motion { 0.0 } else { 0.15 });
    if t <= 0.0 { return; }
    let screen = ctx.viewport_rect();
    let size = Vec2::new(258.0, 32.0);
    let above = button.rect.top() - size.y - 8.0;
    let y = if above >= screen.top() + 4.0 { above } else { button.rect.bottom() + 8.0 };
    let x = (button.rect.center().x - size.x / 2.0).clamp(screen.left() + 4.0, (screen.right() - size.x - 4.0).max(screen.left() + 4.0));
    let drift = if reduce_motion { 0.0 } else { 3.0 * (1.0 - t) };
    egui::Area::new(id.with("area")).order(egui::Order::Tooltip)
        .fixed_pos(egui::pos2(x, y + drift)).interactable(false).show(ctx, |ui| {
            ui.set_opacity(t);
            egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(8)).show(ui, |ui| {
                ui.label(egui::RichText::new(TEXT).size(12.0));
            });
        });
}
