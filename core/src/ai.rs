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

    /// Load with exactly the requested accelerator and no silent fallback, so the
    /// caller can choose a different model for its fallback (GPU uses fp16, CPU fp32).
    pub fn load_strict(path: &Path, accel: Accel) -> Result<Model, String> {
        Self::load_with(path, accel, None)
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
            // Run the operators one by one instead of fusing the whole network into a single DirectML
            // graph: measured on an RTX 4070 (8 GB) this is faster (0.34 s instead of 0.52 s per
            // image) and peaks at 5.4 GB instead of 7.5 GB of video memory. CUTOUT_DML_FUSION=1 restores fusion.
            if std::env::var_os("CUTOUT_DML_FUSION").is_none() {
                b = b.with_config_entry("ep.dml.disable_graph_fusion", "1").map_err(|e| e.to_string())?;
            }
            // Adapter: by default the one Windows ranks as high-performance (the discrete GPU on a
            // hybrid laptop - its DXGI index differs from process to process, so an index is not
            // reliable). CUTOUT_DML_DEVICE forces a DXGI adapter index (tests, troubleshooting).
            let ep = match std::env::var("CUTOUT_DML_DEVICE").ok().and_then(|v| v.parse::<i32>().ok()) {
                Some(dev) => ort::ep::DirectML::default().with_device_id(dev),
                None => ort::ep::DirectML::default()
                    .with_device_filter(ort::ep::directml::DeviceFilter::Gpu)
                    .with_performance_preference(ort::ep::directml::PerformancePreference::HighPerformance),
            };
            // error_on_failure: without it a provider that cannot be created is skipped silently and
            // the fp16 graph would run on the CPU provider while the app reports "GPU".
            b = b.with_execution_providers([ep.build().error_on_failure()]).map_err(|e| e.to_string())?;
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

    /// Run the network once on a blank input and discard the result. The first run of a session
    /// is slower than later ones (GPU pipelines and buffers are created on demand), so doing it
    /// ahead of time makes the first real prediction as fast as the following ones.
    pub fn warm_up(&self) -> Result<(), String> {
        let (iw, ih) = self.input_size;
        let shape = [1usize, 3, ih, iw];
        let mut session = self.session.lock().unwrap();
        let r = if self.input_f16 {
            let t = Tensor::from_array((shape, vec![half::f16::ZERO; 3 * iw * ih])).map_err(|e| e.to_string())?;
            session.run(ort::inputs![self.input_name.as_str() => t]).map(|_| ())
        } else {
            let t = Tensor::from_array((shape, vec![0f32; 3 * iw * ih])).map_err(|e| e.to_string())?;
            session.run(ort::inputs![self.input_name.as_str() => t]).map(|_| ())
        };
        r.map_err(|e| format!("Inference failed: {e}"))
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
    use rayon::prelude::*;
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || iw == 0 || ih == 0 || rgba.len() < w * h * 4 {
        return Err("bad dims".into());
    }
    // Resize with a triangle filter, rows first, exactly like `image::imageops::resize` (the tests
    // compare the two), but in parallel and without a full-size intermediate copy of the image.
    // Existing transparency is flattened onto neutral grey on the fly, so garbage RGB under
    // transparent pixels doesn't confuse the network.
    let ytaps = resample_taps(h, ih, triangle, 1.0);
    let xtaps = resample_taps(w, iw, triangle, 1.0);
    let mut rows = vec![0f32; ih * w * 3];
    rows.par_chunks_mut(w * 3).zip(ytaps.par_iter()).for_each(|(out, (top, ws))| {
        for (k, wt) in ws.iter().enumerate() {
            let line = &rgba[(top + k) * w * 4..(top + k + 1) * w * 4];
            for (o, p) in out.chunks_exact_mut(3).zip(line.chunks_exact(4)) {
                let a = p[3] as u32;
                for ch in 0..3 {
                    let v = if a == 255 { p[ch] } else { ((p[ch] as u32 * a + 128 * (255 - a) + 127) / 255) as u8 };
                    o[ch] += v as f32 * wt;
                }
            }
        }
    });
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let plane = iw * ih;
    let mut input = vec![0f32; 3 * plane];
    let (r, rest) = input.split_at_mut(plane);
    let (g, bl) = rest.split_at_mut(plane);
    r.par_chunks_mut(iw).zip(g.par_chunks_mut(iw)).zip(bl.par_chunks_mut(iw)).zip(rows.par_chunks(w * 3)).for_each(|(((r, g), bl), line)| {
        for (x, (left, ws)) in xtaps.iter().enumerate() {
            let mut t = [0f32; 3];
            for (px, wt) in line[left * 3..(left + ws.len()) * 3].chunks_exact(3).zip(ws) {
                for ch in 0..3 {
                    t[ch] += px[ch] * wt;
                }
            }
            // the resized image is 8-bit, as before: round to the nearest level
            let q = |v: f32| v.clamp(0.0, 255.0).round() as u8 as f32 / 255.0;
            r[x] = (q(t[0]) - MEAN[0]) / STD[0];
            g[x] = (q(t[1]) - MEAN[1]) / STD[1];
            bl[x] = (q(t[2]) - MEAN[2]) / STD[2];
        }
    });
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

/// Catmull-Rom kernel (cubic B/C spline with B = 0, C = 0.5), support 2.
fn catmull_rom(x: f32) -> f32 {
    let a = x.abs();
    let k = if a < 1.0 {
        9.0 * a.powi(3) - 15.0 * a.powi(2) + 6.0
    } else if a < 2.0 {
        -3.0 * a.powi(3) + 15.0 * a.powi(2) - 24.0 * a + 12.0
    } else {
        0.0
    };
    k / 6.0
}

/// Triangle (bilinear) kernel, support 1.
fn triangle(x: f32) -> f32 {
    if x.abs() < 1.0 {
        1.0 - x.abs()
    } else {
        0.0
    }
}

/// Resampling taps for one axis: for every output index, the first source index and the
/// normalised weights (the sampling scheme of `image::imageops::resize`).
fn resample_taps(src: usize, dst: usize, kernel: fn(f32) -> f32, kernel_support: f32) -> Vec<(usize, Vec<f32>)> {
    let ratio = src as f32 / dst as f32;
    let sratio = ratio.max(1.0);
    let support = kernel_support * sratio;
    (0..dst)
        .map(|o| {
            let centre = (o as f32 + 0.5) * ratio;
            let left = ((centre - support).floor() as i64).clamp(0, src as i64 - 1);
            let right = ((centre + support).ceil() as i64).clamp(left + 1, src as i64);
            let centre = centre - 0.5;
            let mut ws: Vec<f32> = (left..right).map(|i| kernel((i as f32 - centre) / sratio)).collect();
            let sum: f32 = ws.iter().sum();
            ws.iter_mut().for_each(|w| *w /= sum);
            (left as usize, ws)
        })
        .collect()
}

/// Bicubic (Catmull-Rom) resampling of a probability map to `nw`×`nh`, as 0..255.
///
/// Separable and parallel over rows: rows first (into a `nh`×`p.w` float buffer), then each
/// output row is written directly as bytes. It computes what `image::imageops::resize` with
/// `FilterType::CatmullRom` computes (the tests compare the two) but uses every core and never
/// holds a full-resolution float image: 0.16 s instead of 1.4 s for a 36 MP matte on 8 cores.
pub fn upscale_bicubic(p: &FPlane, nw: usize, nh: usize) -> Vec<u8> {
    use rayon::prelude::*;
    if p.w == 0 || p.h == 0 || nw == 0 || nh == 0 {
        return vec![0; nw * nh];
    }
    let (sw, src) = (p.w, &p.data);
    let ytaps = resample_taps(p.h, nh, catmull_rom, 2.0);
    let xtaps = resample_taps(p.w, nw, catmull_rom, 2.0);
    let mut rows = vec![0f32; nh * sw];
    rows.par_chunks_mut(sw).zip(ytaps.par_iter()).for_each(|(out, (top, ws))| {
        for (k, w) in ws.iter().enumerate() {
            let line = &src[(top + k) * sw..(top + k + 1) * sw];
            for (o, s) in out.iter_mut().zip(line) {
                *o += s * w;
            }
        }
    });
    let mut out = vec![0u8; nw * nh];
    out.par_chunks_mut(nw).zip(rows.par_chunks(sw)).for_each(|(orow, line)| {
        for (o, (left, ws)) in orow.iter_mut().zip(xtaps.iter()) {
            let mut t = 0f32;
            for (s, w) in line[*left..*left + ws.len()].iter().zip(ws) {
                t += s * w;
            }
            *o = (t.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The previous implementation (single-threaded, through the `image` crate).
    fn upscale_reference(p: &FPlane, nw: usize, nh: usize) -> Vec<u8> {
        let img = image::ImageBuffer::<image::Luma<f32>, Vec<f32>>::from_raw(p.w as u32, p.h as u32, p.data.clone()).expect("dims");
        let up = image::imageops::resize(&img, nw as u32, nh as u32, image::imageops::FilterType::CatmullRom);
        up.into_raw().into_iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
    }

    fn test_plane(w: usize, h: usize) -> FPlane {
        // hard edges, soft ramps and noise: everything a matte contains
        let mut s = 0x2545F491u32;
        let data = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let noise = (s & 0xffff) as f32 / 65535.0;
                let disc = if (x - w as f32 * 0.4).hypot(y - h as f32 * 0.5) < w as f32 * 0.3 { 1.0 } else { 0.0 };
                let ramp = (x / w as f32).clamp(0.0, 1.0);
                match (i / w) * 3 / h {
                    0 => disc,
                    1 => ramp * disc + 0.02 * noise,
                    _ => noise,
                }
            })
            .collect();
        FPlane { w, h, data }
    }

    #[test]
    fn upscale_matches_the_image_crate() {
        for &(sw, sh, nw, nh) in &[(64usize, 64usize, 300usize, 211usize), (96, 64, 96, 64), (64, 96, 1000, 37), (128, 128, 90, 70), (32, 32, 33, 31)] {
            let p = test_plane(sw, sh);
            let (a, b) = (upscale_bicubic(&p, nw, nh), upscale_reference(&p, nw, nh));
            assert_eq!(a.len(), b.len());
            let max = a.iter().zip(&b).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();
            let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
            // Same taps and weights; only the float summation order across the two passes may
            // differ, which can move a value across a rounding boundary by one level.
            assert!(max <= 1, "{sw}x{sh}->{nw}x{nh}: max diff {max}");
            assert!(differing * 200 <= a.len(), "{sw}x{sh}->{nw}x{nh}: {differing} of {} values differ", a.len());
        }
    }

    /// The previous implementation (single-threaded, through the `image` crate).
    fn preprocess_reference(rgba: &[u8], w: u32, h: u32, iw: usize, ih: usize) -> Vec<f32> {
        let mut flat = Vec::with_capacity(w as usize * h as usize * 3);
        for p in rgba.chunks_exact(4) {
            let a = p[3] as u32;
            for c in 0..3 {
                flat.push(((p[c] as u32 * a + 128 * (255 - a) + 127) / 255) as u8);
            }
        }
        let img = image::RgbImage::from_raw(w, h, flat).unwrap();
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
        input
    }

    #[test]
    fn preprocess_matches_the_image_crate() {
        let mut s = 0x9E3779B9u32;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        for &(w, h, iw, ih) in &[(640u32, 480u32, 128usize, 128usize), (97, 211, 64, 64), (64, 64, 128, 128), (300, 100, 64, 96), (128, 128, 128, 128)] {
            let rgba: Vec<u8> = (0..w * h * 4).map(|i| if i % 4 == 3 && (i / 4) % 7 != 0 { 255 } else { (next() >> 8) as u8 }).collect();
            let (a, b) = (preprocess(&rgba, w, h, iw, ih).unwrap(), preprocess_reference(&rgba, w, h, iw, ih));
            assert_eq!(a.len(), b.len());
            // one 8-bit level of the resized image is 1/255/std = 0.0171..0.0175 in network units
            let off: Vec<f32> = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).filter(|d| *d > 1e-6).collect();
            assert!(off.iter().all(|d| *d < 0.018), "{w}x{h}->{iw}x{ih}: more than one level apart");
            assert!(off.len() * 500 <= a.len(), "{w}x{h}->{iw}x{ih}: {} of {} values differ", off.len(), a.len());
        }
    }

    #[test]
    fn solid_regions_stay_solid() {
        let p = FPlane { w: 40, h: 40, data: (0..1600).map(|i| if i % 40 < 20 { 0.0 } else { 1.0 }).collect() };
        let up = upscale_bicubic(&p, 400, 400);
        assert_eq!(up[200 * 400 + 5], 0);
        assert_eq!(up[200 * 400 + 395], 255);
    }
}
