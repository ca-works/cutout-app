//! Stroke-based brushes: Eraser, Restore, Paint, plus refine-edge brushing.
//!
//! A stroke accumulates *coverage* (max of dab footprints) in a stroke buffer
//! and the layer is recomputed from its pre-stroke state each time, so the
//! opacity setting is a true per-stroke ceiling (no build-up where dabs
//! overlap), exactly like a professional editor.

use crate::document::Layer;
use crate::geometry::IRect;
use crate::image::{Plane, Rgba};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrushMode {
    /// Hide pixels via the layer mask (non-destructive).
    Erase,
    /// Reveal masked pixels again (and restore pre-decontamination colours).
    Restore,
    /// Paint colour onto the layer pixels.
    Paint,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BrushParams {
    pub mode: BrushMode,
    /// Diameter in canvas pixels.
    pub size: f64,
    /// 0 = very soft, 1 = hard edge.
    pub hardness: f64,
    /// 0..1 stroke opacity / strength.
    pub opacity: f64,
    #[serde(default = "white")]
    pub color: [u8; 4],
}

fn white() -> [u8; 4] {
    [0, 0, 0, 255]
}

pub struct Stroke {
    pub params: BrushParams,
    pub layer_id: i64,
    /// Layer state at stroke start.
    pub base: Layer,
    /// Coverage in *layer* pixel space.
    pub coverage: Vec<u8>,
    pub last: Option<(f64, f64)>,
    pub dirty_layer: IRect,
    /// Optional canvas-space selection limiting the stroke.
    pub limit: Option<Plane>,
}

impl Stroke {
    pub fn new(params: BrushParams, layer: &Layer, limit: Option<Plane>) -> Self {
        Stroke {
            params,
            layer_id: layer.id,
            base: layer.clone(),
            coverage: vec![0; layer.w() as usize * layer.h() as usize],
            last: None,
            dirty_layer: IRect::default(),
            limit,
        }
    }

    /// Add dabs from the last point to canvas point (x, y). Returns the
    /// layer-space rectangle that changed.
    pub fn add_point(&mut self, layer: &mut Layer, x: f64, y: f64) -> IRect {
        let inv = layer.inverse();
        let (lx, ly) = inv.apply(x, y);
        let scale = layer.matrix().mean_scale().max(1e-6);
        let radius = (self.params.size / 2.0 / scale).max(0.5);
        let spacing = (radius * 0.2).max(0.5);
        let mut dabs = Vec::new();
        match self.last {
            None => dabs.push((lx, ly)),
            Some((px, py)) => {
                let d = ((lx - px).powi(2) + (ly - py).powi(2)).sqrt();
                let n = (d / spacing).floor() as usize;
                for i in 1..=n {
                    let t = i as f64 * spacing / d;
                    dabs.push((px + (lx - px) * t, py + (ly - py) * t));
                }
                if n == 0 {
                    return IRect::default();
                }
            }
        }
        let mut changed = IRect::default();
        for &(dx, dy) in &dabs {
            changed = changed.union(&self.dab(dx, dy, radius));
            self.last = Some((dx, dy));
        }
        if self.last.is_none() {
            self.last = Some((lx, ly));
        }
        if !changed.is_empty() {
            self.apply(layer, changed);
            self.dirty_layer = self.dirty_layer.union(&changed);
        }
        changed
    }

    fn dab(&mut self, cx: f64, cy: f64, r: f64) -> IRect {
        let (w, h) = (self.base.w() as i32, self.base.h() as i32);
        let rect = IRect::from_bounds(cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0).intersect(&IRect::new(0, 0, w, h));
        if rect.is_empty() {
            return rect;
        }
        let hard = self.params.hardness.clamp(0.0, 1.0);
        let inner = r * hard;
        let strength = self.params.opacity.clamp(0.0, 1.0);
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
                let f = if d <= inner {
                    1.0
                } else if d >= r + 0.5 {
                    0.0
                } else if hard >= 0.999 {
                    // 1px anti-aliased rim
                    (r + 0.5 - d).clamp(0.0, 1.0)
                } else {
                    let t = ((d - inner) / (r + 0.5 - inner)).clamp(0.0, 1.0);
                    // smoothstep falloff
                    1.0 - t * t * (3.0 - 2.0 * t)
                };
                let v = (f * strength * 255.0).round() as u8;
                let i = y as usize * w as usize + x as usize;
                if v > self.coverage[i] {
                    self.coverage[i] = v;
                }
            }
        }
        rect
    }

    fn limit_at(&self, layer: &Layer, x: i32, y: i32) -> u32 {
        match &self.limit {
            None => 255,
            Some(sel) => {
                let (cx, cy) = layer.matrix().apply(x as f64 + 0.5, y as f64 + 0.5);
                sel.get_clamped(cx.floor() as i64, cy.floor() as i64) as u32
            }
        }
    }

    fn apply(&self, layer: &mut Layer, rect: IRect) {
        let w = layer.w() as usize;
        match self.params.mode {
            BrushMode::Erase | BrushMode::Restore => {
                let base_mask = self.base.mask.clone();
                let restore = self.params.mode == BrushMode::Restore;
                let orig = if restore { self.base.original.clone() } else { None };
                let limits: Vec<u32> = if self.limit.is_some() {
                    (rect.y..rect.bottom()).flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y))).map(|(x, y)| self.limit_at(layer, x, y)).collect()
                } else {
                    Vec::new()
                };
                let mask = layer.ensure_mask();
                let mut k = 0;
                for y in rect.y..rect.bottom() {
                    for x in rect.x..rect.right() {
                        let i = y as usize * w + x as usize;
                        let mut c = self.coverage[i] as u32;
                        if !limits.is_empty() {
                            c = crate::image::mul255(c, limits[k]);
                        }
                        k += 1;
                        let b = base_mask.as_ref().map_or(255, |m| m.data[i] as u32);
                        mask[i] = if restore { b + crate::image::mul255(255 - b, c) } else { b - crate::image::mul255(b, c) } as u8;
                    }
                }
                if let Some(orig) = orig {
                    // bring back true colours where restoring
                    let base_px = self.base.pixels.data.clone();
                    let px = layer.pixels.make_mut();
                    for y in rect.y..rect.bottom() {
                        for x in rect.x..rect.right() {
                            let i = y as usize * w + x as usize;
                            let c = self.coverage[i] as u32;
                            if c == 0 {
                                continue;
                            }
                            for ch in 0..4 {
                                let a = base_px[i * 4 + ch] as u32;
                                let o = orig.data[i * 4 + ch] as u32;
                                px[i * 4 + ch] = ((a * (255 - c) + o * c + 127) / 255) as u8;
                            }
                        }
                    }
                }
            }
            BrushMode::Paint => {
                let col = self.params.color;
                let base_px = self.base.pixels.data.clone();
                let base_mask = self.base.mask.clone();
                let limits: Vec<u32> = (rect.y..rect.bottom())
                    .flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y)))
                    .map(|(x, y)| self.limit_at(layer, x, y))
                    .collect();
                {
                    let px = layer.pixels.make_mut();
                    let mut k = 0;
                    for y in rect.y..rect.bottom() {
                        for x in rect.x..rect.right() {
                            let i = y as usize * w + x as usize;
                            let c = crate::image::mul255(crate::image::mul255(self.coverage[i] as u32, limits[k]), col[3] as u32);
                            k += 1;
                            let p = &base_px[i * 4..i * 4 + 4];
                            // straight-alpha source-over of brush colour onto pixel
                            let da = p[3] as f32 / 255.0;
                            let sa = c as f32 / 255.0;
                            let oa = sa + da * (1.0 - sa);
                            let o = &mut px[i * 4..i * 4 + 4];
                            if oa <= 0.0 {
                                o.copy_from_slice(p);
                                continue;
                            }
                            for ch in 0..3 {
                                let v = (col[ch] as f32 * sa + p[ch] as f32 * da * (1.0 - sa)) / oa;
                                o[ch] = (v + 0.5).min(255.0) as u8;
                            }
                            o[3] = (oa * 255.0 + 0.5) as u8;
                        }
                    }
                }
                // painting reveals the painted area even if it was masked
                if let Some(bm) = base_mask {
                    let mask = layer.ensure_mask();
                    let mut k = 0;
                    for y in rect.y..rect.bottom() {
                        for x in rect.x..rect.right() {
                            let i = y as usize * w + x as usize;
                            let c = crate::image::mul255(self.coverage[i] as u32, limits[k]);
                            k += 1;
                            let b = bm.data[i] as u32;
                            mask[i] = (b + crate::image::mul255(255 - b, c)) as u8;
                        }
                    }
                }
            }
        }
    }
}

/// Rasterise a shape into a new RGBA layer image. Returns (image, top-left).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapeParams {
    pub kind: String, // rect | ellipse | line | arrow
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub stroke: [u8; 4],
    pub fill: Option<[u8; 4]>,
    pub width: f64,
    #[serde(default)]
    pub corner: f64,
}

pub fn rasterize_shape(s: &ShapeParams) -> Option<(Rgba, (i32, i32))> {
    let pad = s.width * 2.0 + 4.0 + if s.kind == "arrow" { s.width * 4.0 } else { 0.0 };
    let (minx, maxx) = (s.x0.min(s.x1) - pad, s.x0.max(s.x1) + pad);
    let (miny, maxy) = (s.y0.min(s.y1) - pad, s.y0.max(s.y1) + pad);
    let ox = minx.floor() as i32;
    let oy = miny.floor() as i32;
    let w = (maxx.ceil() as i32 - ox).max(1) as u32;
    let h = (maxy.ceil() as i32 - oy).max(1) as u32;
    if crate::image::check_dimensions(w, h).is_err() {
        return None;
    }
    let (ax, ay) = (s.x0 - ox as f64, s.y0 - oy as f64);
    let (bx, by) = (s.x1 - ox as f64, s.y1 - oy as f64);
    let hw = s.width / 2.0;
    // signed distance functions (negative inside)
    let rect_sdf = |px: f64, py: f64| -> f64 {
        let cx = (ax + bx) / 2.0;
        let cy = (ay + by) / 2.0;
        let ex = (bx - ax).abs() / 2.0;
        let ey = (by - ay).abs() / 2.0;
        let r = s.corner.min(ex).min(ey).max(0.0);
        let qx = (px - cx).abs() - ex + r;
        let qy = (py - cy).abs() - ey + r;
        let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
        outside + qx.max(qy).min(0.0) - r
    };
    let ellipse_sdf = |px: f64, py: f64| -> f64 {
        let cx = (ax + bx) / 2.0;
        let cy = (ay + by) / 2.0;
        let rx = ((bx - ax).abs() / 2.0).max(0.5);
        let ry = ((by - ay).abs() / 2.0).max(0.5);
        let nx = (px - cx) / rx;
        let ny = (py - cy) / ry;
        let k = (nx * nx + ny * ny).sqrt();
        // first-order approximation of distance
        let grad = ((nx / rx).powi(2) + (ny / ry).powi(2)).sqrt().max(1e-9);
        (k * k - 1.0) / (2.0 * k.max(1e-9) * grad)
    };
    let seg_dist = |px: f64, py: f64, x0: f64, y0: f64, x1: f64, y1: f64| -> f64 {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 { (((px - x0) * dx + (py - y0) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        ((px - x0 - t * dx).powi(2) + (py - y0 - t * dy).powi(2)).sqrt()
    };
    let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt().max(1e-6);
    let (ux, uy) = ((bx - ax) / len, (by - ay) / len);
    let head = (s.width * 4.0).max(10.0).min(len * 0.6);
    let (hx, hy) = (bx - ux * head, by - uy * head);
    let (nx, ny) = (-uy, ux);
    let tri = [(bx, by), (hx + nx * head * 0.5, hy + ny * head * 0.5), (hx - nx * head * 0.5, hy - ny * head * 0.5)];
    let in_tri = |px: f64, py: f64| -> bool {
        let sgn = |a: (f64, f64), b: (f64, f64)| (px - b.0) * (a.1 - b.1) - (a.0 - b.0) * (py - b.1);
        let d1 = sgn(tri[0], tri[1]);
        let d2 = sgn(tri[1], tri[2]);
        let d3 = sgn(tri[2], tri[0]);
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    };
    let mut out = vec![0u8; w as usize * h as usize * 4];
    const SS: usize = 3;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut stroke_c = 0f64;
            let mut fill_c = 0f64;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f64 + (sx as f64 + 0.5) / SS as f64;
                    let py = y as f64 + (sy as f64 + 0.5) / SS as f64;
                    match s.kind.as_str() {
                        "rect" | "ellipse" => {
                            let d = if s.kind == "rect" { rect_sdf(px, py) } else { ellipse_sdf(px, py) };
                            if d.abs() <= hw && s.width > 0.0 {
                                stroke_c += 1.0;
                            } else if d < 0.0 {
                                fill_c += 1.0;
                            }
                        }
                        "arrow" => {
                            if seg_dist(px, py, ax, ay, hx, hy) <= hw || in_tri(px, py) {
                                stroke_c += 1.0;
                            }
                        }
                        _ => {
                            if seg_dist(px, py, ax, ay, bx, by) <= hw {
                                stroke_c += 1.0;
                            }
                        }
                    }
                }
            }
            let n = (SS * SS) as f64;
            let sc = stroke_c / n;
            let fc = fill_c / n;
            let i = (y * w as usize + x) * 4;
            let mut rgba = [0f64; 4];
            if let Some(f) = s.fill {
                let a = fc * f[3] as f64 / 255.0;
                rgba = [f[0] as f64 * a, f[1] as f64 * a, f[2] as f64 * a, a];
            }
            if sc > 0.0 {
                let a = sc * s.stroke[3] as f64 / 255.0;
                for c in 0..3 {
                    rgba[c] = s.stroke[c] as f64 * a + rgba[c] * (1.0 - a);
                }
                rgba[3] = a + rgba[3] * (1.0 - a);
            }
            if rgba[3] > 0.0 {
                for c in 0..3 {
                    out[i + c] = (rgba[c] / rgba[3]).round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (rgba[3] * 255.0).round() as u8;
            }
        }
    }
    Some((Rgba::from_vec(w, h, out).ok()?, (ox, oy)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Transform;

    fn layer() -> Layer {
        let mut l = Layer::new(1, "l", Rgba::filled(50, 50, [200, 100, 50, 255]));
        l.transform = Transform::at(0.0, 0.0, 50, 50);
        l
    }

    #[test]
    fn erase_then_restore_is_lossless() {
        let mut l = layer();
        let p = BrushParams { mode: BrushMode::Erase, size: 20.0, hardness: 1.0, opacity: 1.0, color: [0; 4] };
        let mut s = Stroke::new(p, &l, None);
        s.add_point(&mut l, 25.0, 25.0);
        assert_eq!(l.mask.as_ref().unwrap().get(25, 25), 0);
        assert_eq!(l.pixels.px(25, 25), [200, 100, 50, 255], "pixels untouched");
        let p = BrushParams { mode: BrushMode::Restore, ..p };
        let mut s = Stroke::new(p, &l, None);
        s.add_point(&mut l, 25.0, 25.0);
        assert_eq!(l.mask.as_ref().unwrap().get(25, 25), 255);
    }

    #[test]
    fn stroke_opacity_does_not_build_up() {
        let mut l = layer();
        let p = BrushParams { mode: BrushMode::Erase, size: 10.0, hardness: 1.0, opacity: 0.5, color: [0; 4] };
        let mut s = Stroke::new(p, &l, None);
        for i in 0..20 {
            s.add_point(&mut l, 20.0 + (i % 2) as f64 * 3.0, 20.0);
        }
        let m = l.mask.as_ref().unwrap().get(21, 20);
        assert!((126..=129).contains(&m), "mask {m}");
    }

    #[test]
    fn paint_colour() {
        let mut l = layer();
        let p = BrushParams { mode: BrushMode::Paint, size: 6.0, hardness: 1.0, opacity: 1.0, color: [0, 0, 255, 255] };
        let mut s = Stroke::new(p, &l, None);
        s.add_point(&mut l, 10.0, 10.0);
        assert_eq!(l.pixels.px(10, 10), [0, 0, 255, 255]);
    }

    #[test]
    fn shape_rect_has_fill_and_stroke() {
        let (img, (ox, oy)) = rasterize_shape(&ShapeParams {
            kind: "rect".into(),
            x0: 10.0,
            y0: 10.0,
            x1: 50.0,
            y1: 40.0,
            stroke: [255, 0, 0, 255],
            fill: Some([0, 255, 0, 255]),
            width: 4.0,
            corner: 0.0,
        })
        .unwrap();
        let c = img.px((30 - ox) as u32, (25 - oy) as u32);
        assert_eq!(c, [0, 255, 0, 255]);
        let e = img.px((10 - ox) as u32, (25 - oy) as u32);
        assert_eq!(e, [255, 0, 0, 255]);
    }
}
