//! The editing engine: owns the document, selection, history and display
//! buffer, and exposes a small JSON command protocol used by the native UIs
//! (SwiftUI on macOS today, a Windows UI later) plus a handful of binary
//! entry points for bulk pixel transfer.

use crate::adjust::{self, Adjustments};
use crate::brush::{rasterize_shape, BrushParams, ShapeParams, Stroke};
use crate::color::ColorKey;
use crate::composite::{composite_region, composite_to_straight};
use crate::document::{Document, Layer, LayerId, LayerKind, TextProps};
use crate::filters;
use crate::geometry::{IRect, Transform};
use crate::history::{History, Snapshot};
use crate::image::{check_dimensions, mul255, Plane, Rgba};
use crate::selection::{self, Combine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct Frame<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub dirty: IRect,
    pub generation: u64,
}

struct WandCache {
    sample_all: bool,
    layer: Option<LayerId>,
    version: u64,
    seed_color: [u8; 4],
    source: Arc<Vec<u8>>,
    dist: Vec<u16>,
}

struct SourceCache {
    sample_all: bool,
    layer: Option<LayerId>,
    version: u64,
    data: Arc<Vec<u8>>,
}

pub struct Engine {
    pub doc: Document,
    pub selection: Option<Plane>,
    history: History,
    display: Vec<u8>,
    display_size: (u32, u32),
    dirty: IRect,
    generation: u64,
    version: u64,
    saved_version: u64,
    stroke: Option<Stroke>,
    preview: Option<Layer>,
    wand: Option<WandCache>,
    source: Option<SourceCache>,
    quick_base: Option<Option<Plane>>,
    quick_mode: Combine,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

fn err(msg: impl Into<String>) -> Value {
    json!({"ok": false, "error": msg.into()})
}

fn ok() -> Value {
    json!({"ok": true})
}

fn f(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(d)
}
fn b(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(d)
}
fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str())
}
fn id_of(v: &Value) -> Option<LayerId> {
    v.get("id").and_then(|x| x.as_i64())
}
fn color_of(v: &Value, k: &str) -> Option<[u8; 4]> {
    let a = v.get(k)?.as_array()?;
    if a.len() < 3 {
        return None;
    }
    let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(255.0).clamp(0.0, 255.0) as u8;
    Some([g(0), g(1), g(2), if a.len() > 3 { g(3) } else { 255 }])
}
fn combine_of(v: &Value) -> Combine {
    match s(v, "combine") {
        Some("add") => Combine::Add,
        Some("subtract") => Combine::Subtract,
        Some("intersect") => Combine::Intersect,
        _ => Combine::Replace,
    }
}

#[derive(Deserialize, Default, Clone, Copy, Debug)]
#[serde(default)]
pub struct RefineParams {
    /// Edge-aware refinement radius in px (0 = off).
    pub radius: f32,
    /// Smoothing (px sigma) of jagged boundaries.
    pub smooth: f32,
    /// Feather (px sigma).
    pub feather: f32,
    /// 0..1, steepens the matte transition.
    pub contrast: f32,
    /// Shift edge in px (negative = contract).
    pub shift: f32,
    /// Remove background colour fringing from semi-transparent edges.
    pub decontaminate: bool,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            doc: Document::default(),
            selection: None,
            history: History::default(),
            display: Vec::new(),
            display_size: (0, 0),
            dirty: IRect::default(),
            generation: 0,
            version: 0,
            saved_version: 0,
            stroke: None,
            preview: None,
            wand: None,
            source: None,
            quick_base: None,
            quick_mode: Combine::Add,
        }
    }

    // ---------------------------------------------------------------- helpers

    fn touch(&mut self, r: IRect) {
        self.version += 1;
        self.dirty = self.dirty.union(&r.intersect(&self.doc.canvas_rect()));
    }
    fn touch_all(&mut self) {
        self.version += 1;
        self.dirty = self.doc.canvas_rect();
    }
    fn touch_layer(&mut self, id: LayerId) {
        if let Some(l) = self.doc.layer(id) {
            let r = l.canvas_bounds().inflate(1);
            self.touch(r);
        }
    }

    pub fn checkpoint(&mut self, label: &str) {
        let snap = Snapshot { label: label.to_string(), doc: self.doc.clone(), selection: self.selection.clone() };
        self.history.push(snap, &self.doc);
    }

    fn current_snapshot(&self) -> Snapshot {
        Snapshot { label: String::new(), doc: self.doc.clone(), selection: self.selection.clone() }
    }

    fn restore(&mut self, s: Snapshot) {
        let size_changed = s.doc.width != self.doc.width || s.doc.height != self.doc.height;
        self.doc = s.doc;
        self.selection = s.selection;
        self.preview = None;
        self.stroke = None;
        self.wand = None;
        if size_changed {
            self.ensure_display();
        }
        self.touch_all();
    }

    fn ensure_display(&mut self) {
        let (w, h) = (self.doc.width, self.doc.height);
        if self.display_size != (w, h) {
            self.display = vec![0; w as usize * h as usize * 4];
            self.display_size = (w, h);
            self.dirty = self.doc.canvas_rect();
        }
    }

    pub fn is_modified(&self) -> bool {
        self.version != self.saved_version && !self.doc.is_empty()
    }

    fn active_id(&self) -> Result<LayerId, Value> {
        self.doc.active.filter(|id| self.doc.layer(*id).is_some()).ok_or_else(|| err("No layer is selected."))
    }

    fn layers_for_render(&self, unmasked: bool) -> Vec<Layer> {
        self.doc
            .layers
            .iter()
            .map(|l| {
                let mut l = match &self.preview {
                    Some(p) if p.id == l.id => p.clone(),
                    _ => l.clone(),
                };
                if unmasked {
                    l.mask = None;
                    if let Some(o) = l.original.take() {
                        l.pixels = o;
                    }
                }
                l
            })
            .collect()
    }

    /// Straight RGBA canvas-space image of the active layer, or of all
    /// visible layers. Cached by document version.
    fn source_image(&mut self, sample_all: bool) -> Arc<Vec<u8>> {
        let layer = if sample_all { None } else { self.doc.active };
        if let Some(c) = &self.source {
            if c.sample_all == sample_all && c.layer == layer && c.version == self.version {
                return c.data.clone();
            }
        }
        let rect = self.doc.canvas_rect();
        let data = if sample_all {
            let refs: Vec<&Layer> = self.doc.layers.iter().collect();
            composite_to_straight(&refs, rect)
        } else {
            match self.doc.active_layer() {
                Some(l) => {
                    let mut l = l.clone();
                    l.visible = true;
                    l.opacity = 1.0;
                    composite_to_straight(&[&l], rect)
                }
                None => vec![0; rect.w as usize * rect.h as usize * 4],
            }
        };
        let data = Arc::new(data);
        self.source = Some(SourceCache { sample_all, layer, version: self.version, data: data.clone() });
        data
    }

    fn canvas_point(&self, v: &Value) -> Option<(u32, u32)> {
        let x = f(v, "x", -1.0).floor();
        let y = f(v, "y", -1.0).floor();
        if x < 0.0 || y < 0.0 || x >= self.doc.width as f64 || y >= self.doc.height as f64 {
            None
        } else {
            Some((x as u32, y as u32))
        }
    }

    /// Colour region (canvas coverage) for wand-type tools.
    fn color_region(&mut self, v: &Value) -> Result<Vec<u8>, Value> {
        let (x, y) = self.canvas_point(v).ok_or_else(|| err("Click inside the image."))?;
        let sample_all = b(v, "sample_all", false);
        let tol = f(v, "tolerance", 12.0) as f32;
        let contiguous = b(v, "contiguous", true);
        let soft = if b(v, "antialias", true) { (tol * 0.5).max(1.5) } else { 0.01 };
        let src = self.source_image(sample_all);
        let w = self.doc.width;
        let i = (y as usize * w as usize + x as usize) * 4;
        let seed = [src[i], src[i + 1], src[i + 2], src[i + 3]];
        let layer = if sample_all { None } else { self.doc.active };
        let reuse = matches!(&self.wand, Some(c) if c.sample_all == sample_all && c.layer == layer && c.version == self.version && c.seed_color == seed && Arc::ptr_eq(&c.source, &src));
        if !reuse {
            let dist = selection::distance_map(&src, ColorKey::new(seed));
            self.wand = Some(WandCache { sample_all, layer, version: self.version, seed_color: seed, source: src.clone(), dist });
        }
        let c = self.wand.as_ref().unwrap();
        Ok(selection::threshold_region(&c.dist, w, self.doc.height, (x, y), tol, soft, contiguous))
    }

    fn set_selection(&mut self, cov: Vec<u8>, mode: Combine, feather: f32) {
        let (w, h) = (self.doc.width, self.doc.height);
        let cov = if feather > 0.0 { filters::gaussian_u8(&cov, w as usize, h as usize, feather) } else { cov };
        let p = selection::combine(self.selection.as_ref(), cov, w, h, mode);
        self.selection = if p.any_nonzero() { Some(p) } else { None };
    }

    /// Apply canvas-space coverage to a layer mask. `reveal` = restore.
    fn apply_coverage_to_mask(layer: &mut Layer, cov: &[u8], cw: u32, ch: u32, reveal: bool) {
        let m = layer.matrix();
        let (lw, lh) = (layer.w() as usize, layer.h() as usize);
        let fast = if m.is_integer_translation() { Some((m.e.round() as i64, m.f.round() as i64)) } else { None };
        let cov_plane = Plane { w: cw, h: ch, data: Arc::new(Vec::new()) };
        let _ = cov_plane;
        let sample = |x: usize, y: usize| -> u32 {
            match fast {
                Some((ox, oy)) => {
                    let cx = x as i64 + ox;
                    let cy = y as i64 + oy;
                    if cx < 0 || cy < 0 || cx >= cw as i64 || cy >= ch as i64 {
                        0
                    } else {
                        cov[cy as usize * cw as usize + cx as usize] as u32
                    }
                }
                None => {
                    let (cx, cy) = m.apply(x as f64 + 0.5, y as f64 + 0.5);
                    let (cx, cy) = (cx.floor() as i64, cy.floor() as i64);
                    if cx < 0 || cy < 0 || cx >= cw as i64 || cy >= ch as i64 {
                        0
                    } else {
                        cov[cy as usize * cw as usize + cx as usize] as u32
                    }
                }
            }
        };
        let mut vals = vec![0u8; lw * lh];
        for y in 0..lh {
            for x in 0..lw {
                vals[y * lw + x] = sample(x, y) as u8;
            }
        }
        if !vals.iter().any(|&v| v > 0) {
            return;
        }
        let mask = layer.ensure_mask();
        for (mv, &c) in mask.iter_mut().zip(vals.iter()) {
            let (mo, c) = (*mv as u32, c as u32);
            *mv = if reveal { mo + mul255(255 - mo, c) } else { mo - mul255(mo, c) } as u8;
        }
    }

    fn paint_coverage(layer: &mut Layer, cov: &[u8], cw: u32, ch: u32, color: [u8; 4]) {
        let m = layer.matrix();
        let (lw, lh) = (layer.w() as usize, layer.h() as usize);
        let mut vals = vec![0u8; lw * lh];
        for y in 0..lh {
            for x in 0..lw {
                let (cx, cy) = m.apply(x as f64 + 0.5, y as f64 + 0.5);
                let (cx, cy) = (cx.floor() as i64, cy.floor() as i64);
                if cx >= 0 && cy >= 0 && cx < cw as i64 && cy < ch as i64 {
                    vals[y * lw + x] = cov[cy as usize * cw as usize + cx as usize];
                }
            }
        }
        {
            let px = layer.pixels.make_mut();
            for (i, &c) in vals.iter().enumerate() {
                if c == 0 {
                    continue;
                }
                let sa = mul255(c as u32, color[3] as u32) as f32 / 255.0;
                let p = &mut px[i * 4..i * 4 + 4];
                let da = p[3] as f32 / 255.0;
                let oa = sa + da * (1.0 - sa);
                if oa <= 0.0 {
                    continue;
                }
                for k in 0..3 {
                    p[k] = ((color[k] as f32 * sa + p[k] as f32 * da * (1.0 - sa)) / oa + 0.5).min(255.0) as u8;
                }
                p[3] = (oa * 255.0 + 0.5) as u8;
            }
        }
        if layer.mask.is_some() {
            let mask = layer.ensure_mask();
            for (mv, &c) in mask.iter_mut().zip(vals.iter()) {
                let mo = *mv as u32;
                *mv = (mo + mul255(255 - mo, c as u32)) as u8;
            }
        }
    }

    fn insert_layer(&mut self, mut layer: Layer, above_active: bool) -> LayerId {
        layer.name = self.doc.unique_name(&layer.name);
        let id = layer.id;
        let idx = if above_active { self.doc.active_index().map(|i| i + 1).unwrap_or(self.doc.layers.len()) } else { self.doc.layers.len() };
        self.doc.layers.insert(idx, layer);
        self.doc.active = Some(id);
        self.touch_layer(id);
        id
    }

    // ------------------------------------------------------------ binary API

    /// Import straight RGBA. `as_layer = false` replaces the document.
    pub fn import_rgba(&mut self, rgba: Vec<u8>, w: u32, h: u32, name: &str, as_layer: bool) -> Result<LayerId, String> {
        check_dimensions(w, h)?;
        let pixels = Rgba::from_vec(w, h, rgba)?;
        if !as_layer || self.doc.is_empty() {
            if !self.doc.is_empty() || self.history.can_undo() || self.history.can_redo() {
                self.checkpoint("Open Image");
            }
            let mut doc = Document { width: w, height: h, ..Default::default() };
            let id = doc.alloc_id();
            let mut l = Layer::new(id, if name.is_empty() { "Image" } else { name }, pixels);
            l.transform = Transform::at(0.0, 0.0, w, h);
            doc.layers.push(l);
            doc.active = Some(id);
            self.doc = doc;
            self.selection = None;
            self.preview = None;
            self.ensure_display();
            self.touch_all();
            if self.history.is_empty() {
                self.saved_version = self.version;
            }
            return Ok(id);
        }
        self.checkpoint("Paste Layer");
        let id = self.doc.alloc_id();
        let mut l = Layer::new(id, if name.is_empty() { "Layer" } else { name }, pixels);
        // centre, scaled down to fit the canvas if larger (pixels preserved)
        let (cw, ch) = (self.doc.width as f64, self.doc.height as f64);
        let fit = (cw / w as f64).min(ch / h as f64).min(1.0);
        l.transform = Transform { x: cw / 2.0, y: ch / 2.0, scale_x: fit, scale_y: fit, rotation: 0.0 };
        if fit == 1.0 {
            // keep pixel-exact placement when possible
            let x = ((cw - w as f64) / 2.0).round();
            let y = ((ch - h as f64) / 2.0).round();
            l.transform = Transform::at(x, y, w, h);
        }
        Ok(self.insert_layer(l, true))
    }

    pub fn render(&mut self) -> Frame<'_> {
        self.ensure_display();
        let dirty = self.dirty.intersect(&self.doc.canvas_rect());
        if !dirty.is_empty() {
            let layers = self.layers_for_render(false);
            let refs: Vec<&Layer> = layers.iter().collect();
            composite_region(&refs, &mut self.display, self.doc.width, self.doc.height, (0, 0), dirty);
            self.generation += 1;
        }
        self.dirty = IRect::default();
        Frame { data: &self.display, width: self.doc.width, height: self.doc.height, dirty, generation: self.generation }
    }

    /// Flattened straight-alpha image of the canvas.
    pub fn export_rgba(&self, unmasked: bool) -> (Vec<u8>, u32, u32) {
        let layers = self.layers_for_render(unmasked);
        let refs: Vec<&Layer> = layers.iter().collect();
        (composite_to_straight(&refs, self.doc.canvas_rect()), self.doc.width, self.doc.height)
    }

    /// Premultiplied canvas rendering with all masks disabled (before/after).
    pub fn render_unmasked_premultiplied(&self) -> Vec<u8> {
        let layers = self.layers_for_render(true);
        let refs: Vec<&Layer> = layers.iter().collect();
        let mut out = vec![0u8; self.doc.width as usize * self.doc.height as usize * 4];
        composite_region(&refs, &mut out, self.doc.width, self.doc.height, (0, 0), self.doc.canvas_rect());
        out
    }

    /// Layer source pixels (no mask) for AI inference.
    pub fn layer_pixels(&self, id: LayerId) -> Option<(Vec<u8>, u32, u32)> {
        let l = self.doc.layer(id)?;
        let p = l.original.as_ref().unwrap_or(&l.pixels);
        Some(((*p.data).clone(), p.w, p.h))
    }

    pub fn layer_rgba(&self, id: LayerId) -> Option<Rgba> {
        self.doc.layer(id).map(|layer| layer.pixels.clone())
    }

    /// Intersect with the layer's current visibility, retaining edits to its pixels.
    pub fn intersect_layer_mask(&mut self, id: LayerId, mut mask: Plane, label: &str) -> Result<(), String> {
        let layer = self.doc.layer(id).ok_or("Layer not found")?;
        if (layer.w(), layer.h()) != (mask.w, mask.h) {
            return Err("Mask size does not match the layer.".into());
        }
        if let Some(previous) = &layer.mask {
            for (next, previous) in std::sync::Arc::make_mut(&mut mask.data).iter_mut().zip(previous.data.iter()) {
                *next = (*next).min(*previous);
            }
            if mask.data == previous.data {
                return Ok(());
            }
        }
        self.checkpoint(label);
        self.doc.layer_mut(id).unwrap().mask = Some(mask);
        self.touch_layer(id);
        Ok(())
    }

    pub fn set_layer_mask(&mut self, id: LayerId, alpha: Vec<u8>, w: u32, h: u32, label: &str) -> Result<(), String> {
        let l = self.doc.layer(id).ok_or("Layer not found")?;
        if l.w() != w || l.h() != h {
            return Err("Mask size does not match the layer.".into());
        }
        self.checkpoint(label);
        let l = self.doc.layer_mut(id).unwrap();
        l.mask = Some(Plane::from_vec(w, h, alpha)?);
        if let Some(o) = l.original.take() {
            l.pixels = o;
        }
        self.touch_layer(id);
        Ok(())
    }

    pub fn layer_thumbnail(&self, id: LayerId, max: u32) -> Option<(Vec<u8>, u32, u32)> {
        let l = self.doc.layer(id)?;
        let flat = l.flattened_pixels();
        let scale = (max as f64 / l.w().max(l.h()) as f64).min(1.0);
        let tw = ((l.w() as f64 * scale).round() as u32).max(1);
        let th = ((l.h() as f64 * scale).round() as u32).max(1);
        let mut out = vec![0u8; tw as usize * th as usize * 4];
        for y in 0..th {
            for x in 0..tw {
                // box filter over source footprint
                let x0 = (x as f64 / scale) as u32;
                let y0 = (y as f64 / scale) as u32;
                let x1 = (((x + 1) as f64 / scale) as u32).clamp(x0 + 1, l.w());
                let y1 = (((y + 1) as f64 / scale) as u32).clamp(y0 + 1, l.h());
                let (sx, sy) = (((x1 - x0) / 4).max(1), ((y1 - y0) / 4).max(1));
                let mut acc = [0f64; 4];
                let mut n = 0f64;
                let mut yy = y0;
                while yy < y1 {
                    let mut xx = x0;
                    while xx < x1 {
                        let p = flat.px(xx, yy);
                        let a = p[3] as f64;
                        acc[0] += p[0] as f64 * a;
                        acc[1] += p[1] as f64 * a;
                        acc[2] += p[2] as f64 * a;
                        acc[3] += a;
                        n += 1.0;
                        xx += sx;
                    }
                    yy += sy;
                }
                let i = (y as usize * tw as usize + x as usize) * 4;
                if acc[3] > 0.0 {
                    out[i] = (acc[0] / acc[3]) as u8;
                    out[i + 1] = (acc[1] / acc[3]) as u8;
                    out[i + 2] = (acc[2] / acc[3]) as u8;
                    out[i + 3] = (acc[3] / n) as u8;
                }
            }
        }
        Some((out, tw, th))
    }

    pub fn selection_outline(&self) -> Vec<f32> {
        match &self.selection {
            Some(s) => selection::outline_segments(s, 400_000),
            None => Vec::new(),
        }
    }

    pub fn add_text_layer(&mut self, props: TextProps, rgba: Vec<u8>, w: u32, h: u32, x: f64, y: f64) -> Result<LayerId, String> {
        check_dimensions(w, h)?;
        if self.doc.is_empty() {
            return Err("Open an image first.".into());
        }
        self.checkpoint("Add Text");
        let id = self.doc.alloc_id();
        let name: String = props.text.lines().next().unwrap_or("Text").chars().take(32).collect();
        let mut l = Layer::new(id, if name.trim().is_empty() { "Text".to_string() } else { name }, Rgba::from_vec(w, h, rgba)?);
        l.kind = LayerKind::Text(props);
        l.transform = Transform::at(x.round(), y.round(), w, h);
        Ok(self.insert_layer(l, true))
    }

    pub fn update_text_layer(&mut self, id: LayerId, props: TextProps, rgba: Vec<u8>, w: u32, h: u32) -> Result<(), String> {
        check_dimensions(w, h)?;
        let l = self.doc.layer(id).ok_or("Layer not found")?;
        if !l.is_text() {
            return Err("Not a text layer".into());
        }
        self.checkpoint("Edit Text");
        self.touch_layer(id);
        let l = self.doc.layer_mut(id).unwrap();
        // keep the top-left anchored
        let m = l.matrix();
        let (tlx, tly) = m.apply(0.0, 0.0);
        let t = l.transform;
        l.pixels = Rgba::from_vec(w, h, rgba)?;
        l.mask = None;
        l.original = None;
        let name: String = props.text.lines().next().unwrap_or("Text").chars().take(32).collect();
        l.name = if name.trim().is_empty() { "Text".into() } else { name };
        l.kind = LayerKind::Text(props);
        let mut nt = Transform { x: 0.0, y: 0.0, ..t };
        let (ox, oy) = nt.matrix(w, h).apply(0.0, 0.0);
        nt.x = tlx - ox;
        nt.y = tly - oy;
        l.transform = nt;
        self.touch_layer(id);
        Ok(())
    }

    pub fn save_project(&mut self, path: &std::path::Path) -> Result<(), String> {
        if self.doc.is_empty() {
            return Err("Nothing to save.".into());
        }
        let (rgba, w, h) = self.export_rgba(false);
        // small preview (≤ 512 px)
        let scale = (512.0 / w.max(h) as f64).min(1.0);
        let pw = ((w as f64 * scale).round() as u32).max(1);
        let ph = ((h as f64 * scale).round() as u32).max(1);
        let prev = crate::image::resize_rgba(&Rgba::from_vec(w, h, rgba)?, pw, ph);
        crate::project::save_to_path(&self.doc, Some((&prev.data, pw, ph)), path)?;
        self.saved_version = self.version;
        Ok(())
    }

    pub fn open_project(&mut self, path: &std::path::Path) -> Result<(), String> {
        let doc = crate::project::load_from_path(path)?;
        self.doc = doc;
        self.selection = None;
        self.preview = None;
        self.history.clear();
        self.ensure_display();
        self.touch_all();
        self.saved_version = self.version;
        Ok(())
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    // -------------------------------------------------------------- commands

    pub fn command(&mut self, v: &Value) -> Value {
        let cmd = s(v, "cmd").unwrap_or("");
        let needs_doc = !matches!(cmd, "state" | "clear" | "new_document" | "undo" | "redo" | "open_project");
        if needs_doc && self.doc.is_empty() {
            return err("Open or paste an image first.");
        }
        let record = b(v, "record", true);
        match self.dispatch(cmd, v, record) {
            Ok(Value::Null) => ok(),
            Ok(mut r) => {
                if let Some(o) = r.as_object_mut() {
                    o.insert("ok".into(), Value::Bool(true));
                }
                r
            }
            Err(e) => e,
        }
    }

    pub fn state(&self) -> Value {
        let layers: Vec<Value> = self
            .doc
            .layers
            .iter()
            .map(|l| {
                let mut o = json!({
                    "id": l.id, "name": l.name, "visible": l.visible, "opacity": l.opacity, "locked": l.locked,
                    "width": l.w(), "height": l.h(), "has_mask": l.mask.is_some(),
                    "transform": l.transform, "kind": match &l.kind { LayerKind::Raster => "raster", LayerKind::Text(_) => "text" },
                    "bounds": l.canvas_bounds(),
                });
                if let LayerKind::Text(t) = &l.kind {
                    o["text"] = serde_json::to_value(t).unwrap_or(Value::Null);
                }
                o
            })
            .collect();
        json!({
            "ok": true,
            "width": self.doc.width, "height": self.doc.height,
            "layers": layers, "active": self.doc.active,
            "can_undo": self.history.can_undo(), "can_redo": self.history.can_redo(),
            "undo_label": self.history.undo_label(), "redo_label": self.history.redo_label(),
            "has_selection": self.selection.is_some(),
            "selection_bounds": self.selection.as_ref().and_then(|s| s.bbox(0)),
            "modified": self.is_modified(),
            "previewing": self.preview.is_some(),
            "version": self.version,
        })
    }

    fn dispatch(&mut self, cmd: &str, v: &Value, record: bool) -> Result<Value, Value> {
        macro_rules! cp {
            ($l:expr) => {
                if record {
                    self.checkpoint($l);
                }
            };
        }
        match cmd {
            "state" => Ok(self.state()),
            "clear" => {
                if b(v, "undoable", false) {
                    if !self.doc.is_empty() {
                        cp!("Clear Workspace");
                        self.restore(Snapshot { label: String::new(), doc: Document::default(), selection: None });
                        self.quick_base = None;
                        self.source = None;
                    }
                } else {
                    *self = Engine { generation: self.generation + 1, ..Engine::new() };
                }
                Ok(Value::Null)
            }
            "new_document" => {
                let w = f(v, "width", 1920.0) as u32;
                let h = f(v, "height", 1080.0) as u32;
                let c = color_of(v, "color").unwrap_or([0, 0, 0, 0]);
                self.import_rgba((*Rgba::filled(w, h, c).data).clone(), w, h, "Background", false).map_err(err)?;
                Ok(Value::Null)
            }
            "undo" => {
                let cur = self.current_snapshot();
                match self.history.undo(cur) {
                    Some(s) => {
                        let label = s.label.clone();
                        self.restore(s);
                        Ok(json!({"label": label}))
                    }
                    None => Err(err("Nothing to undo.")),
                }
            }
            "redo" => {
                let cur = self.current_snapshot();
                match self.history.redo(cur) {
                    Some(s) => {
                        let label = s.label.clone();
                        self.restore(s);
                        Ok(json!({"label": label}))
                    }
                    None => Err(err("Nothing to redo.")),
                }
            }
            "checkpoint" => {
                self.checkpoint(s(v, "label").unwrap_or("Edit"));
                Ok(Value::Null)
            }
            "select_layer" => {
                let id = id_of(v).ok_or_else(|| err("missing id"))?;
                if self.doc.layer(id).is_none() {
                    return Err(err("Layer not found"));
                }
                self.doc.active = Some(id);
                Ok(Value::Null)
            }
            "set_layer" => {
                let id = id_of(v).ok_or_else(|| err("missing id"))?;
                if self.doc.layer(id).is_none() {
                    return Err(err("Layer not found"));
                }
                let label = if v.get("name").is_some() {
                    "Rename Layer"
                } else if v.get("visible").is_some() {
                    "Layer Visibility"
                } else if v.get("opacity").is_some() {
                    "Layer Opacity"
                } else {
                    "Layer Properties"
                };
                cp!(label);
                let l = self.doc.layer_mut(id).unwrap();
                if let Some(n) = s(v, "name") {
                    let n = n.trim();
                    if !n.is_empty() {
                        l.name = n.chars().take(256).collect();
                    }
                }
                if let Some(vis) = v.get("visible").and_then(|x| x.as_bool()) {
                    l.visible = vis;
                }
                if let Some(o) = v.get("opacity").and_then(|x| x.as_f64()) {
                    l.opacity = o.clamp(0.0, 1.0) as f32;
                }
                if let Some(lk) = v.get("locked").and_then(|x| x.as_bool()) {
                    l.locked = lk;
                }
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "move_layer" => {
                let id = id_of(v).ok_or_else(|| err("missing id"))?;
                let from = self.doc.index_of(id).ok_or_else(|| err("Layer not found"))?;
                let n = self.doc.layers.len();
                let to = match s(v, "direction") {
                    Some("up") => (from + 1).min(n - 1),
                    Some("down") => from.saturating_sub(1),
                    Some("top") => n - 1,
                    Some("bottom") => 0,
                    _ => (f(v, "index", from as f64) as usize).min(n - 1),
                };
                if to == from {
                    return Ok(Value::Null);
                }
                cp!("Reorder Layers");
                let l = self.doc.layers.remove(from);
                self.doc.layers.insert(to, l);
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "duplicate_layer" => {
                let id = id_of(v).or(self.doc.active).ok_or_else(|| err("No layer"))?;
                let src = self.doc.layer(id).ok_or_else(|| err("Layer not found"))?.clone();
                cp!("Duplicate Layer");
                let nid = self.doc.alloc_id();
                let mut l = src;
                l.id = nid;
                l.name = format!("{} copy", l.name);
                self.doc.active = Some(id);
                let new_id = self.insert_layer(l, true);
                Ok(json!({"id": new_id}))
            }
            "delete_layer" => {
                let id = id_of(v).or(self.doc.active).ok_or_else(|| err("No layer"))?;
                let idx = self.doc.index_of(id).ok_or_else(|| err("Layer not found"))?;
                if self.doc.layers.len() == 1 {
                    return Err(err("An image needs at least one layer. Use Clear to start over."));
                }
                cp!("Delete Layer");
                self.touch_layer(id);
                self.doc.layers.remove(idx);
                self.doc.active = self.doc.layers.get(idx.saturating_sub(if idx >= self.doc.layers.len() { 1 } else { 0 })).map(|l| l.id).or(self.doc.layers.last().map(|l| l.id));
                Ok(Value::Null)
            }
            "merge_down" => {
                let id = id_of(v).or(self.doc.active).ok_or_else(|| err("No layer"))?;
                let idx = self.doc.index_of(id).ok_or_else(|| err("Layer not found"))?;
                if idx == 0 {
                    return Err(err("There is no layer below to merge into."));
                }
                let below = self.doc.layers[idx - 1].id;
                cp!("Merge Down");
                self.merge(&[below, id]).map_err(err)?;
                Ok(Value::Null)
            }
            "merge_layers" => {
                let ids: Vec<LayerId> = v.get("ids").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
                if ids.len() < 2 {
                    return Err(err("Select two or more layers to merge."));
                }
                cp!("Merge Layers");
                self.merge(&ids).map_err(err)?;
                Ok(Value::Null)
            }
            "flatten" => {
                cp!("Flatten Image");
                let (rgba, w, h) = self.export_rgba(false);
                let id = self.doc.alloc_id();
                let mut l = Layer::new(id, "Flattened", Rgba::from_vec(w, h, rgba).map_err(err)?);
                l.transform = Transform::at(0.0, 0.0, w, h);
                self.doc.layers = vec![l];
                self.doc.active = Some(id);
                self.touch_all();
                Ok(Value::Null)
            }
            "set_transform" => {
                let id = id_of(v).or(self.doc.active).ok_or_else(|| err("No layer"))?;
                let t: Transform = serde_json::from_value(v.get("transform").cloned().unwrap_or(Value::Null)).map_err(|e| err(e.to_string()))?;
                if ![t.x, t.y, t.scale_x, t.scale_y, t.rotation].iter().all(|x| x.is_finite()) || t.scale_x.abs() < 1e-4 || t.scale_y.abs() < 1e-4 {
                    return Err(err("Invalid transform"));
                }
                cp!(s(v, "label").unwrap_or("Transform Layer"));
                self.touch_layer(id);
                self.doc.layer_mut(id).ok_or_else(|| err("Layer not found"))?.transform = t;
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "nudge" => {
                let id = self.active_id()?;
                cp!("Move Layer");
                self.touch_layer(id);
                let l = self.doc.layer_mut(id).unwrap();
                l.transform.x += f(v, "dx", 0.0);
                l.transform.y += f(v, "dy", 0.0);
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "reset_transform" => {
                let id = self.active_id()?;
                cp!("Reset Transform");
                self.touch_layer(id);
                let (cw, ch) = (self.doc.width as f64, self.doc.height as f64);
                let l = self.doc.layer_mut(id).unwrap();
                l.transform = Transform { x: cw / 2.0, y: ch / 2.0, ..Default::default() };
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "flip_layer" => {
                let id = self.active_id()?;
                cp!("Flip Layer");
                let horiz = s(v, "axis") != Some("v");
                let l = self.doc.layer_mut(id).unwrap();
                flip_layer_pixels(l, horiz);
                self.touch_layer(id);
                Ok(Value::Null)
            }

            // ---- brushes
            "brush_begin" => {
                let id = self.active_id()?;
                let p: BrushParams = serde_json::from_value(v.clone()).map_err(|e| err(format!("bad brush: {e}")))?;
                let layer = self.doc.layer(id).unwrap();
                if layer.locked {
                    return Err(err("The layer is locked."));
                }
                if layer.is_text() && p.mode == crate::brush::BrushMode::Paint {
                    return Err(err("Rasterize the text layer before painting on it."));
                }
                let label = match p.mode {
                    crate::brush::BrushMode::Erase => "Erase",
                    crate::brush::BrushMode::Restore => "Restore",
                    crate::brush::BrushMode::Paint => "Brush",
                };
                let layer = layer.clone();
                self.checkpoint(label);
                let mut st = Stroke::new(p, &layer, self.selection.clone());
                let l = self.doc.layer_mut(id).unwrap();
                let r = st.add_point(l, f(v, "x", 0.0), f(v, "y", 0.0));
                let m = l.matrix();
                self.stroke = Some(st);
                self.touch(m.map_rect_bounds_rect(r));
                Ok(Value::Null)
            }
            "brush_move" => {
                let mut st = self.stroke.take().ok_or_else(|| err("No stroke"))?;
                let pts: Vec<(f64, f64)> = match v.get("points").and_then(|p| p.as_array()) {
                    Some(a) => a.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect(),
                    None => vec![(f(v, "x", 0.0), f(v, "y", 0.0))],
                };
                let mut dirty = IRect::default();
                if let Some(l) = self.doc.layer_mut(st.layer_id) {
                    for (x, y) in pts {
                        let r = st.add_point(l, x, y);
                        dirty = dirty.union(&l.matrix().map_rect_bounds_rect(r));
                    }
                }
                self.stroke = Some(st);
                self.touch(dirty);
                Ok(Value::Null)
            }
            "brush_end" => {
                self.stroke = None;
                Ok(Value::Null)
            }

            // ---- selections
            "select_all" => {
                cp!("Select All");
                self.selection = Some(Plane::new(self.doc.width, self.doc.height, 255));
                Ok(Value::Null)
            }
            "deselect" => {
                if self.selection.is_some() {
                    cp!("Deselect");
                }
                self.selection = None;
                Ok(Value::Null)
            }
            "invert_selection" => {
                cp!("Invert Selection");
                let (w, h) = (self.doc.width, self.doc.height);
                let inv: Vec<u8> = match &self.selection {
                    Some(sel) => sel.data.iter().map(|&x| 255 - x).collect(),
                    None => vec![255; w as usize * h as usize],
                };
                self.selection = Plane::from_vec(w, h, inv).ok().filter(|p| p.any_nonzero());
                Ok(Value::Null)
            }
            "select_rect" | "select_ellipse" | "select_polygon" => {
                let (w, h) = (self.doc.width, self.doc.height);
                let cov = match cmd {
                    "select_rect" => selection::rect_coverage(w, h, IRect::from_bounds(f(v, "x", 0.0), f(v, "y", 0.0), f(v, "x", 0.0) + f(v, "w", 0.0), f(v, "y", 0.0) + f(v, "h", 0.0))),
                    "select_ellipse" => selection::ellipse_coverage(w, h, f(v, "x", 0.0), f(v, "y", 0.0), f(v, "w", 0.0), f(v, "h", 0.0)),
                    _ => {
                        let pts: Vec<(f64, f64)> = v.get("points").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect()).unwrap_or_default();
                        if pts.len() < 3 {
                            return Err(err("A lasso needs at least three points."));
                        }
                        selection::polygon_coverage(w, h, &pts)
                    }
                };
                cp!("Selection");
                self.set_selection(cov, combine_of(v), f(v, "feather", 0.0) as f32);
                Ok(Value::Null)
            }
            "magic_wand" => {
                let cov = self.color_region(v)?;
                cp!("Magic Wand");
                self.set_selection(cov, combine_of(v), f(v, "feather", 0.0) as f32);
                Ok(json!({"bounds": self.selection.as_ref().and_then(|s| s.bbox(0))}))
            }
            "quick_select" => {
                let pts: Vec<(f64, f64)> = v.get("points").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect()).unwrap_or_default();
                let begin = b(v, "begin", false);
                if begin {
                    cp!("Quick Selection");
                    self.quick_base = Some(self.selection.clone());
                    self.quick_mode = if s(v, "combine") == Some("subtract") { Combine::Subtract } else { Combine::Add };
                }
                let src = self.source_image(b(v, "sample_all", true));
                let (w, h) = (self.doc.width, self.doc.height);
                let cov = selection::quick_select(&src, w, h, &pts, f(v, "radius", 10.0), f(v, "edge", 0.5) as f32);
                let mode = self.quick_mode;
                self.set_selection(cov, mode, 0.0);
                Ok(Value::Null)
            }
            "feather_selection" | "grow_selection" | "smooth_selection" => {
                let sel = self.selection.clone().ok_or_else(|| err("Nothing is selected."))?;
                let (w, h) = (sel.w as usize, sel.h as usize);
                let amt = f(v, "amount", 2.0);
                let out = match cmd {
                    "feather_selection" => filters::gaussian_u8(&sel.data, w, h, amt as f32),
                    "grow_selection" => filters::morph_u8(&sel.data, w, h, amt.round() as i32),
                    _ => {
                        let bl = filters::gaussian_u8(&sel.data, w, h, amt as f32);
                        bl.iter().map(|&x| (((x as f32 - 128.0) * 3.0 + 128.0).clamp(0.0, 255.0)) as u8).collect()
                    }
                };
                cp!(match cmd {
                    "feather_selection" => "Feather Selection",
                    "grow_selection" => if amt >= 0.0 { "Expand Selection" } else { "Contract Selection" },
                    _ => "Smooth Selection",
                });
                self.selection = Plane::from_vec(sel.w, sel.h, out).ok().filter(|p| p.any_nonzero());
                Ok(Value::Null)
            }
            "delete_selection" | "restore_selection" => {
                let id = self.active_id()?;
                let sel = self.selection.clone().ok_or_else(|| err("Nothing is selected."))?;
                cp!(if cmd == "delete_selection" { "Delete Selection" } else { "Restore Selection" });
                let (w, h) = (self.doc.width, self.doc.height);
                let l = self.doc.layer_mut(id).unwrap();
                Self::apply_coverage_to_mask(l, &sel.data, w, h, cmd == "restore_selection");
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "magic_erase" | "color_delete" => {
                let id = self.active_id()?;
                if self.doc.layer(id).map(|l| l.locked).unwrap_or(false) {
                    return Err(err("The layer is locked."));
                }
                let mut v2 = v.clone();
                v2["sample_all"] = Value::Bool(false);
                if cmd == "color_delete" {
                    v2["contiguous"] = Value::Bool(false);
                }
                let mut cov = self.color_region(&v2)?;
                if let Some(sel) = &self.selection {
                    for (c, &s) in cov.iter_mut().zip(sel.data.iter()) {
                        *c = mul255(*c as u32, s as u32) as u8;
                    }
                }
                let feather = f(v, "feather", 0.0) as f32;
                if feather > 0.0 {
                    cov = filters::gaussian_u8(&cov, self.doc.width as usize, self.doc.height as usize, feather);
                }
                cp!(if cmd == "magic_erase" { "Magic Eraser" } else { "Delete Colour" });
                let (w, h) = (self.doc.width, self.doc.height);
                let l = self.doc.layer_mut(id).unwrap();
                Self::apply_coverage_to_mask(l, &cov, w, h, false);
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "fill" => {
                let id = self.active_id()?;
                let color = color_of(v, "color").unwrap_or([0, 0, 0, 255]);
                let opacity = f(v, "opacity", 1.0).clamp(0.0, 1.0);
                let mut cov = if b(v, "selection", false) {
                    self.selection.as_ref().map(|s| (*s.data).clone()).ok_or_else(|| err("Nothing is selected."))?
                } else {
                    let mut c = self.color_region(v)?;
                    if let Some(sel) = &self.selection {
                        for (x, &s) in c.iter_mut().zip(sel.data.iter()) {
                            *x = mul255(*x as u32, s as u32) as u8;
                        }
                    }
                    c
                };
                if opacity < 1.0 {
                    let o = (opacity * 255.0) as u32;
                    cov.iter_mut().for_each(|c| *c = mul255(*c as u32, o) as u8);
                }
                if self.doc.layer(id).map(|l| l.is_text()).unwrap_or(false) {
                    return Err(err("Rasterize the text layer before filling it."));
                }
                cp!("Fill");
                let (w, h) = (self.doc.width, self.doc.height);
                let l = self.doc.layer_mut(id).unwrap();
                Self::paint_coverage(l, &cov, w, h, color);
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "color_at" => {
                let (x, y) = self.canvas_point(v).ok_or_else(|| err("Outside the image"))?;
                let src = self.source_image(b(v, "sample_all", true));
                let i = (y as usize * self.doc.width as usize + x as usize) * 4;
                Ok(json!({"color": [src[i], src[i + 1], src[i + 2], src[i + 3]]}))
            }

            // ---- masks
            "clear_mask" => {
                let id = self.active_id()?;
                cp!("Restore All");
                let l = self.doc.layer_mut(id).unwrap();
                l.mask = None;
                if let Some(o) = l.original.take() {
                    l.pixels = o;
                }
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "invert_mask" => {
                let id = self.active_id()?;
                cp!("Invert Mask");
                let l = self.doc.layer_mut(id).unwrap();
                let m = l.ensure_mask();
                m.iter_mut().for_each(|x| *x = 255 - *x);
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "apply_mask" => {
                let id = self.active_id()?;
                cp!("Apply Mask");
                let l = self.doc.layer_mut(id).unwrap();
                l.pixels = l.flattened_pixels();
                l.mask = None;
                l.original = None;
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "mask_from_selection" => {
                let id = self.active_id()?;
                let sel = self.selection.clone().ok_or_else(|| err("Nothing is selected."))?;
                cp!("Keep Selection");
                let (w, h) = (self.doc.width, self.doc.height);
                let inv: Vec<u8> = sel.data.iter().map(|&x| 255 - x).collect();
                let l = self.doc.layer_mut(id).unwrap();
                Self::apply_coverage_to_mask(l, &inv, w, h, false);
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "refine_preview" | "refine_commit" => {
                let id = self.active_id()?;
                let p: RefineParams = serde_json::from_value(v.clone()).map_err(|e| err(e.to_string()))?;
                let base = self.doc.layer(id).unwrap().clone();
                let refined = refine_layer(&base, &p);
                if cmd == "refine_commit" {
                    self.preview = None;
                    cp!("Refine Edges");
                    *self.doc.layer_mut(id).unwrap() = refined;
                } else {
                    self.preview = Some(refined);
                }
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "adjust_preview" | "adjust_commit" => {
                let id = self.active_id()?;
                let a: Adjustments = serde_json::from_value(v.get("adjustments").cloned().unwrap_or(json!({}))).map_err(|e| err(e.to_string()))?;
                let base = self.doc.layer(id).unwrap().clone();
                if base.is_text() {
                    return Err(err("Rasterize the text layer before adjusting it."));
                }
                let mut l = base.clone();
                if !a.is_identity() {
                    let src = base.original.as_ref().unwrap_or(&base.pixels);
                    l.pixels = Rgba::from_vec(src.w, src.h, adjust::apply(&src.data, src.w as usize, src.h as usize, &a)).map_err(err)?;
                    l.original = None;
                }
                if cmd == "adjust_commit" {
                    self.preview = None;
                    if !a.is_identity() {
                        cp!("Adjust Image");
                        *self.doc.layer_mut(id).unwrap() = l;
                    }
                } else {
                    self.preview = Some(l);
                }
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "auto_levels" => {
                let id = self.active_id()?;
                cp!("Auto Levels");
                let l = self.doc.layer_mut(id).unwrap();
                let src = l.original.take().unwrap_or_else(|| l.pixels.clone());
                l.pixels = Rgba { w: src.w, h: src.h, data: Arc::new(adjust::auto_levels(&src.data)) };
                self.touch_layer(id);
                Ok(Value::Null)
            }
            "preview_cancel" => {
                if let Some(p) = self.preview.take() {
                    self.touch_layer(p.id);
                }
                Ok(Value::Null)
            }
            "rasterize_layer" => {
                let id = self.active_id()?;
                cp!("Rasterize Layer");
                let l = self.doc.layer_mut(id).unwrap();
                l.kind = LayerKind::Raster;
                Ok(Value::Null)
            }

            // ---- geometry
            "crop" | "crop_to_selection" | "trim" => {
                let rect = match cmd {
                    "crop" => IRect::from_bounds(f(v, "x", 0.0), f(v, "y", 0.0), f(v, "x", 0.0) + f(v, "w", 0.0), f(v, "y", 0.0) + f(v, "h", 0.0)),
                    "crop_to_selection" => self.selection.as_ref().and_then(|s| s.bbox(0)).ok_or_else(|| err("Nothing is selected."))?,
                    _ => {
                        let (rgba, w, h) = self.export_rgba(false);
                        let alpha: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
                        Plane::from_vec(w, h, alpha).unwrap().bbox(0).ok_or_else(|| err("The image is fully transparent."))?
                    }
                };
                let rect = rect.intersect(&self.doc.canvas_rect());
                if rect.is_empty() || rect.w < 1 || rect.h < 1 {
                    return Err(err("The crop area is empty."));
                }
                cp!(if cmd == "trim" { "Trim" } else { "Crop" });
                self.crop_to(rect);
                Ok(Value::Null)
            }
            "resize_image" => {
                let nw = f(v, "width", 0.0).round() as u32;
                let nh = f(v, "height", 0.0).round() as u32;
                check_dimensions(nw, nh).map_err(err)?;
                cp!("Resize Image");
                self.resize_to(nw, nh);
                Ok(Value::Null)
            }
            "canvas_size" => {
                let nw = f(v, "width", 0.0).round() as u32;
                let nh = f(v, "height", 0.0).round() as u32;
                check_dimensions(nw, nh).map_err(err)?;
                let ax = f(v, "anchor_x", 0.5);
                let ay = f(v, "anchor_y", 0.5);
                cp!("Canvas Size");
                let dx = (nw as f64 - self.doc.width as f64) * ax;
                let dy = (nh as f64 - self.doc.height as f64) * ay;
                for l in &mut self.doc.layers {
                    l.transform.x += dx.round();
                    l.transform.y += dy.round();
                }
                self.doc.width = nw;
                self.doc.height = nh;
                self.selection = None;
                self.ensure_display();
                self.touch_all();
                Ok(Value::Null)
            }
            "rotate_canvas" => {
                let dir = s(v, "direction").unwrap_or("cw");
                cp!("Rotate Image");
                match dir {
                    "ccw" => {
                        self.rotate90(false);
                    }
                    "180" => {
                        self.rotate90(true);
                        self.rotate90(true);
                    }
                    _ => {
                        self.rotate90(true);
                    }
                }
                Ok(Value::Null)
            }
            "flip_canvas" => {
                let horiz = s(v, "axis") != Some("v");
                cp!(if horiz { "Flip Horizontal" } else { "Flip Vertical" });
                let (cw, ch) = (self.doc.width as f64, self.doc.height as f64);
                for l in &mut self.doc.layers {
                    flip_layer_pixels(l, horiz);
                    if horiz {
                        l.transform.x = cw - l.transform.x;
                    } else {
                        l.transform.y = ch - l.transform.y;
                    }
                    l.transform.rotation = -l.transform.rotation;
                }
                if let Some(sel) = self.selection.take() {
                    self.selection = Some(flip_plane(&sel, horiz));
                }
                self.touch_all();
                Ok(Value::Null)
            }
            "add_shape" => {
                let p: ShapeParams = serde_json::from_value(v.clone()).map_err(|e| err(format!("bad shape: {e}")))?;
                let (img, (ox, oy)) = rasterize_shape(&p).ok_or_else(|| err("Shape is too large"))?;
                cp!("Add Shape");
                let id = self.doc.alloc_id();
                let name = match p.kind.as_str() {
                    "rect" => "Rectangle",
                    "ellipse" => "Ellipse",
                    "arrow" => "Arrow",
                    _ => "Line",
                };
                let mut l = Layer::new(id, name, img);
                l.transform = Transform::at(ox as f64, oy as f64, l.w(), l.h());
                let nid = self.insert_layer(l, true);
                Ok(json!({"id": nid}))
            }
            "save_project" => {
                let p = s(v, "path").ok_or_else(|| err("missing path"))?;
                let prev = self.saved_version;
                self.save_project(std::path::Path::new(p)).map_err(err)?;
                if b(v, "recovery", false) {
                    self.saved_version = prev; // a recovery copy is not a user save
                }
                Ok(Value::Null)
            }
            "open_project" => {
                let p = s(v, "path").ok_or_else(|| err("missing path"))?;
                self.open_project(std::path::Path::new(p)).map_err(err)?;
                Ok(Value::Null)
            }
            "mark_saved" => {
                self.mark_saved();
                Ok(Value::Null)
            }
            "has_transparency" => {
                let (rgba, _, _) = self.export_rgba(false);
                Ok(json!({"value": rgba.chunks_exact(4).any(|p| p[3] < 255)}))
            }
            _ => Err(err(format!("Unknown command '{cmd}'"))),
        }
    }

    fn merge(&mut self, ids: &[LayerId]) -> Result<(), String> {
        let mut idxs: Vec<usize> = ids.iter().filter_map(|id| self.doc.index_of(*id)).collect();
        idxs.sort_unstable();
        idxs.dedup();
        if idxs.len() < 2 {
            return Err("Select two or more layers to merge.".into());
        }
        let layers: Vec<&Layer> = idxs.iter().map(|&i| &self.doc.layers[i]).filter(|l| l.visible).collect();
        let mut bounds = IRect::default();
        for l in &layers {
            bounds = bounds.union(&l.canvas_bounds());
        }
        if bounds.is_empty() {
            bounds = self.doc.canvas_rect();
        }
        check_dimensions(bounds.w as u32, bounds.h as u32)?;
        let rgba = composite_to_straight(&layers, bounds);
        let name = self.doc.layers[idxs[0]].name.clone();
        let id = self.doc.alloc_id();
        let mut nl = Layer::new(id, name, Rgba::from_vec(bounds.w as u32, bounds.h as u32, rgba)?);
        nl.transform = Transform::at(bounds.x as f64, bounds.y as f64, bounds.w as u32, bounds.h as u32);
        let insert_at = idxs[0];
        for &i in idxs.iter().rev() {
            self.doc.layers.remove(i);
        }
        self.doc.layers.insert(insert_at, nl);
        self.doc.active = Some(id);
        self.touch_all();
        Ok(())
    }

    fn crop_to(&mut self, rect: IRect) {
        for l in &mut self.doc.layers {
            l.transform.x -= rect.x as f64;
            l.transform.y -= rect.y as f64;
        }
        if let Some(sel) = self.selection.take() {
            let mut out = vec![0u8; rect.w as usize * rect.h as usize];
            for y in 0..rect.h as usize {
                let sy = y + rect.y as usize;
                out[y * rect.w as usize..(y + 1) * rect.w as usize].copy_from_slice(&sel.data[sy * sel.w as usize + rect.x as usize..sy * sel.w as usize + rect.x as usize + rect.w as usize]);
            }
            self.selection = Plane::from_vec(rect.w as u32, rect.h as u32, out).ok().filter(|p| p.any_nonzero());
        }
        self.doc.width = rect.w as u32;
        self.doc.height = rect.h as u32;
        self.ensure_display();
        self.touch_all();
    }

    fn resize_to(&mut self, nw: u32, nh: u32) {
        let sx = nw as f64 / self.doc.width as f64;
        let sy = nh as f64 / self.doc.height as f64;
        for l in &mut self.doc.layers {
            let lw = ((l.w() as f64 * sx).round() as u32).max(1);
            let lh = ((l.h() as f64 * sy).round() as u32).max(1);
            l.pixels = crate::image::resize_rgba(&l.pixels, lw, lh);
            l.mask = l.mask.as_ref().map(|m| crate::image::resize_plane(m, lw, lh));
            l.original = l.original.as_ref().map(|o| crate::image::resize_rgba(o, lw, lh));
            // keep the same on-canvas footprint relative to the new canvas
            let rx = lw as f64 / (l.w() as f64 * sx).max(1e-9);
            let _ = rx;
            l.transform.x *= sx;
            l.transform.y *= sy;
        }
        if let Some(sel) = self.selection.take() {
            self.selection = Some(crate::image::resize_plane(&sel, nw, nh));
        }
        self.doc.width = nw;
        self.doc.height = nh;
        self.ensure_display();
        self.touch_all();
    }

    fn rotate90(&mut self, cw: bool) {
        let (w, h) = (self.doc.width as f64, self.doc.height as f64);
        for l in &mut self.doc.layers {
            rotate_layer_pixels(l, cw);
            let (x, y) = (l.transform.x, l.transform.y);
            if cw {
                l.transform.x = h - y;
                l.transform.y = x;
            } else {
                l.transform.x = y;
                l.transform.y = w - x;
            }
            std::mem::swap(&mut l.transform.scale_x, &mut l.transform.scale_y);
        }
        if let Some(sel) = self.selection.take() {
            self.selection = Some(rotate_plane(&sel, cw));
        }
        self.doc.width = h as u32;
        self.doc.height = w as u32;
        self.ensure_display();
        self.touch_all();
    }
}

impl crate::geometry::Affine {
    /// Canvas bounds of a layer-space integer rect.
    pub fn map_rect_bounds_rect(&self, r: IRect) -> IRect {
        if r.is_empty() {
            return r;
        }
        let shifted = crate::geometry::Affine::translate(r.x as f64, r.y as f64);
        self.then_after(&shifted).map_rect_bounds(r.w as f64, r.h as f64).inflate(1)
    }
}

pub fn refine_layer(base: &Layer, p: &RefineParams) -> Layer {
    let mut l = base.clone();
    let (w, h) = (l.w() as usize, l.h() as usize);
    let src_px = base.original.clone().unwrap_or_else(|| base.pixels.clone());
    let mut alpha: Vec<u8> = match &base.mask {
        Some(m) => (*m.data).clone(),
        None => vec![255; w * h],
    };
    if p.radius > 0.0 {
        alpha = filters::refine_alpha(&src_px.data, &alpha, w, h, p.radius, 1e-4);
    }
    if p.smooth > 0.0 {
        let bl = filters::gaussian_u8(&alpha, w, h, p.smooth);
        alpha = bl.iter().map(|&x| (((x as f32 - 128.0) * 2.0 + 128.0).clamp(0.0, 255.0)) as u8).collect();
    }
    if p.shift.abs() >= 0.5 {
        alpha = filters::morph_u8(&alpha, w, h, p.shift.round() as i32);
    }
    if p.contrast > 0.0 {
        let k = 1.0 + p.contrast * 8.0;
        alpha = alpha.iter().map(|&x| (((x as f32 / 255.0 - 0.5) * k + 0.5).clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
    }
    if p.feather > 0.0 {
        alpha = filters::gaussian_u8(&alpha, w, h, p.feather);
    }
    // combine with the layer's own pixel alpha is implicit (mask multiplies)
    l.mask = Some(Plane::from_vec(w as u32, h as u32, alpha.clone()).unwrap());
    if p.decontaminate {
        let mut px = (*src_px.data).clone();
        let eff: Vec<u8> = alpha.iter().zip(px.chunks_exact(4)).map(|(&a, p)| mul255(a as u32, p[3] as u32) as u8).collect();
        filters::decontaminate(&mut px, &eff, w, h, (p.radius.max(2.0)) * 1.5);
        l.original = Some(src_px);
        l.pixels = Rgba { w: w as u32, h: h as u32, data: Arc::new(px) };
    } else {
        if base.original.is_some() {
            l.pixels = src_px;
        }
        l.original = None;
    }
    l
}

fn flip_plane(p: &Plane, horiz: bool) -> Plane {
    let (w, h) = (p.w as usize, p.h as usize);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = if horiz { (w - 1 - x, y) } else { (x, h - 1 - y) };
            out[y * w + x] = p.data[sy * w + sx];
        }
    }
    Plane::from_vec(p.w, p.h, out).unwrap()
}

fn flip_rgba(p: &Rgba, horiz: bool) -> Rgba {
    let (w, h) = (p.w as usize, p.h as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = if horiz { (w - 1 - x, y) } else { (x, h - 1 - y) };
            out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&p.data[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4]);
        }
    }
    Rgba::from_vec(p.w, p.h, out).unwrap()
}

fn flip_layer_pixels(l: &mut Layer, horiz: bool) {
    l.pixels = flip_rgba(&l.pixels, horiz);
    l.mask = l.mask.as_ref().map(|m| flip_plane(m, horiz));
    l.original = l.original.as_ref().map(|o| flip_rgba(o, horiz));
}

fn rotate_plane(p: &Plane, cw: bool) -> Plane {
    let (w, h) = (p.w as usize, p.h as usize);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = if cw { (h - 1 - y, x) } else { (y, w - 1 - x) };
            out[ny * h + nx] = p.data[y * w + x];
        }
    }
    Plane::from_vec(p.h, p.w, out).unwrap()
}

fn rotate_rgba(p: &Rgba, cw: bool) -> Rgba {
    let (w, h) = (p.w as usize, p.h as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = if cw { (h - 1 - y, x) } else { (y, w - 1 - x) };
            out[(ny * h + nx) * 4..(ny * h + nx) * 4 + 4].copy_from_slice(&p.data[(y * w + x) * 4..(y * w + x) * 4 + 4]);
        }
    }
    Rgba::from_vec(p.h, p.w, out).unwrap()
}

fn rotate_layer_pixels(l: &mut Layer, cw: bool) {
    l.pixels = rotate_rgba(&l.pixels, cw);
    l.mask = l.mask.as_ref().map(|m| rotate_plane(m, cw));
    l.original = l.original.as_ref().map(|o| rotate_rgba(o, cw));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine_with(w: u32, h: u32, c: [u8; 4]) -> Engine {
        let mut e = Engine::new();
        e.import_rgba((*Rgba::filled(w, h, c).data).clone(), w, h, "base", false).unwrap();
        e
    }

    #[test]
    fn merge_down_respects_opacity() {
        let mut e = engine_with(4, 4, [0, 0, 0, 0]);
        e.import_rgba((*Rgba::filled(4, 4, [0, 0, 255, 255]).data).clone(), 4, 4, "top", true).unwrap();
        let top = e.doc.active.unwrap();
        assert_eq!(e.command(&json!({"cmd":"set_layer","id":top,"opacity":0.5}))["ok"], true);
        assert_eq!(e.command(&json!({"cmd":"merge_down","id":top}))["ok"], true);
        assert_eq!(e.doc.layers.len(), 1);
        let (px, _, _) = e.export_rgba(false);
        assert!((120..=135).contains(&px[3]), "alpha {}", px[3]);
    }

    #[test]
    fn undo_restores_previous_state_and_redo_reapplies() {
        let mut e = engine_with(8, 8, [10, 20, 30, 255]);
        e.command(&json!({"cmd":"select_rect","x":0,"y":0,"w":4,"h":8}));
        e.command(&json!({"cmd":"delete_selection"}));
        assert_eq!(e.export_rgba(false).0[3], 0);
        e.command(&json!({"cmd":"undo"}));
        assert_eq!(e.export_rgba(false).0[3], 255);
        e.command(&json!({"cmd":"redo"}));
        assert_eq!(e.export_rgba(false).0[3], 0);
        // restore selection brings pixels back non-destructively
        e.command(&json!({"cmd":"restore_selection"}));
        assert_eq!(&e.export_rgba(false).0[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn crop_rotate_flip_keep_pixels_consistent() {
        let mut e = engine_with(6, 4, [0, 0, 0, 255]);
        // mark top-left pixel red via fill of a 1px selection
        e.command(&json!({"cmd":"select_rect","x":0,"y":0,"w":1,"h":1}));
        e.command(&json!({"cmd":"fill","selection":true,"color":[255,0,0,255]}));
        e.command(&json!({"cmd":"deselect"}));
        e.command(&json!({"cmd":"rotate_canvas","direction":"cw"}));
        assert_eq!((e.doc.width, e.doc.height), (4, 6));
        let (px, w, _) = e.export_rgba(false);
        // top-left moves to top-right after a clockwise rotation
        let i = ((w - 1) * 4) as usize;
        assert_eq!(&px[i..i + 3], &[255, 0, 0]);
        e.command(&json!({"cmd":"flip_canvas","axis":"h"}));
        let (px, _, _) = e.export_rgba(false);
        assert_eq!(&px[0..3], &[255, 0, 0]);
        e.command(&json!({"cmd":"crop","x":0,"y":0,"w":2,"h":2}));
        assert_eq!((e.doc.width, e.doc.height), (2, 2));
        assert_eq!(&e.export_rgba(false).0[0..3], &[255, 0, 0]);
    }

    #[test]
    fn text_layer_update_keeps_top_left() {
        let mut e = engine_with(100, 100, [255; 4]);
        let props = TextProps { text: "a".into(), font: "x".into(), size: 10.0, color: [0, 0, 0, 255], bold: false, italic: false, align: "left".into() };
        let id = e.add_text_layer(props.clone(), vec![0; 10 * 5 * 4], 10, 5, 20.0, 30.0).unwrap();
        e.update_text_layer(id, TextProps { text: "abc".into(), ..props }, vec![0; 30 * 5 * 4], 30, 5).unwrap();
        let b = e.doc.layer(id).unwrap().canvas_bounds();
        assert_eq!((b.x, b.y, b.w), (20, 30, 30));
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        let mut e = Engine::new();
        assert_eq!(e.command(&json!({"cmd":"magic_wand","x":1,"y":1}))["ok"], false);
        let mut e = engine_with(4, 4, [0; 4]);
        assert_eq!(e.command(&json!({"cmd":"nope"}))["ok"], false);
        assert_eq!(e.command(&json!({"cmd":"set_transform","transform":{"x":0,"y":0,"scale_x":0,"scale_y":1,"rotation":0}}))["ok"], false);
        assert_eq!(e.command(&json!({"cmd":"delete_layer"}))["ok"], false, "cannot delete last layer");
    }
}
