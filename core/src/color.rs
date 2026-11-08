//! Colour science used by the colour-based tools.
//!
//! Selection tolerance is measured as CIEDE2000 ΔE in CIELAB (D65) so that
//! perceptually different colours of similar brightness – e.g. a beige
//! backdrop and a white machined part – are kept apart, which a plain RGB
//! distance or luminance threshold cannot do.

use std::sync::OnceLock;

fn srgb_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0f32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        t
    })
}

#[inline]
pub fn srgb_to_linear(v: u8) -> f32 {
    srgb_lut()[v as usize]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

#[inline]
fn lab_f(t: f32) -> f32 {
    const D: f32 = 6.0 / 29.0;
    if t > D * D * D {
        t.cbrt()
    } else {
        t / (3.0 * D * D) + 4.0 / 29.0
    }
}

pub fn rgb_to_lab(r: u8, g: u8, b: u8) -> Lab {
    let (r, g, b) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let (fx, fy, fz) = (lab_f(x), lab_f(y), lab_f(z));
    Lab { l: 116.0 * fy - 16.0, a: 500.0 * (fx - fy), b: 200.0 * (fy - fz) }
}

/// CIEDE2000 colour difference (Sharma, Wu & Dalal 2005 formulation).
pub fn delta_e2000(p: Lab, q: Lab) -> f32 {
    let (l1, a1, b1) = (p.l as f64, p.a as f64, p.b as f64);
    let (l2, a2, b2) = (q.l as f64, q.a as f64, q.b as f64);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cbar = (c1 + c2) / 2.0;
    let cbar7 = cbar.powi(7);
    let g = 0.5 * (1.0 - (cbar7 / (cbar7 + 25f64.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let hp = |b: f64, a: f64| -> f64 {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            let h = b.atan2(a).to_degrees();
            if h < 0.0 {
                h + 360.0
            } else {
                h
            }
        }
    };
    let h1p = hp(b1, a1p);
    let h2p = hp(b2, a2p);
    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d.abs() <= 180.0 {
            d
        } else if d > 180.0 {
            d - 360.0
        } else {
            d + 360.0
        }
    };
    let dhp_big = 2.0 * (c1p * c2p).sqrt() * (dhp.to_radians() / 2.0).sin();
    let lbarp = (l1 + l2) / 2.0;
    let cbarp = (c1p + c2p) / 2.0;
    let hbarp = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hbarp - 30.0).to_radians().cos() + 0.24 * (2.0 * hbarp).to_radians().cos()
        + 0.32 * (3.0 * hbarp + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hbarp - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hbarp - 275.0) / 25.0).powi(2)).exp();
    let cbarp7 = cbarp.powi(7);
    let rc = 2.0 * (cbarp7 / (cbarp7 + 25f64.powi(7))).sqrt();
    let lb50 = (lbarp - 50.0).powi(2);
    let sl = 1.0 + 0.015 * lb50 / (20.0 + lb50).sqrt();
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -(2.0 * dtheta.to_radians()).sin() * rc;
    let v = (dlp / sl).powi(2) + (dcp / sc).powi(2) + (dhp_big / sh).powi(2) + rt * (dcp / sc) * (dhp_big / sh);
    v.max(0.0).sqrt() as f32
}

/// A reference colour for distance queries, pre-converted to Lab.
#[derive(Clone, Copy, Debug)]
pub struct ColorKey {
    pub lab: Lab,
    pub alpha: u8,
}

impl ColorKey {
    pub fn new(c: [u8; 4]) -> Self {
        ColorKey { lab: rgb_to_lab(c[0], c[1], c[2]), alpha: c[3] }
    }
    /// Perceptual distance between this key and a pixel with effective alpha.
    /// Transparency participates: fully transparent pixels match only a
    /// transparent key, and partially transparent pixels are compared with
    /// their alpha difference added in ΔE-equivalent units.
    #[inline]
    pub fn distance(&self, px: [u8; 4]) -> f32 {
        let (ka, pa) = (self.alpha as f32 / 255.0, px[3] as f32 / 255.0);
        if ka < 0.03 && pa < 0.03 {
            return 0.0;
        }
        let da = (ka - pa).abs() * 100.0;
        if ka < 0.03 || pa < 0.03 {
            return 100.0f32.max(da);
        }
        let de = delta_e2000(self.lab, rgb_to_lab(px[0], px[1], px[2]));
        (de * de + da * da).sqrt()
    }
}

/// Rec. 709 luma for 0..=255 input, returned 0..=1.
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> f32 {
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| {
        if t < 0.0 {
            t += 1.0
        }
        if t > 1.0 {
            t -= 1.0
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ciede2000_reference_pairs() {
        // Sharma et al. test data, pairs 1 and 7 (expected 2.0425 and 0.0)
        let d = delta_e2000(Lab { l: 50.0, a: 2.6772, b: -79.7751 }, Lab { l: 50.0, a: 0.0, b: -82.7485 });
        assert!((d - 2.0425).abs() < 1e-3, "{d}");
        let d = delta_e2000(Lab { l: 50.0, a: 0.0, b: 0.0 }, Lab { l: 50.0, a: -1.0, b: 2.0 });
        assert!((d - 2.3669).abs() < 1e-3, "{d}");
        let d = delta_e2000(Lab { l: 60.2574, a: -34.0099, b: 36.2677 }, Lab { l: 60.4626, a: -34.1751, b: 39.4387 });
        assert!((d - 1.2644).abs() < 1e-3, "{d}");
    }
    #[test]
    fn beige_and_white_are_far_apart() {
        let beige = ColorKey::new([232, 220, 196, 255]);
        let white = beige.distance([250, 250, 250, 255]);
        let beige_shadow = beige.distance([222, 210, 188, 255]);
        assert!(white > 10.0, "white vs beige ΔE {white}");
        assert!(beige_shadow < white / 2.0, "shadowed beige {beige_shadow}");
    }
    #[test]
    fn transparent_matches_only_transparent() {
        let k = ColorKey::new([0, 0, 0, 0]);
        assert_eq!(k.distance([255, 0, 0, 0]), 0.0);
        assert!(k.distance([255, 0, 0, 255]) >= 100.0);
    }
}
