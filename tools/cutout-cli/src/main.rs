//! cutout-cli — benchmark and batch tool for the CutOut engine.
//!
//! ```text
//! cutout-cli synth <out-dir>                         generate ground-truth scenes
//! cutout-cli bench <model.onnx> <cpu|coreml> <images-dir> <out-dir> [--no-refine]
//! cutout-cli remove <model.onnx> <input> <output.png> [cpu|coreml]
//! cutout-cli matte <cutout.png> <matte.png>          alpha channel → small greyscale PNG (golden mattes)
//! cutout-cli parity <images> <preds> <tag> <golden-dir> [max-mae] [min-iou]   compare against golden mattes (exit 1 on failure)
//! ```

use cutout_core::ai::{Accel, Model, PredictOptions};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(|s| s.as_str()) {
        Some("synth") => synth(Path::new(args.get(2).map(|s| s.as_str()).unwrap_or("bench/synth"))),
        Some("bench") if args.len() >= 6 => bench(&args[2], &args[3], Path::new(&args[4]), Path::new(&args[5]), !args.iter().any(|a| a == "--no-refine")),
        Some("score") if args.len() >= 5 => score(Path::new(&args[2]), Path::new(&args[3]), &args[4], args.get(5).map(|s| s.as_str())),
        Some("matte") if args.len() >= 4 => matte(Path::new(&args[2]), Path::new(&args[3])),
        Some("parity") if args.len() >= 6 => parity(Path::new(&args[2]), Path::new(&args[3]), &args[4], Path::new(&args[5]), args.get(6).and_then(|v| v.parse().ok()).unwrap_or(0.004), args.get(7).and_then(|v| v.parse().ok()).unwrap_or(0.998)),
        Some("perf") => perf(args.get(2).map(|s| s.as_str())),
        Some("remove") if args.len() >= 5 => remove(&args[2], &args[3], &args[4], args.get(5).map(|s| s.as_str()).unwrap_or("coreml")),
        _ => Err("usage: cutout-cli synth <dir> | bench <model> <cpu|coreml> <images> <out> [--no-refine] | remove <model> <in> <out> [accel] | score <images> <preds> <tag> [ref_tag]".into()),
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn accel(s: &str) -> Accel {
    if s == "cpu" {
        Accel::Cpu
    } else if s == "coreml-nn" {
        Accel::CoreMlNeuralNetwork
    } else {
        Accel::CoreMl
    }
}

fn load_rgba(p: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(p).map_err(|e| format!("{}: {e}", p.display()))?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Caches/CutOut/coreml")
}

fn remove(model: &str, input: &str, output: &str, acc: &str) -> Result<(), String> {
    let m = Model::load(Path::new(model), accel(acc), Some(&cache_dir()))?;
    let (rgba, w, h) = load_rgba(Path::new(input))?;
    let (alpha, t) = m.predict(&rgba, w, h, PredictOptions::default())?;
    let mut out = rgba.clone();
    for (p, a) in out.chunks_exact_mut(4).zip(alpha.iter()) {
        p[3] = ((p[3] as u32 * *a as u32) / 255) as u8;
    }
    image::save_buffer(output, &out, w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    println!("load {} ms, inference {} ms, post {} ms", m.load_ms, t.inference_ms, t.postprocess_ms);
    Ok(())
}

// ---------------------------------------------------------------- synthetic

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash(x: u32, y: u32, s: u32) -> f64 {
    let mut h = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263) ^ s.wrapping_mul(2147483647);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xffff) as f64 / 65535.0
}

/// Write a scene: image.png (composited) and gt.png (alpha).
fn scene(dir: &Path, name: &str, w: u32, h: u32, bg: impl Fn(f64, f64) -> [f64; 3], fg: impl Fn(f64, f64) -> ([f64; 3], f64)) -> Result<(), String> {
    let mut img = vec![0u8; (w * h * 4) as usize];
    let mut gt = vec![0u8; (w * h) as usize];
    const SS: u32 = 3;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0f64; 3];
            let mut aa = 0f64;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f64 + (sx as f64 + 0.5) / SS as f64;
                    let py = y as f64 + (sy as f64 + 0.5) / SS as f64;
                    let (fc, a) = fg(px, py);
                    let bc = bg(px, py);
                    for c in 0..3 {
                        acc[c] += fc[c] * a + bc[c] * (1.0 - a);
                    }
                    aa += a;
                }
            }
            let n = (SS * SS) as f64;
            let i = ((y * w + x) * 4) as usize;
            let noise = (hash(x, y, 7) - 0.5) * 3.0;
            for c in 0..3 {
                img[i + c] = (acc[c] / n + noise).round().clamp(0.0, 255.0) as u8;
            }
            img[i + 3] = 255;
            gt[(y * w + x) as usize] = (aa / n * 255.0).round() as u8;
        }
    }
    let d = dir.join(name);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    image::save_buffer(d.join("image.png"), &img, w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    image::save_buffer(d.join("gt.png"), &gt, w, h, image::ExtendedColorType::L8).map_err(|e| e.to_string())?;
    println!("wrote {}", d.display());
    Ok(())
}

fn synth(dir: &Path) -> Result<(), String> {
    let beige = |x: f64, y: f64| {
        let v = 1.0 - 0.06 * (x / 2000.0) - 0.04 * (y / 2000.0);
        [232.0 * v, 220.0 * v, 196.0 * v]
    };
    // 1. White machined gear with bolt holes on beige (white-on-light).
    scene(dir, "white_gear_on_beige", 1600, 1200, beige, |x, y| {
        let (cx, cy) = (800.0, 600.0);
        let (dx, dy) = (x - cx, y - cy);
        let r = (dx * dx + dy * dy).sqrt();
        let th = dy.atan2(dx);
        let tooth = 430.0 + 40.0 * (((th * 24.0).sin()).signum() * 0.5 + 0.5);
        let mut a = 1.0 - smoothstep(-0.5, 0.5, r - tooth);
        a *= smoothstep(-0.5, 0.5, r - 120.0); // centre bore
        for k in 0..6 {
            let t = k as f64 * std::f64::consts::TAU / 6.0;
            let (hx, hy) = (cx + 270.0 * t.cos(), cy + 270.0 * t.sin());
            let rh = ((x - hx).powi(2) + (y - hy).powi(2)).sqrt();
            a *= smoothstep(-0.5, 0.5, rh - 50.0);
        }
        let shade = 244.0 + 8.0 * (th * 3.0).cos();
        ([shade, shade, shade + 2.0], a)
    })?;
    // 2. Reflective metal bracket (gradient chrome) with slot on light grey.
    scene(dir, "chrome_bracket", 1600, 1000, |_, y| [214.0 + y * 0.01, 216.0 + y * 0.01, 220.0], |x, y| {
        let inside = (200.0..1400.0).contains(&x) && (250.0..750.0).contains(&y);
        let edge = smoothstep(-0.5, 0.5, (x - 200.0).min(1400.0 - x).min(y - 250.0).min(750.0 - y));
        let slot = {
            let (dx, dy) = (((x - 800.0).abs() - 300.0).max(0.0), y - 500.0);
            smoothstep(-0.5, 0.5, (dx * dx + dy * dy).sqrt() - 70.0)
        };
        let a = if inside || edge > 0.0 { edge * slot } else { 0.0 };
        let band = ((y - 250.0) / 500.0 * 9.0).sin();
        let v = 150.0 + 100.0 * band;
        ([v, v + 3.0, v + 8.0], a)
    })?;
    // 3. Fine dark strands (hair / wire) on light background.
    scene(dir, "fine_strands", 1400, 1400, |x, y| [200.0 + 20.0 * (x / 1400.0), 205.0, 210.0 - 15.0 * (y / 1400.0)], |x, y| {
        let mut a: f64 = 0.0;
        // solid head
        let (dx, dy) = (x - 700.0, y - 900.0);
        a = a.max(1.0 - smoothstep(-0.5, 0.5, (dx * dx + dy * dy).sqrt() - 380.0));
        // strands of width 1–3 px radiating upward
        for k in 0..60 {
            let t = -std::f64::consts::PI * (0.15 + 0.7 * k as f64 / 60.0);
            let wv = 0.8 + 1.6 * hash(k, 3, 1);
            let (ux, uy) = (t.cos(), t.sin());
            let (px, py) = (x - 700.0, y - 900.0);
            let along = px * ux + py * uy;
            if !(300.0..700.0 + 100.0 * hash(k, 1, 2)).contains(&along) {
                continue;
            }
            let wob = 12.0 * ((along / 60.0) + k as f64).sin();
            let perp = (-px * uy + py * ux - wob).abs();
            a = a.max(1.0 - smoothstep(wv / 2.0 - 0.5, wv / 2.0 + 0.5, perp));
        }
        ([40.0, 30.0, 24.0], a)
    })?;
    // 4. Semi-transparent tinted glass disc (alpha 0.45) over texture.
    scene(dir, "translucent_disc", 1200, 1200, |x, y| {
        let c = if ((x / 80.0).floor() + (y / 80.0).floor()) as i64 % 2 == 0 { 225.0 } else { 190.0 };
        [c, c - 5.0, c - 10.0]
    }, |x, y| {
        let r = ((x - 600.0).powi(2) + (y - 600.0).powi(2)).sqrt();
        let a = 1.0 - smoothstep(-0.5, 0.5, r - 380.0);
        let rim = 1.0 - smoothstep(-0.5, 0.5, (r - 370.0).abs() - 10.0);
        ([60.0, 120.0, 160.0], a * (0.45 + 0.55 * rim))
    })?;
    Ok(())
}

// ------------------------------------------------------------------- bench

fn metrics(pred: &[u8], gt: &[u8], w: usize, h: usize) -> (f64, f64, f64) {
    let mut mae = 0f64;
    let (mut inter, mut uni) = (0f64, 0f64);
    for (&p, &g) in pred.iter().zip(gt.iter()) {
        mae += (p as f64 - g as f64).abs() / 255.0;
        let (pb, gb) = (p >= 128, g >= 128);
        if pb && gb {
            inter += 1.0;
        }
        if pb || gb {
            uni += 1.0;
        }
    }
    // edge band MAE: pixels within 6 px of a GT transition
    let band = cutout_core::filters::morph_u8(gt, w, h, 6);
    let core = cutout_core::filters::morph_u8(gt, w, h, -6);
    let (mut e, mut n) = (0f64, 0f64);
    for i in 0..gt.len() {
        if band[i] > 4 && core[i] < 251 {
            e += (pred[i] as f64 - gt[i] as f64).abs() / 255.0;
            n += 1.0;
        }
    }
    (mae / gt.len() as f64, inter / uni.max(1.0), e / n.max(1.0))
}

fn bench(model: &str, acc: &str, images: &Path, out: &Path, refine: bool) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let t = Instant::now();
    let m = Model::load(Path::new(model), accel(acc), Some(&cache_dir()))?;
    let load = t.elapsed().as_millis();
    let mname = Path::new(model).file_stem().unwrap().to_string_lossy().to_string();
    println!("model {mname} accel {acc} load {load} ms input {:?}", m.input_size);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(images).map_err(|e| e.to_string())?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    let mut rows = Vec::new();
    let mut warm = false;
    for p in entries {
        let (img_path, gt_path) = if p.is_dir() { (p.join("image.png"), Some(p.join("gt.png"))) } else { (p.clone(), None) };
        if !img_path.is_file() {
            continue;
        }
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let (rgba, w, h) = load_rgba(&img_path)?;
        if !warm {
            // first CoreML run compiles; measure separately
            let t0 = Instant::now();
            let _ = m.predict(&rgba, w, h, PredictOptions { refine: false })?;
            println!("  warm-up {} ms", t0.elapsed().as_millis());
            warm = true;
        }
        let t0 = Instant::now();
        let (alpha, tm) = m.predict(&rgba, w, h, PredictOptions { refine })?;
        let total = t0.elapsed().as_millis();
        let mut cut = rgba.clone();
        for (px, a) in cut.chunks_exact_mut(4).zip(alpha.iter()) {
            px[3] = *a;
        }
        image::save_buffer(out.join(format!("{name}__{mname}_{acc}.png")), &cut, w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
        let mut row = serde_json::json!({"image": name, "w": w, "h": h, "total_ms": total, "inference_ms": tm.inference_ms, "post_ms": tm.postprocess_ms});
        if let Some(g) = gt_path.filter(|g| g.is_file()) {
            let gimg = image::open(&g).map_err(|e| e.to_string())?.to_luma8();
            let (mae, iou, edge) = metrics(&alpha, gimg.as_raw(), w as usize, h as usize);
            row["mae"] = serde_json::json!((mae * 10000.0).round() / 10000.0);
            row["iou"] = serde_json::json!((iou * 10000.0).round() / 10000.0);
            row["edge_mae"] = serde_json::json!((edge * 10000.0).round() / 10000.0);
        }
        println!("  {row}");
        rows.push(row);
    }
    let report = serde_json::json!({"model": mname, "accel": acc, "refine": refine, "load_ms": load, "results": rows});
    std::fs::write(out.join(format!("report_{mname}_{acc}{}.json", if refine { "" } else { "_noref" })), serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Save the alpha channel of an RGBA PNG as a greyscale PNG.
fn matte(input: &Path, output: &Path) -> Result<(), String> {
    let (a, w, h) = load_matte(input)?;
    image::save_buffer(output, &a, w, h, image::ExtendedColorType::L8).map_err(|e| e.to_string())
}

/// Compare `<preds>/<name>__<tag>.png` with the golden matte `<golden>/<name>.png` for every
/// scene directory in `images`. Fails when the mean absolute difference exceeds `max_mae`
/// (0..1) or the IoU of the binarised mattes falls below `min_iou`.
fn parity(images: &Path, preds: &Path, tag: &str, golden: &Path, max_mae: f64, min_iou: f64) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(images).map_err(|e| e.to_string())?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    let mut failed = Vec::new();
    for p in entries {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let (a, w, h) = load_matte(&preds.join(format!("{name}__{tag}.png")))?;
        let (g, gw, gh) = load_matte(&golden.join(format!("{name}.png")))?;
        if (w, h) != (gw, gh) {
            return Err(format!("{name}: size mismatch ({w}x{h} vs golden {gw}x{gh})"));
        }
        let mae = a.iter().zip(&g).map(|(x, y)| (*x as f64 - *y as f64).abs()).sum::<f64>() / (a.len() as f64 * 255.0);
        let (mut inter, mut uni) = (0f64, 0f64);
        for (x, y) in a.iter().zip(&g) {
            let (xa, ya) = (*x >= 128, *y >= 128);
            inter += (xa && ya) as u8 as f64;
            uni += (xa || ya) as u8 as f64;
        }
        let iou = inter / uni.max(1.0);
        let maxd = a.iter().zip(&g).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap_or(0);
        println!("{name}: mae {mae:.5} (max {max_mae}) iou {iou:.5} (min {min_iou}) max|diff| {maxd}");
        if mae > max_mae || iou < min_iou {
            failed.push(name);
        }
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("parity failed for: {}", failed.join(", "))) }
}

/// Load a matte: the alpha channel of an RGBA PNG, or the grey value of a
/// greyscale PNG.
fn load_matte(p: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let (w, h) = (img.width(), img.height());
    let a = if img.color().has_alpha() { img.to_rgba8().pixels().map(|px| px.0[3]).collect() } else { img.to_luma8().into_raw() };
    Ok((a, w, h))
}

/// Score `<preds>/<name>__<tag>.png` against ground truth (synthetic scenes)
/// and, with `ref_tag`, against another model's mattes (parity).
fn score(images: &Path, preds: &Path, tag: &str, ref_tag: Option<&str>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(images).map_err(|e| e.to_string())?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    let r4 = |v: f64| (v * 10000.0).round() / 10000.0;
    let mut rows = Vec::new();
    for p in entries {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let pred_path = preds.join(format!("{name}__{tag}.png"));
        if !pred_path.is_file() {
            continue;
        }
        let (pred, w, h) = load_matte(&pred_path)?;
        let mut row = serde_json::json!({"image": name});
        let gt = p.join("gt.png");
        if p.is_dir() && gt.is_file() {
            let g = image::open(&gt).map_err(|e| e.to_string())?.to_luma8();
            let (mae, iou, edge) = metrics(&pred, g.as_raw(), w as usize, h as usize);
            row["mae"] = serde_json::json!(r4(mae));
            row["iou"] = serde_json::json!(r4(iou));
            row["edge_mae"] = serde_json::json!(r4(edge));
            if let Some(rt) = ref_tag {
                if let Ok((r, rw, rh)) = load_matte(&preds.join(format!("{name}__{rt}.png"))) {
                    if (rw, rh) == (w, h) {
                        let (rmae, riou, redge) = metrics(&r, g.as_raw(), w as usize, h as usize);
                        row["ref_iou"] = serde_json::json!(r4(riou));
                        row["ref_edge_mae"] = serde_json::json!(r4(redge));
                        row["d_iou"] = serde_json::json!(r4(iou - riou));
                        row["d_edge_mae"] = serde_json::json!(r4(edge - redge));
                        row["d_mae"] = serde_json::json!(r4(mae - rmae));
                    }
                }
            }
        }
        if let Some(rt) = ref_tag {
            let (r, rw, rh) = load_matte(&preds.join(format!("{name}__{rt}.png")))?;
            if (rw, rh) != (w, h) {
                return Err(format!("{name}: size mismatch with {rt}"));
            }
            // agreement with the reference matte (as if it were ground truth)
            let (mae, iou, edge) = metrics(&pred, &r, w as usize, h as usize);
            let maxd = pred.iter().zip(&r).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap_or(0);
            row["vs_ref"] = serde_json::json!({"mae": r4(mae), "iou": r4(iou), "edge_mae": r4(edge), "max_abs": maxd});
        }
        println!("  {row}");
        rows.push(row);
    }
    let out = preds.join(format!("score_{tag}{}.json", ref_tag.map(|r| format!("_vs_{r}")).unwrap_or_default()));
    std::fs::write(&out, serde_json::to_string_pretty(&serde_json::json!({"tag": tag, "ref": ref_tag, "results": rows})).unwrap()).map_err(|e| e.to_string())?;
    println!("wrote {}", out.display());
    Ok(())
}

// -------------------------------------------------------------------- perf

fn perf(input: Option<&str>) -> Result<(), String> {
    use cutout_core::engine::Engine;
    use serde_json::json;
    let (rgba, w, h) = match input {
        Some(p) => load_rgba(Path::new(p))?,
        None => {
            // 4000×3000 synthetic photo-like gradient with noise
            let (w, h) = (4000u32, 3000u32);
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let n = (hash(x, y, 1) * 20.0) as u8;
                    v.extend_from_slice(&[(x * 255 / w) as u8 ^ n, (y * 255 / h) as u8, 128u8.wrapping_add(n), 255]);
                }
            }
            (v, w, h)
        }
    };
    println!("image {w}×{h} ({:.1} MP)", (w * h) as f64 / 1e6);
    let mut e = Engine::new();
    let t = |label: &str, f: &mut dyn FnMut()| {
        let t0 = Instant::now();
        f();
        println!("  {:<34} {:>8.1} ms", label, t0.elapsed().as_secs_f64() * 1000.0);
    };
    t("import", &mut || {
        e.import_rgba(rgba.clone(), w, h, "img", false).unwrap();
    });
    t("full composite (render)", &mut || {
        let _ = e.render();
    });
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    t("magic wand (ΔE2000 map + flood)", &mut || {
        e.command(&json!({"cmd":"magic_wand","x":cx,"y":cy,"tolerance":10}));
    });
    t("magic wand, new tolerance (cached)", &mut || {
        e.command(&json!({"cmd":"magic_wand","x":cx,"y":cy,"tolerance":20}));
    });
    t("magic erase", &mut || {
        e.command(&json!({"cmd":"magic_erase","x":10,"y":10,"tolerance":10}));
    });
    t("render after erase", &mut || {
        let _ = e.render();
    });
    e.command(&json!({"cmd":"deselect"}));
    t("brush begin (100 px)", &mut || {
        e.command(&json!({"cmd":"brush_begin","mode":"erase","size":100,"hardness":0.8,"opacity":1,"x":cx,"y":cy}));
    });
    let mut n = 0;
    t("brush: 60 move events + renders", &mut || {
        for i in 0..60 {
            e.command(&json!({"cmd":"brush_move","x":cx + i as f64 * 8.0,"y":cy}));
            let _ = e.render();
            n += 1;
        }
    });
    e.command(&json!({"cmd":"brush_end"}));
    t("quick select (drag segment)", &mut || {
        e.command(&json!({"cmd":"quick_select","points":[[cx,cy],[cx+20.0,cy]],"radius":15,"begin":true}));
    });
    t("refine edges r=8 + decontaminate", &mut || {
        e.command(&json!({"cmd":"refine_commit","radius":8,"decontaminate":true}));
    });
    t("undo", &mut || {
        e.command(&json!({"cmd":"undo"}));
    });
    t("export (flatten)", &mut || {
        let _ = e.export_rgba(false);
    });
    let tmp = std::env::temp_dir().join("cutout-perf.cutout");
    t("save project", &mut || {
        e.save_project(&tmp).unwrap();
    });
    t("open project", &mut || {
        e.open_project(&tmp).unwrap();
    });
    let _ = std::fs::remove_file(tmp);
    let _ = n;
    Ok(())
}
