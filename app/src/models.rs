//! AI model management: locating, verifying, loading (lazily, off the UI
//! thread), running and unloading the BiRefNet ONNX models.
//!
//! Search order for a model file:
//! 1. `<install dir>\Models` — shipped by the installer (read-only, covered by
//!    the installer's integrity check),
//! 2. `%LOCALAPPDATA%\CutOut\Models` — installed from a file or downloaded by the user.
//!
//! Every model is verified against its pinned SHA-256 before first use (the
//! verification result is cached by size + modification time).

use crate::engine::Image;
use crate::platform::paths;
use cutout_core::ai::{Accel, Model, PredictOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub file_name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub license: &'static str,
}

impl ModelSpec {
    pub fn size_label(&self) -> String {
        format!("{:.0} MB", self.bytes as f64 / 1_000_000.0)
    }
}

const LICENSE: &str = "MIT (ZhengPeng7/BiRefNet; ONNX export by onnx-community)";

pub const BIREFNET_GENERAL: ModelSpec = ModelSpec {
    id: "birefnet-general-fp32",
    title: "BiRefNet (Best Quality)",
    detail: "High-resolution dichotomous segmentation. Excellent on hair, fur, thin structures and mechanical parts.",
    file_name: "birefnet-general-fp32.onnx",
    url: "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model.onnx",
    sha256: "58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c",
    bytes: 972_666_916,
    license: LICENSE,
};

pub const BIREFNET_COMPACT: ModelSpec = ModelSpec {
    id: "birefnet-general-fp16",
    title: "BiRefNet (Best Quality, compact)",
    detail: "The same model stored at half precision: half the download, identical results in our tests, about 15% slower.",
    file_name: "birefnet-general-fp16.onnx",
    url: "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model_fp16.onnx",
    sha256: "3654c741eb80bd926ada8fed1713b506ccf8d30eb1f6487e87eb9f234f33df09",
    bytes: 489_666_272,
    license: LICENSE,
};

pub const BIREFNET_LITE: ModelSpec = ModelSpec {
    id: "birefnet-lite-fp32",
    title: "BiRefNet Lite (Faster)",
    detail: "Smaller variant. Noticeably weaker on enclosed holes and low-contrast edges; not faster on CPU in our tests.",
    file_name: "birefnet-lite-fp32.onnx",
    url: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/de15b22ba131738a16dff04aab8bdf8dc32e3ac1/onnx/model.onnx",
    sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
    bytes: 224_005_088,
    license: LICENSE,
};

pub const ALL: [ModelSpec; 3] = [BIREFNET_GENERAL, BIREFNET_COMPACT, BIREFNET_LITE];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalMode {
    Best,
    Fast,
}

pub fn spec_by_id(id: &str) -> Option<&'static ModelSpec> {
    ALL.iter().find(|s| s.id == id)
}

/// Where `spec` is installed, if it is (bundled copy first).
pub fn installed_path(spec: &ModelSpec) -> Option<PathBuf> {
    if let Some(d) = paths::bundled_models_dir() {
        let p = d.join(spec.file_name);
        if p.is_file() {
            return Some(p);
        }
    }
    let p = paths::models_dir().join(spec.file_name);
    p.is_file().then_some(p)
}

pub fn is_bundled(spec: &ModelSpec) -> bool {
    paths::bundled_models_dir().map(|d| d.join(spec.file_name).is_file()).unwrap_or(false)
}

/// The model "Best Quality" will use: the preferred one if installed, else the
/// best installed alternative of the same network.
pub fn best_spec(preferred: &str) -> Option<(&'static ModelSpec, PathBuf)> {
    let order = [spec_by_id(preferred), Some(&BIREFNET_GENERAL), Some(&BIREFNET_COMPACT), Some(&BIREFNET_LITE)];
    order.into_iter().flatten().find_map(|s| installed_path(s).map(|p| (s, p)))
}

pub fn has_best_model(preferred: &str) -> bool {
    best_spec(preferred).is_some()
}

/// "Fast" on Windows uses the Lite network when installed (there is no Apple Vision here).
pub fn fast_spec() -> Option<(&'static ModelSpec, PathBuf)> {
    installed_path(&BIREFNET_LITE).map(|p| (&BIREFNET_LITE, p)).or_else(|| best_spec(BIREFNET_COMPACT.id))
}

// ------------------------------------------------------------ verification ---

#[derive(Serialize, Deserialize, Default)]
struct VerifiedCache(Vec<VerifiedEntry>);

#[derive(Serialize, Deserialize, Clone)]
struct VerifiedEntry {
    path: String,
    size: u64,
    mtime_secs: u64,
    sha256: String,
}

fn cache_file() -> PathBuf {
    paths::data_dir().join("verified-models.json")
}

fn stamp(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    Some((m.len(), t))
}

pub fn sha256_file(path: &Path, progress: &mut dyn FnMut(f64) -> bool) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let total = f.metadata().map(|m| m.len()).unwrap_or(1).max(1);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    let mut done = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done += n as u64;
        if !progress(done as f64 / total as f64) {
            return Err("Cancelled".into());
        }
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Verify `path` against `spec` (cached by size + mtime so it costs a few
/// seconds only the first time a model file is used).
pub fn verify(spec: &ModelSpec, path: &Path) -> Result<(), String> {
    let (size, mtime) = stamp(path).ok_or("Model file not found.")?;
    if size != spec.bytes {
        return Err(format!("{} has an unexpected size; it is damaged or incomplete.", spec.file_name));
    }
    let mut cache: VerifiedCache = std::fs::read(cache_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let key = path.to_string_lossy().to_string();
    if cache.0.iter().any(|e| e.path == key && e.size == size && e.mtime_secs == mtime && e.sha256 == spec.sha256) {
        return Ok(());
    }
    let digest = sha256_file(path, &mut |_| true)?;
    if digest != spec.sha256 {
        return Err(format!("{} failed its integrity check (SHA-256 mismatch). Delete it and install it again.", spec.file_name));
    }
    cache.0.retain(|e| e.path != key);
    cache.0.push(VerifiedEntry { path: key, size, mtime_secs: mtime, sha256: digest });
    if let Some(d) = cache_file().parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec(&cache) {
        let _ = std::fs::write(cache_file(), b);
    }
    Ok(())
}

// --------------------------------------------------------------- install ---

/// Install a user-supplied ONNX file after verifying that it is one of the pinned models.
pub fn install_from_file(src: &Path, progress: &mut dyn FnMut(f64) -> bool) -> Result<&'static ModelSpec, String> {
    let size = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    let spec = ALL.iter().find(|s| s.bytes == size).ok_or("This isn't one of the CutOut BiRefNet ONNX models (unexpected file size).")?;
    let digest = sha256_file(src, progress)?;
    if digest != spec.sha256 {
        return Err("This file failed the SHA-256 check for the model it claims to be.".into());
    }
    let dir = paths::models_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(spec.file_name);
    let tmp = dest.with_extension("onnx.part");
    std::fs::copy(src, &tmp).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(spec)
}

pub fn delete(spec: &ModelSpec) -> Result<(), String> {
    let p = paths::models_dir().join(spec.file_name);
    if p.is_file() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Download a pinned model over HTTPS (only when the user asks). The result is
/// checked against the pinned size and SHA-256 before it is moved into place.
pub fn download(spec: &'static ModelSpec, progress: &mut dyn FnMut(f64) -> bool) -> Result<(), String> {
    if !spec.url.starts_with("https://") {
        return Err("Refusing a non-HTTPS download.".into());
    }
    let dir = paths::models_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(spec.file_name);
    let tmp = dest.with_extension("onnx.part");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(3600))).build().into();
    let resp = agent.get(spec.url).call().map_err(|e| format!("Download failed: {e}"))?;
    let mut reader = resp.into_body().into_with_config().limit(spec.bytes + (1 << 20)).reader();
    let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    let result = (|| -> Result<(), String> {
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if !progress((done as f64 / spec.bytes as f64).min(1.0)) {
                return Err("Cancelled".into());
            }
        }
        out.flush().map_err(|e| e.to_string())?;
        let digest: String = hasher.clone().finalize().iter().map(|b| format!("{b:02x}")).collect();
        if done != spec.bytes || digest != spec.sha256 {
            return Err("The downloaded file failed its size / SHA-256 check and was discarded.".into());
        }
        Ok(())
    })();
    drop(out);
    match result {
        Ok(()) => std::fs::rename(&tmp, &dest).map_err(|e| e.to_string()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// -------------------------------------------------------------- inference ---

struct Loaded {
    path: PathBuf,
    accel: Accel,
    model: Arc<Model>,
}

static LOADED: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();
static LAST_USED: AtomicU64 = AtomicU64::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();

fn loaded() -> &'static Mutex<Option<Loaded>> {
    LOADED.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Free the (multi-GB) inference session shortly after use.
fn schedule_unload() {
    LAST_USED.store(now_ms(), Ordering::SeqCst);
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(45));
        if now_ms().saturating_sub(LAST_USED.load(Ordering::SeqCst)) >= 44_000 {
            if let Ok(mut g) = loaded().lock() {
                *g = None;
            }
        }
    });
}

pub fn unload() {
    if let Ok(mut g) = loaded().lock() {
        *g = None;
    }
}

pub struct Removal {
    pub rx: Receiver<Result<(Vec<u8>, String), String>>,
    cancel: Arc<AtomicBool>,
    active: Arc<Mutex<Option<Arc<Model>>>>,
}

impl Removal {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(m) = self.active.lock().unwrap().as_ref() {
            m.cancel();
        }
    }
}

/// Start a background removal. The reply is `(alpha matte, backend label)`.
pub fn start_removal(pixels: Image, spec: ModelSpec, path: PathBuf, use_directml: bool) -> Removal {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let active: Arc<Mutex<Option<Arc<Model>>>> = Arc::new(Mutex::new(None));
    let (c2, a2) = (cancel.clone(), active.clone());
    std::thread::Builder::new()
        .name("cutout-inference".into())
        .stack_size(64 << 20)
        .spawn(move || {
            let r = (|| -> Result<(Vec<u8>, String), String> {
                verify(&spec, &path)?;
                if c2.load(Ordering::SeqCst) {
                    return Err("Cancelled".into());
                }
                let accel = if use_directml && cutout_core::ai::accel_available(Accel::DirectMl) { Accel::DirectMl } else { Accel::Cpu };
                let model = {
                    let mut g = loaded().lock().unwrap();
                    match g.as_ref() {
                        Some(l) if l.path == path && l.accel == accel => l.model.clone(),
                        _ => {
                            *g = None;
                            let m = Arc::new(Model::load(&path, accel, None)?);
                            *g = Some(Loaded { path: path.clone(), accel, model: m.clone() });
                            m
                        }
                    }
                };
                *a2.lock().unwrap() = Some(model.clone());
                if c2.load(Ordering::SeqCst) {
                    return Err("Cancelled".into());
                }
                let (alpha, _t) = model.predict(&pixels.data, pixels.width, pixels.height, PredictOptions { refine: true })?;
                let backend = if accel == Accel::DirectMl { "ONNX Runtime · DirectML".to_string() } else { "ONNX Runtime · CPU".to_string() };
                Ok((alpha, backend))
            })();
            *a2.lock().unwrap() = None;
            schedule_unload();
            let _ = tx.send(r);
        })
        .expect("spawn inference thread");
    Removal { rx, cancel, active }
}
