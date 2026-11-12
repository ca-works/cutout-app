//! The editing canvas: zoom/pan, tile rendering, overlays and every pointer
//! interaction. A port of the macOS `CanvasNSView`.

use crate::app::{Action, App, CopyAction, PanelKind, Tool};
use crate::engine::Transform;
use crate::tiles::{self, TileSet};
use eframe::egui::{self, epaint::Shadow, Color32, ColorImage, CursorIcon, Event, Key, MouseWheelUnit, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureWrapMode, Vec2};
use serde_json::json;
use std::f64::consts::PI;
use std::time::Instant;

type P = (f64, f64);

#[derive(Clone)]
pub enum Drag {
    None,
    Pan { start: Pos2, off0: Vec2 },
    Brush,
    Marquee { start: P, ellipse: bool },
    Lasso(Vec<P>),
    Quick(Vec<P>),
    Move { start: P, t0: Transform, began: bool },
    Scale { start: P, t0: Transform, began: bool },
    Rotate { start: P, t0: Transform, began: bool },
    Shape { start: P },
    Crop { start: P },
    Compare,
}

struct Anim {
    t0: Instant,
    from: (f32, Vec2),
    to: (f32, Vec2),
}

pub struct CanvasState {
    pub tiles: Option<TileSet>,
    pub cmp_tiles: Option<TileSet>,
    pub reveal: Option<(TileSet, Instant)>,
    checker: Option<(TextureHandle, bool)>,
    /// Points per image pixel.
    pub scale: f32,
    /// Position of image pixel (0,0) relative to the canvas rectangle.
    pub offset: Vec2,
    pub rect: Rect,
    pub img_size: (usize, usize),
    pub fitted: bool,
    anim: Option<Anim>,
    pub drag: Drag,
    pub drag_current: P,
    pub polygon: Vec<P>,
    pub crop_rect: Option<[f64; 4]>,
    pub space_down: bool,
    pub last_mouse: Option<Pos2>,
    pub compare_fraction: f32,
    last_click: Option<(Instant, Pos2)>,
    pub ppp: f32,
    pub hover: bool,
    dark: bool,
    /// Canvas size and side-panel visibility seen on the previous frame (viewport-change detection).
    last_view: Option<(Vec2, bool, bool)>,
    /// Image-space rectangle to fit (None = the document). Refreshed only when a fit is requested,
    /// never while the user is dragging, so editing can't make the view jump.
    pub fit_bounds: Option<Rect>,
    pub bounds_dirty: bool,
}

impl CanvasState {
    pub fn new() -> CanvasState {
        CanvasState {
            tiles: None,
            cmp_tiles: None,
            reveal: None,
            checker: None,
            scale: 1.0,
            offset: Vec2::ZERO,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)),
            img_size: (0, 0),
            fitted: true,
            anim: None,
            drag: Drag::None,
            drag_current: (0.0, 0.0),
            polygon: vec![],
            crop_rect: None,
            space_down: false,
            last_mouse: None,
            compare_fraction: 0.5,
            last_click: None,
            ppp: 1.0,
            hover: false,
            dark: true,
            last_view: None,
            fit_bounds: None,
            bounds_dirty: true,
        }
    }

    pub fn theme_changed(&mut self) {
        self.checker = None;
    }

    pub fn view_to_image(&self, p: Pos2) -> P {
        let o = self.rect.min + self.offset;
        (((p.x - o.x) / self.scale) as f64, ((p.y - o.y) / self.scale) as f64)
    }

    pub fn image_to_view(&self, p: P) -> Pos2 {
        self.rect.min + self.offset + Vec2::new(p.0 as f32 * self.scale, p.1 as f32 * self.scale)
    }

    fn image_rect(&self) -> Rect {
        Rect::from_min_size(self.rect.min + self.offset, Vec2::new(self.img_size.0 as f32 * self.scale, self.img_size.1 as f32 * self.scale))
    }

    /// Image-space rectangle the fit has to show: the document, plus (see `App::compute_fit_bounds`)
    /// the active layer's transform box when it extends past the document.
    pub fn has_anim(&self) -> bool {
        self.anim.is_some()
    }

    pub fn bounds(&self) -> Rect {
        self.fit_bounds.unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, Vec2::new(self.img_size.0 as f32, self.img_size.1 as f32)))
    }

    fn fit_scale(&self) -> f32 {
        if self.img_size.0 == 0 {
            return 1.0;
        }
        let pad = 48.0; // leaves room for the rotation handle (26 pt beyond the edge) and the corner handles
        let b = self.bounds();
        let s = ((self.rect.width() - pad * 2.0) / b.width().max(1.0)).min((self.rect.height() - pad * 2.0) / b.height().max(1.0));
        s.clamp(0.005, 1.0) // never upscale beyond 1 pt per pixel when fitting
    }

    fn fit_target(&self) -> (f32, Vec2) {
        let s = self.fit_scale();
        let b = self.bounds();
        (s, Vec2::new(self.rect.width() / 2.0 - b.center().x * s, self.rect.height() / 2.0 - b.center().y * s))
    }

    fn clamp_offset(&mut self) {
        let r = self.image_rect();
        let m = 64.0;
        let b = self.rect;
        if r.max.x < b.min.x + m {
            self.offset.x += b.min.x + m - r.max.x;
        }
        if r.min.x > b.max.x - m {
            self.offset.x -= r.min.x - (b.max.x - m);
        }
        if r.max.y < b.min.y + m {
            self.offset.y += b.min.y + m - r.max.y;
        }
        if r.min.y > b.max.y - m {
            self.offset.y -= r.min.y - (b.max.y - m);
        }
    }

    pub fn zoom_to_fit(&mut self, animated: bool) {
        self.fitted = true;
        self.bounds_dirty = true;
        if animated && self.img_size.0 > 0 {
            self.anim = Some(Anim { t0: Instant::now(), from: (self.scale, self.offset), to: self.fit_target() });
        } else {
            let (s, o) = self.fit_target();
            self.scale = s;
            self.offset = o;
            self.anim = None;
        }
    }

    /// `device_zoom` = device pixels per image pixel (1 = Actual Size).
    pub fn set_zoom_device(&mut self, device_zoom: f32, anchor: Option<Pos2>, animated: bool) {
        if self.img_size.0 == 0 {
            return;
        }
        let new_scale = (device_zoom / self.ppp).clamp(0.005, 64.0);
        let a = anchor.unwrap_or_else(|| self.rect.center());
        let ip = self.view_to_image(a);
        let from = (self.scale, self.offset);
        let off = Vec2::new(a.x - self.rect.min.x - ip.0 as f32 * new_scale, a.y - self.rect.min.y - ip.1 as f32 * new_scale);
        self.fitted = false;
        if animated {
            self.scale = new_scale;
            self.offset = off;
            self.clamp_offset();
            let to = (self.scale, self.offset);
            self.scale = from.0;
            self.offset = from.1;
            self.anim = Some(Anim { t0: Instant::now(), from, to });
        } else {
            self.anim = None;
            self.scale = new_scale;
            self.offset = off;
            self.clamp_offset();
        }
    }

    pub fn zoom_by(&mut self, factor: f32, anchor: Option<Pos2>, animated: bool) {
        let cur = self.target_scale() * self.ppp;
        self.set_zoom_device(cur * factor, anchor, animated);
    }

    fn target_scale(&self) -> f32 {
        self.anim.as_ref().map(|a| a.to.0).unwrap_or(self.scale)
    }

    pub fn zoom_step(&mut self, up: bool) {
        const STEPS: [f32; 21] = [0.05, 0.1, 0.125, 0.167, 0.25, 0.333, 0.5, 0.667, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0];
        let z = self.target_scale() * self.ppp;
        let f = if up {
            STEPS.iter().find(|&&n| n > z * 1.01).map(|n| n / z)
        } else {
            STEPS.iter().rev().find(|&&n| n < z * 0.99).map(|n| n / z)
        };
        if let Some(f) = f {
            self.zoom_by(f, None, true);
        }
    }

    /// React to the visible canvas area changing (side panels opening/closing, window or
    /// splitter resize). Purely a camera change: document coordinates are never touched.
    /// * A panel toggle re-fits the whole document into the new area. While the panel slides,
    ///   the fitted state tracks the shrinking/growing rectangle every frame, which is the
    ///   smooth transition and cannot accumulate error (the target is recomputed from scratch).
    /// * Any other size change keeps a fitted view fitted, and keeps a zoomed view anchored on
    ///   the same image point at the centre of the canvas.
    pub fn viewport_changed(&mut self, tools: bool, layers: bool, editing: bool) {
        let size = self.rect.size();
        let prev = self.last_view.replace((size, tools, layers));
        let Some((psize, ptools, players)) = prev else { return };
        if self.img_size.0 == 0 || editing {
            return;
        }
        if tools != ptools || layers != players {
            self.fitted = true;
            self.bounds_dirty = true;
            self.anim = None;
        } else if !self.fitted && (size - psize).length() > 0.5 {
            self.offset += (size - psize) / 2.0;
            self.anim = None;
            self.clamp_offset();
        }
    }

    /// Advance zoom animation / fit; returns true while something is animating.
    fn tick(&mut self) -> bool {
        // A fit animation always aims at the *current* fit target, so a fit started while a panel is
        // still sliding (or the window is resizing) cannot end on a stale size and snap afterwards.
        if self.fitted && self.anim.is_some() {
            let to = self.fit_target();
            if let Some(a) = &mut self.anim {
                a.to = to;
            }
        }
        if let Some(a) = &self.anim {
            let t = (a.t0.elapsed().as_secs_f32() / 0.22).min(1.0);
            let e = 1.0 - (1.0 - t).powi(3);
            self.scale = a.from.0 + (a.to.0 - a.from.0) * e;
            self.offset = a.from.1 + (a.to.1 - a.from.1) * e;
            if t >= 1.0 {
                self.anim = None;
            }
            return true;
        }
        if self.fitted {
            let (s, o) = self.fit_target();
            self.scale = s;
            self.offset = o;
        }
        false
    }

    pub fn set_compare(&mut self, on: bool, premult: Option<(&egui::Context, &[u8], usize, usize)>) {
        self.cmp_tiles = match (on, premult) {
            (true, Some((ctx, data, w, h))) => Some(TileSet::new(ctx, "cmp", data, w, h, self.scale * self.ppp >= 2.0)),
            _ => None,
        };
    }

    fn checker_texture(&mut self, ctx: &egui::Context, dark: bool) -> TextureHandle {
        if let Some((t, d)) = &self.checker {
            if *d == dark {
                return t.clone();
            }
        }
        // 2×2 cells of 8 pt, drawn at 4 px per point so it stays crisp when scaled.
        let (a, b) = if dark { (0.30, 0.24) } else { (1.0, 0.86) };
        let ca = Color32::from_gray((a * 255.0) as u8);
        let cb = Color32::from_gray((b * 255.0) as u8);
        let n = 8;
        let mut px = vec![ca; n * 2 * n * 2];
        for y in 0..n * 2 {
            for x in 0..n * 2 {
                if (x < n && y < n) || (x >= n && y >= n) {
                    px[y * n * 2 + x] = cb;
                }
            }
        }
        // `cb` occupies the top-left and bottom-right cells, as in the macOS app.
        let img = ColorImage { size: [n * 2, n * 2], source_size: Vec2::new((n * 2) as f32, (n * 2) as f32), pixels: px };
        let t = ctx.load_texture("checker", img, egui::TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Nearest, wrap_mode: TextureWrapMode::Repeat, mipmap_mode: None });
        self.checker = Some((t.clone(), dark));
        t
    }
}

fn mode_for(app: &App, m: egui::Modifiers) -> &'static str {
    if m.shift && m.alt {
        "intersect"
    } else if m.shift {
        "add"
    } else if m.alt {
        "subtract"
    } else {
        app.sel_mode.id()
    }
}

fn pts_json(p: &[P]) -> serde_json::Value {
    json!(p.iter().map(|q| [q.0, q.1]).collect::<Vec<_>>())
}

impl App {
    // ------------------------------------------------ engine → tiles ---

    pub fn canvas_update_from_engine(&mut self, full: bool) {
        let ctx = self.ctx.clone();
        let nearest = self.canvas.scale * self.canvas.ppp >= 2.0;
        let frame = self.doc.eng.render();
        let (w, h) = (frame.width as usize, frame.height as usize);
        let dirty = (frame.dirty.x, frame.dirty.y, frame.dirty.w, frame.dirty.h);
        let size_changed = (w, h) != self.canvas.img_size;
        if size_changed || full {
            self.canvas.tiles = if w > 0 && h > 0 { Some(TileSet::new(&ctx, "doc", frame.data, w, h, nearest)) } else { None };
            self.canvas.img_size = (w, h);
            if size_changed {
                self.canvas.bounds_dirty = true;
                if self.tool == Tool::Crop {
                    self.canvas.crop_rect = if w > 0 { Some([0.0, 0.0, w as f64, h as f64]) } else { None };
                }
                if !self.canvas.fitted && w > 0 {
                    self.canvas.clamp_offset();
                }
            }
        } else if let Some(t) = &mut self.canvas.tiles {
            t.update(frame.data, dirty);
            // Switch the magnification filter when crossing 200 %.
            t.set_nearest(frame.data, nearest);
        }
    }

    pub fn canvas_prepare_reveal(&mut self) {
        if self.canvas.img_size.0 == 0 || self.reduce_motion {
            return;
        }
        let ctx = self.ctx.clone();
        let nearest = self.canvas.scale * self.canvas.ppp >= 2.0;
        let (w, h) = self.canvas.img_size;
        let frame = self.doc.eng.render();
        if frame.width as usize == w && frame.height as usize == h {
            self.canvas.reveal = Some((TileSet::new(&ctx, "reveal", frame.data, w, h, nearest), Instant::now()));
        }
    }

    pub fn apply_compare(&mut self) {
        if self.compare_on && !self.is_empty() {
            let data = self.doc.eng.render_unmasked_premultiplied();
            let (w, h) = (self.state().width as usize, self.state().height as usize);
            let ctx = self.ctx.clone();
            self.canvas.set_compare(true, Some((&ctx, &data, w, h)));
            self.canvas.compare_fraction = 0.5;
        } else {
            self.compare_on = false;
            self.canvas.set_compare(false, None);
        }
    }

    pub fn canvas_tool_changed(&mut self) {
        self.canvas.polygon.clear();
        if self.tool != Tool::Crop {
            self.canvas.crop_rect = None;
        } else if self.canvas.crop_rect.is_none() && self.canvas.img_size.0 > 0 {
            self.canvas.crop_rect = Some([0.0, 0.0, self.canvas.img_size.0 as f64, self.canvas.img_size.1 as f64]);
        }
    }

    pub fn apply_crop(&mut self) {
        let Some([x0, y0, x1, y1]) = self.canvas.crop_rect else { return };
        let (x, y, w, h) = (x0.round(), y0.round(), (x1 - x0).round(), (y1 - y0).round());
        if w < 1.0 || h < 1.0 {
            return;
        }
        if (w as usize, h as usize) == self.canvas.img_size && x == 0.0 && y == 0.0 {
            return;
        }
        self.run("crop", json!({"x": x, "y": y, "w": w, "h": h}));
        self.canvas.crop_rect = Some([0.0, 0.0, self.canvas.img_size.0 as f64, self.canvas.img_size.1 as f64]);
        self.canvas.zoom_to_fit(true);
    }

    fn finish_polygon(&mut self, mode: Option<&str>) {
        if self.canvas.polygon.len() > 2 {
            let m = mode.unwrap_or(self.sel_mode.id()).to_string();
            let pts = pts_json(&self.canvas.polygon);
            let f = self.settings.tools.feather;
            self.run("select_polygon", json!({"points": pts, "combine": m, "feather": f}));
        }
        self.canvas.polygon.clear();
    }

    /// Corners of the active layer in view coordinates (tl, tr, br, bl).
    pub fn active_quad(&self) -> Option<[Pos2; 4]> {
        let l = self.state().active_layer()?;
        let t = l.transform;
        let (w, h) = (l.width as f64, l.height as f64);
        let (c, s) = (t.rotation.cos(), t.rotation.sin());
        let map = |x: f64, y: f64| {
            let (lx, ly) = ((x - w / 2.0) * t.scale_x, (y - h / 2.0) * t.scale_y);
            self.canvas.image_to_view((c * lx - s * ly + t.x, s * lx + c * ly + t.y))
        };
        Some([map(0.0, 0.0), map(w, 0.0), map(w, h), map(0.0, h)])
    }

    /// Image-space bounds to fit: the document, grown to include the active layer's transform box
    /// and rotation handle when that box sticks out of the document (capped at one extra document
    /// size per side so an extreme scale can't shrink the view to nothing). Camera-only.
    fn compute_fit_bounds(&self) -> Option<Rect> {
        let (w, h) = (self.canvas.img_size.0 as f32, self.canvas.img_size.1 as f32);
        let doc = Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h));
        let l = self.state().active_layer()?;
        let t = l.transform;
        let (lw, lh) = (l.width as f64, l.height as f64);
        let (c, s) = (t.rotation.cos(), t.rotation.sin());
        let map = |x: f64, y: f64| {
            let (lx, ly) = ((x - lw / 2.0) * t.scale_x, (y - lh / 2.0) * t.scale_y);
            Pos2::new((c * lx - s * ly + t.x) as f32, (s * lx + c * ly + t.y) as f32)
        };
        let mut b = doc;
        for p in [map(0.0, 0.0), map(lw, 0.0), map(lw, lh), map(0.0, lh)] {
            b.extend_with(p);
        }
        let cap = doc.expand2(Vec2::new(w, h));
        let b = b.intersect(cap);
        (b != doc && b.is_positive()).then_some(b)
    }

    pub fn rotate_handle(q: &[Pos2; 4]) -> Pos2 {
        let top = Pos2::new((q[0].x + q[1].x) / 2.0, (q[0].y + q[1].y) / 2.0);
        let center = Pos2::new((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0);
        let d = top - center;
        let len = d.length().max(1.0);
        top + d / len * 26.0
    }

    fn set_transform(&mut self, t: Transform) {
        if let Some(id) = self.state().active {
            self.run_ex("set_transform", json!({"id": id, "transform": t.json(), "record": false}), true, true);
        }
    }

    fn brush_args(&self, p: P) -> serde_json::Value {
        let s = &self.settings.tools;
        match self.tool {
            Tool::Eraser => json!({"mode": "erase", "size": s.eraser_size, "hardness": s.eraser_hardness, "opacity": s.eraser_strength, "color": [0, 0, 0, 0], "x": p.0, "y": p.1}),
            Tool::Restore => json!({"mode": "restore", "size": s.restore_size, "hardness": s.restore_hardness, "opacity": s.restore_strength, "color": [0, 0, 0, 0], "x": p.0, "y": p.1}),
            _ => json!({"mode": "paint", "size": s.brush_size, "hardness": s.brush_hardness, "opacity": s.brush_opacity, "color": s.color_rgba(), "x": p.0, "y": p.1}),
        }
    }

    // ------------------------------------------------------- pointer ---

    fn canvas_mouse_down(&mut self, v: Pos2, m: egui::Modifiers) {
        let p = self.canvas.view_to_image(v);
        self.canvas.drag_current = p;
        if self.is_empty() {
            return;
        }
        let now = Instant::now();
        let double = self.canvas.last_click.map(|(t, q)| now.duration_since(t).as_millis() < 450 && (q - v).length() < 6.0).unwrap_or(false);
        self.canvas.last_click = if double { None } else { Some((now, v)) };
        if self.compare_on {
            self.canvas.drag = Drag::Compare;
            self.move_compare(v);
            return;
        }
        if self.canvas.space_down || self.tool == Tool::Hand {
            self.canvas.anim = None;
            self.canvas.drag = Drag::Pan { start: v, off0: self.canvas.offset };
            return;
        }
        let s = self.settings.tools.clone();
        match self.tool {
            Tool::Zoom => {
                self.canvas.zoom_by(if m.alt { 0.5 } else { 2.0 }, Some(v), true);
            }
            Tool::Move => {
                if let Some(q) = self.active_quad() {
                    let rh = Self::rotate_handle(&q);
                    let t = self.state().active_layer().map(|l| l.transform);
                    if let Some(t) = t {
                        if (v - rh).length() < 10.0 {
                            self.canvas.drag = Drag::Rotate { start: p, t0: t, began: false };
                            return;
                        }
                        if q.iter().any(|c| (v - *c).length() < 9.0) {
                            self.canvas.drag = Drag::Scale { start: p, t0: t, began: false };
                            return;
                        }
                    }
                }
                // auto-select the top-most layer under the cursor
                let hit = self.state().layers.iter().rev().find(|l| l.visible && l.bounds.contains(p.0, p.1)).map(|l| l.id);
                let active_contains = self.state().active_layer().map(|l| l.bounds.contains(p.0, p.1)).unwrap_or(false);
                if let Some(id) = hit {
                    if Some(id) != self.state().active && !active_contains {
                        self.select_layer(id, false);
                    }
                }
                if double {
                    if let Some(l) = self.state().active_layer() {
                        if l.is_text {
                            let id = l.id;
                            self.open_text_sheet(Some(id), (0.0, 0.0));
                            return;
                        }
                    }
                }
                if let Some(t) = self.state().active_layer().map(|l| l.transform) {
                    self.canvas.drag = Drag::Move { start: p, t0: t, began: false };
                }
            }
            Tool::RectSelect | Tool::EllipseSelect => self.canvas.drag = Drag::Marquee { start: p, ellipse: self.tool == Tool::EllipseSelect },
            Tool::Lasso => self.canvas.drag = Drag::Lasso(vec![p]),
            Tool::PolygonLasso => {
                let near_first = self.canvas.polygon.len() > 2 && (v - self.canvas.image_to_view(self.canvas.polygon[0])).length() < 8.0;
                if double || near_first {
                    let mode = mode_for(self, m);
                    self.finish_polygon(Some(mode));
                } else {
                    self.canvas.polygon.push(p);
                }
            }
            Tool::MagicWand => {
                let mode = mode_for(self, m);
                self.run("magic_wand", json!({"x": p.0, "y": p.1, "tolerance": s.tolerance, "contiguous": s.contiguous, "antialias": s.antialias, "sample_all": s.sample_all, "combine": mode, "feather": s.feather}));
            }
            Tool::QuickSelect => {
                self.canvas.drag = Drag::Quick(vec![p]);
                let mode = if m.alt { "subtract" } else { "add" };
                self.run("quick_select", json!({"points": [[p.0, p.1]], "radius": s.quick_size / 2.0, "edge": s.quick_edge, "begin": true, "combine": mode, "sample_all": true}));
            }
            Tool::MagicEraser => {
                self.run("magic_erase", json!({"x": p.0, "y": p.1, "tolerance": s.tolerance, "contiguous": s.contiguous, "antialias": s.antialias, "feather": s.erase_feather}));
            }
            Tool::Eraser | Tool::Restore | Tool::Brush => {
                let args = self.brush_args(p);
                if self.run_ex("brush_begin", args, false, false).is_some() {
                    self.canvas.drag = Drag::Brush;
                    self.canvas_update_from_engine(false);
                } else {
                    self.refresh(false);
                }
            }
            Tool::Fill => {
                self.run("fill", json!({"x": p.0, "y": p.1, "tolerance": s.tolerance, "contiguous": s.contiguous, "color": s.color_rgba(), "opacity": s.fill_opacity, "sample_all": s.sample_all}));
            }
            Tool::Eyedropper => {
                if let Some(r) = self.run_ex("color_at", json!({"x": p.0, "y": p.1, "sample_all": true}), false, false) {
                    if let Some(c) = r.get("color").and_then(|c| c.as_array()) {
                        let v: Vec<f32> = c.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32 / 255.0).collect();
                        if v.len() >= 3 {
                            self.settings.tools.color = [v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0)];
                            self.show_toast("Picked colour", crate::icons::PIPETTE, false);
                        }
                    }
                }
            }
            Tool::Text => self.open_text_sheet(None, p),
            Tool::Shape => self.canvas.drag = Drag::Shape { start: p },
            Tool::Crop => self.canvas.drag = Drag::Crop { start: p },
            Tool::Hand => {}
        }
    }

    fn canvas_mouse_dragged(&mut self, v: Pos2, m: egui::Modifiers) {
        let p = self.canvas.view_to_image(v);
        self.canvas.drag_current = p;
        self.canvas.last_mouse = Some(v);
        let scale = self.canvas.scale as f64;
        match self.canvas.drag.clone() {
            Drag::None => {}
            Drag::Compare => self.move_compare(v),
            Drag::Pan { start, off0 } => {
                self.canvas.offset = off0 + (v - start);
                self.canvas.fitted = false;
                self.canvas.clamp_offset();
            }
            Drag::Brush => {
                if self.run_ex("brush_move", json!({"x": p.0, "y": p.1}), false, true).is_some() {
                    self.canvas_update_from_engine(false);
                }
            }
            Drag::Marquee { .. } | Drag::Shape { .. } => {}
            Drag::Lasso(mut pts) => {
                if let Some(l) = pts.last() {
                    if ((l.0 - p.0).hypot(l.1 - p.1)) * scale > 2.0 {
                        pts.push(p);
                        self.canvas.drag = Drag::Lasso(pts);
                    }
                }
            }
            Drag::Quick(mut pts) => {
                if let Some(l) = pts.last().copied() {
                    if ((l.0 - p.0).hypot(l.1 - p.1)) * scale > 3.0 {
                        pts.push(p);
                        self.canvas.drag = Drag::Quick(pts);
                        let s = &self.settings.tools;
                        let args = json!({"points": [[l.0, l.1], [p.0, p.1]], "radius": s.quick_size / 2.0, "edge": s.quick_edge, "sample_all": true});
                        self.run_ex("quick_select", args, true, true);
                    }
                }
            }
            Drag::Move { start, t0, began } => {
                if !began {
                    self.run_ex("checkpoint", json!({"label": "Move Layer"}), false, false);
                    self.canvas.drag = Drag::Move { start, t0, began: true };
                }
                let (mut dx, mut dy) = (p.0 - start.0, p.1 - start.1);
                if m.shift {
                    if dx.abs() > dy.abs() {
                        dy = 0.0
                    } else {
                        dx = 0.0
                    }
                }
                let mut t = t0;
                t.x += dx;
                t.y += dy;
                self.set_transform(t);
            }
            Drag::Scale { start, t0, began } => {
                if !began {
                    self.run_ex("checkpoint", json!({"label": "Scale Layer"}), false, false);
                    self.canvas.drag = Drag::Scale { start, t0, began: true };
                }
                let c = (t0.x, t0.y);
                let d0 = ((start.0 - c.0).hypot(start.1 - c.1)).max(1.0);
                let f = ((p.0 - c.0).hypot(p.1 - c.1) / d0).max(0.01);
                let mut t = t0;
                t.scale_x = t0.scale_x * f;
                t.scale_y = t0.scale_y * f;
                self.set_transform(t);
            }
            Drag::Rotate { start, t0, began } => {
                if !began {
                    self.run_ex("checkpoint", json!({"label": "Rotate Layer"}), false, false);
                    self.canvas.drag = Drag::Rotate { start, t0, began: true };
                }
                let c = (t0.x, t0.y);
                let mut a = t0.rotation + (p.1 - c.1).atan2(p.0 - c.0) - (start.1 - c.1).atan2(start.0 - c.0);
                if m.shift {
                    a = (a / (PI / 12.0)).round() * (PI / 12.0);
                }
                let mut t = t0;
                t.rotation = a;
                self.set_transform(t);
            }
            Drag::Crop { start } => {
                let (w, h) = (self.canvas.img_size.0 as f64, self.canvas.img_size.1 as f64);
                let x0 = start.0.min(p.0).clamp(0.0, w);
                let y0 = start.1.min(p.1).clamp(0.0, h);
                let x1 = start.0.max(p.0).clamp(0.0, w);
                let y1 = start.1.max(p.1).clamp(0.0, h);
                self.canvas.crop_rect = if x1 > x0 && y1 > y0 { Some([x0, y0, x1, y1]) } else { None };
            }
        }
    }

    fn canvas_mouse_up(&mut self, v: Pos2, m: egui::Modifiers) {
        let p = self.canvas.view_to_image(v);
        let s = self.settings.tools.clone();
        let scale = self.canvas.scale as f64;
        let drag = std::mem::replace(&mut self.canvas.drag, Drag::None);
        match drag {
            Drag::Brush => {
                self.run("brush_end", json!({}));
            }
            Drag::Marquee { start, ellipse } => {
                let (x, y, w, h) = (start.0.min(p.0), start.1.min(p.1), (p.0 - start.0).abs(), (p.1 - start.1).abs());
                let mode = mode_for(self, m);
                if w * scale < 2.0 || h * scale < 2.0 {
                    if mode == "replace" {
                        self.run("deselect", json!({}));
                    }
                } else {
                    self.run(if ellipse { "select_ellipse" } else { "select_rect" }, json!({"x": x, "y": y, "w": w, "h": h, "combine": mode, "feather": s.feather}));
                }
            }
            Drag::Lasso(pts) => {
                if pts.len() > 2 {
                    let mode = mode_for(self, m);
                    self.run("select_polygon", json!({"points": pts_json(&pts), "combine": mode, "feather": s.feather}));
                }
            }
            Drag::Shape { start } => {
                if (p.0 - start.0).hypot(p.1 - start.1) * scale > 3.0 {
                    let kind = s.shape_kind.clone();
                    let mut args = json!({"kind": kind, "x0": start.0, "y0": start.1, "x1": p.0, "y1": p.1, "stroke": s.color_rgba(), "width": s.shape_stroke, "corner": s.shape_corner});
                    if s.shape_filled && (kind == "rect" || kind == "ellipse") {
                        args["fill"] = json!(s.fill_rgba());
                    }
                    self.run("add_shape", args);
                }
            }
            _ => {}
        }
    }

    fn move_compare(&mut self, v: Pos2) {
        let r = self.canvas.image_rect();
        if r.width() > 0.0 {
            self.canvas.compare_fraction = ((v.x - r.min.x) / r.width()).clamp(0.0, 1.0);
        }
    }

    // ---------------------------------------------------------- keys ---

    fn canvas_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.sheet.is_some() {
            return;
        }
        let (space, keys, mods) = ctx.input(|i| {
            let ks: Vec<(Key, egui::Modifiers)> = i.events.iter().filter_map(|e| if let Event::Key { key, pressed: true, modifiers, repeat, .. } = e { if *repeat && !matches!(key, Key::OpenBracket | Key::CloseBracket | Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown) { None } else { Some((*key, *modifiers)) } } else { None }).collect();
            (i.key_down(Key::Space), ks, i.modifiers)
        });
        if space != self.canvas.space_down {
            self.canvas.space_down = space;
        }
        let _ = mods;
        for (key, m) in keys {
            let plain = !(m.command || m.ctrl || m.alt);
            match key {
                Key::Escape => {
                    if !self.canvas.polygon.is_empty() {
                        self.canvas.polygon.clear();
                    } else if self.tool == Tool::Crop {
                        self.canvas.crop_rect = Some([0.0, 0.0, self.canvas.img_size.0 as f64, self.canvas.img_size.1 as f64]);
                    } else if self.compare_on {
                        self.compare_on = false;
                        self.apply_compare();
                    } else if self.is_removing() {
                        self.cancel_removal();
                    } else if self.state().has_selection {
                        self.run("deselect", json!({}));
                    }
                }
                Key::Enter if plain => {
                    if self.tool == Tool::PolygonLasso {
                        self.finish_polygon(None);
                    } else if self.tool == Tool::Crop {
                        self.apply_crop();
                    }
                }
                Key::Backspace | Key::Delete if plain => {
                    if self.state().has_selection {
                        self.run("delete_selection", json!({}));
                    }
                }
                Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown if self.tool == Tool::Move && plain => {
                    let step = if m.shift { 10.0 } else { 1.0 };
                    let (dx, dy) = match key {
                        Key::ArrowLeft => (-step, 0.0),
                        Key::ArrowRight => (step, 0.0),
                        Key::ArrowUp => (0.0, -step),
                        _ => (0.0, step),
                    };
                    self.run("nudge", json!({"dx": dx, "dy": dy}));
                }
                Key::OpenBracket | Key::CloseBracket if plain => {
                    let t = self.tool;
                    let cur = size_for(&self.settings.tools, t);
                    let nv = if key == Key::CloseBracket { cur * 1.15 + 1.0 } else { cur / 1.15 - 1.0 };
                    set_size_for(&mut self.settings.tools, t, nv);
                }
                k if plain => {
                    if let Some(t) = Tool::ALL.into_iter().find(|t| t.key() == k) {
                        self.set_tool(t);
                    }
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------ ui ---

    pub fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.available_rect_before_wrap();
        let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
        self.canvas.rect = rect;
        self.canvas.ppp = ctx.pixels_per_point();
        let enabled = self.sheet.is_none() && self.busy.is_none();
        self.canvas.hover = response.contains_pointer() && enabled;

        if !self.is_empty() {
            self.canvas_keys(&ctx);
        }
        let events: Vec<Event> = ctx.input(|i| i.events.clone());
        let mods = ctx.input(|i| i.modifiers);
        if !self.is_empty() && enabled {
            for e in events {
                match e {
                    Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers } => {
                        if self.canvas.hover && rect.contains(pos) {
                            self.canvas.last_mouse = Some(pos);
                            self.canvas_mouse_down(pos, modifiers);
                        }
                    }
                    Event::PointerMoved(pos) => {
                        self.canvas.last_mouse = Some(pos);
                        if !matches!(self.canvas.drag, Drag::None) {
                            self.canvas_mouse_dragged(pos, mods);
                        }
                    }
                    Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers } => {
                        if !matches!(self.canvas.drag, Drag::None) {
                            self.canvas_mouse_up(pos, modifiers);
                        }
                    }
                    Event::MouseWheel { unit, delta, modifiers, .. } if self.canvas.hover => {
                        let anchor = self.canvas.last_mouse;
                        let zoom = modifiers.command || modifiers.ctrl || (unit == MouseWheelUnit::Line && !modifiers.shift);
                        if zoom {
                            let dy = if unit == MouseWheelUnit::Line { delta.y * 0.12 } else { delta.y * 0.01 };
                            self.canvas.zoom_by((dy).exp(), anchor, false);
                        } else {
                            let k = if unit == MouseWheelUnit::Line { 24.0 } else { 1.0 };
                            self.canvas.anim = None;
                            self.canvas.offset += Vec2::new(delta.x, delta.y) * k;
                            self.canvas.fitted = false;
                            self.canvas.clamp_offset();
                        }
                    }
                    _ => {}
                }
            }
        }
        // Zoom gestures from a trackpad (pinch) arrive as a zoom delta.
        if self.canvas.hover && !self.is_empty() {
            let z = ctx.input(|i| i.zoom_delta());
            if (z - 1.0).abs() > 1e-4 && !mods.command && !mods.ctrl {
                let a = self.canvas.last_mouse;
                self.canvas.zoom_by(z, a, false);
            }
        }
        let editing = !matches!(self.canvas.drag, Drag::None);
        self.canvas.viewport_changed(self.settings.show_tools, self.settings.show_layers, editing);
        if self.canvas.bounds_dirty && !editing {
            self.canvas.bounds_dirty = false;
            let nb = if self.is_empty() { None } else { self.compute_fit_bounds() };
            if nb != self.canvas.fit_bounds {
                self.canvas.fit_bounds = nb;
                if self.canvas.anim.is_some() && self.canvas.fitted {
                    let to = self.canvas.fit_target();
                    if let Some(a) = &mut self.canvas.anim {
                        a.to = to;
                    }
                }
            }
        }
        if self.canvas.tick() {
            ctx.request_repaint();
        }

        let painter = ui.painter_at(rect);
        let p = self.palette;
        let dark = p.dark;

        if self.is_empty() {
            self.empty_state(ui, rect);
        } else {
            self.paint_document(&painter, &ctx, dark);
            self.paint_overlays(&painter, &ctx);
            self.set_cursor(&ctx);
            self.canvas_context_menu(&response);
        }
        // Brush ring.
        if self.canvas.hover && self.tool.is_brush() && !self.canvas.space_down && !self.is_empty() {
            if let Some(m) = self.canvas.last_mouse {
                let d = size_for(&self.settings.tools, self.tool) as f32 * self.canvas.scale;
                painter.circle_stroke(m, d / 2.0 + 0.5, Stroke::new(2.0, Color32::from_black_alpha(110)));
                painter.circle_stroke(m, d / 2.0, Stroke::new(1.0, Color32::WHITE));
            }
        }
    }

    fn set_cursor(&self, ctx: &egui::Context) {
        if !self.canvas.hover {
            return;
        }
        let icon = if matches!(self.canvas.drag, Drag::Pan { .. }) {
            CursorIcon::Grabbing
        } else if self.canvas.space_down || self.tool == Tool::Hand {
            CursorIcon::Grab
        } else if self.compare_on {
            CursorIcon::ResizeHorizontal
        } else {
            match self.tool {
                Tool::Move => CursorIcon::Default,
                Tool::Text => CursorIcon::Text,
                _ => CursorIcon::Crosshair,
            }
        };
        ctx.set_cursor_icon(icon);
    }

    fn paint_document(&mut self, painter: &Painter, ctx: &egui::Context, dark: bool) {
        let c = &mut self.canvas;
        let ir = c.image_rect();
        if ir.width() < 1.0 || c.tiles.is_none() {
            return;
        }
        let ppp = c.ppp;
        let snap = |v: f32| (v * ppp).round() / ppp;
        let ir_s = Rect::from_min_max(Pos2::new(snap(ir.min.x), snap(ir.min.y)), Pos2::new(snap(ir.max.x), snap(ir.max.y)));
        // soft shadow under the image
        painter.add(Shadow { offset: [0, 2], blur: 16, spread: 0, color: Color32::from_black_alpha(if dark { 128 } else { 46 }) }.as_shape(ir_s, 0));
        // checkerboard (8 pt cells, fixed in view space)
        let ck = c.checker_texture(ctx, dark);
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(ir_s.width() / 16.0, ir_s.height() / 16.0));
        painter.image(ck.id(), ir_s, uv, Color32::WHITE);
        let origin = Pos2::new(snap(ir.min.x), snap(ir.min.y));
        if let Some(t) = &c.tiles {
            t.paint(painter, origin, c.scale, Color32::WHITE);
        }
        if let Some((r, t0)) = &c.reveal {
            let t = (t0.elapsed().as_secs_f32() / 0.55).min(1.0);
            let e = t * t * (3.0 - 2.0 * t);
            let a = ((1.0 - e) * 255.0) as u8;
            if t >= 1.0 {
                c.reveal = None;
            } else {
                r.paint(painter, origin, c.scale, Color32::from_white_alpha(a));
                ctx.request_repaint();
            }
        }
        if self.compare_on {
            if let Some(cmp) = &self.canvas.cmp_tiles {
                let ir = self.canvas.image_rect();
                let x = ir.min.x + ir.width() * self.canvas.compare_fraction;
                let clip = Rect::from_min_max(ir.min, Pos2::new(x, ir.max.y));
                let sub = painter.with_clip_rect(painter.clip_rect().intersect(clip));
                sub.rect_filled(ir_s, 0.0, Color32::TRANSPARENT);
                // repaint the checkerboard below the "before" half too so it matches
                sub.image(self.canvas.checker.as_ref().unwrap().0.id(), ir_s, uv, Color32::WHITE);
                cmp.paint(&sub, origin, self.canvas.scale, Color32::WHITE);
                painter.rect_filled(Rect::from_min_max(Pos2::new(x - 1.0, ir.min.y), Pos2::new(x + 1.0, ir.max.y)), 0.0, Color32::WHITE);
                painter.rect_stroke(Rect::from_min_max(Pos2::new(x - 1.5, ir.min.y), Pos2::new(x + 1.5, ir.max.y)), 0.0, Stroke::new(0.5, Color32::from_black_alpha(90)), StrokeKind::Outside);
            }
        }
    }

    fn paint_overlays(&mut self, painter: &Painter, ctx: &egui::Context) {
        let c = &self.canvas;
        let accent = self.palette.accent;
        let white = Stroke::new(1.0, Color32::from_white_alpha(240));
        let black = Stroke::new(1.0, Color32::from_black_alpha(205));
        let dash = |painter: &Painter, pts: &[Pos2], closed: bool| {
            let mut v = pts.to_vec();
            if closed {
                if let Some(f) = v.first().copied() {
                    v.push(f);
                }
            }
            painter.add(Shape::line(v.clone(), white));
            painter.extend(Shape::dashed_line(&v, black, 4.0, 4.0));
        };

        // Selection outline (marching ants).
        if !self.outline.is_empty() && !self.compare_on {
            let t = ctx.input(|i| i.time) as f32;
            let phase = (t * 14.0) % 8.0;
            let n = self.outline.len() / 4;
            let animate = n <= 4000 && !self.reduce_motion;
            let clip = painter.clip_rect().expand(2.0);
            let mut white_shapes: Vec<Shape> = Vec::with_capacity(n.min(8000));
            let mut black_shapes: Vec<Shape> = Vec::new();
            for i in 0..n {
                let o = &self.outline[i * 4..i * 4 + 4];
                let a = c.image_to_view((o[0] as f64, o[1] as f64));
                let b = c.image_to_view((o[2] as f64, o[3] as f64));
                if !clip.intersects(Rect::from_two_pos(a, b).expand(1.0)) {
                    continue;
                }
                white_shapes.push(Shape::line_segment([a, b], white));
                if animate {
                    black_shapes.extend(Shape::dashed_line_with_offset(&[a, b], black, &[4.0], &[4.0], phase));
                } else if i % 2 == 0 {
                    black_shapes.push(Shape::line_segment([a, b], black));
                }
            }
            painter.extend(white_shapes);
            painter.extend(black_shapes);
            // dashes are painted before/after in the same pass; keep animating
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }

        let scale = c.scale;
        let cur = c.drag_current;
        match &c.drag {
            Drag::Marquee { start, ellipse } => {
                let (a, b) = (c.image_to_view(*start), c.image_to_view(cur));
                let r = Rect::from_two_pos(a, b);
                if *ellipse {
                    let pts = ellipse_pts(r);
                    dash(painter, &pts, true);
                } else {
                    dash(painter, &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], true);
                }
            }
            Drag::Lasso(pts) => {
                let v: Vec<Pos2> = pts.iter().map(|p| c.image_to_view(*p)).collect();
                if v.len() > 1 {
                    dash(painter, &v, false);
                }
            }
            Drag::Shape { start } => {
                let (a, b) = (c.image_to_view(*start), c.image_to_view(cur));
                match self.settings.tools.shape_kind.as_str() {
                    "ellipse" => dash(painter, &ellipse_pts(Rect::from_two_pos(a, b)), true),
                    "line" | "arrow" => dash(painter, &[a, b], false),
                    _ => {
                        let r = Rect::from_two_pos(a, b);
                        dash(painter, &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], true)
                    }
                }
            }
            _ => {}
        }
        if self.tool == Tool::PolygonLasso && !c.polygon.is_empty() {
            let mut v: Vec<Pos2> = c.polygon.iter().map(|p| c.image_to_view(*p)).collect();
            if let Some(m) = c.last_mouse {
                if c.hover {
                    v.push(m);
                }
            }
            dash(painter, &v, false);
        }
        if self.tool == Tool::Crop {
            if let Some([x0, y0, x1, y1]) = c.crop_rect {
                let ir = c.image_rect();
                let a = c.image_to_view((x0, y0));
                let b = c.image_to_view((x1, y1));
                let r = Rect::from_two_pos(a, b);
                let dim = Color32::from_black_alpha(115);
                painter.rect_filled(Rect::from_min_max(ir.min, Pos2::new(ir.max.x, r.min.y)), 0.0, dim);
                painter.rect_filled(Rect::from_min_max(Pos2::new(ir.min.x, r.max.y), ir.max), 0.0, dim);
                painter.rect_filled(Rect::from_min_max(Pos2::new(ir.min.x, r.min.y), Pos2::new(r.min.x, r.max.y)), 0.0, dim);
                painter.rect_filled(Rect::from_min_max(Pos2::new(r.max.x, r.min.y), Pos2::new(ir.max.x, r.max.y)), 0.0, dim);
                dash(painter, &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], true);
                for i in 1..=2 {
                    let fx = r.min.x + r.width() * i as f32 / 3.0;
                    let fy = r.min.y + r.height() * i as f32 / 3.0;
                    painter.line_segment([Pos2::new(fx, r.min.y), Pos2::new(fx, r.max.y)], Stroke::new(1.0, Color32::from_white_alpha(110)));
                    painter.line_segment([Pos2::new(r.min.x, fy), Pos2::new(r.max.x, fy)], Stroke::new(1.0, Color32::from_white_alpha(110)));
                }
                for corner in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
                    let h = Rect::from_center_size(corner, Vec2::splat(8.0));
                    painter.rect(h, 0.0, Color32::WHITE, Stroke::new(1.0, accent), StrokeKind::Middle);
                }
            }
        }
        if self.tool == Tool::Move && !self.compare_on {
            if let Some(q) = self.active_quad() {
                let mut line = q.to_vec();
                line.push(q[0]);
                painter.add(Shape::line(line, Stroke::new(1.0, accent)));
                let rh = Self::rotate_handle(&q);
                let top = Pos2::new((q[0].x + q[1].x) / 2.0, (q[0].y + q[1].y) / 2.0);
                painter.line_segment([top, rh], Stroke::new(1.0, accent));
                for corner in q {
                    painter.circle(corner, 4.5, Color32::WHITE, Stroke::new(1.0, accent));
                }
                painter.circle(rh, 5.0, Color32::WHITE, Stroke::new(1.0, accent));
            }
        }
        let _ = scale;
    }

    fn canvas_context_menu(&mut self, response: &egui::Response) {
        let act: std::cell::RefCell<Option<Action>> = std::cell::RefCell::new(None);
        let has_sel = self.state().has_selection;
        response.context_menu(|ui| {
            let item = |ui: &mut egui::Ui, label: &str, a: Action| {
                if ui.button(label).clicked() {
                    *act.borrow_mut() = Some(a);
                    ui.close();
                }
            };
            item(ui, "Copy", Action::Copy(CopyAction::Copy, None));
            item(ui, "Paste as Layer", Action::Paste);
            ui.separator();
            item(ui, "Remove Background", Action::RemoveFlatBackground);
            item(ui, "AI Remove Background", Action::RemoveBg(crate::models::RemovalMode::Best));
            if has_sel {
                item(ui, "Delete Selected Pixels", Action::DeleteSelection);
                item(ui, "Keep Only Selection", Action::KeepOnlySelection);
                item(ui, "Invert Selection", Action::InvertSelection);
                item(ui, "Crop to Selection", Action::CropToSelection);
                item(ui, "Deselect", Action::Deselect);
            } else {
                item(ui, "Select All", Action::SelectAll);
            }
            ui.separator();
            item(ui, "Duplicate Layer", Action::DuplicateLayer);
            item(ui, "Merge Down", Action::MergeDown);
            item(ui, "Restore All Erased Pixels", Action::RestoreAll);
        });
        let chosen = act.into_inner();
        if let Some(a) = chosen {
            self.perform(a);
        }
        let _ = PanelKind::None;
    }

    fn empty_state(&mut self, ui: &mut egui::Ui, rect: Rect) {
        crate::ui::empty_state(self, ui, rect);
    }
}

fn ellipse_pts(r: Rect) -> Vec<Pos2> {
    let (c, rx, ry) = (r.center(), r.width() / 2.0, r.height() / 2.0);
    (0..96).map(|i| {
        let a = i as f32 / 96.0 * std::f32::consts::TAU;
        Pos2::new(c.x + rx * a.cos(), c.y + ry * a.sin())
    }).collect()
}

pub fn size_for(s: &crate::settings::ToolSettings, t: Tool) -> f64 {
    match t {
        Tool::Eraser => s.eraser_size,
        Tool::Restore => s.restore_size,
        Tool::QuickSelect => s.quick_size,
        _ => s.brush_size,
    }
}

pub fn set_size_for(s: &mut crate::settings::ToolSettings, t: Tool, v: f64) {
    let v = v.clamp(1.0, 2000.0);
    match t {
        Tool::Eraser => s.eraser_size = v,
        Tool::Restore => s.restore_size = v,
        Tool::QuickSelect => s.quick_size = v,
        _ => s.brush_size = v,
    }
}

#[allow(dead_code)]
fn _unused(_: &tiles::TileSet) {}
