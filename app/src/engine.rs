//! Typed wrapper over `cutout_core::engine::Engine`'s JSON command protocol —
//! the same protocol the macOS app drives through the C ABI.

use cutout_core::engine::Engine;
use cutout_core::image::Rgba;
use serde_json::{json, Value};

pub use cutout_core::document::TextProps;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub rotation: f64,
}

impl Transform {
    pub fn json(&self) -> Value {
        json!({"x": self.x, "y": self.y, "scale_x": self.scale_x, "scale_y": self.scale_y, "rotation": self.rotation})
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x as f64 && py >= self.y as f64 && px < (self.x + self.w) as f64 && py < (self.y + self.h) as f64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerInfo {
    pub id: i64,
    pub name: String,
    pub visible: bool,
    pub opacity: f64,
    pub locked: bool,
    pub width: i32,
    pub height: i32,
    pub has_mask: bool,
    pub is_text: bool,
    pub text: Option<TextProps>,
    pub transform: Transform,
    pub bounds: IRect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub width: i32,
    pub height: i32,
    pub layers: Vec<LayerInfo>,
    pub active: Option<i64>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub has_selection: bool,
    pub selection_bounds: Option<IRect>,
    pub modified: bool,
    pub version: i64,
}

impl State {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() || self.width == 0
    }
    pub fn active_layer(&self) -> Option<&LayerInfo> {
        let a = self.active?;
        self.layers.iter().find(|l| l.id == a)
    }

    fn from_json(v: &Value) -> State {
        let i = |x: &Value, k: &str| x.get(k).and_then(Value::as_i64).unwrap_or(0);
        let f = |x: &Value, k: &str, d: f64| x.get(k).and_then(Value::as_f64).unwrap_or(d);
        let b = |x: &Value, k: &str| x.get(k).and_then(Value::as_bool).unwrap_or(false);
        let rect = |x: &Value| -> Option<IRect> {
            let o = x.as_object()?;
            Some(IRect { x: o.get("x")?.as_i64()? as i32, y: o.get("y")?.as_i64()? as i32, w: o.get("w")?.as_i64()? as i32, h: o.get("h")?.as_i64()? as i32 })
        };
        let layers = v
            .get("layers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|l| {
                        let t = l.get("transform").cloned().unwrap_or(Value::Null);
                        LayerInfo {
                            id: i(l, "id"),
                            name: l.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                            visible: l.get("visible").and_then(Value::as_bool).unwrap_or(true),
                            opacity: f(l, "opacity", 1.0),
                            locked: b(l, "locked"),
                            width: i(l, "width") as i32,
                            height: i(l, "height") as i32,
                            has_mask: b(l, "has_mask"),
                            is_text: l.get("kind").and_then(Value::as_str) == Some("text"),
                            text: l.get("text").and_then(|t| serde_json::from_value(t.clone()).ok()),
                            transform: Transform { x: f(&t, "x", 0.0), y: f(&t, "y", 0.0), scale_x: f(&t, "scale_x", 1.0), scale_y: f(&t, "scale_y", 1.0), rotation: f(&t, "rotation", 0.0) },
                            bounds: l.get("bounds").and_then(rect).unwrap_or_default(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        State {
            width: i(v, "width") as i32,
            height: i(v, "height") as i32,
            layers,
            active: v.get("active").and_then(Value::as_i64),
            can_undo: b(v, "can_undo"),
            can_redo: b(v, "can_redo"),
            undo_label: v.get("undo_label").and_then(Value::as_str).map(String::from),
            redo_label: v.get("redo_label").and_then(Value::as_str).map(String::from),
            has_selection: b(v, "has_selection"),
            selection_bounds: v.get("selection_bounds").and_then(rect),
            modified: b(v, "modified"),
            version: i(v, "version"),
        }
    }
}

/// A straight-alpha, 8-bit sRGB RGBA image.
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Image {
    pub fn has_transparency(&self) -> bool {
        self.data.chunks_exact(4).any(|p| p[3] < 255)
    }

    /// Composite onto an opaque colour (used for JPEG and the legacy DIB format).
    pub fn flattened(&self, bg: [u8; 3]) -> Image {
        let mut out = self.clone();
        for p in out.data.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a < 255 {
                for c in 0..3 {
                    p[c] = ((p[c] as u32 * a + bg[c] as u32 * (255 - a) + 127) / 255) as u8;
                }
                p[3] = 255;
            }
        }
        out
    }
}

pub struct Doc {
    pub eng: Engine,
    pub state: State,
}

impl Doc {
    pub fn new() -> Doc {
        let eng = Engine::new();
        let state = State::from_json(&eng.state());
        Doc { eng, state }
    }

    /// Execute a command; returns the reply object or the error message.
    pub fn run(&mut self, cmd: &str, args: Value) -> Result<Value, String> {
        let mut payload = match args {
            Value::Object(m) => Value::Object(m),
            _ => json!({}),
        };
        payload["cmd"] = Value::String(cmd.to_string());
        let reply = self.eng.command(&payload);
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(reply)
        } else {
            Err(reply.get("error").and_then(Value::as_str).unwrap_or("Unknown error").to_string())
        }
    }

    pub fn refresh_state(&mut self) -> bool {
        let s = State::from_json(&self.eng.state());
        if s != self.state {
            self.state = s;
            true
        } else {
            false
        }
    }

    pub fn import(&mut self, img: Image, name: &str, as_layer: bool) -> Result<i64, String> {
        let was_empty = self.state.is_empty();
        let id = self.eng.import_rgba(img.data, img.width, img.height, name, as_layer && !was_empty)?;
        Ok(id as i64)
    }

    pub fn export(&self, unmasked: bool) -> Option<Image> {
        let (data, width, height) = self.eng.export_rgba(unmasked);
        if width == 0 || data.is_empty() {
            None
        } else {
            Some(Image { width, height, data })
        }
    }

    pub fn layer_pixels(&self, id: i64) -> Option<Image> {
        self.eng.layer_pixels(id).map(|(data, width, height)| Image { width, height, data })
    }

    pub fn thumbnail(&self, id: i64, max: u32) -> Option<Image> {
        self.eng.layer_thumbnail(id, max).map(|(data, width, height)| Image { width, height, data })
    }

    pub fn set_layer_mask(&mut self, id: i64, alpha: Vec<u8>, w: u32, h: u32, label: &str) -> Result<(), String> {
        self.eng.set_layer_mask(id, alpha, w, h, label)
    }

    pub fn selection_outline(&self) -> Vec<f32> {
        self.eng.selection_outline()
    }

    pub fn resize(img: &Image, w: u32, h: u32) -> Option<Image> {
        let src = Rgba::from_vec(img.width, img.height, img.data.clone()).ok()?;
        let r = cutout_core::image::resize_rgba(&src, w, h);
        Some(Image { width: w, height: h, data: (*r.data).clone() })
    }
}

#[cfg(test)]
mod windows_mask_integration {
    use super::*;
    #[test]
    fn conventional_mask_multiplies_source_alpha_once_and_undo_restores_it() {
        let mut doc=Doc::new();
        let mut data=[255;4].repeat(16*16);
        for y in 4..12 { for x in 4..12 { data[(y*16+x)*4..(y*16+x)*4+4].copy_from_slice(&[0,0,0,128]); } }
        let id=doc.import(Image { width:16,height:16,data:data.clone() },"alpha",false).unwrap();
        let mask=crate::background::mask(&data,16,16).unwrap();
        doc.set_layer_mask(id,mask,16,16,"Remove Background").unwrap();
        let result=doc.export(false).unwrap();
        assert_eq!(result.data[3],0); assert_eq!(result.data[(8*16+8)*4+3],128);
        assert_eq!((result.width,result.height),(16,16));
        doc.run("undo",json!({})).unwrap(); assert_eq!(doc.export(false).unwrap().data,data);
        doc.run("redo",json!({})).unwrap(); assert_eq!(doc.export(false).unwrap().data,result.data);
    }
}
