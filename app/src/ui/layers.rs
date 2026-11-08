//! Layers inspector (port of `LayersPanel.swift`).

use super::widgets::*;
use crate::app::{Action, App};
use crate::engine::LayerInfo;
use crate::icons;
use crate::theme::{self, icon};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use serde_json::json;

pub fn layers_panel(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(icon(icons::LAYERS, 16.0));
        ui.label(egui::RichText::new("Layers").font(FontId::new(14.5, theme::semibold_family())));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(app.state().layers.len().to_string()).monospace().color(p.secondary));
        });
    });
    if let Some(active) = app.state().active_layer().cloned() {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(secondary(p, "Opacity"));
            let mut v = active.opacity;
            ui.spacing_mut().slider_width = (ui.available_width() - 56.0).max(60.0);
            let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false).trailing_fill(true));
            if r.changed() {
                let begin = !app.opacity_editing;
                app.opacity_editing = true;
                app.set_opacity(active.id, v, begin);
            }
            if r.drag_stopped() || (!r.dragged() && !r.is_pointer_button_down_on()) {
                app.opacity_editing = false;
            }
            ui.label(egui::RichText::new(format!("{}%", (active.opacity * 100.0).round() as i64)).monospace());
        });
    }
    ui.add_space(2.0);

    let footer_h = 40.0;
    let list_h = (ui.available_height() - footer_h).max(60.0);
    egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let layers: Vec<LayerInfo> = app.state().layers.iter().rev().cloned().collect();
        let n = layers.len();
        let mut row_rects: Vec<Rect> = vec![];
        for (i, layer) in layers.iter().enumerate() {
            let r = layer_row(app, ui, layer, i, &layers);
            row_rects.push(r);
        }
        // drag-to-reorder: draw an insertion line and perform the move on release
        if let Some(dragged) = app.layer_drag {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                let mut to = n;
                for (i, r) in row_rects.iter().enumerate() {
                    if pos.y < r.center().y {
                        to = i;
                        break;
                    }
                }
                let y = if to < n { row_rects[to].min.y - 1.0 } else { row_rects.last().map(|r| r.max.y + 1.0).unwrap_or(0.0) };
                let x0 = row_rects.first().map(|r| r.min.x).unwrap_or(0.0);
                let x1 = row_rects.first().map(|r| r.max.x).unwrap_or(0.0);
                ui.painter().line_segment([Pos2::new(x0 + 6.0, y), Pos2::new(x1 - 6.0, y)], Stroke::new(2.0, p.accent));
                if !ui.ctx().input(|i| i.pointer.any_down()) {
                    let from = layers.iter().position(|l| l.id == dragged).unwrap_or(0);
                    let d = if to > from { to - 1 } else { to };
                    let dst = n - 1 - d;
                    app.layer_drag = None;
                    if d != from {
                        app.run("move_layer", json!({"id": dragged, "index": dst.min(n - 1)}));
                    }
                }
            } else {
                app.layer_drag = None;
            }
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        let multi = app.state().layers.len() > 1;
        if icon_button(ui, p, icons::COPY_PLUS, "Paste as New Layer (Ctrl+V)", true, false).clicked() {
            app.perform(Action::Paste);
        }
        if icon_button(ui, p, icons::COPY, "Duplicate Layer (Ctrl+J)", true, false).clicked() {
            app.perform(Action::DuplicateLayer);
        }
        if icon_button(ui, p, icons::ARROW_DOWN_TO_LINE, "Merge Down / Merge Selected (Ctrl+E)", multi, false).clicked() {
            app.perform(Action::MergeDown);
        }
        if icon_button(ui, p, icons::LAYERS_2, "Flatten Image", true, false).clicked() {
            app.perform(Action::Flatten);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, p, icons::TRASH_2, "Delete Layer", multi, false).clicked() {
                app.perform(Action::DeleteLayer);
            }
            if icon_button(ui, p, icons::CHEVRON_DOWN, "Move Layer Down", true, false).clicked() {
                app.perform(Action::LayerOrder("down"));
            }
            if icon_button(ui, p, icons::CHEVRON_UP, "Move Layer Up", true, false).clicked() {
                app.perform(Action::LayerOrder("up"));
            }
        });
    });
}

fn layer_row(app: &mut App, ui: &mut Ui, layer: &LayerInfo, index: usize, all: &[LayerInfo]) -> Rect {
    let p = app.palette;
    let selected = app.selected_layers.contains(&layer.id) || (app.selected_layers.is_empty() && app.state().active == Some(layer.id));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click_and_drag());
    let bg = if selected {
        p.accent.gamma_multiply(if p.dark { 0.38 } else { 0.22 })
    } else if resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, format!("Layer {}{}", layer.name, if layer.visible { "" } else { " (hidden)" })));
    let row = rect.shrink2(Vec2::new(6.0, 0.0));
    ui.painter().rect_filled(row, CornerRadius::same(8), bg);
    let dim = if layer.visible { 1.0 } else { 0.55 };

    // visibility toggle
    let eye = Rect::from_center_size(Pos2::new(row.min.x + 16.0, row.center().y), Vec2::new(22.0, 22.0));
    let eye_resp = ui.interact(eye, resp.id.with("eye"), Sense::click());
    ui.painter().text(eye.center(), egui::Align2::CENTER_CENTER, if layer.visible { icons::EYE } else { icons::EYE_OFF }, FontId::new(15.0, theme::icon_family()), if layer.visible { p.secondary } else { p.tertiary });
    if eye_resp.on_hover_text(if layer.visible { "Hide layer" } else { "Show layer" }).clicked() {
        app.run("set_layer", json!({"id": layer.id, "visible": !layer.visible}));
    }

    // thumbnail
    let th = Rect::from_min_size(Pos2::new(row.min.x + 34.0, row.center().y - 19.0), Vec2::splat(38.0));
    ui.painter().rect_filled(th, CornerRadius::same(7), Color32::from_gray(235));
    checker_swatch(ui, th);
    if let Some(tex) = app.thumbnail(layer) {
        let (iw, ih) = (layer.width.max(1) as f32, layer.height.max(1) as f32);
        let s = (38.0 / iw).min(38.0 / ih);
        let r = Rect::from_center_size(th.center(), Vec2::new(iw * s, ih * s));
        ui.painter().image(tex, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(dim));
    }
    ui.painter().rect_stroke(th, CornerRadius::same(7), Stroke::new(1.0, p.card_stroke), StrokeKind::Inside);

    // name + badges
    let name_pos = Pos2::new(th.max.x + 8.0, row.min.y + 8.0);
    let renaming = app.renaming.as_ref().filter(|(id, _)| *id == layer.id).is_some();
    if renaming {
        let name_rect = Rect::from_min_size(name_pos - Vec2::new(0.0, 2.0), Vec2::new(row.max.x - name_pos.x - 24.0, 20.0));
        let mut commit = false;
        let mut cancel = false;
        if let Some((_, draft)) = app.renaming.as_mut() {
            let r = ui.put(name_rect, egui::TextEdit::singleline(draft).frame(egui::Frame::NONE));
            r.request_focus();
            if r.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                } else {
                    commit = true;
                }
            }
        }
        if commit {
            if let Some((id, name)) = app.renaming.take() {
                app.run("set_layer", json!({"id": id, "name": name}));
            }
        }
        if cancel {
            app.renaming = None;
        }
    } else {
        let galley = ui.painter().layout(layer.name.clone(), FontId::new(13.5, egui::FontFamily::Proportional), p.text.gamma_multiply(dim), row.max.x - name_pos.x - 28.0);
        ui.painter().with_clip_rect(Rect::from_min_max(name_pos, Pos2::new(row.max.x - 24.0, name_pos.y + 18.0))).galley(name_pos, galley, p.text);
    }
    let mut bx = name_pos.x;
    let by = row.min.y + 28.0;
    let col = p.secondary.gamma_multiply(dim);
    if layer.is_text {
        ui.painter().text(Pos2::new(bx + 6.0, by + 7.0), egui::Align2::CENTER_CENTER, icons::TYPE, FontId::new(12.0, theme::icon_family()), col);
        bx += 16.0;
    }
    if layer.has_mask {
        ui.painter().text(Pos2::new(bx + 6.0, by + 7.0), egui::Align2::CENTER_CENTER, icons::CONTRAST, FontId::new(12.0, theme::icon_family()), col);
        bx += 16.0;
    }
    if layer.opacity < 1.0 {
        ui.painter().text(Pos2::new(bx, by + 7.0), egui::Align2::LEFT_CENTER, format!("{}%", (layer.opacity * 100.0) as i64), FontId::new(11.0, egui::FontFamily::Proportional), col);
    }
    if layer.locked {
        ui.painter().text(Pos2::new(row.max.x - 14.0, row.center().y), egui::Align2::CENTER_CENTER, icons::LOCK, FontId::new(13.0, theme::icon_family()), p.secondary);
    }

    // interactions
    if resp.double_clicked() && !renaming {
        app.renaming = Some((layer.id, layer.name.clone()));
    } else if resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        if m.shift && app.state().active.is_some() {
            // range select from the active layer
            let active = app.state().active.unwrap();
            let a = all.iter().position(|l| l.id == active).unwrap_or(index);
            let (lo, hi) = (a.min(index), a.max(index));
            app.selected_layers = all[lo..=hi].iter().map(|l| l.id).collect();
            app.run("select_layer", json!({"id": layer.id}));
        } else if m.command || m.ctrl {
            app.select_layer(layer.id, true);
        } else {
            app.select_layer(layer.id, false);
        }
    }
    if resp.drag_started() {
        app.layer_drag = Some(layer.id);
    }
    let ctx_layer = layer.clone();
    resp.context_menu(|ui| layer_menu(app, ui, &ctx_layer));
    rect
}

fn layer_menu(app: &mut App, ui: &mut Ui, layer: &LayerInfo) {
    let n = app.state().layers.len();
    if ui.button("Rename").clicked() {
        app.renaming = Some((layer.id, layer.name.clone()));
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        app.run("duplicate_layer", json!({"id": layer.id}));
        ui.close();
    }
    if ui.button(if layer.visible { "Hide" } else { "Show" }).clicked() {
        app.run("set_layer", json!({"id": layer.id, "visible": !layer.visible}));
        ui.close();
    }
    if ui.button(if layer.locked { "Unlock" } else { "Lock" }).clicked() {
        app.run("set_layer", json!({"id": layer.id, "locked": !layer.locked}));
        ui.close();
    }
    ui.separator();
    if ui.button("Merge Down").clicked() {
        app.run("merge_down", json!({"id": layer.id}));
        ui.close();
    }
    if app.selected_layers.len() >= 2 && ui.button("Merge Selected Layers").clicked() {
        app.merge_selected_or_down();
        ui.close();
    }
    if ui.button("Flatten Image").clicked() {
        app.run("flatten", json!({}));
        ui.close();
    }
    ui.separator();
    if layer.has_mask {
        if ui.button("Apply Mask").clicked() {
            app.run("select_layer", json!({"id": layer.id}));
            app.run("apply_mask", json!({}));
            ui.close();
        }
        if ui.button("Restore All Pixels").clicked() {
            app.run("select_layer", json!({"id": layer.id}));
            app.run("clear_mask", json!({}));
            ui.close();
        }
    }
    if layer.is_text {
        if ui.button("Edit Text…").clicked() {
            app.open_text_sheet(Some(layer.id), (0.0, 0.0));
            ui.close();
        }
        if ui.button("Rasterize").clicked() {
            app.run("select_layer", json!({"id": layer.id}));
            app.run("rasterize_layer", json!({}));
            ui.close();
        }
    }
    ui.separator();
    if ui.add_enabled(n > 1, egui::Button::new(egui::RichText::new("Delete").color(Color32::from_rgb(230, 70, 60)))).clicked() {
        app.run("delete_layer", json!({"id": layer.id}));
        ui.close();
    }
}

fn checker_swatch(ui: &mut Ui, r: Rect) {
    let c = 5.0;
    let painter = ui.painter().with_clip_rect(r);
    painter.rect_filled(r, 0.0, Color32::from_gray(235));
    let mut y = r.min.y;
    let mut row = 0;
    while y < r.max.y {
        let mut x = r.min.x + if row % 2 == 0 { 0.0 } else { c };
        while x < r.max.x {
            painter.rect_filled(Rect::from_min_size(Pos2::new(x, y), Vec2::splat(c)), 0.0, Color32::from_gray(199));
            x += c * 2.0;
        }
        y += c;
        row += 1;
    }
}
