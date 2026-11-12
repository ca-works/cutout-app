//! Modal sheets: model setup, resize / canvas size, text, JPEG warning,
//! settings, about and keyboard shortcuts (port of `Sheets.swift`).

use super::widgets::*;
use crate::app::{Action, App, CopyAction, ModelTask, Sheet};
use crate::codec::Format;
use crate::engine::TextProps;
use crate::icons;
use crate::models::{self, RemovalMode};
use crate::settings::{Appearance, ClipFormat};
use crate::theme::{self, icon};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, KeyboardShortcut, Layout, Modifiers, Stroke, Ui, Vec2};

type After = Vec<Box<dyn FnOnce(&mut App)>>;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut sheet) = app.sheet.take() else { return };
    let p = app.palette;
    let mut close = false;
    let mut after: After = vec![];
    let frame = egui::Frame::new().fill(p.chrome).stroke(Stroke::new(1.0, p.card_stroke)).corner_radius(CornerRadius::same(14)).inner_margin(egui::Margin::same(22)).shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(110) });
    let dismissible = !matches!(sheet, Sheet::Text(_));
    let resp = egui::Modal::new(egui::Id::new("sheet")).backdrop_color(Color32::from_black_alpha(80)).frame(frame).show(ctx, |ui| {
        match &mut sheet {
            Sheet::ClearWorkspace => {
                ui.set_width(360.0);
                title(ui, "Clear Workspace?");
                ui.label("Remove all images, layers and selections? This cannot be undone. Files, clipboard and preferences will be kept.");
                ui.horizontal(|ui| {
                    if small_button(ui, p, "Cancel", true).clicked() { close = true; }
                    if primary_small(ui, p, "Clear Workspace", true).clicked() {
                        close = true;
                        after.push(Box::new(|app| app.clear_workspace(false)));
                    }
                });
            }

            Sheet::ModelSetup => model_setup(app, ui, &mut close, &mut after),
            Sheet::Resize(r) => resize(app, ui, r, &mut close),
            Sheet::Text(t) => text_sheet(app, ui, t, &mut close),
            Sheet::LowMemory { mode, avail_gb, need_gb } => low_memory(app, ui, *mode, *avail_gb, *need_gb, &mut close, &mut after),
            Sheet::JpegWarning { action, dont_ask } => jpeg_warning(app, ui, *action, dont_ask, &mut close, &mut after),
            Sheet::Settings => settings(app, ui, &mut close),
            Sheet::About => about(app, ui, &mut close),
            Sheet::Shortcuts => shortcuts_sheet(app, ui, &mut close),
        }
    });
    if (resp.should_close() && dismissible) || close {
        // keep nothing
    } else if app.sheet.is_none() {
        app.sheet = Some(sheet);
    }
    for f in after {
        f(app);
    }
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(FontId::new(17.0, theme::semibold_family())));
}

// ------------------------------------------------------------ model setup ---

fn model_setup(app: &mut App, ui: &mut Ui, close: &mut bool, after: &mut After) {
    let p = app.palette;
    ui.set_width(580.0);
    ui.spacing_mut().item_spacing.y = 10.0;
    ui.horizontal(|ui| {
        ui.label(icon(icons::WAND_SPARKLES, 30.0).color(p.accent));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            title(ui, "Best-Quality Background Removal");
            ui.label(secondary(p, "BiRefNet runs entirely on this PC. Once a model is present CutOut works offline."));
        });
    });
    let best = models::best_spec(&app.settings.best_model);
    let uses = match &best {
        Some((s, _)) => format!("ONNX Runtime · {} ({})", if crate::models::gpu_candidate(app.settings.use_directml, &app.settings.best_model).is_some() { "DirectML" } else { "CPU" }, s.title),
        None => "None installed — download or install a model below".to_string(),
    };
    ui.horizontal(|ui| {
        ui.label(secondary(p, "Best Quality uses"));
        ui.label(egui::RichText::new(uses).font(FontId::new(13.5, theme::semibold_family())));
    });
    if let Some(b) = &app.active_backend {
        ui.horizontal(|ui| {
            ui.label(secondary(p, "Last removal ran on"));
            ui.label(secondary(p, b.clone()));
        });
    }
    if cutout_core::ai::accel_available(cutout_core::ai::Accel::DirectMl) {
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("GPU acceleration (DirectML)").font(FontId::new(13.5, theme::semibold_family())));
                    let hint = match crate::platform::gpu::high_performance_adapter() {
                        Some(a) if crate::platform::gpu::suitable_for_model() => format!("{} — used automatically; falls back to the CPU if anything goes wrong.", a.name),
                        Some(a) => format!("{} has too little dedicated video memory for this model, so the CPU is used.", a.name),
                        None => "No DirectX 12 graphics adapter found, so the CPU is used.".to_string(),
                    };
                    ui.label(small(p, hint));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.checkbox(&mut app.settings.use_directml, "Use").changed() {
                        models::unload();
                    }
                });
            });
        });
    }
    ui.label(egui::RichText::new("Models").font(FontId::new(14.5, theme::semibold_family())));
    for spec in models::ALL.iter() {
        model_row(app, ui, spec);
    }
    if let Some(msg) = &app.model_msg {
        ui.label(egui::RichText::new(msg).color(if msg.contains("fail") || msg.contains("could not") || msg.contains("rror") { Color32::from_rgb(230, 70, 60) } else { p.secondary }));
    }
    ui.horizontal(|ui| {
        if small_button(ui, p, "Install from File…", app.model_task.is_none()).on_hover_text("Choose a BiRefNet .onnx file; it is verified against its SHA-256 before it is installed").clicked() {
            if let Some(f) = app.file_dialog().add_filter("ONNX model", &["onnx"]).set_title("Choose a CutOut BiRefNet ONNX model").pick_file() {
                app.start_install(f);
            }
        }
    });
    ui.label(small(p, "Downloads come from huggingface.co at pinned revisions and are SHA-256 verified. No images are ever uploaded."));
    ui.horizontal(|ui| {
        if small_button(ui, p, "Use Fast Mode Now", true).on_hover_text("BiRefNet Lite, when installed").clicked() {
            *close = true;
            after.push(Box::new(|a: &mut App| a.remove_background(RemovalMode::Fast)));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if models::has_best_model(&app.settings.best_model) && !app.is_empty() && primary_small(ui, p, "Remove Background", true).clicked() {
                *close = true;
                after.push(Box::new(|a: &mut App| a.remove_background(RemovalMode::Best)));
            }
            if primary_small(ui, p, "Done", true).clicked() {
                *close = true;
            }
        });
    });
}

fn model_row(app: &mut App, ui: &mut Ui, spec: &'static models::ModelSpec) {
    let p = app.palette;
    let installed = models::installed_path(spec).is_some();
    let bundled = models::is_bundled(spec);
    let preferred = app.settings.best_model == spec.id;
    let downloading = matches!(&app.model_task, Some(ModelTask::Download { spec: s, .. }) if s.id == spec.id);
    let busy = app.model_task.is_some();
    ui.horizontal(|ui| {
        ui.label(icon(if installed { icons::CIRCLE_CHECK } else { icons::DOWNLOAD }, 16.0).color(if installed { Color32::from_rgb(52, 199, 89) } else { p.secondary }));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(spec.title).font(FontId::new(13.5, theme::semibold_family())));
                ui.label(secondary(p, spec.size_label()));
                if preferred {
                    ui.label(egui::RichText::new(" Preferred ").size(11.0).background_color(p.accent.gamma_multiply(0.2)).color(p.accent));
                }
                if bundled {
                    ui.label(egui::RichText::new(" Included ").size(11.0).background_color(p.hover).color(p.secondary));
                }
            });
            ui.label(small(p, spec.detail));
            if let Some(ModelTask::Download { spec: s, progress, .. }) = &app.model_task {
                if s.id == spec.id {
                    ui.add(egui::ProgressBar::new(*progress as f32).text(format!("Downloading… {}%", (*progress * 100.0) as i32)).desired_height(14.0));
                }
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if installed {
                if !bundled && small_button(ui, p, "Delete", true).clicked() {
                    let _ = models::delete(spec);
                    models::unload();
                }
                if !preferred && small_button(ui, p, "Prefer", true).clicked() {
                    app.settings.best_model = spec.id.to_string();
                    models::unload();
                }
            } else if downloading {
                if small_button(ui, p, "Cancel", true).clicked() {
                    if let Some(ModelTask::Download { cancel, .. }) = &app.model_task {
                        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            } else if !spec.downloadable() {
                ui.label(small(p, "Ships with the full installer"));
            } else if small_button(ui, p, "Download", !busy).clicked() {
                app.start_download(spec);
            }
        });
    });
}

// ----------------------------------------------------------------- resize ---

fn resize(app: &mut App, ui: &mut Ui, r: &mut crate::app::ResizeSheet, close: &mut bool) {
    let p = app.palette;
    ui.set_width(340.0);
    ui.spacing_mut().item_spacing.y = 10.0;
    let (w0, h0) = (app.state().width.max(1) as f64, app.state().height.max(1) as f64);
    title(ui, if r.canvas { "Canvas Size" } else { "Resize Image" });
    ui.label(secondary(p, format!("Current: {} × {} px", w0 as i64, h0 as i64)));
    egui::Grid::new("resize-grid").num_columns(3).spacing(Vec2::new(8.0, 8.0)).show(ui, |ui| {
        ui.label("Width");
        let wr = ui.add(egui::DragValue::new(&mut r.width).range(1..=16384).speed(1));
        ui.label("px");
        ui.end_row();
        ui.label("Height");
        let hr = ui.add(egui::DragValue::new(&mut r.height).range(1..=16384).speed(1));
        ui.label("px");
        ui.end_row();
        if r.keep_aspect && !r.canvas {
            if wr.changed() {
                r.height = ((r.width as f64 * h0 / w0).round() as i64).clamp(1, 16384);
            } else if hr.changed() {
                r.width = ((r.height as f64 * w0 / h0).round() as i64).clamp(1, 16384);
            }
        }
    });
    if !r.canvas {
        ui.checkbox(&mut r.keep_aspect, "Keep proportions");
        ui.horizontal(|ui| {
            for pct in [25i64, 50, 200] {
                if small_button(ui, p, &format!("{pct}%"), true).clicked() {
                    r.width = (w0 as i64 * pct / 100).max(1);
                    r.height = (h0 as i64 * pct / 100).max(1);
                }
            }
        });
    } else {
        ui.label("Anchor");
        egui::Grid::new("anchor").spacing(Vec2::new(4.0, 4.0)).show(ui, |ui| {
            for row in 0..3 {
                for col in 0..3 {
                    let i = row * 3 + col;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, if r.anchor == i { "●" } else { "○" }, FontId::new(15.0, egui::FontFamily::Proportional), if r.anchor == i { p.accent } else { p.secondary });
                    if resp.clicked() {
                        r.anchor = i;
                    }
                }
                ui.end_row();
            }
        });
    }
    let valid = r.width >= 1 && r.height >= 1 && r.width <= 16384 && r.height <= 16384;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let apply = primary_small(ui, p, if r.canvas { "Apply" } else { "Resize" }, valid).clicked() || (valid && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if small_button(ui, p, "Cancel", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *close = true;
            }
            if apply {
                if r.canvas {
                    app.run("canvas_size", serde_json::json!({"width": r.width, "height": r.height, "anchor_x": (r.anchor % 3) as f64 / 2.0, "anchor_y": (r.anchor / 3) as f64 / 2.0}));
                } else {
                    app.run("resize_image", serde_json::json!({"width": r.width, "height": r.height}));
                }
                app.canvas.zoom_to_fit(true);
                *close = true;
            }
        });
    });
}

// ------------------------------------------------------------------- text ---

fn text_sheet(app: &mut App, ui: &mut Ui, t: &mut crate::app::TextSheet, close: &mut bool) {
    let p = app.palette;
    ui.set_width(460.0);
    ui.spacing_mut().item_spacing.y = 10.0;
    title(ui, if t.layer.is_none() { "Add Text" } else { "Edit Text" });
    let edit = egui::TextEdit::multiline(&mut t.text).desired_rows(4).desired_width(f32::INFINITY).font(FontId::new(15.0, egui::FontFamily::Proportional)).hint_text("Type your text");
    let r = ui.add(edit);
    if t.layer.is_none() && !r.has_focus() && t.text.is_empty() {
        r.request_focus();
    }
    ui.horizontal(|ui| {
        let list = app.font_list.get_or_insert_with(crate::text::families);
        egui::ComboBox::from_id_salt("text-font").selected_text(app.settings.tools.font_name.clone()).width(220.0).show_ui(ui, |ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for f in list.iter() {
                    ui.selectable_value(&mut app.settings.tools.font_name, f.clone(), f);
                }
            });
        });
        ui.add(egui::DragValue::new(&mut app.settings.tools.font_size).range(1.0..=2000.0).speed(1.0).suffix(" pt"));
    });
    ui.horizontal(|ui| {
        toggle_chip(ui, p, "Bold", &mut app.settings.tools.bold);
        toggle_chip(ui, p, "Italic", &mut app.settings.tools.italic);
        let items = [("left".to_string(), icons::ALIGN_LEFT, "Align left"), ("center".to_string(), icons::ALIGN_CENTER, "Align centre"), ("right".to_string(), icons::ALIGN_RIGHT, "Align right")];
        let idx = items.iter().position(|(a, _, _)| *a == t.align).unwrap_or(0);
        let mut i = idx;
        let it: Vec<(usize, char, &str)> = items.iter().enumerate().map(|(i, (_, g, s))| (i, *g, *s)).collect();
        if segmented(ui, p, &it, &mut i) {
            t.align = items[i].0.clone();
        }
        color_row(ui, "", &mut app.settings.tools.color);
    });
    let empty = t.text.trim().is_empty();
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let go = ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::Enter)));
            let add = primary_small(ui, p, if t.layer.is_none() { "Add" } else { "Update" }, !empty).clicked() || (go && !empty);
            if small_button(ui, p, "Cancel", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *close = true;
            }
            if add {
                let s = &app.settings.tools;
                let props = TextProps { text: t.text.clone(), font: s.font_name.clone(), size: s.font_size, color: s.color_rgba(), bold: s.bold, italic: s.italic, align: t.align.clone() };
                let (layer, point) = (t.layer, t.point);
                app.commit_text(props, layer, point);
                *close = true;
            }
        });
    });
}

// ------------------------------------------------------------ low memory ---

fn low_memory(app: &mut App, ui: &mut Ui, mode: RemovalMode, avail_gb: f64, need_gb: f64, close: &mut bool, after: &mut After) {
    let p = app.palette;
    ui.set_width(480.0);
    ui.spacing_mut().item_spacing.y = 10.0;
    ui.horizontal(|ui| {
        ui.label(icon(icons::TRIANGLE_ALERT, 18.0).color(Color32::from_rgb(255, 159, 10)));
        title(ui, "Not much free memory");
    });
    ui.label(secondary(p, format!("Best Quality removal needs about {need_gb:.0} GB of free memory and this PC has {avail_gb:.1} GB available right now. Windows may become slow while it runs, or the removal may fail.")));
    ui.label(small(p, "Closing other programs frees memory. BiRefNet Lite needs far less and is a little less precise on holes and low-contrast edges."));
    let lite = models::installed_path(&models::BIREFNET_LITE).is_some();
    ui.horizontal(|ui| {
        if small_button(ui, p, "Continue anyway", true).clicked() {
            *close = true;
            after.push(Box::new(move |a: &mut App| a.remove_background_ex(mode, true)));
        }
        if small_button(ui, p, "Use BiRefNet Lite", lite).on_hover_text(if lite { "Faster, much lower memory use" } else { "Not installed — get it in Manage AI Models" }).clicked() {
            *close = true;
            after.push(Box::new(|a: &mut App| a.remove_background(RemovalMode::Fast)));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_small(ui, p, "Cancel", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *close = true;
            }
        });
    });
}

// ---------------------------------------------------------- JPEG warning ---

fn jpeg_warning(app: &mut App, ui: &mut Ui, action: CopyAction, dont_ask: &mut bool, close: &mut bool, after: &mut After) {
    let p = app.palette;
    ui.set_width(480.0);
    ui.spacing_mut().item_spacing.y = 10.0;
    ui.horizontal(|ui| {
        ui.label(icon(icons::TRIANGLE_ALERT, 18.0).color(Color32::from_rgb(255, 159, 10)));
        title(ui, "JPEG doesn't support transparency");
    });
    ui.label(secondary(p, "Transparent areas will be filled with white. Copy as PNG to keep the transparent background."));
    ui.checkbox(dont_ask, "Don't ask again");
    ui.horizontal(|ui| {
        if small_button(ui, p, "Copy as PNG", true).clicked() {
            *close = true;
            after.push(Box::new(move |a: &mut App| a.copy(action, Some(Format::Png), false)));
        }
        if small_button(ui, p, "Set PNG as Default", true).clicked() {
            *close = true;
            after.push(Box::new(move |a: &mut App| {
                a.settings.clipboard_format = ClipFormat::Png;
                a.copy(action, Some(Format::Png), false)
            }));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_small(ui, p, "Copy JPEG", true).clicked() {
                *close = true;
                let dont = *dont_ask;
                after.push(Box::new(move |a: &mut App| {
                    if dont {
                        a.settings.jpeg_no_warn = true;
                    }
                    a.copy(action, Some(Format::Jpeg), true)
                }));
            }
            if small_button(ui, p, "Cancel", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *close = true;
            }
        });
    });
}

// --------------------------------------------------------------- settings ---

fn settings(app: &mut App, ui: &mut Ui, close: &mut bool) {
    let p = app.palette;
    ui.set_width(480.0);
    ui.spacing_mut().item_spacing.y = 10.0;
    title(ui, "Settings");
    ui.label(egui::RichText::new("General").font(FontId::new(14.0, theme::semibold_family())));
    ui.horizontal(|ui| {
        ui.label("Appearance");
        for a in Appearance::ALL {
            if ui.selectable_label(app.settings.appearance == a, a.title()).clicked() {
                app.settings.appearance = a;
                app.apply_appearance();
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Default copy format");
        ui.radio_value(&mut app.settings.clipboard_format, ClipFormat::Png, "PNG — keeps transparency");
        ui.radio_value(&mut app.settings.clipboard_format, ClipFormat::Jpeg, "JPEG — white background");
    });
    let mut warn = !app.settings.jpeg_no_warn;
    if ui.checkbox(&mut warn, "Warn when JPEG would discard transparency").changed() {
        app.settings.jpeg_no_warn = !warn;
    }
    ui.label(small(p, "Copies always include a PNG and a 32-bit bitmap with alpha, so PowerPoint, Paint and most editors keep transparency."));
    ui.separator();
    ui.label(egui::RichText::new("AI models").font(FontId::new(14.0, theme::semibold_family())));
    if cutout_core::ai::accel_available(cutout_core::ai::Accel::DirectMl) {
        if ui.checkbox(&mut app.settings.use_directml, "Use the GPU (DirectML) when available (falls back to the CPU automatically)").changed() {
            models::unload();
        }
    }
    ui.horizontal(|ui| {
        ui.label(secondary(p, "Fallback"));
        ui.label("ONNX Runtime · all CPU cores");
    });
    for spec in models::ALL.iter() {
        ui.horizontal(|ui| {
            ui.label(secondary(p, spec.title));
            let s = if models::installed_path(spec).is_some() { "Installed".to_string() } else { format!("Not installed ({})", spec.size_label()) };
            ui.label(s);
        });
    }
    ui.horizontal(|ui| {
        if small_button(ui, p, "Show Models Folder", true).clicked() {
            let d = crate::platform::paths::models_dir();
            let _ = std::fs::create_dir_all(&d);
            reveal(&d);
        }
        if small_button(ui, p, "Manage AI Models…", true).clicked() {
            *close = true;
        }
    });
    ui.label(small(p, "CutOut never sends images anywhere. The only network access is a model download you start yourself."));
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_small(ui, p, "Done", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *close = true;
            }
        });
    });
}

fn reveal(path: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

// ------------------------------------------------------------------ about ---

fn about(app: &mut App, ui: &mut Ui, close: &mut bool) {
    let p = app.palette;
    ui.set_width(380.0);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        if let Some(t) = &app.icon_tex {
            ui.image((t.id(), Vec2::splat(84.0)));
        }
        ui.label(egui::RichText::new("CutOut").font(FontId::new(22.0, theme::semibold_family())));
        ui.label(secondary(p, format!("Version {}", env!("CARGO_PKG_VERSION"))));
        ui.label("Remove backgrounds, edit, layer and copy — fast, native and completely offline.");
        ui.label(small(p, "No telemetry, no accounts. The clipboard is only read when you paste."));
        ui.label(small(p, "Background removal: BiRefNet (MIT) via ONNX Runtime. Icons: Lucide (ISC). Third-party notices are installed next to CutOut.exe."));
        if primary_small(ui, p, "Close", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *close = true;
        }
    });
}

// -------------------------------------------------------------- shortcuts ---

fn shortcuts_sheet(app: &mut App, ui: &mut Ui, close: &mut bool) {
    let p = app.palette;
    ui.set_width(520.0);
    title(ui, "Keyboard shortcuts");
    ui.add_space(6.0);
    let rows: [(&str, &str); 24] = [
        ("Paste (new layer) / as new image", "Ctrl+V / Ctrl+Alt+V"),
        ("Copy / Copy & Clear / Copy & Close", "Ctrl+C / Ctrl+Shift+C / Ctrl+Alt+C"),
        ("Remove background / Fast", "Ctrl+B / Ctrl+Shift+B"),
        ("Compare before / after", "\\"),
        ("Undo / Redo", "Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y"),
        ("Open / Open project", "Ctrl+O / Ctrl+Shift+O"),
        ("Save project / Save as", "Ctrl+S / Ctrl+Shift+S"),
        ("Export PNG", "Ctrl+Shift+E"),
        ("Tools sidebar / Layers panel", "Ctrl+Alt+T / Ctrl+Alt+L"),
        ("Fit / Actual size / Zoom", "Ctrl+0 / Ctrl+1 / Ctrl+Plus, Ctrl+Minus"),
        ("Select all / Deselect / Invert", "Ctrl+A / Ctrl+D / Ctrl+Shift+I"),
        ("Duplicate / Merge down layer", "Ctrl+J / Ctrl+E"),
        ("Bring forward / Send backward", "Ctrl+] / Ctrl+["),
        ("Rotate right / left", "Ctrl+R / Ctrl+L"),
        ("Resize image / Canvas size", "Ctrl+Alt+I / Ctrl+Alt+Shift+C"),
        ("Brush size", "[ and ]"),
        ("Move V · Marquee M · Ellipse O · Lasso L · Polygon P", ""),
        ("Wand W · Quick selection Q · Magic eraser E · Eraser X", ""),
        ("Restore R · Brush B · Bucket G · Eyedropper I", ""),
        ("Text T · Shapes U · Crop C · Hand H (or Space) · Zoom Z", ""),
        ("Delete selected pixels", "Delete"),
        ("Cancel / deselect", "Esc"),
        ("Nudge layer (Move tool)", "Arrow keys (Shift = 10 px)"),
        ("Keyboard shortcuts", "F1"),
    ];
    egui::Grid::new("shortcuts").num_columns(2).spacing(Vec2::new(18.0, 6.0)).show(ui, |ui| {
        for (a, k) in rows {
            ui.label(a);
            ui.label(egui::RichText::new(k).monospace().color(p.secondary));
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    ui.label(small(p, "Ctrl corresponds to ⌘ on the Mac and Alt to ⌥."));
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if primary_small(ui, p, "Done", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *close = true;
        }
    });
}

#[allow(dead_code)]
fn _u(_: Action) {}
