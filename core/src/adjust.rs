//! Global image adjustments applied to a layer's pixels.

use crate::color::{hsl_to_rgb, rgb_to_hsl};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Adjustments {
    /// -1..1
    pub brightness: f32,
    /// -1..1
    pub contrast: f32,
    /// stops, -3..3
    pub exposure: f32,
    /// -1..1 (‑1 = greyscale)
    pub saturation: f32,
    /// degrees -180..180
    pub hue: f32,
    /// -1 (cool) .. 1 (warm)
    pub temperature: f32,
    /// -1 (magenta) .. 1 (green)
    pub tint: f32,
    /// -1..1, highlights recovery / lift
    pub highlights: f32,
    /// -1..1
    pub shadows: f32,
    /// 0..1
    pub sharpen: f32,
    pub invert: bool,
    pub grayscale: bool,
}

impl Adjustments {
    pub fn is_identity(&self) -> bool {
        *self == Adjustments::default()
    }
}

/// Apply adjustments to straight RGBA pixels (alpha untouched).
pub fn apply(src: &[u8], w: usize, h: usize, a: &Adjustments) -> Vec<u8> {
    // Build a per-channel tone LUT for the pointwise operations that don't
    // depend on other channels.
    let mut lut = [0f32; 256];
    let exp = 2f32.powf(a.exposure);
    let c = a.contrast.clamp(-1.0, 1.0);
    let cf = if c >= 0.0 { 1.0 + c * 2.0 } else { 1.0 + c };
    for (i, v) in lut.iter_mut().enumerate() {
        // exposure in linear light
        let lin = crate::color::srgb_to_linear(i as u8) * exp;
        let mut x = if lin <= 0.0031308 { lin * 12.92 } else { 1.055 * lin.min(1.0).powf(1.0 / 2.4) - 0.055 };
        x += a.brightness * 0.5;
        x = (x - 0.5) * cf + 0.5;
        // shadows / highlights (smooth tone curve)
        let sh = a.shadows * (1.0 - x).powi(2) * x * 1.5;
        let hi = a.highlights * x * x * (1.0 - x) * 1.5;
        x += sh + hi;
        *v = x.clamp(0.0, 1.0);
    }
    let temp = a.temperature * 0.12;
    let tint = a.tint * 0.08;
    let sat = 1.0 + a.saturation.clamp(-1.0, 1.0);
    let hue = a.hue / 360.0;
    let mut out = vec![0u8; src.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let s = &src[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let i = x * 4;
            let mut r = lut[s[i] as usize];
            let mut g = lut[s[i + 1] as usize];
            let mut b = lut[s[i + 2] as usize];
            if temp != 0.0 || tint != 0.0 {
                r += temp;
                b -= temp;
                g += tint;
                r -= tint * 0.5;
                b -= tint * 0.5;
            }
            if sat != 1.0 || hue != 0.0 {
                let (hh, ss, ll) = rgb_to_hsl(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
                let (nr, ng, nb) = hsl_to_rgb((hh + hue).rem_euclid(1.0), (ss * sat).clamp(0.0, 1.0), ll);
                r = nr;
                g = ng;
                b = nb;
            }
            if a.grayscale {
                let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = l;
                g = l;
                b = l;
            }
            if a.invert {
                r = 1.0 - r;
                g = 1.0 - g;
                b = 1.0 - b;
            }
            row[i] = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            row[i + 1] = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            row[i + 2] = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            row[i + 3] = s[i + 3];
        }
    });
    if a.sharpen > 0.0 {
        out = unsharp(&out, w, h, a.sharpen * 1.5, 1.2);
    }
    out
}

/// Unsharp mask on RGB channels.
pub fn unsharp(src: &[u8], w: usize, h: usize, amount: f32, sigma: f32) -> Vec<u8> {
    let mut out = src.to_vec();
    for c in 0..3 {
        let plane = crate::image::FPlane { w, h, data: (0..w * h).map(|i| src[i * 4 + c] as f32 / 255.0).collect() };
        let bl = crate::filters::gaussian(&plane, sigma);
        for i in 0..w * h {
            let v = plane.data[i] + (plane.data[i] - bl.data[i]) * amount;
            out[i * 4 + c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

/// Stretch each channel so that the 0.1 % darkest / brightest opaque pixels
/// map to 0 / 255.
pub fn auto_levels(src: &[u8]) -> Vec<u8> {
    let mut hist = [[0u64; 256]; 3];
    let mut n = 0u64;
    for p in src.chunks_exact(4) {
        if p[3] < 128 {
            continue;
        }
        n += 1;
        for c in 0..3 {
            hist[c][p[c] as usize] += 1;
        }
    }
    if n == 0 {
        return src.to_vec();
    }
    let clip = (n / 1000).max(1);
    let mut lo = [0u8; 3];
    let mut hi = [255u8; 3];
    for c in 0..3 {
        let mut acc = 0;
        for i in 0..256 {
            acc += hist[c][i];
            if acc >= clip {
                lo[c] = i as u8;
                break;
            }
        }
        acc = 0;
        for i in (0..256).rev() {
            acc += hist[c][i];
            if acc >= clip {
                hi[c] = i as u8;
                break;
            }
        }
    }
    let mut out = src.to_vec();
    for p in out.chunks_exact_mut(4) {
        for c in 0..3 {
            let (l, h) = (lo[c] as f32, hi[c] as f32);
            if h > l {
                p[c] = (((p[c] as f32 - l) / (h - l)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_is_lossless() {
        let src: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 200]).collect();
        let out = apply(&src, 256, 1, &Adjustments::default());
        for (a, b) in src.iter().zip(out.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1);
        }
    }
    #[test]
    fn invert_and_gray() {
        let out = apply(&[255, 0, 0, 255], 1, 1, &Adjustments { invert: true, ..Default::default() });
        assert_eq!(out, vec![0, 255, 255, 255]);
        let g = apply(&[255, 0, 0, 255], 1, 1, &Adjustments { grayscale: true, ..Default::default() });
        assert_eq!(g[0], g[1]);
    }
}
