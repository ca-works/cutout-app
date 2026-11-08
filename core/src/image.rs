//! Pixel containers.
//!
//! All colour images are 8-bit sRGB RGBA with *straight* (non-premultiplied)
//! alpha. Buffers live behind `Arc` so that undo snapshots are copy-on-write:
//! cloning an image is O(1) and only the buffers an operation actually
//! touches are duplicated (`Arc::make_mut`).

use std::sync::Arc;

/// Hard safety limits for any image entering the engine.
pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_PIXELS: u64 = 120_000_000;

pub fn check_dimensions(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("Image has no pixels.".into());
    }
    if w > MAX_DIMENSION || h > MAX_DIMENSION || (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(format!(
            "Image is too large ({w}×{h}). The maximum is {MAX_DIMENSION} px per side and {} megapixels.",
            MAX_PIXELS / 1_000_000
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub data: Arc<Vec<u8>>,
}

impl Rgba {
    pub fn new(w: u32, h: u32) -> Self {
        Rgba { w, h, data: Arc::new(vec![0; w as usize * h as usize * 4]) }
    }
    pub fn from_vec(w: u32, h: u32, v: Vec<u8>) -> Result<Self, String> {
        if v.len() != w as usize * h as usize * 4 {
            return Err("RGBA buffer length does not match dimensions".into());
        }
        Ok(Rgba { w, h, data: Arc::new(v) })
    }
    pub fn filled(w: u32, h: u32, c: [u8; 4]) -> Self {
        let mut v = Vec::with_capacity(w as usize * h as usize * 4);
        for _ in 0..(w as usize * h as usize) {
            v.extend_from_slice(&c);
        }
        Rgba { w, h, data: Arc::new(v) }
    }
    #[inline]
    pub fn idx(&self, x: u32, y: u32) -> usize {
        (y as usize * self.w as usize + x as usize) * 4
    }
    #[inline]
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.idx(x, y);
        let d = &self.data;
        [d[i], d[i + 1], d[i + 2], d[i + 3]]
    }
    pub fn make_mut(&mut self) -> &mut Vec<u8> {
        Arc::make_mut(&mut self.data)
    }
    pub fn bytes(&self) -> usize {
        self.data.len()
    }
}

/// Single-channel 8-bit plane (masks, selections, alpha mattes).
#[derive(Clone, Debug)]
pub struct Plane {
    pub w: u32,
    pub h: u32,
    pub data: Arc<Vec<u8>>,
}

impl Plane {
    pub fn new(w: u32, h: u32, v: u8) -> Self {
        Plane { w, h, data: Arc::new(vec![v; w as usize * h as usize]) }
    }
    pub fn from_vec(w: u32, h: u32, v: Vec<u8>) -> Result<Self, String> {
        if v.len() != w as usize * h as usize {
            return Err("Plane buffer length does not match dimensions".into());
        }
        Ok(Plane { w, h, data: Arc::new(v) })
    }
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.data[y as usize * self.w as usize + x as usize]
    }
    #[inline]
    pub fn get_clamped(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            0
        } else {
            self.data[y as usize * self.w as usize + x as usize]
        }
    }
    pub fn make_mut(&mut self) -> &mut Vec<u8> {
        Arc::make_mut(&mut self.data)
    }
    pub fn is_all(&self, v: u8) -> bool {
        self.data.iter().all(|&p| p == v)
    }
    pub fn any_nonzero(&self) -> bool {
        self.data.iter().any(|&p| p != 0)
    }
    /// Bounding box of pixels with value > threshold, as (x, y, w, h).
    pub fn bbox(&self, threshold: u8) -> Option<crate::geometry::IRect> {
        let (w, h) = (self.w as usize, self.h as usize);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for y in 0..h {
            let row = &self.data[y * w..(y + 1) * w];
            if let Some(first) = row.iter().position(|&v| v > threshold) {
                let last = row.iter().rposition(|&v| v > threshold).unwrap();
                x0 = x0.min(first);
                x1 = x1.max(last);
                y0 = y0.min(y);
                y1 = y;
            }
        }
        if x0 == usize::MAX {
            None
        } else {
            Some(crate::geometry::IRect::new(x0 as i32, y0 as i32, (x1 - x0 + 1) as i32, (y1 - y0 + 1) as i32))
        }
    }
}

/// Floating point single-channel plane used by filters.
#[derive(Clone, Debug)]
pub struct FPlane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl FPlane {
    pub fn new(w: usize, h: usize) -> Self {
        FPlane { w, h, data: vec![0.0; w * h] }
    }
    pub fn from_u8(p: &[u8], w: usize, h: usize) -> Self {
        FPlane { w, h, data: p.iter().map(|&v| v as f32 / 255.0).collect() }
    }
    pub fn to_u8(&self) -> Vec<u8> {
        self.data.iter().map(|&v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
    }
    #[inline]
    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        let x = x.clamp(0.0, (self.w - 1) as f32);
        let y = y.clamp(0.0, (self.h - 1) as f32);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let a = self.data[y0 * self.w + x0];
        let b = self.data[y0 * self.w + x1];
        let c = self.data[y1 * self.w + x0];
        let d = self.data[y1 * self.w + x1];
        (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
    }
}

#[inline]
pub fn mul255(a: u32, b: u32) -> u32 {
    // exact rounding of a*b/255
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// Premultiply a straight RGBA buffer in place.
pub fn premultiply(buf: &mut [u8]) {
    for p in buf.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a == 255 {
            continue;
        }
        p[0] = mul255(p[0] as u32, a) as u8;
        p[1] = mul255(p[1] as u32, a) as u8;
        p[2] = mul255(p[2] as u32, a) as u8;
    }
}

/// Convert premultiplied RGBA to straight alpha in place.
pub fn unpremultiply(buf: &mut [u8]) {
    for p in buf.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a == 255 {
            continue;
        }
        if a == 0 {
            p[0] = 0;
            p[1] = 0;
            p[2] = 0;
            continue;
        }
        p[0] = ((p[0] as u32 * 255 + a / 2) / a).min(255) as u8;
        p[1] = ((p[1] as u32 * 255 + a / 2) / a).min(255) as u8;
        p[2] = ((p[2] as u32 * 255 + a / 2) / a).min(255) as u8;
    }
}

/// High quality resize of a straight-alpha RGBA image (alpha-aware so that
/// transparent pixels never bleed colour into the result).
pub fn resize_rgba(src: &Rgba, nw: u32, nh: u32) -> Rgba {
    if src.w == nw && src.h == nh {
        return src.clone();
    }
    let mut pm = (*src.data).clone();
    premultiply(&mut pm);
    let img = image::RgbaImage::from_raw(src.w, src.h, pm).expect("valid dims");
    let filter = if nw < src.w || nh < src.h {
        image::imageops::FilterType::Lanczos3
    } else {
        image::imageops::FilterType::CatmullRom
    };
    let out = image::imageops::resize(&img, nw.max(1), nh.max(1), filter);
    let mut v = out.into_raw();
    // Lanczos may ring: keep colour <= alpha before unpremultiplying.
    for p in v.chunks_exact_mut(4) {
        let a = p[3];
        p[0] = p[0].min(a);
        p[1] = p[1].min(a);
        p[2] = p[2].min(a);
    }
    unpremultiply(&mut v);
    Rgba { w: nw.max(1), h: nh.max(1), data: Arc::new(v) }
}

pub fn resize_plane(src: &Plane, nw: u32, nh: u32) -> Plane {
    if src.w == nw && src.h == nh {
        return src.clone();
    }
    let img = image::GrayImage::from_raw(src.w, src.h, (*src.data).clone()).expect("valid dims");
    let filter = if nw < src.w || nh < src.h {
        image::imageops::FilterType::Triangle
    } else {
        image::imageops::FilterType::CatmullRom
    };
    let out = image::imageops::resize(&img, nw.max(1), nh.max(1), filter);
    Plane { w: nw.max(1), h: nh.max(1), data: Arc::new(out.into_raw()) }
}
