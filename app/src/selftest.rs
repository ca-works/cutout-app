//! Scripted end-to-end UI tests. With `CUTOUT_SCRIPT=<file.json>` the app runs a
//! list of steps against the real window (clicks, drags, keys, screenshots,
//! assertions) and exits with a non-zero status if an assertion fails. The same
//! script runs on Windows, macOS and Linux, which is how the Windows CI checks
//! the installed-style binary without a human.
//!
//! Steps (objects with one key): `wait` (frames), `open` (path), `paste`,
//! `tool`, `action`, `run` ({cmd,…}), `click` ([x,y], optional `mods`), `drag`
//! ([[x,y],…]), `key`, `scroll`, `appearance`, `shot` (png path), `expect`,
//! `clipboard_roundtrip`, `export` (png path), `quit`.

use crate::app::{Action, App, CopyAction, Tool};
use crate::codec::Format;
use crate::models::RemovalMode;
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct Script {
    steps: Vec<Value>,
    idx: usize,
    wait: u32,
    sleep_until: Option<std::time::Instant>,
    pub pending: Vec<Event>,
    log: Vec<Value>,
    failed: bool,
    shot_path: Option<PathBuf>,
    result_path: Option<PathBuf>,
    started: std::time::Instant,
}

impl Script {
    pub fn from_env() -> Option<Script> {
        let p = std::env::var_os("CUTOUT_SCRIPT")?;
        let mut text = std::fs::read_to_string(&p).ok()?;
        // ${DATA} / ${OUT} expand to CUTOUT_TEST_DATA / CUTOUT_TEST_OUT (JSON-escaped paths).
        for (tok, var) in [("${DATA}", "CUTOUT_TEST_DATA"), ("${OUT}", "CUTOUT_TEST_OUT")] {
            if let Some(v) = std::env::var_os(var) {
                let esc = v.to_string_lossy().replace('\\', "/");
                text = text.replace(tok, &esc);
            }
        }
        let steps: Vec<Value> = serde_json::from_str(&text).ok()?;
        Some(Script { steps, idx: 0, wait: 3, sleep_until: None, pending: vec![], log: vec![], failed: false, shot_path: None, result_path: std::env::var_os("CUTOUT_SCRIPT_RESULT").map(PathBuf::from), started: std::time::Instant::now() })
    }
}

fn mods(s: &str) -> Modifiers {
    let mut m = Modifiers::NONE;
    for part in s.split('+') {
        match part.trim().to_lowercase().as_str() {
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            "ctrl" | "cmd" | "command" => {
                m.ctrl = true;
                m.command = true;
            }
            _ => {}
        }
    }
    m
}

fn action_named(n: &str) -> Option<Action> {
    Some(match n {
        "Open" => Action::Open,
        "Paste" => Action::Paste,
        "PasteNewImage" => Action::PasteNewImage,
        "Copy" => Action::Copy(CopyAction::Copy, None),
        "CopyPng" => Action::Copy(CopyAction::Copy, Some(Format::Png)),
        "CopyAndClear" => Action::Copy(CopyAction::CopyAndClear, None),
        "Undo" => Action::Undo,
        "Redo" => Action::Redo,
        "SelectAll" => Action::SelectAll,
        "Deselect" => Action::Deselect,
        "DeleteSelection" => Action::DeleteSelection,
        "InvertSelection" => Action::InvertSelection,
        "CropToSelection" => Action::CropToSelection,
        "KeepOnlySelection" => Action::KeepOnlySelection,
        "RemoveBg" => Action::RemoveBg(RemovalMode::Best),
        "RemoveBgFast" => Action::RemoveBg(RemovalMode::Fast),
        "Compare" => Action::Compare,
        "ToggleTools" => Action::ToggleTools,
        "ToggleLayers" => Action::ToggleLayers,
        "Fit" => Action::Fit,
        "Actual" => Action::Actual,
        "ZoomIn" => Action::ZoomIn,
        "ZoomOut" => Action::ZoomOut,
        "Flatten" => Action::Flatten,
        "DuplicateLayer" => Action::DuplicateLayer,
        "MergeDown" => Action::MergeDown,
        "ClearWorkspace" => Action::ClearWorkspace,
        "ManageModels" => Action::ManageModels,
        "Settings" => Action::Settings,
        "About" => Action::About,
        "Shortcuts" => Action::Shortcuts,
        "ResizeImage" => Action::ResizeImage,
        "CanvasSize" => Action::CanvasSize,
        "Trim" => Action::Trim,
        "RefineEdges" => Action::RefineEdges,
        _ => return None,
    })
}

fn key_named(n: &str) -> Option<egui::Key> {
    egui::Key::from_name(n)
}

impl App {
    /// Feed events produced by the script into egui's raw input.
    pub fn script_inject(&mut self, raw: &mut egui::RawInput) {
        if let Some(s) = &mut self.script {
            raw.events.append(&mut s.pending);
        }
    }

    pub fn script_screenshot_received(&mut self, image: &egui::ColorImage) {
        let Some(s) = &mut self.script else { return };
        if let Some(p) = s.shot_path.take() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let buf: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            if let Some(img) = image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, buf) {
                let _ = img.save(&p);
            }
            s.log.push(json!({"shot": p.to_string_lossy()}));
        }
    }

    fn script_fail(&mut self, msg: String) {
        eprintln!("SCRIPT FAIL: {msg}");
        if let Some(s) = &mut self.script {
            s.failed = true;
            s.log.push(json!({"fail": msg}));
        }
    }

    fn script_expect(&mut self, e: &Value) {
        let st = self.doc.state.clone();
        let mut problems = vec![];
        if let Some(n) = e.get("layers").and_then(Value::as_u64) {
            if st.layers.len() as u64 != n {
                problems.push(format!("layers: expected {n}, got {}", st.layers.len()));
            }
        }
        if let Some(b) = e.get("selection").and_then(Value::as_bool) {
            if st.has_selection != b {
                problems.push(format!("selection: expected {b}, got {}", st.has_selection));
            }
        }
        if let Some(a) = e.get("size").and_then(Value::as_array) {
            let (w, h) = (a[0].as_i64().unwrap_or(0) as i32, a[1].as_i64().unwrap_or(0) as i32);
            if (st.width, st.height) != (w, h) {
                problems.push(format!("size: expected {w}x{h}, got {}x{}", st.width, st.height));
            }
        }
        if let Some(b) = e.get("empty").and_then(Value::as_bool) {
            if st.is_empty() != b {
                problems.push(format!("empty: expected {b}"));
            }
        }
        if let Some(b) = e.get("can_undo").and_then(Value::as_bool) {
            if st.can_undo != b {
                problems.push(format!("can_undo: expected {b}"));
            }
        }
        if let Some(b) = e.get("has_mask").and_then(Value::as_bool) {
            let got = st.active_layer().map(|l| l.has_mask).unwrap_or(false);
            if got != b {
                problems.push(format!("has_mask: expected {b}, got {got}"));
            }
        }
        if let Some(t) = e.get("tool").and_then(Value::as_str) {
            if self.tool.id() != t {
                problems.push(format!("tool: expected {t}, got {}", self.tool.id()));
            }
        }
        if let Some(b) = e.get("transparent_pixels").and_then(Value::as_bool) {
            let got = self.doc.export(false).map(|i| i.has_transparency()).unwrap_or(false);
            if got != b {
                problems.push(format!("transparent_pixels: expected {b}, got {got}"));
            }
        }
        if let Some(b) = e.get("toast_error").and_then(Value::as_bool) {
            let got = self.toast.as_ref().map(|t| t.error).unwrap_or(false);
            if got != b {
                problems.push(format!("toast_error: expected {b}"));
            }
        }
        if let Some(b) = e.get("show_tools").and_then(Value::as_bool) {
            if self.settings.show_tools != b {
                problems.push(format!("show_tools: expected {b}"));
            }
        }
        if let Some(b) = e.get("show_layers").and_then(Value::as_bool) {
            if self.settings.show_layers != b {
                problems.push(format!("show_layers: expected {b}"));
            }
        }
        if let Some(b) = e.get("sheet").and_then(Value::as_bool) {
            if self.sheet.is_some() != b {
                problems.push(format!("sheet open: expected {b}"));
            }
        }
        if let Some(b) = e.get("busy").and_then(Value::as_bool) {
            if self.busy.is_some() != b {
                problems.push(format!("busy: expected {b}"));
            }
        }
        if let Some(r) = e.get("pixel") {
            // {"pixel": {"x":..,"y":..,"rgba":[..]}} on the flattened export
            let (x, y) = (r["x"].as_u64().unwrap_or(0) as usize, r["y"].as_u64().unwrap_or(0) as usize);
            if let Some(img) = self.doc.export(false) {
                let o = (y * img.width as usize + x) * 4;
                let got: Vec<u64> = img.data[o..o + 4].iter().map(|v| *v as u64).collect();
                let want: Vec<u64> = r["rgba"].as_array().map(|a| a.iter().map(|v| v.as_u64().unwrap_or(0)).collect()).unwrap_or_default();
                let tol = r.get("tol").and_then(Value::as_u64).unwrap_or(2);
                if got.iter().zip(want.iter()).any(|(a, b)| a.abs_diff(*b) > tol) {
                    problems.push(format!("pixel ({x},{y}): expected {want:?}, got {got:?}"));
                }
            }
        }
        if let Some(r) = e.get("pixel_alpha") {
            let (x, y) = (r["x"].as_u64().unwrap_or(0) as usize, r["y"].as_u64().unwrap_or(0) as usize);
            if let Some(img) = self.doc.export(false) {
                let a = img.data[(y * img.width as usize + x) * 4 + 3] as u64;
                if let Some(le) = r.get("le").and_then(Value::as_u64) {
                    if a > le {
                        problems.push(format!("alpha at ({x},{y}) is {a}, expected <= {le}"));
                    }
                }
                if let Some(ge) = r.get("ge").and_then(Value::as_u64) {
                    if a < ge {
                        problems.push(format!("alpha at ({x},{y}) is {a}, expected >= {ge}"));
                    }
                }
            }
        }
        for p in problems {
            self.script_fail(p);
        }
    }

    pub fn script_tick(&mut self, ctx: &egui::Context) {
        let Some(s) = &mut self.script else { return };
        // Screenshots can take a frame or two to arrive.
        if s.shot_path.is_some() {
            ctx.request_repaint();
            return;
        }
        if let Some(t) = s.sleep_until {
            if std::time::Instant::now() < t {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
                return;
            }
            s.sleep_until = None;
        }
        if s.wait > 0 {
            s.wait -= 1;
            ctx.request_repaint();
            return;
        }
        // Don't advance while a background removal / model task is running.
        if self.removal.is_some() || self.model_task.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let Some(s) = &mut self.script else { return };
        if s.idx >= s.steps.len() {
            self.script_finish(ctx);
            return;
        }
        let step = s.steps[s.idx].clone();
        s.idx += 1;
        s.log.push(step.clone());
        ctx.request_repaint();
        let Some(obj) = step.as_object() else { return };
        let Some((k, v)) = obj.iter().next() else { return };
        match k.as_str() {
            "wait" => {
                if let Some(s) = &mut self.script {
                    s.wait = v.as_u64().unwrap_or(1) as u32;
                }
            }
            "sleep" => {
                // real time, so animations finish before a screenshot
                if let Some(s) = &mut self.script {
                    s.sleep_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(v.as_u64().unwrap_or(500)));
                }
            }
            "open" => {
                let p = PathBuf::from(v.as_str().unwrap_or(""));
                let as_layers = !self.is_empty();
                self.open_paths(&[p], as_layers);
            }
            "paste" => self.perform(Action::Paste),
            "tool" => match Tool::from_id(v.as_str().unwrap_or("")) {
                Some(t) => self.set_tool(t),
                None => self.script_fail(format!("unknown tool {v}")),
            },
            "action" => match action_named(v.as_str().unwrap_or("")) {
                Some(a) => self.perform(a),
                None => self.script_fail(format!("unknown action {v}")),
            },
            "run" => {
                let cmd = v.get("cmd").and_then(Value::as_str).unwrap_or("").to_string();
                if self.run_ex(&cmd, v.clone(), true, false).is_none() && obj.get("expect_error").is_none() {
                    self.script_fail(format!("command {cmd} failed"));
                }
            }
            "set" => {
                // {"set": {"tolerance": 20, ...}} — tool settings by field name
                if let Ok(mut cur) = serde_json::to_value(&self.settings.tools) {
                    if let (Some(c), Some(n)) = (cur.as_object_mut(), v.as_object()) {
                        for (k, val) in n {
                            c.insert(k.clone(), val.clone());
                        }
                    }
                    if let Ok(t) = serde_json::from_value(cur) {
                        self.settings.tools = t;
                    }
                }
            }
            "click" => {
                let a = v.as_array().cloned().unwrap_or_default();
                let pos = Pos2::new(a.first().and_then(Value::as_f64).unwrap_or(0.0) as f32, a.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32);
                let m = obj.get("mods").and_then(Value::as_str).map(mods).unwrap_or(Modifiers::NONE);
                if let Some(s) = &mut self.script {
                    s.pending.push(Event::PointerMoved(pos));
                    s.pending.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: m });
                    s.pending.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: m });
                    s.wait = 2;
                }
            }
            "drag" => {
                let pts: Vec<Pos2> = v.as_array().cloned().unwrap_or_default().iter().filter_map(|p| p.as_array().map(|a| Pos2::new(a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32))).collect();
                let m = obj.get("mods").and_then(Value::as_str).map(mods).unwrap_or(Modifiers::NONE);
                if let (Some(s), Some(first)) = (&mut self.script, pts.first().copied()) {
                    s.pending.push(Event::PointerMoved(first));
                    s.pending.push(Event::PointerButton { pos: first, button: PointerButton::Primary, pressed: true, modifiers: m });
                    for p in pts.iter().skip(1) {
                        s.pending.push(Event::PointerMoved(*p));
                    }
                    let last = *pts.last().unwrap();
                    s.pending.push(Event::PointerButton { pos: last, button: PointerButton::Primary, pressed: false, modifiers: m });
                    s.wait = 3;
                }
            }
            "key" => {
                let name = v.as_str().unwrap_or("");
                let m = obj.get("mods").and_then(Value::as_str).map(mods).unwrap_or(Modifiers::NONE);
                match key_named(name) {
                    Some(key) => {
                        if let Some(s) = &mut self.script {
                            s.pending.push(Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: m });
                            s.pending.push(Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: m });
                            s.wait = 2;
                        }
                    }
                    None => self.script_fail(format!("unknown key {name}")),
                }
            }
            "appearance" => {
                self.settings.appearance = match v.as_str().unwrap_or("") {
                    "light" => crate::settings::Appearance::Light,
                    "dark" => crate::settings::Appearance::Dark,
                    _ => crate::settings::Appearance::System,
                };
                self.apply_appearance();
                if let Some(s) = &mut self.script {
                    s.wait = 4;
                }
            }
            "shot" => {
                let p = PathBuf::from(v.as_str().unwrap_or("shot.png"));
                if let Some(s) = &mut self.script {
                    s.shot_path = Some(p);
                }
                let slot = self.shot_slot.clone();
                let c2 = ctx.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    *slot.lock().unwrap() = Some(img);
                    c2.request_repaint();
                })));
            }
            "expect" => self.script_expect(v),
            "export" => {
                let p = PathBuf::from(v.as_str().unwrap_or("out.png"));
                match self.doc.export(false).ok_or("nothing to export".to_string()).and_then(|i| crate::codec::encode(&i, Format::Png)).and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string())) {
                    Ok(()) => {}
                    Err(e) => self.script_fail(format!("export: {e}")),
                }
            }
            "clipboard_roundtrip" => self.script_clipboard_roundtrip(),
            "text" => {
                let props = crate::engine::TextProps { text: v.get("text").and_then(Value::as_str).unwrap_or("CutOut").to_string(), font: self.settings.tools.font_name.clone(), size: v.get("size").and_then(Value::as_f64).unwrap_or(64.0), color: [255, 255, 255, 255], bold: true, italic: false, align: "left".into() };
                let (x, y) = (v.get("x").and_then(Value::as_f64).unwrap_or(40.0), v.get("y").and_then(Value::as_f64).unwrap_or(40.0));
                self.commit_text(props, None, (x, y));
            }
            "quit" => self.script_finish(ctx),
            other => self.script_fail(format!("unknown step {other}")),
        }
    }

    fn script_clipboard_roundtrip(&mut self) {
        if !cfg!(windows) {
            return; // the development clipboard on other systems does not preserve alpha exactly
        }
        // 3×2 image with opaque, semi-transparent and fully transparent pixels.
        let img = crate::engine::Image { width: 3, height: 2, data: vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 255, 200, 100, 50, 64, 255, 255, 255, 255] };
        if let Err(e) = crate::platform::clipboard::write(&img, Format::Png) {
            return self.script_fail(format!("clipboard write: {e}"));
        }
        match crate::platform::clipboard::read() {
            Ok(items) => {
                let got = &items[0].image;
                // Straight-alpha colours under alpha 0 are not meaningful; compare alpha and visible colour.
                let mut ok = got.width == 3 && got.height == 2;
                if ok {
                    for i in 0..6 {
                        let (a, b) = (&img.data[i * 4..i * 4 + 4], &got.data[i * 4..i * 4 + 4]);
                        if a[3] != b[3] || (a[3] > 0 && a[..3].iter().zip(b[..3].iter()).any(|(x, y)| x.abs_diff(*y) > 1)) {
                            ok = false;
                        }
                    }
                }
                if !ok {
                    self.script_fail(format!("clipboard round trip mismatch: {:?}", got.data));
                }
            }
            Err(e) => self.script_fail(format!("clipboard read: {e}")),
        }
    }

    fn script_finish(&mut self, _ctx: &egui::Context) {
        let Some(s) = &mut self.script else { return };
        let failed = s.failed;
        if let Some(p) = &s.result_path {
            let report = json!({"ok": !failed, "steps": s.idx, "elapsed_ms": s.started.elapsed().as_millis() as u64, "log": s.log});
            let _ = std::fs::write(p, serde_json::to_vec_pretty(&report).unwrap_or_default());
        }
        self.script = None;
        self.settings.save();
        std::process::exit(if failed { 1 } else { 0 });
    }
}
