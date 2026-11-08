//! The advanced-tools sidebar: tool grid, per-tool options and the
//! collapsible action sections (port of `ToolSidebar.swift`).

use super::widgets::*;
use crate::app::{Action, App, PanelKind, SelMode, Tool};
use crate::icons;
use crate::models::RemovalMode;
use crate::theme::{self, icon};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Layout, Sense, Stroke, Ui, Vec2};
use serde_json::json;

pub fn tool_sidebar(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for (name, tools) in Tool::GROUPS {
                ui.add_space(4.0);
                section_header(ui, p, name);
                let cols = 4;
                let w = (ui.available_width() - 6.0 * (cols as f32 - 1.0)) / cols as f32;
                egui::Grid::new(format!("tools-{name}")).spacing(Vec2::new(6.0, 6.0)).show(ui, |ui| {
                    for (i, t) in tools.iter().enumerate() {
                        tool_button(app, ui, *t, w);
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
            }
            ui.add_space(8.0);
            card_frame(p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                tool_options(app, ui);
            });
            ui.add_space(8.0);
            actions_section(app, ui);
        });
    });
}

fn tool_button(app: &mut App, ui: &mut Ui, tool: Tool, width: f32) {
    let p = app.palette;
    let selected = app.tool == tool;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 36.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered() && !selected, 0.12);
    let sel = ui.ctx().animate_bool_with_time(resp.id.with("s"), selected, 0.18);
    let down = resp.is_pointer_button_down_on();
    let r = if down { rect.shrink(1.5) } else { rect };
    if hover > 0.0 {
        ui.painter().rect_filled(r, CornerRadius::same(9), p.hover.gamma_multiply(hover));
    }
    if sel > 0.0 {
        let c = p.accent;
        ui.painter().add(egui::epaint::Shadow { offset: [0, 2], blur: 6, spread: 0, color: c.gamma_multiply(0.35 * sel) }.as_shape(r, CornerRadius::same(9)));
        ui.painter().rect_filled(r, CornerRadius::same(9), c.gamma_multiply(0.82 * sel + 0.18 * sel));
        // vertical gradient: lighter at the top (accent → accent × 0.82)
        let top = egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.center().y));
        ui.painter().rect_filled(top, CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 }, Color32::from_white_alpha((22.0 * sel) as u8));
    }
    let col = if selected { Color32::WHITE } else { p.text };
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, tool.icon(), FontId::new(17.0, theme::icon_family()), col);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, tool.title()));
    if resp.on_hover_text(format!("{} ({})", tool.title(), tool.shortcut_char())).clicked() {
        app.set_tool(tool);
    }
}

fn font_picker(ui: &mut Ui, fonts: &mut Option<Vec<String>>, name: &mut String) {
    let list = fonts.get_or_insert_with(crate::text::families);
    egui::ComboBox::from_id_salt("font-picker").selected_text(name.clone()).width(ui.available_width().min(220.0)).show_ui(ui, |ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for f in list.iter() {
                ui.selectable_value(name, f.clone(), f);
            }
        });
    });
}

fn selection_mode_picker(app: &mut App, ui: &mut Ui) {
    let items: Vec<(SelMode, char, &str)> = SelMode::ALL.iter().map(|m| (*m, m.icon(), m.title())).collect();
    let mut v = app.sel_mode;
    if segmented(ui, app.palette, &items, &mut v) {
        app.sel_mode = v;
    }
}

fn color_tolerance(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let t = &mut app.settings.tools;
    slider_row(ui, p, "Tolerance (ΔE)", &mut t.tolerance, 0.0..=100.0, &|v| format!("{:.0}", v), None);
    ui.checkbox(&mut t.contiguous, "Contiguous").on_hover_text("Only affect connected pixels. Turn off to affect the colour everywhere in the layer.");
    ui.checkbox(&mut t.antialias, "Anti-alias edges");
}

fn tool_options(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let tool = app.tool;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.label(icon(tool.icon(), 16.0));
            ui.label(egui::RichText::new(tool.title()).font(FontId::new(15.0, theme::semibold_family())));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| key_cap(ui, p, &tool.shortcut_char().to_string()));
        });
        ui.label(small(p, tool.help()));
        match tool {
            Tool::RectSelect | Tool::EllipseSelect | Tool::Lasso | Tool::PolygonLasso => {
                selection_mode_picker(app, ui);
                slider_row(ui, p, "Feather", &mut app.settings.tools.feather, 0.0..=50.0, &px, None);
            }
            Tool::MagicWand => {
                selection_mode_picker(app, ui);
                color_tolerance(app, ui);
                ui.checkbox(&mut app.settings.tools.sample_all, "Sample all layers");
                slider_row(ui, p, "Feather", &mut app.settings.tools.feather, 0.0..=50.0, &px, None);
            }
            Tool::QuickSelect => {
                let t = &mut app.settings.tools;
                slider_row(ui, p, "Brush size", &mut t.quick_size, 2.0..=400.0, &px, None);
                slider_row(ui, p, "Edge sensitivity", &mut t.quick_edge, 0.0..=0.95, &pct, None);
                ui.label(small(p, "Hold Alt to subtract. Use [ and ] to resize."));
            }
            Tool::MagicEraser => {
                color_tolerance(app, ui);
                slider_row(ui, p, "Edge feather", &mut app.settings.tools.erase_feather, 0.0..=20.0, &px, None);
            }
            Tool::Eraser => {
                let t = &mut app.settings.tools;
                slider_row(ui, p, "Size", &mut t.eraser_size, 1.0..=800.0, &px, None);
                slider_row(ui, p, "Hardness", &mut t.eraser_hardness, 0.0..=1.0, &pct, None);
                slider_row(ui, p, "Strength", &mut t.eraser_strength, 0.05..=1.0, &pct, None);
            }
            Tool::Restore => {
                let t = &mut app.settings.tools;
                slider_row(ui, p, "Size", &mut t.restore_size, 1.0..=800.0, &px, None);
                slider_row(ui, p, "Hardness", &mut t.restore_hardness, 0.0..=1.0, &pct, None);
                slider_row(ui, p, "Restore strength", &mut t.restore_strength, 0.05..=1.0, &pct, None);
            }
            Tool::Brush => {
                let t = &mut app.settings.tools;
                color_row(ui, "Colour", &mut t.color);
                slider_row(ui, p, "Size", &mut t.brush_size, 1.0..=800.0, &px, None);
                slider_row(ui, p, "Hardness", &mut t.brush_hardness, 0.0..=1.0, &pct, None);
                slider_row(ui, p, "Opacity", &mut t.brush_opacity, 0.05..=1.0, &pct, None);
            }
            Tool::Fill => {
                color_row(ui, "Colour", &mut app.settings.tools.color);
                color_tolerance(app, ui);
                slider_row(ui, p, "Opacity", &mut app.settings.tools.fill_opacity, 0.05..=1.0, &pct, None);
                let has = app.state().has_selection;
                if small_button(ui, p, "Fill Selection", has).clicked() {
                    let (c, o) = (app.settings.tools.color_rgba(), app.settings.tools.fill_opacity);
                    app.run("fill", json!({"selection": true, "color": c, "opacity": o}));
                }
            }
            Tool::Eyedropper => color_row(ui, "Current colour", &mut app.settings.tools.color),
            Tool::Text => {
                color_row(ui, "Colour", &mut app.settings.tools.color);
                font_picker(ui, &mut app.font_list, &mut app.settings.tools.font_name);
                slider_row(ui, p, "Size", &mut app.settings.tools.font_size, 8.0..=600.0, &|v| format!("{} pt", v as i64), None);
                ui.horizontal(|ui| {
                    toggle_chip(ui, p, "Bold", &mut app.settings.tools.bold);
                    toggle_chip(ui, p, "Italic", &mut app.settings.tools.italic);
                });
                if let Some(l) = app.state().active_layer() {
                    if l.is_text {
                        let id = l.id;
                        if small_button(ui, p, "Edit Selected Text…", true).clicked() {
                            app.open_text_sheet(Some(id), (0.0, 0.0));
                        }
                    }
                }
            }
            Tool::Shape => {
                let kinds: [(&str, char, &str); 4] = [("rect", icons::SQUARE, "Rectangle"), ("ellipse", icons::CIRCLE, "Ellipse"), ("line", icons::MINUS, "Line"), ("arrow", icons::ARROW_UP_RIGHT, "Arrow")];
                let k = app.settings.tools.shape_kind.clone();
                let mut idx = kinds.iter().position(|(id, _, _)| *id == k).unwrap_or(0);
                let items2: Vec<(usize, char, &str)> = kinds.iter().enumerate().map(|(i, (_, g, t))| (i, *g, *t)).collect();
                if segmented(ui, p, &items2, &mut idx) {
                    app.settings.tools.shape_kind = kinds[idx].0.to_string();
                }
                let t = &mut app.settings.tools;
                color_row(ui, "Stroke", &mut t.color);
                slider_row(ui, p, "Stroke width", &mut t.shape_stroke, 0.0..=80.0, &px, None);
                ui.checkbox(&mut t.shape_filled, "Fill");
                if t.shape_filled {
                    color_row(ui, "Fill colour", &mut t.fill_color);
                }
                if t.shape_kind == "rect" {
                    slider_row(ui, p, "Corner radius", &mut t.shape_corner, 0.0..=200.0, &px, None);
                }
            }
            Tool::Crop => {
                ui.horizontal(|ui| {
                    if primary_small(ui, p, "Apply Crop", true).clicked() {
                        app.apply_crop();
                    }
                    if small_button(ui, p, "Trim Transparent", true).clicked() {
                        app.perform(Action::Trim);
                    }
                });
            }
            Tool::Move => {
                if let Some(l) = app.state().active_layer().cloned() {
                    transform_fields(app, ui, &l);
                }
            }
            Tool::Hand | Tool::Zoom => {
                ui.horizontal(|ui| {
                    if small_button(ui, p, "Fit", true).clicked() {
                        app.canvas.zoom_to_fit(true);
                    }
                    if small_button(ui, p, "100%", true).clicked() {
                        app.canvas.set_zoom_device(1.0, None, true);
                    }
                });
            }
        }
    });
}

fn transform_fields(app: &mut App, ui: &mut Ui, layer: &crate::engine::LayerInfo) {
    let p = app.palette;
    let t = layer.transform;
    egui::Grid::new("transform-grid").num_columns(2).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        ui.label(secondary(p, "Position"));
        ui.label(egui::RichText::new(format!("{}, {}", (t.x - layer.width as f64 * t.scale_x / 2.0) as i64, (t.y - layer.height as f64 * t.scale_y / 2.0) as i64)).monospace());
        ui.end_row();
        ui.label(secondary(p, "Scale"));
        ui.label(egui::RichText::new(format!("{:.0}%", t.scale_x * 100.0)).monospace());
        ui.end_row();
        ui.label(secondary(p, "Rotation"));
        ui.label(egui::RichText::new(format!("{:.1}°", t.rotation * 180.0 / std::f64::consts::PI)).monospace());
        ui.end_row();
    });
    ui.horizontal(|ui| {
        if small_button(ui, p, "Reset", true).clicked() {
            app.run("reset_transform", json!({}));
        }
        if icon_button(ui, p, icons::FLIP_HORIZONTAL_2, "Flip layer horizontally", true, false).clicked() {
            app.run("flip_layer", json!({"axis": "h"}));
        }
        if icon_button(ui, p, icons::FLIP_VERTICAL_2, "Flip layer vertically", true, false).clicked() {
            app.run("flip_layer", json!({"axis": "v"}));
        }
        if icon_button(ui, p, icons::ROTATE_CW, "Rotate layer 90°", true, false).clicked() {
            let mut nt = t;
            nt.rotation += std::f64::consts::FRAC_PI_2;
            app.run("set_transform", json!({"id": layer.id, "transform": nt.json(), "label": "Rotate Layer"}));
        }
    });
    ui.label(small(p, "Shift constrains movement and snaps rotation to 15°. Arrow keys nudge."));
}

fn disclosure(ui: &mut Ui, p: crate::theme::Palette, title: &str, open: &mut bool, force: bool, body: impl FnOnce(&mut Ui)) {
    let want = *open || force;
    let id = ui.make_persistent_id(("disc", title));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
    let t = ui.ctx().animate_bool_with_time(id, want, 0.15);
    ui.painter().text(egui::pos2(rect.min.x + 6.0, rect.center().y), egui::Align2::CENTER_CENTER, icons::CHEVRON_RIGHT, FontId::new(12.0, theme::icon_family()), p.secondary);
    // rotate the chevron by drawing the down variant once more than half open
    if t > 0.5 {
        ui.painter().rect_filled(egui::Rect::from_center_size(egui::pos2(rect.min.x + 6.0, rect.center().y), Vec2::splat(14.0)), 0.0, p.sidebar);
        ui.painter().text(egui::pos2(rect.min.x + 6.0, rect.center().y), egui::Align2::CENTER_CENTER, icons::CHEVRON_DOWN, FontId::new(12.0, theme::icon_family()), p.secondary);
    }
    ui.painter().text(egui::pos2(rect.min.x + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, title.to_uppercase(), FontId::new(10.5, theme::semibold_family()), p.secondary);
    if resp.clicked() {
        *open = !want;
    }
    if want {
        ui.add_space(4.0);
        body(ui);
        ui.add_space(6.0);
    }
}

fn actions_section(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let mut bg = app.settings.sec_background;
    let mut sel = app.settings.sec_selection;
    let mut img = app.settings.sec_image;
    let mut adj = app.settings.sec_adjust;
    let force_refine = app.panel == PanelKind::Refine;
    let force_adjust = app.panel == PanelKind::Adjust;

    disclosure(ui, p, "Background & Edges", &mut bg, force_refine, |ui| {
        ui.horizontal(|ui| {
            let busy = app.is_removing();
            if primary_small(ui, p, "Best Quality", !busy).clicked() {
                app.perform(Action::RemoveBg(RemovalMode::Best));
            }
            if small_button(ui, p, "Fast", !busy).clicked() {
                app.perform(Action::RemoveBg(RemovalMode::Fast));
            }
        });
        refine_panel(app, ui);
        ui.horizontal(|ui| {
            if small_button(ui, p, "Restore All", true).on_hover_text("Bring back every erased or removed pixel on this layer").clicked() {
                app.run("clear_mask", json!({}));
            }
            if small_button(ui, p, "Invert Mask", true).clicked() {
                app.run("invert_mask", json!({}));
            }
        });
    });
    if !bg && !force_refine {
        if app.panel == PanelKind::Refine {
            app.panel = PanelKind::None;
        }
    }
    let has_sel = app.state().has_selection;
    disclosure(ui, p, "Selection", &mut sel, false, |ui| {
        ui.horizontal(|ui| {
            if small_button(ui, p, "All", true).clicked() {
                app.perform(Action::SelectAll);
            }
            if small_button(ui, p, "None", has_sel).clicked() {
                app.perform(Action::Deselect);
            }
            if small_button(ui, p, "Invert", true).clicked() {
                app.perform(Action::InvertSelection);
            }
        });
        ui.horizontal(|ui| {
            if small_button(ui, p, "Delete Pixels", has_sel).clicked() {
                app.perform(Action::DeleteSelection);
            }
            if small_button(ui, p, "Keep Only", has_sel).clicked() {
                app.run("mask_from_selection", json!({}));
            }
        });
        ui.horizontal(|ui| {
            for (label, cmd, amount) in [("Grow", "grow_selection", 3), ("Shrink", "grow_selection", -3), ("Smooth", "smooth_selection", 2), ("Feather", "feather_selection", 3)] {
                if small_button(ui, p, label, has_sel).clicked() {
                    app.run(cmd, json!({"amount": amount}));
                }
            }
        });
        if small_button(ui, p, "Crop to Selection", has_sel).clicked() {
            app.perform(Action::CropToSelection);
        }
    });
    disclosure(ui, p, "Image", &mut img, false, |ui| {
        ui.horizontal(|ui| {
            if small_button(ui, p, "Resize…", true).clicked() {
                app.perform(Action::ResizeImage);
            }
            if small_button(ui, p, "Canvas…", true).clicked() {
                app.perform(Action::CanvasSize);
            }
            if small_button(ui, p, "Trim", true).clicked() {
                app.perform(Action::Trim);
            }
        });
        ui.horizontal(|ui| {
            if icon_button(ui, p, icons::ROTATE_CCW, "Rotate 90° left", true, false).clicked() {
                app.perform(Action::Rotate(false));
            }
            if icon_button(ui, p, icons::ROTATE_CW, "Rotate 90° right", true, false).clicked() {
                app.perform(Action::Rotate(true));
            }
            if icon_button(ui, p, icons::ARROW_LEFT_RIGHT, "Flip horizontal", true, false).clicked() {
                app.perform(Action::Flip("h"));
            }
            if icon_button(ui, p, icons::ARROW_UP_DOWN, "Flip vertical", true, false).clicked() {
                app.perform(Action::Flip("v"));
            }
        });
    });
    disclosure(ui, p, "Adjustments", &mut adj, force_adjust, |ui| adjustments_panel(app, ui));
    app.settings.sec_background = bg;
    app.settings.sec_selection = sel;
    app.settings.sec_image = img;
    app.settings.sec_adjust = adj;
}

fn refine_panel(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    egui::Frame::new().fill(p.hover).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(egui::RichText::new("Refine Edges").font(FontId::new(13.5, theme::semibold_family())));
        let mut stop = false;
        let r = &mut app.refine;
        stop |= slider_row(ui, p, "Edge detection radius", &mut r.radius, 0.0..=30.0, &px, None).1;
        stop |= slider_row(ui, p, "Smooth", &mut r.smooth, 0.0..=10.0, &|v| format!("{v:.1}"), None).1;
        stop |= slider_row(ui, p, "Feather", &mut r.feather, 0.0..=20.0, &|v| format!("{v:.1} px"), None).1;
        stop |= slider_row(ui, p, "Contrast", &mut r.contrast, 0.0..=1.0, &pct, None).1;
        stop |= slider_row(ui, p, "Shift edge", &mut r.shift, -20.0..=20.0, &|v| format!("{} px", v as i64), Some(1.0)).1;
        if ui.checkbox(&mut r.decontaminate, "Decontaminate colours").on_hover_text("Removes background colour fringing from semi-transparent edges").changed() {
            stop = true;
        }
        if stop {
            refine_preview(app);
        }
        ui.horizontal(|ui| {
            let ident = app.refine.is_identity();
            if primary_small(ui, p, "Apply", !ident).clicked() {
                let params = app.refine.params();
                app.run("refine_commit", params);
                app.refine = Default::default();
            }
            if small_button(ui, p, "Cancel", app.refine.active).clicked() {
                app.run("preview_cancel", json!({}));
                app.refine = Default::default();
            }
        });
    });
}

fn refine_preview(app: &mut App) {
    if app.refine.is_identity() {
        app.run("preview_cancel", json!({}));
        app.refine.active = false;
        return;
    }
    app.refine.active = true;
    let params = app.refine.params();
    app.run("refine_preview", params);
}

const ADJUST_ROWS: [(&str, &str, f64, f64); 10] = [
    ("exposure", "Exposure", -3.0, 3.0),
    ("brightness", "Brightness", -1.0, 1.0),
    ("contrast", "Contrast", -1.0, 1.0),
    ("highlights", "Highlights", -1.0, 1.0),
    ("shadows", "Shadows", -1.0, 1.0),
    ("saturation", "Saturation", -1.0, 1.0),
    ("temperature", "Temperature", -1.0, 1.0),
    ("tint", "Tint", -1.0, 1.0),
    ("hue", "Hue", -180.0, 180.0),
    ("sharpen", "Sharpen", 0.0, 1.0),
];

fn adjustments_panel(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut preview = false;
    for (key, title, lo, hi) in ADJUST_ROWS {
        let mut v = app.adjust.values.get(key).copied().unwrap_or(0.0);
        let (_, stopped) = slider_row(ui, p, title, &mut v, lo..=hi, &move |x| if key == "hue" { format!("{}°", x as i64) } else { format!("{x:+.2}") }, None);
        app.adjust.values.insert(key, v);
        preview |= stopped;
    }
    ui.horizontal(|ui| {
        preview |= ui.checkbox(&mut app.adjust.invert, "Invert").changed();
        preview |= ui.checkbox(&mut app.adjust.gray, "Greyscale").changed();
    });
    if preview {
        let payload = app.adjust.payload();
        app.run("adjust_preview", payload);
    }
    ui.horizontal(|ui| {
        if primary_small(ui, p, "Apply", true).clicked() {
            let payload = app.adjust.payload();
            app.run("adjust_commit", payload);
            app.adjust = Default::default();
        }
        if small_button(ui, p, "Reset", true).clicked() {
            app.run("preview_cancel", json!({}));
            app.adjust = Default::default();
        }
        if small_button(ui, p, "Auto Levels", true).clicked() {
            app.run("auto_levels", json!({}));
        }
    });
}

#[allow(dead_code)]
fn _unused(_: Stroke, _: Align) {}
