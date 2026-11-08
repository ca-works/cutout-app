//! Document and layer model.
//!
//! A layer keeps its *source pixels* untouched by masking operations: erasing,
//! magic-erasing and background removal only write the layer **mask**, so any
//! removed area can be restored later with the Restore brush. When an
//! operation must modify colours (edge decontamination), the pre-edit pixels
//! are preserved in `original` so that restoring brings the true colours back.

use crate::geometry::{Affine, IRect, Transform};
use crate::image::{Plane, Rgba};
use serde::{Deserialize, Serialize};

pub type LayerId = i64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextProps {
    pub text: String,
    pub font: String,
    pub size: f64,
    pub color: [u8; 4],
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub align: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LayerKind {
    Raster,
    /// Editable text. Pixels are a raster cache produced by the platform text
    /// renderer (CoreText on macOS, DirectWrite on Windows).
    Text(TextProps),
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub kind: LayerKind,
    pub pixels: Rgba,
    pub mask: Option<Plane>,
    pub original: Option<Rgba>,
    pub transform: Transform,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>, pixels: Rgba) -> Self {
        Layer {
            id,
            name: name.into(),
            kind: LayerKind::Raster,
            pixels,
            mask: None,
            original: None,
            transform: Transform::default(),
            opacity: 1.0,
            visible: true,
            locked: false,
        }
    }
    pub fn w(&self) -> u32 {
        self.pixels.w
    }
    pub fn h(&self) -> u32 {
        self.pixels.h
    }
    pub fn matrix(&self) -> Affine {
        self.transform.matrix(self.w(), self.h())
    }
    pub fn inverse(&self) -> Affine {
        self.matrix().invert().unwrap_or(Affine::IDENTITY)
    }
    pub fn canvas_bounds(&self) -> IRect {
        self.matrix().map_rect_bounds(self.w() as f64, self.h() as f64)
    }
    /// Effective (pixel alpha × mask) alpha at a layer pixel.
    #[inline]
    pub fn effective_px(&self, x: u32, y: u32) -> [u8; 4] {
        let mut p = self.pixels.px(x, y);
        if let Some(m) = &self.mask {
            p[3] = crate::image::mul255(p[3] as u32, m.get(x, y) as u32) as u8;
        }
        p
    }
    pub fn ensure_mask(&mut self) -> &mut Vec<u8> {
        if self.mask.is_none() {
            self.mask = Some(Plane::new(self.w(), self.h(), 255));
        }
        self.mask.as_mut().unwrap().make_mut()
    }
    pub fn is_text(&self) -> bool {
        matches!(self.kind, LayerKind::Text(_))
    }
    /// Straight RGBA with the mask baked into alpha.
    pub fn flattened_pixels(&self) -> Rgba {
        match &self.mask {
            None => self.pixels.clone(),
            Some(m) => {
                let mut out = (*self.pixels.data).clone();
                for (p, &mv) in out.chunks_exact_mut(4).zip(m.data.iter()) {
                    p[3] = crate::image::mul255(p[3] as u32, mv as u32) as u8;
                }
                Rgba { w: self.w(), h: self.h(), data: std::sync::Arc::new(out) }
            }
        }
    }
    /// Approximate memory footprint in bytes.
    pub fn bytes(&self) -> usize {
        self.pixels.bytes() + self.mask.as_ref().map_or(0, |m| m.data.len()) + self.original.as_ref().map_or(0, |o| o.bytes())
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub width: u32,
    pub height: u32,
    /// Bottom-most first.
    pub layers: Vec<Layer>,
    pub active: Option<LayerId>,
    pub next_id: LayerId,
}

impl Default for Document {
    fn default() -> Self {
        Document { width: 0, height: 0, layers: Vec::new(), active: None, next_id: 1 }
    }
}

impl Document {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() || self.width == 0
    }
    pub fn alloc_id(&mut self) -> LayerId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }
    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }
    pub fn active_layer(&self) -> Option<&Layer> {
        self.active.and_then(|id| self.layer(id))
    }
    pub fn active_index(&self) -> Option<usize> {
        self.active.and_then(|id| self.index_of(id))
    }
    pub fn canvas_rect(&self) -> IRect {
        IRect::new(0, 0, self.width as i32, self.height as i32)
    }
    pub fn unique_name(&self, base: &str) -> String {
        if !self.layers.iter().any(|l| l.name == base) {
            return base.to_string();
        }
        for i in 2.. {
            let n = format!("{base} {i}");
            if !self.layers.iter().any(|l| l.name == n) {
                return n;
            }
        }
        unreachable!()
    }
}
