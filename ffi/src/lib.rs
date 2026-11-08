//! C ABI for the CutOut engine. See `include/cutout.h` for the contract.
//!
//! Every entry point catches panics so that a bug in the engine can never
//! unwind across the FFI boundary into Swift; failures are reported as error
//! values instead.

#![allow(clippy::missing_safety_doc)]

use cutout_core::document::TextProps;
use cutout_core::engine::Engine;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

pub struct CutoutEngine(Engine);

#[repr(C)]
pub struct CutoutFrame {
    pub data: *const u8,
    pub width: u32,
    pub height: u32,
    pub dirty_x: i32,
    pub dirty_y: i32,
    pub dirty_w: i32,
    pub dirty_h: i32,
    pub generation: u64,
}

#[repr(C)]
pub struct CutoutBuffer {
    pub data: *mut u8,
    pub len: usize,
    pub width: u32,
    pub height: u32,
}

impl CutoutBuffer {
    fn empty() -> Self {
        CutoutBuffer { data: ptr::null_mut(), len: 0, width: 0, height: 0 }
    }
    fn from_vec(v: Vec<u8>, w: u32, h: u32) -> Self {
        let mut b = v.into_boxed_slice();
        let len = b.len();
        let data = b.as_mut_ptr();
        std::mem::forget(b);
        CutoutBuffer { data, len, width: w, height: h }
    }
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

unsafe fn cstr<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        ""
    } else {
        CStr::from_ptr(p).to_str().unwrap_or("")
    }
}

fn to_c(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "")).map(|c| c.into_raw()).unwrap_or(ptr::null_mut())
}

unsafe fn set_err(out: *mut *mut c_char, msg: String) {
    if !out.is_null() {
        *out = to_c(msg);
    }
}

#[no_mangle]
pub extern "C" fn cutout_engine_new() -> *mut CutoutEngine {
    guard(ptr::null_mut(), || Box::into_raw(Box::new(CutoutEngine(Engine::new()))))
}

#[no_mangle]
pub unsafe extern "C" fn cutout_engine_free(e: *mut CutoutEngine) {
    if !e.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(e))));
    }
}

#[no_mangle]
pub unsafe extern "C" fn cutout_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[no_mangle]
pub unsafe extern "C" fn cutout_buffer_free(b: CutoutBuffer) {
    if !b.data.is_null() {
        drop(Vec::from_raw_parts(b.data, b.len, b.len));
    }
}

/// Execute a JSON command; returns a JSON reply (free with cutout_string_free).
#[no_mangle]
pub unsafe extern "C" fn cutout_command(e: *mut CutoutEngine, json: *const c_char) -> *mut c_char {
    if e.is_null() {
        return to_c(r#"{"ok":false,"error":"no engine"}"#.into());
    }
    let eng = &mut (*e).0;
    let input = cstr(json).to_string();
    let reply = catch_unwind(AssertUnwindSafe(|| match serde_json::from_str::<serde_json::Value>(&input) {
        Ok(v) => eng.command(&v).to_string(),
        Err(err) => serde_json::json!({"ok": false, "error": format!("bad command JSON: {err}")}).to_string(),
    }))
    .unwrap_or_else(|_| r#"{"ok":false,"error":"internal error"}"#.into());
    to_c(reply)
}

/// Import straight-alpha RGBA. Returns the new layer id, or -1 (see err).
#[no_mangle]
pub unsafe extern "C" fn cutout_import_rgba(e: *mut CutoutEngine, rgba: *const u8, w: u32, h: u32, name: *const c_char, as_layer: bool, err: *mut *mut c_char) -> i64 {
    if e.is_null() || rgba.is_null() {
        return -1;
    }
    let len = w as usize * h as usize * 4;
    let res = guard(Err("internal error".to_string()), || {
        cutout_core::image::check_dimensions(w, h)?;
        let data = std::slice::from_raw_parts(rgba, len).to_vec();
        (*e).0.import_rgba(data, w, h, cstr(name), as_layer)
    });
    match res {
        Ok(id) => id,
        Err(m) => {
            set_err(err, m);
            -1
        }
    }
}

/// Composite pending changes and return the display buffer (premultiplied
/// RGBA, canvas size). The pointer is valid until the next engine call.
#[no_mangle]
pub unsafe extern "C" fn cutout_render(e: *mut CutoutEngine) -> CutoutFrame {
    let empty = CutoutFrame { data: ptr::null(), width: 0, height: 0, dirty_x: 0, dirty_y: 0, dirty_w: 0, dirty_h: 0, generation: 0 };
    if e.is_null() {
        return empty;
    }
    guard(empty, || {
        let f = (*e).0.render();
        CutoutFrame { data: f.data.as_ptr(), width: f.width, height: f.height, dirty_x: f.dirty.x, dirty_y: f.dirty.y, dirty_w: f.dirty.w, dirty_h: f.dirty.h, generation: f.generation }
    })
}

/// Flattened canvas, straight alpha. `unmasked` renders with masks disabled.
#[no_mangle]
pub unsafe extern "C" fn cutout_export_rgba(e: *mut CutoutEngine, unmasked: bool) -> CutoutBuffer {
    if e.is_null() {
        return CutoutBuffer::empty();
    }
    guard(CutoutBuffer::empty(), || {
        let (v, w, h) = (*e).0.export_rgba(unmasked);
        CutoutBuffer::from_vec(v, w, h)
    })
}

#[no_mangle]
pub unsafe extern "C" fn cutout_render_unmasked(e: *mut CutoutEngine) -> CutoutBuffer {
    if e.is_null() {
        return CutoutBuffer::empty();
    }
    guard(CutoutBuffer::empty(), || {
        let eng = &(*e).0;
        CutoutBuffer::from_vec(eng.render_unmasked_premultiplied(), eng.doc.width, eng.doc.height)
    })
}

#[no_mangle]
pub unsafe extern "C" fn cutout_layer_pixels(e: *mut CutoutEngine, id: i64) -> CutoutBuffer {
    if e.is_null() {
        return CutoutBuffer::empty();
    }
    guard(CutoutBuffer::empty(), || match (*e).0.layer_pixels(id) {
        Some((v, w, h)) => CutoutBuffer::from_vec(v, w, h),
        None => CutoutBuffer::empty(),
    })
}

#[no_mangle]
pub unsafe extern "C" fn cutout_layer_thumbnail(e: *mut CutoutEngine, id: i64, max: u32) -> CutoutBuffer {
    if e.is_null() {
        return CutoutBuffer::empty();
    }
    guard(CutoutBuffer::empty(), || match (*e).0.layer_thumbnail(id, max.max(8)) {
        Some((v, w, h)) => CutoutBuffer::from_vec(v, w, h),
        None => CutoutBuffer::empty(),
    })
}

#[no_mangle]
pub unsafe extern "C" fn cutout_set_layer_mask(e: *mut CutoutEngine, id: i64, alpha: *const u8, w: u32, h: u32, label: *const c_char, err: *mut *mut c_char) -> bool {
    if e.is_null() || alpha.is_null() {
        return false;
    }
    let res = guard(Err("internal error".to_string()), || {
        let data = std::slice::from_raw_parts(alpha, w as usize * h as usize).to_vec();
        (*e).0.set_layer_mask(id, data, w, h, cstr(label))
    });
    match res {
        Ok(()) => true,
        Err(m) => {
            set_err(err, m);
            false
        }
    }
}

/// Selection outline as f32 quadruples (x0,y0,x1,y1); len is in floats.
#[no_mangle]
pub unsafe extern "C" fn cutout_selection_outline(e: *mut CutoutEngine) -> CutoutBuffer {
    if e.is_null() {
        return CutoutBuffer::empty();
    }
    guard(CutoutBuffer::empty(), || {
        let segs = (*e).0.selection_outline();
        let n = segs.len();
        let bytes: Vec<u8> = segs.iter().flat_map(|f| f.to_ne_bytes()).collect();
        let mut b = CutoutBuffer::from_vec(bytes, 0, 0);
        b.width = n as u32;
        b
    })
}

#[no_mangle]
pub unsafe extern "C" fn cutout_text_layer(e: *mut CutoutEngine, update_id: i64, props_json: *const c_char, rgba: *const u8, w: u32, h: u32, x: f64, y: f64, err: *mut *mut c_char) -> i64 {
    if e.is_null() || rgba.is_null() {
        return -1;
    }
    let res = guard(Err("internal error".to_string()), || {
        let props: TextProps = serde_json::from_str(cstr(props_json)).map_err(|e| e.to_string())?;
        cutout_core::image::check_dimensions(w, h)?;
        let data = std::slice::from_raw_parts(rgba, w as usize * h as usize * 4).to_vec();
        if update_id > 0 {
            (*e).0.update_text_layer(update_id, props, data, w, h).map(|_| update_id)
        } else {
            (*e).0.add_text_layer(props, data, w, h, x, y)
        }
    });
    match res {
        Ok(id) => id,
        Err(m) => {
            set_err(err, m);
            -1
        }
    }
}

// ----------------------------------------------------------------------- AI

pub struct CutoutModel(cutout_core::ai::Model);

/// Load a background-removal model. `accel`: 0 = CPU, 1 = CoreML.
#[no_mangle]
pub unsafe extern "C" fn cutout_model_load(path: *const c_char, accel: i32, cache_dir: *const c_char, err: *mut *mut c_char) -> *mut CutoutModel {
    let p = cstr(path).to_string();
    let c = cstr(cache_dir).to_string();
    let res = guard(Err("internal error loading model".to_string()), || {
        let a = if accel == 1 { cutout_core::ai::Accel::CoreMl } else { cutout_core::ai::Accel::Cpu };
        let cache = if c.is_empty() { None } else { Some(std::path::PathBuf::from(&c)) };
        cutout_core::ai::Model::load(std::path::Path::new(&p), a, cache.as_deref())
    });
    match res {
        Ok(m) => Box::into_raw(Box::new(CutoutModel(m))),
        Err(m) => {
            set_err(err, m);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn cutout_model_free(m: *mut CutoutModel) {
    if !m.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(m))));
    }
}

/// Thread-safe: may be called from any thread while predict runs.
#[no_mangle]
pub unsafe extern "C" fn cutout_model_cancel(m: *const CutoutModel) {
    if !m.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| (*m).0.cancel()));
    }
}

/// Predict an alpha matte (w*h bytes). Safe to call off the main thread.
#[no_mangle]
pub unsafe extern "C" fn cutout_model_predict(m: *const CutoutModel, rgba: *const u8, w: u32, h: u32, refine: bool, err: *mut *mut c_char) -> CutoutBuffer {
    if m.is_null() || rgba.is_null() {
        return CutoutBuffer::empty();
    }
    let res = guard(Err("internal error during inference".to_string()), || {
        cutout_core::image::check_dimensions(w, h)?;
        let data = std::slice::from_raw_parts(rgba, w as usize * h as usize * 4);
        (*m).0.predict(data, w, h, cutout_core::ai::PredictOptions { refine })
    });
    match res {
        Ok((a, _)) => CutoutBuffer::from_vec(a, w, h),
        Err(msg) => {
            set_err(err, msg);
            CutoutBuffer::empty()
        }
    }
}

/// Refine an externally produced matte (e.g. Apple Vision) at full
/// resolution with the colour-guided filter. Returns a new w*h buffer.
#[no_mangle]
pub unsafe extern "C" fn cutout_refine_alpha(rgba: *const u8, alpha: *const u8, w: u32, h: u32, radius: f32) -> CutoutBuffer {
    if rgba.is_null() || alpha.is_null() {
        return CutoutBuffer::empty();
    }
    guard(CutoutBuffer::empty(), || {
        let n = w as usize * h as usize;
        let px = std::slice::from_raw_parts(rgba, n * 4);
        let a = std::slice::from_raw_parts(alpha, n);
        CutoutBuffer::from_vec(cutout_core::filters::refine_alpha(px, a, w as usize, h as usize, radius, 1e-4), w, h)
    })
}

/// Fill `out` (3*iw*ih floats, planar NCHW) with the network input for a
/// straight-RGBA image — the exact preprocessing of the ONNX path. Used by
/// the app's Core ML path. Returns false on bad arguments.
#[no_mangle]
pub unsafe extern "C" fn cutout_ai_preprocess(rgba: *const u8, w: u32, h: u32, iw: u32, ih: u32, out: *mut f32) -> bool {
    if rgba.is_null() || out.is_null() || iw == 0 || ih == 0 {
        return false;
    }
    guard(false, || {
        if cutout_core::image::check_dimensions(w, h).is_err() {
            return false;
        }
        let px = std::slice::from_raw_parts(rgba, w as usize * h as usize * 4);
        match cutout_core::ai::preprocess(px, w, h, iw as usize, ih as usize) {
            Ok(v) => {
                std::ptr::copy_nonoverlapping(v.as_ptr(), out, v.len());
                true
            }
            Err(_) => false,
        }
    })
}

/// Turn a network output map (`iw`×`ih` logits or probabilities) into a
/// full-resolution w*h matte: bicubic upscale + optional guided refinement,
/// identical to the ONNX path.
#[no_mangle]
pub unsafe extern "C" fn cutout_ai_postprocess(probs: *const f32, iw: u32, ih: u32, rgba: *const u8, w: u32, h: u32, refine: bool) -> CutoutBuffer {
    if probs.is_null() || rgba.is_null() || iw == 0 || ih == 0 {
        return CutoutBuffer::empty();
    }
    guard(CutoutBuffer::empty(), || {
        if cutout_core::image::check_dimensions(w, h).is_err() {
            return CutoutBuffer::empty();
        }
        let p = std::slice::from_raw_parts(probs, iw as usize * ih as usize);
        let px = std::slice::from_raw_parts(rgba, w as usize * h as usize * 4);
        CutoutBuffer::from_vec(cutout_core::ai::postprocess(p, iw as usize, ih as usize, px, w, h, refine), w, h)
    })
}
