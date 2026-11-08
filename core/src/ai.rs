//! Local AI background removal with ONNX Runtime.
//!
//! Default model: **BiRefNet** (general, MIT licence) — a 1024×1024
//! dichotomous-segmentation network that produces a soft alpha matte. The
//! low-resolution prediction is upscaled to the source resolution and then
//! refined with a colour-guided filter restricted to the uncertain edge band,
//! so the final matte follows real image edges at full resolution.
//!
//! The model file is loaded lazily on first use, never at application
//! start-up. ONNX files are pure computation graphs – there is no code
//! execution comparable to pickle loading – and model hashes are verified by
//! the app before installation.

use crate::filters;
use crate::image::FPlane;
use ort::session::{builder::GraphOptimizationLevel, RunOptions, Session};
use ort::value::{Tensor, TensorElementType, ValueType};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accel {
    Cpu,
    /// CoreML execution provider (ML Program) with CPU fallback.
    CoreMl,
    /// CoreML execution provider, NeuralNetwork format on CPU+GPU (no ANE compile).
    CoreMlNeuralNetwork,
    /// DirectML execution provider (Windows, any DirectX 12 GPU) with CPU fallback.
    /// Requires the `directml` feature; without it this behaves like `Cpu`.
    DirectMl,
}

pub struct Model {
    session: Mutex<Session>,
    run_opts: Mutex<Option<Arc<RunOptions>>>,
    input_name: String,
    input_f16: bool,
    pub input_size: (usize, usize),
    pub load_ms: u128,
}

#[derive(Clone, Copy, Debug)]
pub struct PredictOptions {
    /// Guided-filter refinement of the upscaled matte.
    pub refine: bool,
}

impl Default for PredictOptions {
    fn default() -> Self {
        PredictOptions { refine: true }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Timing {
    pub preprocess_ms: u128,
    pub inference_ms: u128,
    pub postprocess_ms: u128,
}

/// Whether the requested accelerator exists in this build.
pub fn accel_available(accel: Accel) -> bool {
    match accel {
        Accel::Cpu => true,
        Accel::CoreMl | Accel::CoreMlNeuralNetwork => cfg!(target_os = "macos"),
        Accel::DirectMl => cfg!(feature = "directml"),
    }
}

fn env() -> Result<&'static ort::environment::Environment, String> {
    static ENV: std::sync::OnceLock<Result<ort::environment::Environment, String>> = std::sync::OnceLock::new();
    ENV.get_or_init(|| ort::init().with_name("cutout").with_telemetry(false).build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| e.clone())
}

impl Model {
    /// Load a model. If the CoreML provider cannot compile the graph, the
    /// CPU provider is used instead.
    pub fn load(path: &Path, accel: Accel, cache_dir: Option<&Path>) -> Result<Model, String> {
        match Self::load_with(path, accel, cache_dir) {
            Err(e) if accel != Accel::Cpu => Self::load_with(path, Accel::Cpu, None).map_err(|e2| format!("{e}; CPU fallback: {e2}")),
            r => r,
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn load_with(path: &Path, accel: Accel, cache_dir: Option<&Path>) -> Result<Model, String> {
        let t0 = Instant::now();
        if !path.is_file() {
            return Err(format!("Model file not found: {}", path.display()));
        }
        let env = env()?;
        let mut b = Session::builder(env).map_err(|e| e.to_string())?;
        let level = match std::env::var("CUTOUT_ORT_OPT").as_deref() {
            Ok("0") => GraphOptimizationLevel::Disable,
            Ok("1") => GraphOptimizationLevel::Level1,
            Ok("2") => GraphOptimizationLevel::Level2,
            _ => GraphOptimizationLevel::All,
        };
        b = b.with_optimization_level(level).map_err(|e| e.to_string())?;
        let gpu_ep = accel != Accel::Cpu && accel_available(accel);
        if !gpu_ep {
            // No arena: activations are returned to the system after each
            // run instead of being kept as a multi-GB high-water mark.
            let arena = std::env::var("CUTOUT_ORT_ARENA").is_ok();
            b = b.with_execution_providers([ort::ep::CPU::default().with_arena_allocator(arena).build()]).map_err(|e| e.to_string())?;
            if !arena {
                b = b.with_memory_pattern(false).map_err(|e| e.to_string())?;
            }
        }
        let threads = std::env::var("CUTOUT_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4));
        b = b.with_intra_threads(threads).map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        if matches!(accel, Accel::CoreMl | Accel::CoreMlNeuralNetwork) {
            let fmt = if accel == Accel::CoreMl { ort::ep::coreml::ModelFormat::MLProgram } else { ort::ep::coreml::ModelFormat::NeuralNetwork };
            let mut ep = ort::ep::CoreML::default()
                .with_model_format(fmt)
                .with_compute_units(if accel == Accel::CoreMl { ort::ep::coreml::ComputeUnits::All } else { ort::ep::coreml::ComputeUnits::CPUAndGPU })
                .with_static_input_shapes(true);
            if let Some(dir) = cache_dir {
                let _ = std::fs::create_dir_all(dir);
                ep = ep.with_model_cache_dir(dir.display().to_string());
            }
            b = b.with_execution_providers([ep.build()]).map_err(|e| e.to_string())?;
        }
        #[cfg(feature = "directml")]
        if accel == Accel::DirectMl {
            // DirectML requires sequential execution and no memory pattern.
            b = b.with_parallel_execution(false).map_err(|e| e.to_string())?.with_memory_pattern(false).map_err(|e| e.to_string())?;
            b = b.with_execution_providers([ort::ep::DirectML::default().build()]).map_err(|e| e.to_string())?;
        }
        let session = b.commit_from_file(path).map_err(|e| format!("Could not load model: {e}"))?;
        let input = session.inputs().first().ok_or("Model has no inputs")?;
        let input_name = input.name().to_string();
        let (input_f16, input_size) = match input.dtype() {
            ValueType::Tensor { ty, shape, .. } => {
                let f16 = *ty == TensorElementType::Float16;
                if !f16 && *ty != TensorElementType::Float32 {
                    return Err(format!("Unsupported model input type {ty:?}"));
                }
                let h = shape.get(2).copied().filter(|&v| v > 0).unwrap_or(1024) as usize;
                let w = shape.get(3).copied().filter(|&v| v > 0).unwrap_or(1024) as usize;
                (f16, (w, h))
            }
            _ => return Err("Unsupported model input".into()),
        };
        Ok(Model {
            session: Mutex::new(session),
            run_opts: Mutex::new(None),
            input_name,
            input_f16,
            input_size,
            load_ms: t0.elapsed().as_millis(),
        })
    }

    /// Request cancellation of an in-flight `predict` (thread-safe).
    pub fn cancel(&self) {
        if let Some(r) = self.run_opts.lock().unwrap().as_ref() {
            let _ = r.terminate();
        }
    }

    /// Predict a full-resolution alpha matte (0..255) for straight RGBA input.
    pub fn predict(&self, rgba: &[u8], w: u32, h: u32, opts: PredictOptions) -> Result<(Vec<u8>, Timing), String> {
        let mut timing = Timing::default();
        let t0 = Instant::now();
        let (iw, ih) = self.input_size;
        let plane = iw * ih;
        let input = preprocess(rgba, w, h, iw, ih)?;
        timing.preprocess_ms = t0.elapsed().as_millis();
        let t1 = Instant::now();
        let shape = [1usize, 3, ih, iw];
        let ropts = Arc::new(RunOptions::new().map_err(|e| e.to_string())?);
        *self.run_opts.lock().unwrap() = Some(ropts.clone());
        let raw: Vec<f32> = {
            let mut session = self.session.lock().unwrap();
            let outputs = if self.input_f16 {
                let data: Vec<half::f16> = input.iter().map(|&v| half::f16::from_f32(v)).collect();
                let t = Tensor::from_array((shape, data)).map_err(|e| e.to_string())?;
                session.run_with_options(ort::inputs![self.input_name.as_str() => t], &ropts)
            } else {
                let t = Tensor::from_array((shape, input)).map_err(|e| e.to_string())?;
                session.run_with_options(ort::inputs![self.input_name.as_str() => t], &ropts)
            };
            *self.run_opts.lock().unwrap() = None;
            let outputs = outputs.map_err(|e| {
                let s = e.to_string();
                if s.to_lowercase().contains("terminat") {
                    "Cancelled".to_string()
                } else {
                    format!("Inference failed: {s}")
                }
            })?;
            // the last output is the finest prediction for BiRefNet exports
            let out = &outputs[outputs.len() - 1];
            if let Ok((_, d)) = out.try_extract_tensor::<f32>() {
                d.to_vec()
            } else {
                let (_, d) = out.try_extract_tensor::<half::f16>().map_err(|e| e.to_string())?;
                d.iter().map(|v| v.to_f32()).collect()
            }
        };
        timing.inference_ms = t1.elapsed().as_millis();
        let t2 = Instant::now();
        if raw.len() < plane {
            return Err("Unexpected model output size".into());
        }
        let alpha = postprocess(&raw[raw.len() - plane..], iw, ih, rgba, w, h, opts.refine);
        timing.postprocess_ms = t2.elapsed().as_millis();
        Ok((alpha, timing))
    }
}

/// Network input for straight RGBA: transparency flattened onto neutral grey,
/// resized to `iw`×`ih` (triangle filter) and ImageNet-normalised, as a
/// planar 1×3×ih×iw float tensor. Shared by the ONNX path and the app's
/// Core ML path so both see exactly the same input.
pub fn preprocess(rgba: &[u8], w: u32, h: u32, iw: usize, ih: usize) -> Result<Vec<f32>, String> {
    if rgba.len() < w as usize * h as usize * 4 {
        return Err("bad dims".into());
    }
    // flatten any existing transparency onto neutral grey so garbage RGB
    // under transparent pixels doesn't confuse the network
    let mut flat = Vec::with_capacity(w as usize * h as usize * 3);
    for p in rgba.chunks_exact(4) {
        let a = p[3] as u32;
        for c in 0..3 {
            flat.push(((p[c] as u32 * a + 128 * (255 - a) + 127) / 255) as u8);
        }
    }
    let img = image::RgbImage::from_raw(w, h, flat).ok_or("bad dims")?;
    let small = image::imageops::resize(&img, iw as u32, ih as u32, image::imageops::FilterType::Triangle);
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let plane = iw * ih;
    let mut input = vec![0f32; 3 * plane];
    for (i, p) in small.pixels().enumerate() {
        for c in 0..3 {
            input[c * plane + i] = (p.0[c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    Ok(input)
}

/// Turn a low-resolution network output (logits or probabilities, `iw`×`ih`)
/// into a full-resolution 0..255 matte: bicubic upscale, then (optionally)
/// colour-guided refinement of the uncertain edge band.
pub fn postprocess(out: &[f32], iw: usize, ih: usize, rgba: &[u8], w: u32, h: u32, refine: bool) -> Vec<u8> {
    let needs_sigmoid = out.iter().any(|&v| !(-0.001..=1.001).contains(&v));
    let probs = FPlane {
        w: iw,
        h: ih,
        data: out.iter().map(|&v| if needs_sigmoid { 1.0 / (1.0 + (-v).exp()) } else { v }).collect(),
    };
    let alpha = upscale_bicubic(&probs, w as usize, h as usize);
    if refine && (w as usize > iw || h as usize > ih) {
        let radius = (w.max(h) as f32 / iw.max(ih) as f32 * 2.0).clamp(2.0, 12.0);
        filters::refine_alpha(rgba, &alpha, w as usize, h as usize, radius, 1e-4)
    } else {
        alpha
    }
}

/// Bicubic (Catmull-Rom) upscaling of a probability map.
pub fn upscale_bicubic(p: &FPlane, nw: usize, nh: usize) -> Vec<u8> {
    let img = image::ImageBuffer::<image::Luma<f32>, Vec<f32>>::from_raw(p.w as u32, p.h as u32, p.data.clone()).expect("dims");
    let up = image::imageops::resize(&img, nw as u32, nh as u32, image::imageops::FilterType::CatmullRom);
    up.into_raw().into_iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
}
