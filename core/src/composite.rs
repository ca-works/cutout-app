//! Layer compositing (Porter-Duff source-over in premultiplied space).
//!
//! Untransformed layers take an exact integer fast path; transformed layers
//! are sampled bilinearly in premultiplied space with transparent borders so
//! that rotated / scaled edges are anti-aliased and never fringe.

use crate::document::Layer;
use crate::geometry::IRect;
use crate::image::mul255;
use rayon::prelude::*;

/// Composite `layers` (bottom first) into the premultiplied RGBA buffer `out`
/// (`out_w`×`out_h`, canvas coordinates offset by `origin`), touching only
/// `region` (in canvas coordinates). Pixels in the region are overwritten.
pub fn composite_region(layers: &[&Layer], out: &mut [u8], out_w: u32, out_h: u32, origin: (i32, i32), region: IRect) {
    let out_rect = IRect::new(origin.0, origin.1, out_w as i32, out_h as i32);
    let region = region.intersect(&out_rect);
    if region.is_empty() {
        return;
    }
    let prepared: Vec<Prepared> = layers.iter().filter(|l| l.visible && l.opacity > 0.0).map(|l| Prepared::new(l)).collect();
    let stride = out_w as usize * 4;
    out.par_chunks_mut(stride).enumerate().for_each(|(row_idx, row)| {
        let y = row_idx as i32 + origin.1;
        if y < region.y || y >= region.bottom() {
            return;
        }
        let x0 = region.x;
        let x1 = region.right();
        let seg = &mut row[((x0 - origin.0) as usize * 4)..((x1 - origin.0) as usize * 4)];
        seg.iter_mut().for_each(|v| *v = 0);
        for p in &prepared {
            p.blend_row(y, x0, x1, seg);
        }
    });
}

struct Prepared<'a> {
    l: &'a Layer,
    bounds: IRect,
    inv: crate::geometry::Affine,
    int_offset: Option<(i32, i32)>,
    opacity: u32,
    opacity_f: f32,
}

impl<'a> Prepared<'a> {
    fn new(l: &'a Layer) -> Self {
        let m = l.matrix();
        let int_offset = if m.is_integer_translation() { Some((m.e.round() as i32, m.f.round() as i32)) } else { None };
        Prepared {
            l,
            bounds: l.canvas_bounds(),
            inv: l.inverse(),
            int_offset,
            opacity: (l.opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u32,
            opacity_f: l.opacity.clamp(0.0, 1.0),
        }
    }

    #[inline]
    fn premul_at(&self, x: i64, y: i64) -> [f32; 4] {
        let l = self.l;
        if x < 0 || y < 0 || x >= l.w() as i64 || y >= l.h() as i64 {
            return [0.0; 4];
        }
        let i = (y as usize * l.w() as usize + x as usize) * 4;
        let d = &l.pixels.data;
        let mut a = d[i + 3] as f32;
        if let Some(m) = &l.mask {
            a = a * m.data[y as usize * l.w() as usize + x as usize] as f32 / 255.0;
        }
        let k = a / 255.0;
        [d[i] as f32 * k, d[i + 1] as f32 * k, d[i + 2] as f32 * k, a]
    }

    fn blend_row(&self, y: i32, x0: i32, x1: i32, seg: &mut [u8]) {
        if y < self.bounds.y || y >= self.bounds.bottom() {
            return;
        }
        let bx0 = x0.max(self.bounds.x);
        let bx1 = x1.min(self.bounds.right());
        if bx1 <= bx0 {
            return;
        }
        let l = self.l;
        let w = l.w() as usize;
        if let Some((ox, oy)) = self.int_offset {
            let ly = y - oy;
            if ly < 0 || ly >= l.h() as i32 {
                return;
            }
            let ly = ly as usize;
            let src = &l.pixels.data[ly * w * 4..(ly + 1) * w * 4];
            let mrow = l.mask.as_ref().map(|m| &m.data[ly * w..(ly + 1) * w]);
            for x in bx0..bx1 {
                let lx = (x - ox) as usize;
                let si = lx * 4;
                let mut a = src[si + 3] as u32;
                if let Some(mr) = mrow {
                    a = mul255(a, mr[lx] as u32);
                }
                a = mul255(a, self.opacity);
                if a == 0 {
                    continue;
                }
                let di = ((x - x0) as usize) * 4;
                let d = &mut seg[di..di + 4];
                let sr = mul255(src[si] as u32, a);
                let sg = mul255(src[si + 1] as u32, a);
                let sb = mul255(src[si + 2] as u32, a);
                if a == 255 {
                    d[0] = sr as u8;
                    d[1] = sg as u8;
                    d[2] = sb as u8;
                    d[3] = 255;
                } else {
                    let ia = 255 - a;
                    d[0] = (sr + mul255(d[0] as u32, ia)) as u8;
                    d[1] = (sg + mul255(d[1] as u32, ia)) as u8;
                    d[2] = (sb + mul255(d[2] as u32, ia)) as u8;
                    d[3] = (a + mul255(d[3] as u32, ia)) as u8;
                }
            }
        } else {
            let inv = &self.inv;
            let cy = y as f64 + 0.5;
            for x in bx0..bx1 {
                let (u, v) = inv.apply(x as f64 + 0.5, cy);
                let u = u - 0.5;
                let v = v - 0.5;
                let fx0 = u.floor();
                let fy0 = v.floor();
                let ix = fx0 as i64;
                let iy = fy0 as i64;
                if ix < -1 || iy < -1 || ix >= l.w() as i64 || iy >= l.h() as i64 {
                    continue;
                }
                let fx = (u - fx0) as f32;
                let fy = (v - fy0) as f32;
                let p00 = self.premul_at(ix, iy);
                let p10 = self.premul_at(ix + 1, iy);
                let p01 = self.premul_at(ix, iy + 1);
                let p11 = self.premul_at(ix + 1, iy + 1);
                let mut s = [0f32; 4];
                for c in 0..4 {
                    let top = p00[c] + (p10[c] - p00[c]) * fx;
                    let bot = p01[c] + (p11[c] - p01[c]) * fx;
                    s[c] = (top + (bot - top) * fy) * self.opacity_f;
                }
                if s[3] < 0.5 {
                    continue;
                }
                let di = ((x - x0) as usize) * 4;
                let d = &mut seg[di..di + 4];
                let ia = 1.0 - s[3] / 255.0;
                d[0] = (s[0] + d[0] as f32 * ia + 0.5).min(255.0) as u8;
                d[1] = (s[1] + d[1] as f32 * ia + 0.5).min(255.0) as u8;
                d[2] = (s[2] + d[2] as f32 * ia + 0.5).min(255.0) as u8;
                d[3] = (s[3] + d[3] as f32 * ia + 0.5).min(255.0) as u8;
            }
        }
    }
}

/// Composite the given layers to a new straight-alpha RGBA image of
/// `rect` (canvas coordinates).
pub fn composite_to_straight(layers: &[&Layer], rect: IRect) -> Vec<u8> {
    let mut out = vec![0u8; rect.w as usize * rect.h as usize * 4];
    composite_region(layers, &mut out, rect.w as u32, rect.h as u32, (rect.x, rect.y), rect);
    crate::image::unpremultiply(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Transform;
    use crate::image::Rgba;

    #[test]
    fn source_over_half_alpha() {
        let mut bottom = Layer::new(1, "b", Rgba::filled(2, 2, [0, 0, 255, 255]));
        bottom.transform = Transform::at(0.0, 0.0, 2, 2);
        let mut top = Layer::new(2, "t", Rgba::filled(2, 2, [255, 0, 0, 255]));
        top.transform = Transform::at(0.0, 0.0, 2, 2);
        top.opacity = 0.5;
        let out = composite_to_straight(&[&bottom, &top], IRect::new(0, 0, 2, 2));
        assert_eq!(&out[0..4], &[128, 0, 127, 255]);
    }

    #[test]
    fn mask_hides_pixels() {
        let mut l = Layer::new(1, "b", Rgba::filled(2, 1, [10, 20, 30, 255]));
        l.transform = Transform::at(0.0, 0.0, 2, 1);
        l.mask = Some(crate::image::Plane::from_vec(2, 1, vec![0, 255]).unwrap());
        let out = composite_to_straight(&[&l], IRect::new(0, 0, 2, 1));
        assert_eq!(out[3], 0);
        assert_eq!(&out[4..8], &[10, 20, 30, 255]);
    }

    #[test]
    fn rotated_layer_covers_center() {
        let mut l = Layer::new(1, "b", Rgba::filled(10, 10, [255, 255, 255, 255]));
        l.transform = Transform { x: 10.0, y: 10.0, scale_x: 1.0, scale_y: 1.0, rotation: 0.7 };
        let out = composite_to_straight(&[&l], IRect::new(0, 0, 20, 20));
        assert_eq!(out[(10 * 20 + 10) * 4 + 3], 255);
        assert_eq!(out[3], 0);
    }
}
