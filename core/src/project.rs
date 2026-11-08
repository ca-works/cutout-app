//! `.cutout` project files.
//!
//! A project is a ZIP container:
//!
//! ```text
//! manifest.json                 format, version, canvas size, layer stack
//! layers/<id>/pixels.png        source pixels (RGBA, straight alpha)
//! layers/<id>/mask.png          optional 8-bit layer mask
//! layers/<id>/original.png      optional pre-decontamination pixels
//! preview.png                   flattened preview (for Quick Look / thumbnails)
//! ```
//!
//! Loading is defensive: entry count, per-entry and total decompressed size
//! and image dimensions are bounded, unknown entries are ignored and file
//! names are never used as filesystem paths.

use crate::document::{Document, Layer, LayerKind};
use crate::geometry::Transform;
use crate::image::{check_dimensions, Plane, Rgba};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Seek, Write};

pub const FORMAT: &str = "cutout";
pub const VERSION: u32 = 1;
const MAX_ENTRIES: usize = 2048;
const MAX_ENTRY_BYTES: u64 = 1 << 31;
const MAX_TOTAL_BYTES: u64 = 8 << 30;

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    app: String,
    width: u32,
    height: u32,
    active: Option<i64>,
    layers: Vec<LayerEntry>,
}

#[derive(Serialize, Deserialize)]
struct LayerEntry {
    id: i64,
    name: String,
    kind: LayerKind,
    transform: Transform,
    opacity: f32,
    visible: bool,
    #[serde(default)]
    locked: bool,
    width: u32,
    height: u32,
    has_mask: bool,
    #[serde(default)]
    has_original: bool,
}

fn encode_png(data: &[u8], w: u32, h: u32, color: png::ColorType) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(data).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

fn decode_png(bytes: &[u8], expect: png::ColorType, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().map_err(|e| format!("Corrupt layer image: {e}"))?;
    let info = reader.info();
    if info.width != w || info.height != h {
        return Err("Layer image size does not match the manifest.".into());
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or("Layer image too large")?];
    let frame = reader.next_frame(&mut buf).map_err(|e| format!("Corrupt layer image: {e}"))?;
    buf.truncate(frame.buffer_size());
    let ct = frame.color_type;
    if frame.bit_depth != png::BitDepth::Eight {
        return Err("Unsupported layer bit depth.".into());
    }
    let n = (w * h) as usize;
    let out = match (expect, ct) {
        (a, b) if a == b => buf,
        (png::ColorType::Rgba, png::ColorType::Rgb) => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        (png::ColorType::Grayscale, png::ColorType::GrayscaleAlpha) => buf.chunks_exact(2).map(|p| p[0]).collect(),
        _ => return Err("Unexpected layer colour type.".into()),
    };
    let ch = if expect == png::ColorType::Rgba { 4 } else { 1 };
    if out.len() != n * ch {
        return Err("Layer image data truncated.".into());
    }
    Ok(out)
}

pub fn save<W: Write + Seek>(doc: &Document, preview: Option<(&[u8], u32, u32)>, writer: W) -> Result<(), String> {
    let mut zip = zip::ZipWriter::new(writer);
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        app: format!("CutOut {}", env!("CARGO_PKG_VERSION")),
        width: doc.width,
        height: doc.height,
        active: doc.active,
        layers: doc
            .layers
            .iter()
            .map(|l| LayerEntry {
                id: l.id,
                name: l.name.clone(),
                kind: l.kind.clone(),
                transform: l.transform,
                opacity: l.opacity,
                visible: l.visible,
                locked: l.locked,
                width: l.w(),
                height: l.h(),
                has_mask: l.mask.is_some(),
                has_original: l.original.is_some(),
            })
            .collect(),
    };
    zip.start_file("manifest.json", deflated).map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
    for l in &doc.layers {
        let png = encode_png(&l.pixels.data, l.w(), l.h(), png::ColorType::Rgba)?;
        zip.start_file(format!("layers/{}/pixels.png", l.id), stored).map_err(|e| e.to_string())?;
        zip.write_all(&png).map_err(|e| e.to_string())?;
        if let Some(m) = &l.mask {
            let png = encode_png(&m.data, m.w, m.h, png::ColorType::Grayscale)?;
            zip.start_file(format!("layers/{}/mask.png", l.id), stored).map_err(|e| e.to_string())?;
            zip.write_all(&png).map_err(|e| e.to_string())?;
        }
        if let Some(o) = &l.original {
            let png = encode_png(&o.data, o.w, o.h, png::ColorType::Rgba)?;
            zip.start_file(format!("layers/{}/original.png", l.id), stored).map_err(|e| e.to_string())?;
            zip.write_all(&png).map_err(|e| e.to_string())?;
        }
    }
    if let Some((px, w, h)) = preview {
        let png = encode_png(px, w, h, png::ColorType::Rgba)?;
        zip.start_file("preview.png", stored).map_err(|e| e.to_string())?;
        zip.write_all(&png).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn save_to_path(doc: &Document, preview: Option<(&[u8], u32, u32)>, path: &std::path::Path) -> Result<(), String> {
    // write atomically: temp file in the same directory, then rename
    let dir = path.parent().ok_or("Invalid path")?;
    let tmp = dir.join(format!(".{}.tmp-{}", path.file_name().and_then(|s| s.to_str()).unwrap_or("project"), std::process::id()));
    {
        let f = std::fs::File::create(&tmp).map_err(|e| format!("Cannot write project: {e}"))?;
        let mut bw = std::io::BufWriter::new(f);
        if let Err(e) = save(doc, preview, &mut bw) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        bw.flush().map_err(|e| e.to_string())?;
        bw.get_ref().sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot save project: {e}")
    })
}

fn read_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str, total: &mut u64) -> Result<Option<Vec<u8>>, String> {
    let mut f = match zip.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("Corrupt project: {e}")),
    };
    if f.size() > MAX_ENTRY_BYTES {
        return Err("Project entry is too large.".into());
    }
    let mut buf = Vec::with_capacity(f.size().min(64 << 20) as usize);
    let n = (&mut f).take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buf).map_err(|e| format!("Corrupt project: {e}"))?;
    *total += n as u64;
    if n as u64 > MAX_ENTRY_BYTES || *total > MAX_TOTAL_BYTES {
        return Err("Project is too large.".into());
    }
    Ok(Some(buf))
}

pub fn load<R: Read + Seek>(reader: R) -> Result<Document, String> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|_| "This file is not a CutOut project.".to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err("Project has too many entries.".into());
    }
    let mut total = 0u64;
    let mbytes = read_entry(&mut zip, "manifest.json", &mut total)?.ok_or("Project manifest missing.")?;
    if mbytes.len() > 16 << 20 {
        return Err("Project manifest too large.".into());
    }
    let m: Manifest = serde_json::from_slice(&mbytes).map_err(|e| format!("Invalid project manifest: {e}"))?;
    if m.format != FORMAT {
        return Err("This file is not a CutOut project.".into());
    }
    if m.version > VERSION {
        return Err(format!("This project was created by a newer version of CutOut (format {}).", m.version));
    }
    check_dimensions(m.width, m.height)?;
    let mut doc = Document { width: m.width, height: m.height, layers: Vec::new(), active: None, next_id: 1 };
    let mut seen = std::collections::HashSet::new();
    for e in m.layers {
        if !seen.insert(e.id) || e.id <= 0 {
            return Err("Project contains duplicate layer ids.".into());
        }
        check_dimensions(e.width, e.height)?;
        let px = read_entry(&mut zip, &format!("layers/{}/pixels.png", e.id), &mut total)?.ok_or("Layer pixels missing.")?;
        let pixels = Rgba::from_vec(e.width, e.height, decode_png(&px, png::ColorType::Rgba, e.width, e.height)?)?;
        let mask = if e.has_mask {
            let mb = read_entry(&mut zip, &format!("layers/{}/mask.png", e.id), &mut total)?.ok_or("Layer mask missing.")?;
            Some(Plane::from_vec(e.width, e.height, decode_png(&mb, png::ColorType::Grayscale, e.width, e.height)?)?)
        } else {
            None
        };
        let original = if e.has_original {
            match read_entry(&mut zip, &format!("layers/{}/original.png", e.id), &mut total)? {
                Some(ob) => Some(Rgba::from_vec(e.width, e.height, decode_png(&ob, png::ColorType::Rgba, e.width, e.height)?)?),
                None => None,
            }
        } else {
            None
        };
        let t = e.transform;
        let finite = [t.x, t.y, t.scale_x, t.scale_y, t.rotation].iter().all(|v| v.is_finite());
        if !finite || t.scale_x.abs() < 1e-6 || t.scale_y.abs() < 1e-6 {
            return Err("Project contains an invalid layer transform.".into());
        }
        doc.next_id = doc.next_id.max(e.id + 1);
        doc.layers.push(Layer {
            id: e.id,
            name: e.name.chars().take(256).collect(),
            kind: e.kind,
            pixels,
            mask,
            original,
            transform: t,
            opacity: e.opacity.clamp(0.0, 1.0),
            visible: e.visible,
            locked: e.locked,
        });
    }
    doc.active = m.active.filter(|id| doc.layer(*id).is_some()).or(doc.layers.last().map(|l| l.id));
    Ok(doc)
}

pub fn load_from_path(path: &std::path::Path) -> Result<Document, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("Cannot open project: {e}"))?;
    load(std::io::BufReader::new(f))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let mut doc = Document { width: 8, height: 6, ..Default::default() };
        let id = doc.alloc_id();
        let mut l = Layer::new(id, "Photo", Rgba::filled(8, 6, [1, 2, 3, 200]));
        l.mask = Some(Plane::new(8, 6, 77));
        l.transform = Transform { x: 4.0, y: 3.0, scale_x: 1.0, scale_y: 1.0, rotation: 0.25 };
        doc.layers.push(l);
        let id2 = doc.alloc_id();
        let mut t = Layer::new(id2, "Hello", Rgba::filled(3, 2, [9, 9, 9, 255]));
        t.kind = LayerKind::Text(crate::document::TextProps { text: "Hi".into(), font: "Helvetica".into(), size: 24.0, color: [0, 0, 0, 255], bold: true, italic: false, align: "left".into() });
        t.opacity = 0.5;
        doc.layers.push(t);
        doc.active = Some(id);
        let mut buf = Cursor::new(Vec::new());
        save(&doc, None, &mut buf).unwrap();
        buf.set_position(0);
        let d2 = load(buf).unwrap();
        assert_eq!(d2.width, 8);
        assert_eq!(d2.layers.len(), 2);
        assert_eq!(d2.layers[0].pixels.px(3, 3), [1, 2, 3, 200]);
        assert_eq!(d2.layers[0].mask.as_ref().unwrap().get(1, 1), 77);
        assert_eq!(d2.layers[0].transform.rotation, 0.25);
        assert_eq!(d2.layers[1].kind, doc.layers[1].kind);
        assert_eq!(d2.active, Some(id));
        assert_eq!(d2.next_id, 3);
    }
    #[test]
    fn rejects_garbage() {
        assert!(load(Cursor::new(b"not a zip".to_vec())).is_err());
    }
}
