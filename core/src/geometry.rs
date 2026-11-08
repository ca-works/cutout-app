//! Geometry primitives: integer rectangles, affine transforms and the
//! layer transform model (position / scale / rotation about the centre).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        IRect { x, y, w, h }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn intersect(&self, o: &IRect) -> IRect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            IRect::default()
        } else {
            IRect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }
    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        IRect::new(x0, y0, x1 - x0, y1 - y0)
    }
    pub fn inflate(&self, d: i32) -> IRect {
        IRect::new(self.x - d, self.y - d, self.w + 2 * d, self.h + 2 * d)
    }
    pub fn from_bounds(x0: f64, y0: f64, x1: f64, y1: f64) -> IRect {
        let ix0 = x0.floor() as i32;
        let iy0 = y0.floor() as i32;
        let ix1 = x1.ceil() as i32;
        let iy1 = y1.ceil() as i32;
        IRect::new(ix0, iy0, (ix1 - ix0).max(0), (iy1 - iy0).max(0))
    }
}

/// 2×3 affine matrix: x' = a·x + c·y + e ; y' = b·x + d·y + f
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
    pub fn translate(x: f64, y: f64) -> Affine {
        Affine { e: x, f: y, ..Self::IDENTITY }
    }
    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine { a: sx, d: sy, ..Self::IDENTITY }
    }
    pub fn rotate(r: f64) -> Affine {
        let (s, c) = r.sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }
    /// self ∘ o  (apply `o` first, then `self`)
    pub fn then_after(&self, o: &Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }
    #[inline]
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
    pub fn invert(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let id = 1.0 / det;
        Some(Affine {
            a: self.d * id,
            b: -self.b * id,
            c: -self.c * id,
            d: self.a * id,
            e: (self.c * self.f - self.d * self.e) * id,
            f: (self.b * self.e - self.a * self.f) * id,
        })
    }
    pub fn is_integer_translation(&self) -> bool {
        (self.a - 1.0).abs() < 1e-9
            && self.b.abs() < 1e-9
            && self.c.abs() < 1e-9
            && (self.d - 1.0).abs() < 1e-9
            && (self.e - self.e.round()).abs() < 1e-6
            && (self.f - self.f.round()).abs() < 1e-6
    }
    /// Bounding box of a w×h rectangle mapped through this transform.
    pub fn map_rect_bounds(&self, w: f64, h: f64) -> IRect {
        let pts = [self.apply(0.0, 0.0), self.apply(w, 0.0), self.apply(0.0, h), self.apply(w, h)];
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        IRect::from_bounds(x0, y0, x1, y1)
    }
    /// Average linear scale factor.
    pub fn mean_scale(&self) -> f64 {
        ((self.a * self.d - self.b * self.c).abs()).sqrt()
    }
}

/// Layer placement on the canvas. `x`,`y` is the canvas position of the
/// layer's centre; scale and rotation (radians) are applied about the centre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub rotation: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { x: 0.0, y: 0.0, scale_x: 1.0, scale_y: 1.0, rotation: 0.0 }
    }
}

impl Transform {
    /// Transform that places a w×h layer with its top-left at (x, y).
    pub fn at(x: f64, y: f64, w: u32, h: u32) -> Self {
        Transform { x: x + w as f64 / 2.0, y: y + h as f64 / 2.0, ..Default::default() }
    }
    /// Layer pixel space → canvas space.
    pub fn matrix(&self, w: u32, h: u32) -> Affine {
        let center = Affine::translate(-(w as f64) / 2.0, -(h as f64) / 2.0);
        let s = Affine::scale(self.scale_x, self.scale_y);
        let r = Affine::rotate(self.rotation);
        let t = Affine::translate(self.x, self.y);
        t.then_after(&r.then_after(&s.then_after(&center)))
    }
}

/// Scanline polygon rasterisation with 4× vertical / horizontal supersampling.
/// Returns coverage 0..=255 for a `w`×`h` grid. Points are in pixel units.
pub fn rasterize_polygon(points: &[(f64, f64)], w: u32, h: u32, antialias: bool) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h];
    if points.len() < 3 {
        return out;
    }
    let ss: usize = if antialias { 4 } else { 1 };
    let mut ymin = f64::MAX;
    let mut ymax = f64::MIN;
    for p in points {
        ymin = ymin.min(p.1);
        ymax = ymax.max(p.1);
    }
    let y_start = (ymin.floor().max(0.0)) as usize;
    let y_end = (ymax.ceil().min(h as f64)) as usize;
    let mut acc = vec![0u16; w];
    let mut xs: Vec<f64> = Vec::new();
    for y in y_start..y_end {
        acc.iter_mut().for_each(|v| *v = 0);
        let mut touched = false;
        for sy in 0..ss {
            let fy = y as f64 + (sy as f64 + 0.5) / ss as f64;
            xs.clear();
            let n = points.len();
            for i in 0..n {
                let (x0, y0) = points[i];
                let (x1, y1) = points[(i + 1) % n];
                if (y0 <= fy && y1 > fy) || (y1 <= fy && y0 > fy) {
                    let t = (fy - y0) / (y1 - y0);
                    xs.push(x0 + t * (x1 - x0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks_exact(2) {
                touched = true;
                for sx in 0..ss {
                    let off = (sx as f64 + 0.5) / ss as f64;
                    // pixel x where x+off in [pair0, pair1)
                    let xa = (pair[0] - off).ceil().max(0.0) as i64;
                    let xb = ((pair[1] - off).ceil().min(w as f64)) as i64;
                    for x in xa.max(0)..xb.max(0) {
                        acc[x as usize] += 1;
                    }
                }
            }
        }
        if touched {
            let total = (ss * ss) as u32;
            let row = &mut out[y * w..(y + 1) * w];
            for x in 0..w {
                if acc[x] > 0 {
                    row[x] = ((acc[x] as u32 * 255 + total / 2) / total) as u8;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn affine_inverse_roundtrip() {
        let t = Transform { x: 100.0, y: 50.0, scale_x: 1.5, scale_y: 0.75, rotation: 0.4 };
        let m = t.matrix(200, 100);
        let inv = m.invert().unwrap();
        let (x, y) = m.apply(13.0, 77.0);
        let (bx, by) = inv.apply(x, y);
        assert!((bx - 13.0).abs() < 1e-9 && (by - 77.0).abs() < 1e-9);
    }
    #[test]
    fn identity_at_places_top_left() {
        let t = Transform::at(10.0, 20.0, 30, 40);
        let m = t.matrix(30, 40);
        assert_eq!(m.apply(0.0, 0.0), (10.0, 20.0));
        assert!(m.is_integer_translation());
    }
    #[test]
    fn polygon_square_coverage() {
        let pts = [(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)];
        let m = rasterize_polygon(&pts, 8, 8, true);
        assert_eq!(m[3 * 8 + 3], 255);
        assert_eq!(m[0], 0);
        assert_eq!(m[2 * 8 + 1], 0);
        let sum: u32 = m.iter().map(|&v| v as u32).sum();
        assert_eq!(sum, 16 * 255);
    }
}
