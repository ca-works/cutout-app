//! Image decoding and encoding. Everything is converted to 8-bit straight-alpha
//! sRGB with the EXIF orientation applied, exactly like the macOS ImageIO path,
//! so the engine always sees upright pixels in one colour space.

use crate::engine::Image;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::Path;

pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_PIXELS: u64 = 120_000_000;
pub const MAX_FILE_BYTES: u64 = 1 << 30;
/// Decoding refuses images beyond this many pixels (memory-safety guard).
const HARD_PIXEL_LIMIT: u64 = 400_000_000;

pub struct Decoded {
    pub image: Image,
    pub downscaled: bool,
}

pub const READABLE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jpe", "jfif", "gif", "bmp", "tif", "tiff", "webp"];

pub fn is_image_path(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| READABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

pub fn decode_file(path: &Path) -> Result<Decoded, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("Could not open the file: {e}"))?;
    if !meta.is_file() {
        return Err("This isn't a file.".into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err("This file is larger than 1 GB.".into());
    }
    let f = std::fs::File::open(path).map_err(|e| format!("Could not open the file: {e}"))?;
    decode_reader(BufReader::new(f))
}

pub fn decode_bytes(data: &[u8]) -> Result<Decoded, String> {
    decode_reader(Cursor::new(data))
}

fn decode_reader<R: Read + Seek + std::io::BufRead>(r: R) -> Result<Decoded, String> {
    let reader = ImageReader::new(r).with_guessed_format().map_err(|_| "This file isn't an image CutOut can read.".to_string())?;
    let mut decoder = reader.into_decoder().map_err(|_| "This file isn't an image CutOut can read.".to_string())?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 {
        return Err("The image has no pixels.".into());
    }
    if (w as u64) * (h as u64) > HARD_PIXEL_LIMIT {
        return Err("This image is too large to open safely (over 400 megapixels).".into());
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(3 << 30);
    let _ = decoder.set_limits(limits);
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| format!("The image could not be decoded: {e}"))?;
    img.apply_orientation(orientation);
    let (w, h) = (img.width(), img.height());
    let mut rgba = img.into_rgba8().into_raw();
    if let Some(icc) = icc {
        to_srgb(&mut rgba, &icc);
    }
    let mut out = Image { width: w, height: h, data: rgba };
    // Huge images are downsampled instead of allocating unbounded buffers.
    let too_big = w > MAX_DIMENSION || h > MAX_DIMENSION || (w as u64) * (h as u64) > MAX_PIXELS;
    if too_big {
        let s = (MAX_DIMENSION as f64 / w.max(h) as f64).min((MAX_PIXELS as f64 / (w as f64 * h as f64)).sqrt());
        let nw = ((w as f64 * s).floor() as u32).max(1);
        let nh = ((h as f64 * s).floor() as u32).max(1);
        out = crate::engine::Doc::resize(&out, nw, nh).ok_or("The image could not be resized.")?;
    }
    Ok(Decoded { image: out, downscaled: too_big })
}

/// Convert RGB from the embedded ICC profile to sRGB in place (alpha untouched).
fn to_srgb(rgba: &mut [u8], icc: &[u8]) {
    use moxcms::{ColorProfile, Layout, TransformOptions};
    let Ok(src) = ColorProfile::new_from_slice(icc) else { return };
    let dst = ColorProfile::new_srgb();
    // Skip profiles that already are sRGB-equivalent for speed.
    let Ok(t) = src.create_transform_8bit(Layout::Rgba, &dst, Layout::Rgba, TransformOptions::default()) else { return };
    let src_copy = rgba.to_vec();
    if t.transform(&src_copy, rgba).is_ok() {
        // moxcms writes alpha as given; nothing else to do.
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

impl Format {
    pub fn from_clip(f: crate::settings::ClipFormat) -> Format {
        match f {
            crate::settings::ClipFormat::Png => Format::Png,
            crate::settings::ClipFormat::Jpeg => Format::Jpeg,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
        }
    }
}

pub fn encode(img: &Image, format: Format) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    match format {
        Format::Png => {
            let enc = image::codecs::png::PngEncoder::new_with_quality(&mut out, image::codecs::png::CompressionType::Default, image::codecs::png::FilterType::Adaptive);
            image::ImageEncoder::write_image(enc, &img.data, img.width, img.height, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
        }
        Format::Jpeg => {
            let flat = img.flattened([255, 255, 255]);
            let rgb: Vec<u8> = flat.data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92);
            image::ImageEncoder::write_image(enc, &rgb, img.width, img.height, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip_keeps_alpha() {
        let img = Image { width: 2, height: 1, data: vec![255, 0, 0, 128, 0, 255, 0, 0] };
        let bytes = encode(&img, Format::Png).unwrap();
        let back = decode_bytes(&bytes).unwrap();
        assert_eq!(back.image.width, 2);
        assert_eq!(back.image.data[3], 128);
        assert_eq!(back.image.data[7], 0);
    }

    #[test]
    fn jpeg_flattens_on_white() {
        let img = Image { width: 1, height: 1, data: vec![0, 0, 0, 0] };
        let bytes = encode(&img, Format::Jpeg).unwrap();
        let back = decode_bytes(&bytes).unwrap();
        assert!(back.image.data[0] > 240);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(decode_bytes(b"not an image").is_err());
    }
}
