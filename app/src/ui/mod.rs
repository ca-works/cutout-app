//! Window layout: menu bar, toolbar, tool sidebar, canvas, layers inspector,
//! status bar, toasts and sheets — the structure of the macOS `ContentView`.

pub mod layers;
pub mod sheets;
pub mod sidebar;
pub mod widgets;

use crate::app::{shortcuts, Action, App, CopyAction, PanelKind};
use crate::codec::Format;
use crate::icons;
use crate::models::{self, RemovalMode};
use crate::settings::Appearance;
use crate::theme::{self, icon};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Layout, Panel, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2};
use widgets::*;

pub fn root(app: &mut App, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    let ctx = ui.ctx().clone();
    let p = app.palette;
    let empty = app.is_empty();

    // window title
    let title = if empty { "CutOut".to_string() } else { format!("{} — CutOut", app.suggested_name()) };
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Title(title));

    let chrome = egui::Frame::new().fill(p.chrome).inner_margin(egui::Margin::symmetric(8, 0));
    Panel::top("menubar").exact_size(28.0).frame(chrome).show_separator_line(false).show(ui, |ui| menubar(app, ui));
    Panel::top("toolbar").exact_size(52.0).frame(egui::Frame::new().fill(p.chrome).inner_margin(egui::Margin::symmetric(10, 0))).show(ui, |ui| toolbar(app, ui));
    if !empty {
        Panel::bottom("status").exact_size(28.0).frame(egui::Frame::new().fill(p.chrome).inner_margin(egui::Margin::symmetric(14, 0))).show(ui, |ui| status_bar(app, ui));
    }

    let mut show_tools = app.settings.show_tools && !empty;
    Panel::left("tools")
        .default_size(262.0)
        .size_range(240.0..=330.0)
        .resizable(true)
        .drag_to_open(false)
        .frame(egui::Frame::new().fill(p.sidebar))
        .show_collapsible(ui, &mut show_tools, |ui| sidebar::tool_sidebar(app, ui));
    app.settings.show_tools = show_tools || (app.settings.show_tools && empty);

    let mut show_layers = app.settings.show_layers && !empty;
    Panel::right("layers")
        .default_size(262.0)
        .size_range(230.0..=340.0)
        .resizable(true)
        .drag_to_open(false)
        .frame(egui::Frame::new().fill(p.sidebar))
        .show_collapsible(ui, &mut show_layers, |ui| layers::layers_panel(app, ui));
    app.settings.show_layers = show_layers || (app.settings.show_layers && empty);

    egui::CentralPanel::default().frame(egui::Frame::new().fill(p.canvas)).show_inside(ui, |ui| {
        app.canvas_ui(ui);
        let rect = ui.max_rect();
        overlays(app, &ctx, rect);
    });

    sheets::show(app, &ctx);
}

// ---------------------------------------------------------------- menus ---

fn menu_item(ui: &mut egui::Ui, app: &mut App, label: &str, action: Action) {
    let enabled = app.enabled(&action);
    let sc = shortcuts().into_iter().find(|(_, a)| *a == action).map(|(s, _)| ui.ctx().format_shortcut(&s));
    let mut b = egui::Button::new(label);
    if let Some(sc) = sc {
        b = b.shortcut_text(sc);
    }
    if ui.add_enabled(enabled, b).clicked() {
        app.perform(action);
        ui.close();
    }
}

fn menubar(app: &mut App, ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.spacing_mut().button_padding = Vec2::new(8.0, 3.0);
        ui.menu_button("File", |ui| {
            menu_item(ui, app, "Open…", Action::Open);
            menu_item(ui, app, "Open Project…", Action::OpenProject);
            menu_item(ui, app, "Restore Last Session", Action::RestoreSession);
            ui.separator();
            menu_item(ui, app, "Clear Workspace", Action::ClearWorkspace);
            ui.separator();
            menu_item(ui, app, "Save Project", Action::SaveProject);
            menu_item(ui, app, "Save Project As…", Action::SaveProjectAs);
            ui.separator();
            menu_item(ui, app, "Export PNG…", Action::Export(Format::Png, 1.0));
            menu_item(ui, app, "Export JPEG…", Action::Export(Format::Jpeg, 1.0));
            ui.menu_button("Export at Size", |ui| {
                for s in [0.25f64, 0.5, 2.0] {
                    menu_item(ui, app, &format!("PNG at {}%", (s * 100.0) as i32), Action::Export(Format::Png, s));
                }
            });
            ui.separator();
            menu_item(ui, app, "Settings…", Action::Settings);
            menu_item(ui, app, "Exit", Action::Quit);
        });
        ui.menu_button("Edit", |ui| {
            let ul = app.state().undo_label.clone().map(|l| format!("Undo {l}")).unwrap_or_else(|| "Undo".into());
            let rl = app.state().redo_label.clone().map(|l| format!("Redo {l}")).unwrap_or_else(|| "Redo".into());
            menu_item(ui, app, &ul, Action::Undo);
            menu_item(ui, app, &rl, Action::Redo);
            ui.separator();
            menu_item(ui, app, "Copy", Action::Copy(CopyAction::Copy, None));
            menu_item(ui, app, "Copy & Clear", Action::Copy(CopyAction::CopyAndClear, None));
            menu_item(ui, app, "Copy & Close", Action::Copy(CopyAction::CopyAndClose, None));
            menu_item(ui, app, "Copy as JPEG", Action::Copy(CopyAction::Copy, Some(Format::Jpeg)));
            ui.separator();
            let paste = if app.is_empty() { "Paste" } else { "Paste as New Layer" };
            menu_item(ui, app, paste, Action::Paste);
            menu_item(ui, app, "Paste as New Image", Action::PasteNewImage);
            ui.separator();
            menu_item(ui, app, "Select All", Action::SelectAll);
            menu_item(ui, app, "Deselect", Action::Deselect);
            menu_item(ui, app, "Invert Selection", Action::InvertSelection);
            menu_item(ui, app, "Delete Selected Pixels", Action::DeleteSelection);
        });
        ui.menu_button("Image", |ui| {
            menu_item(ui, app, "Remove Background", Action::RemoveBg(RemovalMode::Best));
            menu_item(ui, app, "Remove Background (Fast)", Action::RemoveBg(RemovalMode::Fast));
            menu_item(ui, app, "Refine Edges…", Action::RefineEdges);
            menu_item(ui, app, "Compare Before / After", Action::Compare);
            ui.separator();
            menu_item(ui, app, "Crop to Selection", Action::CropToSelection);
            menu_item(ui, app, "Trim Transparent Pixels", Action::Trim);
            menu_item(ui, app, "Resize Image…", Action::ResizeImage);
            menu_item(ui, app, "Canvas Size…", Action::CanvasSize);
            ui.separator();
            menu_item(ui, app, "Rotate 90° Right", Action::Rotate(true));
            menu_item(ui, app, "Rotate 90° Left", Action::Rotate(false));
            menu_item(ui, app, "Flip Horizontal", Action::Flip("h"));
            menu_item(ui, app, "Flip Vertical", Action::Flip("v"));
            ui.separator();
            menu_item(ui, app, "Manage AI Models…", Action::ManageModels);
        });
        ui.menu_button("Layer", |ui| {
            menu_item(ui, app, "New Layer from Clipboard", Action::Paste);
            menu_item(ui, app, "Duplicate Layer", Action::DuplicateLayer);
            menu_item(ui, app, "Delete Layer", Action::DeleteLayer);
            ui.separator();
            menu_item(ui, app, "Merge Down / Merge Selected", Action::MergeDown);
            menu_item(ui, app, "Flatten Image", Action::Flatten);
            ui.separator();
            menu_item(ui, app, "Bring to Front", Action::LayerOrder("top"));
            menu_item(ui, app, "Bring Forward", Action::LayerOrder("up"));
            menu_item(ui, app, "Send Backward", Action::LayerOrder("down"));
            menu_item(ui, app, "Send to Back", Action::LayerOrder("bottom"));
            ui.separator();
            menu_item(ui, app, "Restore All Erased Pixels", Action::RestoreAll);
            menu_item(ui, app, "Apply Mask", Action::ApplyMask);
            menu_item(ui, app, "Rasterize Text", Action::RasterizeText);
        });
        ui.menu_button("View", |ui| {
            menu_item(ui, app, "Zoom In", Action::ZoomIn);
            menu_item(ui, app, "Zoom Out", Action::ZoomOut);
            menu_item(ui, app, "Fit in Window", Action::Fit);
            menu_item(ui, app, "Actual Size", Action::Actual);
            ui.separator();
            ui.menu_button("Appearance", |ui| {
                for a in Appearance::ALL {
                    if ui.radio_value(&mut app.settings.appearance, a, a.title()).clicked() {
                        app.apply_appearance();
                        ui.close();
                    }
                }
            });
            ui.separator();
            let t = if app.settings.show_tools { "Hide Tools" } else { "Show Tools" };
            let l = if app.settings.show_layers { "Hide Layers" } else { "Show Layers" };
            menu_item(ui, app, t, Action::ToggleTools);
            menu_item(ui, app, l, Action::ToggleLayers);
        });
        ui.menu_button("Help", |ui| {
            menu_item(ui, app, "Keyboard Shortcuts", Action::Shortcuts);
            menu_item(ui, app, "About CutOut", Action::About);
        });
    });
}

// -------------------------------------------------------------- toolbar ---

fn toolbar(app: &mut App, ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    let empty = app.is_empty();
    let p = app.palette;
    let left_w = 250.0;
    let right_w = if empty { 40.0 } else { 300.0 };
    let left = Rect::from_min_max(rect.min, Pos2::new(rect.min.x + left_w, rect.max.y));
    let right = Rect::from_min_max(Pos2::new(rect.max.x - right_w, rect.min.y), rect.max);
    let center = Rect::from_min_max(Pos2::new(left.max.x, rect.min.y), Pos2::new(right.min.x, rect.max.y));

    // title block
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(Layout::left_to_right(Align::Center)), |ui| {
        if icon_button(ui, app.palette, icons::FOLDER_OPEN, "Open an image or project (Ctrl+O)", true, false).clicked() {
            app.perform(Action::Open);
        }
        ui.add_space(4.0);
        if !empty {
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(egui::RichText::new(app.suggested_name()).font(FontId::new(13.5, theme::semibold_family())).color(p.text));
                ui.label(small(app.palette, format!("{} × {}", app.state().width, app.state().height)));
            });
        } else {
            ui.label(egui::RichText::new("CutOut").font(FontId::new(13.5, theme::semibold_family())).color(p.text));
        }
    });

    if !empty {
        ui.scope_builder(UiBuilder::new().max_rect(center).layout(Layout::centered_and_justified(egui::Direction::LeftToRight)), |ui| {
            ui.horizontal_centered(|ui| {
                let removing = app.is_removing();
                let r = prominent_button(ui, app.palette, icons::WAND_SPARKLES, "Remove Background", !removing).on_hover_text("Remove the background with the best-quality local AI model (Ctrl+B)");
                if r.clicked() {
                    app.perform(Action::RemoveBg(RemovalMode::Best));
                }
                r.context_menu(|ui| {
                    if ui.button("Best Quality").clicked() {
                        app.perform(Action::RemoveBg(RemovalMode::Best));
                        ui.close();
                    }
                    if ui.button("Fast").clicked() {
                        app.perform(Action::RemoveBg(RemovalMode::Fast));
                        ui.close();
                    }
                });
                let resp = icon_button(ui, app.palette, icons::CHEVRON_DOWN, "Removal mode and edge refinement", true, false);
                egui::Popup::menu(&resp).show(|ui| {
                    if ui.button("Best Quality (BiRefNet)").clicked() {
                        app.perform(Action::RemoveBg(RemovalMode::Best));
                        ui.close();
                    }
                    if ui.button("Fast (BiRefNet Lite)").clicked() {
                        app.perform(Action::RemoveBg(RemovalMode::Fast));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Refine Edges…").clicked() {
                        app.perform(Action::RefineEdges);
                        ui.close();
                    }
                    if ui.button("Manage AI Models…").clicked() {
                        app.perform(Action::ManageModels);
                        ui.close();
                    }
                });
                if icon_button(ui, app.palette, icons::COLUMNS_2, "Compare before and after (\\)", true, app.compare_on).clicked() {
                    app.perform(Action::Compare);
                }
            });
        });
        ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), |ui| {
            if icon_button(ui, app.palette, icons::LAYERS, "Show or hide the Layers panel (Ctrl+Alt+L)", true, app.settings.show_layers).clicked() {
                app.perform(Action::ToggleLayers);
            }
            if icon_button(ui, app.palette, icons::SLIDERS_HORIZONTAL, "Show or hide the advanced tools (Ctrl+Alt+T)", true, app.settings.show_tools).clicked() {
                app.perform(Action::ToggleTools);
            }
            // Copy split button (primary Copy + a menu with Clear / Close variants).
            copy_button(app, ui);
            ui.add_space(6.0);
            let (can_undo, can_redo) = (app.state().can_undo, app.state().can_redo);
            let rl = app.state().redo_label.clone().map(|l| format!("Redo {l} (Ctrl+Shift+Z)")).unwrap_or_else(|| "Redo (Ctrl+Shift+Z)".into());
            let ul = app.state().undo_label.clone().map(|l| format!("Undo {l} (Ctrl+Z)")).unwrap_or_else(|| "Undo (Ctrl+Z)".into());
            if icon_button(ui, app.palette, icons::REDO_2, &rl, can_redo, false).clicked() {
                app.perform(Action::Redo);
            }
            if icon_button(ui, app.palette, icons::UNDO_2, &ul, can_undo, false).clicked() {
                app.perform(Action::Undo);
            }
        });
    }
    let _ = (Align2::CENTER_CENTER, StrokeKind::Inside, CornerRadius::ZERO, Stroke::NONE, Sense::hover(), Color32::WHITE);
}

fn copy_button(app: &mut App, ui: &mut egui::Ui) {
    let p = app.palette;
    // right-to-left layout: the menu arrow is added first so it ends up on the right.
    let arrow = icon_button(ui, app.palette, icons::CHEVRON_DOWN, "More copy options", true, false);
    egui::Popup::menu(&arrow).show(|ui| {
        for (label, a) in [
            ("Copy", Action::Copy(CopyAction::Copy, None)),
            ("Copy & Clear", Action::Copy(CopyAction::CopyAndClear, None)),
            ("Copy & Close", Action::Copy(CopyAction::CopyAndClose, None)),
        ] {
            let sc = shortcuts().into_iter().find(|(_, x)| *x == a).map(|(s, _)| ui.ctx().format_shortcut(&s));
            let mut b = egui::Button::new(label);
            if let Some(sc) = sc {
                b = b.shortcut_text(sc);
            }
            if ui.add(b).clicked() {
                app.perform(a);
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Copy as PNG").clicked() {
            app.perform(Action::Copy(CopyAction::Copy, Some(Format::Png)));
            ui.close();
        }
        if ui.button("Copy as JPEG").clicked() {
            app.perform(Action::Copy(CopyAction::Copy, Some(Format::Jpeg)));
            ui.close();
        }
    });
    ui.add_space(-6.0);
    let r = soft_button(ui, app.palette, Some(icons::COPY), "Copy", true).on_hover_text("Copy the result with transparency (Ctrl+C). Menu: Copy & Clear (Ctrl+Shift+C), Copy & Close (Ctrl+Alt+C)");
    if r.clicked() {
        app.perform(Action::Copy(CopyAction::Copy, None));
    }
    let _ = p;
}

// ------------------------------------------------------------ status bar ---

fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    let p = app.palette;
    ui.horizontal_centered(|ui| {
        ui.label(egui::RichText::new(app.tool.help()).size(12.0).color(p.secondary));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // zoom menu
            let z = (app.canvas.scale * app.canvas.ppp * 100.0).round() as i64;
            let r = ui.add(egui::Button::new(egui::RichText::new(format!("{z}%")).monospace().size(12.0)).frame(false)).on_hover_text("Zoom (Ctrl+ / Ctrl− / Ctrl+0 fit / Ctrl+1 actual size)");
            egui::Popup::menu(&r).show(|ui| {
                if ui.button("Fit in Window").clicked() {
                    app.perform(Action::Fit);
                    ui.close();
                }
                if ui.button("Actual Size (100%)").clicked() {
                    app.perform(Action::Actual);
                    ui.close();
                }
                ui.separator();
                for z in [0.25f32, 0.5, 2.0, 4.0, 8.0] {
                    if ui.button(format!("{}%", (z * 100.0) as i32)).clicked() {
                        app.canvas.set_zoom_device(z, None, true);
                        ui.close();
                    }
                }
            });
            ui.separator();
            if let Some(l) = app.state().active_layer() {
                ui.label(egui::RichText::new(l.name.clone()).size(12.0).color(p.secondary));
            }
            backend_pill(app, ui);
        });
    });
}

fn backend_pill(app: &App, ui: &mut egui::Ui) {
    let p = app.palette;
    let (text, glyph) = if let Some(b) = &app.active_backend {
        (b.clone(), icons::CPU)
    } else if models::has_best_model(&app.settings.best_model) {
        (if app.settings.use_directml && cutout_core::ai::accel_available(cutout_core::ai::Accel::DirectMl) { "ONNX (DirectML)" } else { "ONNX (CPU)" }.to_string(), icons::CPU)
    } else {
        ("No model installed".to_string(), icons::TRIANGLE_ALERT)
    };
    let galley = ui.painter().layout_no_wrap(text, FontId::new(11.0, theme::semibold_family()), p.secondary);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 32.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
    ui.painter().text(Pos2::new(rect.min.x + 12.0, rect.center().y), Align2::CENTER_CENTER, glyph, FontId::new(11.0, theme::icon_family()), p.secondary);
    ui.painter().galley(Pos2::new(rect.min.x + 22.0, rect.center().y - galley.size().y / 2.0), galley, p.secondary);
    resp.on_hover_text("Background-removal engine. Everything runs offline on this PC.");
}

// -------------------------------------------------------------- overlays ---

fn overlays(app: &mut App, ctx: &egui::Context, rect: Rect) {
    let p = app.palette;
    if !app.is_empty() && app.drop_targeted {
        egui::Area::new(egui::Id::new("drop")).order(egui::Order::Foreground).fixed_pos(rect.min).interactable(false).show(ctx, |ui| {
            let r = rect.shrink(12.0);
            ui.painter().rect(r, CornerRadius::same(16), p.accent.gamma_multiply(0.08), Stroke::new(2.5, p.accent), StrokeKind::Inside);
            capsule(ui, app, r.center(), icons::COPY, "Drop to add as a new layer", false);
        });
    }
    if let Some(t) = &app.toast {
        let (msg, glyph, err) = (t.message.clone(), t.icon, t.error);
        let fade = {
            let age = t.born.elapsed().as_secs_f32();
            let left = (t.ttl.as_secs_f32() - age).max(0.0);
            (age / 0.2).min(1.0).min((left / 0.3).min(1.0))
        };
        egui::Area::new(egui::Id::new("toast")).order(egui::Order::Foreground).fixed_pos(Pos2::new(rect.center().x, rect.min.y + 14.0 - (1.0 - fade) * 10.0)).pivot(Align2::CENTER_TOP).interactable(false).show(ctx, |ui| {
            ui.set_opacity(fade);
            toast_capsule(ui, app, &msg, glyph, err);
        });
    }
    if app.compare_on {
        egui::Area::new(egui::Id::new("cmp-hint")).order(egui::Order::Foreground).fixed_pos(Pos2::new(rect.center().x, rect.max.y - 44.0)).pivot(Align2::CENTER_BOTTOM).interactable(false).show(ctx, |ui| {
            capsule(ui, app, ui.cursor().min, icons::COLUMNS_2, "Drag across the image to compare before and after", true);
        });
    }
    if let Some(b) = &app.busy {
        let (title, cancellable) = (b.title.clone(), b.cancellable);
        // block clicks to the canvas with a transparent layer
        egui::Area::new(egui::Id::new("busy-block")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
            ui.allocate_rect(rect, Sense::click_and_drag());
        });
        egui::Area::new(egui::Id::new("busy")).order(egui::Order::Tooltip).fixed_pos(rect.center()).pivot(Align2::CENTER_CENTER).show(ctx, |ui| {
            egui::Frame::new().fill(p.chrome).stroke(Stroke::new(1.0, p.card_stroke)).corner_radius(CornerRadius::same(16)).inner_margin(egui::Margin::symmetric(18, 13)).shadow(egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(60) }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(16.0).color(p.accent));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(egui::RichText::new(&title).font(FontId::new(13.5, theme::semibold_family())));
                        ui.label(small(app.palette, "Processing on this PC"));
                    });
                    if cancellable && soft_button(ui, app.palette, None, "Cancel", true).clicked() {
                        app.cancel_removal();
                    }
                });
            });
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    }
}

fn capsule(ui: &mut egui::Ui, app: &App, _center: Pos2, glyph: char, text: &str, at_cursor: bool) {
    let p = app.palette;
    let _ = at_cursor;
    egui::Frame::new().fill(p.chrome).stroke(Stroke::new(1.0, p.card_stroke)).corner_radius(CornerRadius::same(20)).inner_margin(egui::Margin::symmetric(14, 8)).shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(36) }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(icon(glyph, 15.0).color(p.accent));
            ui.label(egui::RichText::new(text).size(13.5));
        });
    });
}

fn toast_capsule(ui: &mut egui::Ui, app: &App, msg: &str, glyph: char, err: bool) {
    let p = app.palette;
    let stroke = if err { Color32::from_rgb(220, 70, 60).gamma_multiply(0.6) } else { p.card_stroke };
    egui::Frame::new().fill(p.chrome).stroke(Stroke::new(1.0, stroke)).corner_radius(CornerRadius::same(20)).inner_margin(egui::Margin::symmetric(16, 9)).shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(36) }).show(ui, |ui| {
        ui.horizontal(|ui| {
            let col = if err { Color32::from_rgb(255, 159, 10) } else if glyph == icons::SPARKLES { Color32::from_rgb(255, 204, 0) } else { Color32::from_rgb(52, 199, 89) };
            ui.label(icon(glyph, 15.0).color(col));
            ui.label(egui::RichText::new(msg).font(FontId::new(13.5, theme::semibold_family())));
        });
    });
}

// ------------------------------------------------------------ empty state ---

pub fn empty_state(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    let p = app.palette;
    let ctx = ui.ctx().clone();
    if app.icon_tex.is_none() {
        if let Ok(img) = image::load_from_memory(include_bytes!("../../../assets/icon/CutOut-512.png")) {
            let rgba = img.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            app.icon_tex = Some(ctx.load_texture("app-icon", ci, egui::TextureOptions::LINEAR));
        }
    }
    if app.wordmark.is_none() {
        if let Ok(img) = image::load_from_memory(include_bytes!("../../assets/wordmark.png")) {
            let rgba = trim_alpha(img.to_rgba8());
            let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            let size = Vec2::new(rgba.width() as f32, rgba.height() as f32);
            app.wordmark = Some((ctx.load_texture("wordmark", ci, egui::TextureOptions { magnification: egui::TextureFilter::Linear, minification: egui::TextureFilter::Linear, wrap_mode: egui::TextureWrapMode::ClampToEdge, mipmap_mode: Some(egui::TextureFilter::Linear) }), size));
        }
    }
    let appear = ctx.animate_bool_with_time(egui::Id::new("empty-appear"), true, 0.5);
    let active = app.drop_targeted || ui.rect_contains_pointer(Rect::from_center_size(rect.center() + Vec2::new(0.0, 40.0), Vec2::new(520.0, 330.0)));
    let card_w = 520.0f32.min(rect.width() - 40.0);
    // wordmark sized to the canvas: ~78% wide, never taller than a fifth of the height
    let wm_h = (rect.width() * 0.78 / 3.65).min(rect.height() * 0.19).min(168.0).max(44.0);
    let card_h = 326.0 + if app.has_recovery() { 24.0 } else { 0.0 };
    let total = wm_h + (wm_h * 0.2).max(14.0) + card_h;
    let top = rect.center().y - total / 2.0 + (1.0 - appear) * 10.0;
    let painter = ui.painter();
    let mut y = top.max(rect.min.y + 8.0);
    if let Some((tex, sz)) = &app.wordmark {
        let w = wm_h * sz.x / sz.y;
        let r = Rect::from_min_size(Pos2::new(rect.center().x - w / 2.0, y), Vec2::new(w, wm_h));
        painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), p.wordmark.gamma_multiply(appear));
    }
    y += wm_h + (wm_h * 0.2).max(14.0);
    let card = Rect::from_center_size(Pos2::new(rect.center().x, y + card_h / 2.0), Vec2::new(card_w, card_h));
    let stroke = if active { Stroke::new(2.0, p.accent.gamma_multiply(0.8)) } else { Stroke::new(1.5, p.card_stroke) };
    painter.rect_filled(card, CornerRadius::same(28), p.card.gamma_multiply(appear));
    if active {
        painter.rect_stroke(card, CornerRadius::same(28), stroke, StrokeKind::Inside);
    } else {
        // dashed outline
        let r = card.shrink(0.75);
        let pts = rounded_rect_pts(r, 28.0);
        painter.extend(egui::Shape::dashed_line(&pts, stroke, 7.0, 6.0));
    }
    let mut cy = card.min.y + 44.0 + 42.0;
    if let Some(t) = &app.icon_tex {
        let s = if active { 88.0 } else { 84.0 };
        painter.image(t.id(), Rect::from_center_size(Pos2::new(card.center().x, cy), Vec2::splat(s)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(appear));
    }
    cy += 42.0 + 26.0;
    let head = if app.drop_targeted { "Drop to open" } else { "Remove any background" };
    painter.text(Pos2::new(card.center().x, cy + 4.0), Align2::CENTER_CENTER, head, FontId::new(25.0, theme::semibold_family()), p.text.gamma_multiply(appear));
    cy += 36.0;
    // "Paste with [Ctrl] [V] or drop an image here"
    let sub = FontId::new(14.0, egui::FontFamily::Proportional);
    let parts = [("Paste with", false), ("Ctrl", true), ("V", true), ("or drop an image here", false)];
    let galleys: Vec<_> = parts.iter().map(|(t, k)| (painter.layout_no_wrap((*t).to_string(), if *k { FontId::new(13.5, theme::semibold_family()) } else { sub.clone() }, p.secondary), *k)).collect();
    let total_w: f32 = galleys.iter().map(|(g, k)| g.size().x + if *k { 12.0 } else { 0.0 }).sum::<f32>() + 5.0 * (galleys.len() as f32 - 1.0);
    let mut x = card.center().x - total_w / 2.0;
    for (g, k) in galleys {
        let w = g.size().x + if k { 12.0 } else { 0.0 };
        if k {
            let r = Rect::from_min_size(Pos2::new(x, cy - 10.0), Vec2::new(w, 20.0));
            painter.rect(r, CornerRadius::same(5), p.hover, Stroke::new(0.5, p.card_stroke), StrokeKind::Inside);
        }
        painter.galley(Pos2::new(x + if k { 6.0 } else { 0.0 }, cy - g.size().y / 2.0), g, p.secondary);
        x += w + 5.0;
    }
    cy += 44.0;
    // buttons
    let bw1 = 128.0;
    let bw2 = 92.0;
    let r1 = Rect::from_center_size(Pos2::new(card.center().x - (bw1 + bw2 + 10.0) / 2.0 + bw1 / 2.0, cy), Vec2::new(bw1, 36.0));
    let r2 = Rect::from_center_size(Pos2::new(card.center().x + (bw1 + bw2 + 10.0) / 2.0 - bw2 / 2.0, cy), Vec2::new(bw2, 36.0));
    let paste = ui.put(r1, |ui: &mut egui::Ui| prominent_button(ui, app.palette, icons::CLIPBOARD, "Paste Image", true));
    let open = ui.put(r2, |ui: &mut egui::Ui| soft_button(ui, app.palette, Some(icons::FOLDER_OPEN), "Open…", true));
    if paste.clicked() {
        app.perform(Action::Paste);
    }
    if open.clicked() {
        app.perform(Action::Open);
    }
    cy += 40.0;
    if app.has_recovery() {
        let r = Rect::from_center_size(Pos2::new(card.center().x, cy), Vec2::new(200.0, 20.0));
        let resp = ui.put(r, egui::Button::new(egui::RichText::new("Restore Last Session").color(p.accent)).frame(false));
        if resp.clicked() {
            app.perform(Action::RestoreSession);
        }
        cy += 24.0;
    }
    let note = ui.painter().layout_no_wrap("Runs entirely on this PC. Nothing is uploaded.".to_string(), FontId::new(12.0, egui::FontFamily::Proportional), p.tertiary);
    let nx = card.center().x - (note.size().x + 18.0) / 2.0;
    ui.painter().text(Pos2::new(nx + 6.0, cy + 8.0), Align2::CENTER_CENTER, icons::LOCK, FontId::new(11.0, theme::icon_family()), p.tertiary);
    ui.painter().galley(Pos2::new(nx + 18.0, cy + 8.0 - note.size().y / 2.0), note, p.tertiary);
}

fn rounded_rect_pts(r: Rect, rad: f32) -> Vec<Pos2> {
    let mut v = Vec::new();
    let corners = [(r.right_top() + Vec2::new(-rad, rad), -90.0f32), (r.right_bottom() + Vec2::new(-rad, -rad), 0.0), (r.left_bottom() + Vec2::new(rad, -rad), 90.0), (r.left_top() + Vec2::new(rad, rad), 180.0)];
    for (c, start) in corners {
        for i in 0..=12 {
            let a = (start + 90.0 * i as f32 / 12.0).to_radians();
            v.push(Pos2::new(c.x + rad * a.cos(), c.y + rad * a.sin()));
        }
    }
    v.push(v[0]);
    v
}

fn trim_alpha(img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 8 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 {
        return img;
    }
    image::imageops::crop_imm(&img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image()
}

#[allow(dead_code)]
fn _keep(_: PanelKind) {}
