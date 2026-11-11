//! AI model management: locating, verifying, loading (lazily, off the UI
//! thread), running and unloading the BiRefNet ONNX models.
//!
//! The default model is the three-file package produced by `tools/convert/export_onnx.py`
//! (one half-precision weights file shared by an fp16 graph for the GPU and an fp32 graph for the
//! CPU). The single-file onnx-community exports remain supported for people who install or
//! download them.
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

/// One pinned file of a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSpec {
    pub file_name: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    /// The graph the CPU runs (for single-file models: the whole model).
    pub file_name: &'static str,
    /// Pinned HTTPS download location; empty when the model only ships with the installer.
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub license: &'static str,
    /// Further files that must sit next to `file_name` (weights shared by the graphs).
    pub companions: &'static [FileSpec],
    /// A graph of the same network for the GPU (half precision), next to `file_name`.
    pub gpu_graph: Option<FileSpec>,
    /// Approximate peak host memory of a CPU removal, in bytes (for the low-memory warning).
    pub cpu_peak_bytes: u64,
}

impl ModelSpec {
    pub fn total_bytes(&self) -> u64 {
        self.bytes + self.companions.iter().map(|c| c.bytes).sum::<u64>() + self.gpu_graph.map_or(0, |g| g.bytes)
    }
    pub fn size_label(&self) -> String {
        format!("{:.0} MB", self.total_bytes() as f64 / 1_000_000.0)
    }
    pub fn downloadable(&self) -> bool {
        !self.url.is_empty()
    }
    fn main_file(&self) -> FileSpec {
        FileSpec { file_name: self.file_name, sha256: self.sha256, bytes: self.bytes }
    }
    /// Every file of the model.
    fn files(&self) -> Vec<FileSpec> {
        let mut v = vec![self.main_file()];
        v.extend_from_slice(self.companions);
        v.extend(self.gpu_graph);
        v
    }
}

const LICENSE: &str = "MIT (ZhengPeng7/BiRefNet; ONNX export by onnx-community)";

/// BiRefNet general re-exported by `tools/convert/export_onnx.py` (hashes in `tools/convert/OUTPUT-onnx.txt`).
/// Same weights as `BIREFNET_GENERAL`; the graph is ~3 500 nodes instead of 22 014, so it is several
/// times faster on a GPU, a third faster on the CPU and needs half the memory.
pub const BIREFNET_V2: ModelSpec = ModelSpec {
    id: "birefnet-general-v2",
    title: "BiRefNet (Best Quality)",
    detail: "High-resolution dichotomous segmentation. Excellent on hair, fur, thin structures and mechanical parts. Runs on the GPU when one is suitable, otherwise on the CPU.",
    file_name: "birefnet-general-v2-cpu.onnx",
    url: "",
    sha256: "813d040fd23b5d1449b453110d22aa99c56d7e8ba87748b9e7ece9a86fe86274",
    bytes: 82_962_350,
    license: "MIT (ZhengPeng7/BiRefNet; ONNX export by CutOut, tools/convert/export_onnx.py)",
    companions: &[FileSpec { file_name: "birefnet-general-v2.weights", sha256: "8fd62d7e2a1792a64ef6ce6d8365716ff95c4f84d8a42fb812f471c24937ef53", bytes: 462_621_184 }],
    gpu_graph: Some(FileSpec { file_name: "birefnet-general-v2-gpu.onnx", sha256: "3f755e07a36f20600f8fe6b776ddb25f7dc097caac3101ea48688d29d6f6e96e", bytes: 42_173_987 }),
    cpu_peak_bytes: 3_500_000_000,
};

pub const BIREFNET_GENERAL: ModelSpec = ModelSpec {
    id: "birefnet-general-fp32",
    title: "BiRefNet (original export)",
    detail: "The onnx-community export of the same network. Slower and needs about 7 GB of memory; kept as a downloadable alternative.",
    file_name: "birefnet-general-fp32.onnx",
    url: "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model.onnx",
    sha256: "58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c",
    bytes: 972_666_916,
    license: LICENSE,
    companions: &[],
    gpu_graph: None,
    cpu_peak_bytes: 7_500_000_000,
};

pub const BIREFNET_COMPACT: ModelSpec = ModelSpec {
    id: "birefnet-general-fp16",
    title: "BiRefNet (original export, compact)",
    detail: "The original export stored at half precision: half the download, about 15% slower on the CPU.",
    file_name: "birefnet-general-fp16.onnx",
    url: "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model_fp16.onnx",
    sha256: "3654c741eb80bd926ada8fed1713b506ccf8d30eb1f6487e87eb9f234f33df09",
    bytes: 489_666_272,
    license: LICENSE,
    companions: &[],
    gpu_graph: None,
    cpu_peak_bytes: 7_500_000_000,
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
    companions: &[],
    gpu_graph: None,
    cpu_peak_bytes: 2_500_000_000,
};

pub const ALL: [ModelSpec; 4] = [BIREFNET_V2, BIREFNET_GENERAL, BIREFNET_COMPACT, BIREFNET_LITE];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalMode {
    Best,
    Fast,
}

pub fn spec_by_id(id: &str) -> Option<&'static ModelSpec> {
    ALL.iter().find(|s| s.id == id)
}

/// True when the CPU graph and the companion files of `spec` are all in `dir`.
fn complete_in(spec: &ModelSpec, dir: &Path) -> bool {
    dir.join(spec.file_name).is_file() && spec.companions.iter().all(|c| dir.join(c.file_name).is_file())
}

/// Where `spec` is installed (the path of its CPU graph), if it is (bundled copy first).
pub fn installed_path(spec: &ModelSpec) -> Option<PathBuf> {
    if let Some(d) = paths::bundled_models_dir() {
        if complete_in(spec, &d) {
            return Some(d.join(spec.file_name));
        }
    }
    let d = paths::models_dir();
    complete_in(spec, &d).then(|| d.join(spec.file_name))
}

pub fn is_bundled(spec: &ModelSpec) -> bool {
    paths::bundled_models_dir().map(|d| complete_in(spec, &d)).unwrap_or(false)
}

/// The model "Best Quality" will use: the preferred one if installed, else the
/// best installed alternative of the same network.
pub fn best_spec(preferred: &str) -> Option<(&'static ModelSpec, PathBuf)> {
    // Settings written by earlier versions name the original export as the default; the re-export
    // replaces it, so that old default means "the best-quality model".
    let preferred = if preferred == BIREFNET_GENERAL.id && installed_path(&BIREFNET_V2).is_some() { BIREFNET_V2.id } else { preferred };
    let order = [spec_by_id(preferred), Some(&BIREFNET_V2), Some(&BIREFNET_GENERAL), Some(&BIREFNET_COMPACT), Some(&BIREFNET_LITE)];
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

/// Verify the graph at `graph` (the CPU or the GPU graph of `spec`) and the companion files next
/// to it against their pinned hashes (cached by size + mtime, so it costs time only the first
/// time a file is used).
pub fn verify(spec: &ModelSpec, graph: &Path) -> Result<(), String> {
    let name = graph.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let first = spec.files().into_iter().find(|f| f.file_name == name).ok_or("Model file not recognised.")?;
    let dir = graph.parent().ok_or("Model file not found.")?;
    let mut cache: VerifiedCache = std::fs::read(cache_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut changed = false;
    for f in std::iter::once(first).chain(spec.companions.iter().copied()) {
        let path = dir.join(f.file_name);
        let (size, mtime) = stamp(&path).ok_or_else(|| format!("{} is missing.", f.file_name))?;
        if size != f.bytes {
            return Err(format!("{} has an unexpected size; it is damaged or incomplete.", f.file_name));
        }
        let key = path.to_string_lossy().to_string();
        if cache.0.iter().any(|e| e.path == key && e.size == size && e.mtime_secs == mtime && e.sha256 == f.sha256) {
            continue;
        }
        let digest = sha256_file(&path, &mut |_| true)?;
        if digest != f.sha256 {
            return Err(format!("{} failed its integrity check (SHA-256 mismatch). Delete it and install it again.", f.file_name));
        }
        cache.0.retain(|e| e.path != key);
        cache.0.push(VerifiedEntry { path: key, size, mtime_secs: mtime, sha256: digest });
        changed = true;
    }
    if changed {
        if let Some(d) = cache_file().parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(b) = serde_json::to_vec(&cache) {
            let _ = std::fs::write(cache_file(), b);
        }
    }
    Ok(())
}

// --------------------------------------------------------------- install ---

/// Install a user-supplied ONNX file after verifying that it is one of the pinned models. For a
/// model made of several files, choose any one of them: the others must be in the same folder,
/// and every file is verified before anything is copied.
pub fn install_from_file(src: &Path, progress: &mut dyn FnMut(f64) -> bool) -> Result<&'static ModelSpec, String> {
    let size = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    let spec = ALL.iter().find(|s| s.files().iter().any(|f| f.bytes == size)).ok_or("This isn't one of the CutOut BiRefNet ONNX models (unexpected file size).")?;
    let src_dir = src.parent().ok_or("Model file not found.")?;
    let files = spec.files();
    let total = spec.total_bytes().max(1) as f64;
    let mut done = 0u64;
    for f in &files {
        let p = src_dir.join(f.file_name);
        let len = std::fs::metadata(&p).map_err(|_| format!("{} must be in the same folder.", f.file_name))?.len();
        if len != f.bytes {
            return Err(format!("{} has an unexpected size.", f.file_name));
        }
        let digest = sha256_file(&p, &mut |frac| progress((done as f64 + frac * f.bytes as f64) / total))?;
        if digest != f.sha256 {
            return Err(format!("{} failed the SHA-256 check for the model it claims to be.", f.file_name));
        }
        done += f.bytes;
    }
    let dir = paths::models_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Companions first, the CPU graph (whose presence marks the model as installed) last.
    for f in files.iter().rev() {
        let dest = dir.join(f.file_name);
        let tmp = dir.join(format!("{}.part", f.file_name));
        std::fs::copy(src_dir.join(f.file_name), &tmp).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    }
    Ok(spec)
}

pub fn delete(spec: &ModelSpec) -> Result<(), String> {
    for f in spec.files() {
        let p = paths::models_dir().join(f.file_name);
        if p.is_file() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Download a pinned model over HTTPS (only when the user asks). The result is
/// checked against the pinned size and SHA-256 before it is moved into place.
pub fn download(spec: &'static ModelSpec, progress: &mut dyn FnMut(f64) -> bool) -> Result<(), String> {
    if !spec.downloadable() {
        return Err("This model ships with the installer and has no download.".into());
    }
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

/// How long an unused inference session stays loaded. Loading costs about 2 s, so someone working
/// through a few images should not pay it each time; an idle session holds video memory (GPU) or
/// about 1.5 GB of RAM (CPU), so it is not kept indefinitely either.
const KEEP_LOADED: Duration = Duration::from_secs(180);

/// Free the inference session some time after its last use.
fn schedule_unload() {
    LAST_USED.store(now_ms(), Ordering::SeqCst);
    std::thread::spawn(|| {
        std::thread::sleep(KEEP_LOADED);
        if now_ms().saturating_sub(LAST_USED.load(Ordering::SeqCst)) >= KEEP_LOADED.as_millis() as u64 - 1000 {
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

/// Set once the GPU path has failed in this session, so later removals go straight to the CPU.
static GPU_FAILED: AtomicBool = AtomicBool::new(false);

/// The model graph to run on the GPU for Best Quality, when GPU removal is enabled, this build has
/// DirectML, Windows provides DirectML.dll, the PC has a suitable adapter (see `platform::gpu`) and
/// a half-precision graph of the chosen model is installed. `preferred` is the Best Quality setting.
pub fn gpu_candidate(use_directml: bool, preferred: &str) -> Option<(ModelSpec, PathBuf)> {
    if !use_directml || GPU_FAILED.load(Ordering::SeqCst) || !cutout_core::ai::accel_available(Accel::DirectMl) {
        return None;
    }
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    if !sys.join("System32").join("DirectML.dll").is_file() || !crate::platform::gpu::suitable_for_model() {
        return None;
    }
    let (spec, path) = best_spec(preferred)?;
    if let Some(g) = spec.gpu_graph {
        let p = path.with_file_name(g.file_name);
        return p.is_file().then_some((*spec, p));
    }
    if spec.id == BIREFNET_LITE.id {
        return None;
    }
    // The original export: its fp32 file needs more than 8 GB of video memory, the fp16 file fits.
    installed_path(&BIREFNET_COMPACT).map(|p| (BIREFNET_COMPACT, p))
}

/// Host memory a GPU removal with `spec` needs to be free, in bytes.
pub fn gpu_host_bytes(spec: &ModelSpec) -> u64 {
    if spec.id == BIREFNET_V2.id {
        2_000_000_000
    } else {
        6_000_000_000
    }
}

/// The session for `graph` on `accel`: the loaded one when it matches, otherwise a fresh load.
/// Returns the model and whether it had to be loaded.
fn session(graph: &Path, accel: Accel) -> Result<(Arc<Model>, bool), String> {
    let mut g = loaded().lock().unwrap();
    match g.as_ref() {
        Some(l) if l.path == graph && l.accel == accel => Ok((l.model.clone(), false)),
        _ => {
            *g = None;
            let m = Arc::new(if accel == Accel::Cpu { Model::load(graph, accel, None)? } else { Model::load_strict(graph, accel)? });
            *g = Some(Loaded { path: graph.to_path_buf(), accel, model: m.clone() });
            Ok((m, true))
        }
    }
}

/// Load the GPU session in the background (after an image has been opened), so the first
/// Remove Background does not wait for it. Does nothing when a session is already loaded. A failure
/// here is the same signal as a failed GPU removal: the session uses the CPU from then on.
pub fn prewarm(spec: ModelSpec, graph: PathBuf) {
    if loaded().lock().map(|g| g.is_some()).unwrap_or(true) {
        return;
    }
    let _ = std::thread::Builder::new().name("cutout-prewarm".into()).stack_size(64 << 20).spawn(move || {
        let t = Instant::now();
        // The warm-up run happens outside the session-table lock, so a removal that starts in the
        // meantime only waits for the network itself (the model serialises its own runs).
        let r = verify(&spec, &graph).and_then(|_| session(&graph, Accel::DirectMl)).and_then(|(m, cold)| if cold { m.warm_up().map(|_| (m, cold)) } else { Ok((m, cold)) });
        match r {
            Ok((_, cold)) => {
                if cold && std::env::var_os("CUTOUT_TRACE_TIMING").is_some() {
                    eprintln!("CutOut timing [prewarm]: GPU session ready in {} ms", t.elapsed().as_millis());
                }
                schedule_unload();
            }
            Err(e) => {
                eprintln!("CutOut: GPU session could not be prepared, using CPU: {e}");
                GPU_FAILED.store(true, Ordering::SeqCst);
                unload();
            }
        }
    });
}

fn run_removal(
    pixels: &Image,
    spec: &ModelSpec,
    path: &Path,
    accel: Accel,
    cancel: &AtomicBool,
    active: &Mutex<Option<Arc<Model>>>,
) -> Result<(Vec<u8>, String), String> {
    let t_start = std::time::Instant::now();
    verify(spec, path)?;
    let t_verified = t_start.elapsed();
    if cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".into());
    }
    // Test hook: pretend the GPU provider failed so the CPU fallback can be exercised on any machine.
    if accel != Accel::Cpu && std::env::var_os("CUTOUT_TEST_FAIL_GPU").is_some() {
        return Err("simulated GPU failure (CUTOUT_TEST_FAIL_GPU)".into());
    }
    let (model, cold_load) = session(path, accel)?;
    *active.lock().unwrap() = Some(model.clone());
    if cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".into());
    }
    let t_loaded = t_start.elapsed();
    let (alpha, t) = model.predict(&pixels.data, pixels.width, pixels.height, PredictOptions { refine: true })?;
    if std::env::var_os("CUTOUT_TRACE_TIMING").is_some() {
        // Local diagnostics only (stderr); nothing is sent anywhere.
        eprintln!(
            "CutOut timing [{:?}{}]: verify {} ms, model load {} ms, preprocess {} ms, inference {} ms, postprocess {} ms, total {} ms",
            accel,
            if cold_load { ", cold" } else { ", warm" },
            t_verified.as_millis(),
            (t_loaded - t_verified).as_millis(),
            t.preprocess_ms,
            t.inference_ms,
            t.postprocess_ms,
            t_start.elapsed().as_millis()
        );
    }
    let backend = if accel == Accel::DirectMl { "ONNX Runtime · DirectML (GPU)".to_string() } else { "ONNX Runtime · CPU".to_string() };
    Ok((alpha, backend))
}

/// Start a background removal. The reply is `(alpha matte, backend label)`. `spec`/`path` is the
/// CPU graph; `gpu`, when given, is tried first and any failure falls back to the CPU graph.
pub fn start_removal(pixels: Image, spec: ModelSpec, path: PathBuf, gpu: Option<(ModelSpec, PathBuf)>) -> Removal {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let active: Arc<Mutex<Option<Arc<Model>>>> = Arc::new(Mutex::new(None));
    let (c2, a2) = (cancel.clone(), active.clone());
    std::thread::Builder::new()
        .name("cutout-inference".into())
        .stack_size(64 << 20)
        .spawn(move || {
            let r = (|| -> Result<(Vec<u8>, String), String> {
                if let Some((gspec, gpath)) = &gpu {
                    match run_removal(&pixels, gspec, gpath, Accel::DirectMl, &c2, &a2) {
                        Ok(r) => return Ok(r),
                        Err(e) if e == "Cancelled" => return Err(e),
                        Err(e) => {
                            // Driver, memory or provider problem: remember it and use the CPU model.
                            eprintln!("CutOut: GPU removal failed, using CPU: {e}");
                            GPU_FAILED.store(true, Ordering::SeqCst);
                            unload();
                        }
                    }
                }
                run_removal(&pixels, &spec, &path, Accel::Cpu, &c2, &a2)
            })();
            *a2.lock().unwrap() = None;
            schedule_unload();
            let _ = tx.send(r);
        })
        .expect("spawn inference thread");
    Removal { rx, cancel, active }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_files_are_listed_once() {
        let f = BIREFNET_V2.files();
        assert_eq!(f.len(), 3);
        assert_eq!(BIREFNET_V2.total_bytes(), f.iter().map(|x| x.bytes).sum::<u64>());
        assert!(!BIREFNET_V2.downloadable() && BIREFNET_GENERAL.downloadable());
        // every pinned file has a distinct size: install-from-file recognises a model by size
        let mut sizes: Vec<u64> = ALL.iter().flat_map(|s| s.files()).map(|x| x.bytes).collect();
        sizes.sort();
        sizes.dedup();
        assert_eq!(sizes.len(), ALL.iter().map(|s| s.files().len()).sum::<usize>());
    }

    #[test]
    fn verify_rejects_a_damaged_companion() {
        let dir = std::env::temp_dir().join(format!("cutout-verify-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Wrong size is detected before any hashing.
        std::fs::write(dir.join(BIREFNET_V2.file_name), b"not a model").unwrap();
        let e = verify(&BIREFNET_V2, &dir.join(BIREFNET_V2.file_name)).unwrap_err();
        assert!(e.contains("unexpected size"), "{e}");
        let e = verify(&BIREFNET_V2, &dir.join("something-else.onnx")).unwrap_err();
        assert!(e.contains("not recognised"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
