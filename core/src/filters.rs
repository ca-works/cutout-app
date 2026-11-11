//! Mask and matte filters: blur, morphology, guided filtering and
//! foreground colour estimation (edge decontamination).

use crate::image::FPlane;
use rayon::prelude::*;

/// Separable box blur with radius `r` (window 2r+1), edge-clamped.
pub fn box_blur(p: &FPlane, r: usize) -> FPlane {
    if r == 0 {
        return p.clone();
    }
    let (w, h) = (p.w, p.h);
    let mut tmp = vec![0f32; w * h];
    // horizontal
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
        let row = &p.data[y * w..(y + 1) * w];
        box_1d(row, out, r, 1);
    });
    // vertical: running sum over rows, vectorised across x
    let mut out = vec![0f32; w * h];
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut acc = vec![0f32; w];
    let row = |yy: isize| -> &[f32] {
        let y = yy.clamp(0, h as isize - 1) as usize;
        &tmp[y * w..(y + 1) * w]
    };
    for k in -(r as isize)..=(r as isize) {
        for (a, v) in acc.iter_mut().zip(row(k)) {
            *a += v;
        }
    }
    for y in 0..h {
        let o = &mut out[y * w..(y + 1) * w];
        for (ov, a) in o.iter_mut().zip(acc.iter()) {
            *ov = a * norm;
        }
        let add = row(y as isize + r as isize + 1);
        let sub = row(y as isize - r as isize);
        for x in 0..w {
            acc[x] += add[x] - sub[x];
        }
    }
    FPlane { w, h, data: out }
}

fn box_1d(src: &[f32], dst: &mut [f32], r: usize, _step: usize) {
    let n = src.len();
    let norm = 1.0 / (2 * r + 1) as f32;
    let get = |i: isize| -> f32 { src[i.clamp(0, n as isize - 1) as usize] };
    let mut acc = 0f32;
    for k in -(r as isize)..=(r as isize) {
        acc += get(k);
    }
    for i in 0..n {
        dst[i] = acc * norm;
        acc += get(i as isize + r as isize + 1) - get(i as isize - r as isize);
    }
}

/// Approximate Gaussian blur (three box passes).
pub fn gaussian(p: &FPlane, sigma: f32) -> FPlane {
    if sigma < 0.3 {
        return p.clone();
    }
    // box radius for 3 passes ≈ sqrt(12σ²/3 + 1)/2
    let r = (((12.0 * sigma * sigma / 3.0) + 1.0).sqrt() / 2.0).round().max(1.0) as usize;
    box_blur(&box_blur(&box_blur(p, r), r), r)
}

pub fn gaussian_u8(data: &[u8], w: usize, h: usize, sigma: f32) -> Vec<u8> {
    gaussian(&FPlane::from_u8(data, w, h), sigma).to_u8()
}

/// Grey-scale dilation (radius > 0) or erosion (radius < 0) with a square
/// structuring element followed by a light blur to keep it round-ish.
pub fn morph_u8(data: &[u8], w: usize, h: usize, radius: i32) -> Vec<u8> {
    if radius == 0 {
        return data.to_vec();
    }
    let r = radius.unsigned_abs() as usize;
    let dilate = radius > 0;
    let pick = |a: u8, b: u8| if dilate { a.max(b) } else { a.min(b) };
    // Iterate radius-1 3x3 cross passes for a diamond, then square pass: use
    // van Herk/Gil-Werman style naive separable window (fast enough here).
    let mut tmp = vec![0u8; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
        let row = &data[y * w..(y + 1) * w];
        for x in 0..w {
            let a = x.saturating_sub(r);
            let b = (x + r).min(w - 1);
            let mut v = row[x];
            for &s in &row[a..=b] {
                v = pick(v, s);
            }
            out[x] = v;
        }
    });
    let mut out = vec![0u8; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, orow)| {
        let a = y.saturating_sub(r);
        let b = (y + r).min(h - 1);
        orow.copy_from_slice(&tmp[y * w..(y + 1) * w]);
        for yy in a..=b {
            let src = &tmp[yy * w..(yy + 1) * w];
            for x in 0..w {
                orow[x] = pick(orow[x], src[x]);
            }
        }
    });
    out
}

/// Grey guided filter (He et al.). `guide` and `src` are 0..1 planes.
pub fn guided_filter(guide: &FPlane, src: &FPlane, r: usize, eps: f32) -> FPlane {
    let (w, h) = (guide.w, guide.h);
    let n = w * h;
    let mean_i = box_blur(guide, r);
    let mean_p = box_blur(src, r);
    let ip = FPlane { w, h, data: (0..n).map(|i| guide.data[i] * src.data[i]).collect() };
    let ii = FPlane { w, h, data: (0..n).map(|i| guide.data[i] * guide.data[i]).collect() };
    let mean_ip = box_blur(&ip, r);
    let mean_ii = box_blur(&ii, r);
    let mut a = FPlane::new(w, h);
    let mut b = FPlane::new(w, h);
    for i in 0..n {
        let cov = mean_ip.data[i] - mean_i.data[i] * mean_p.data[i];
        let var = mean_ii.data[i] - mean_i.data[i] * mean_i.data[i];
        let av = cov / (var + eps);
        a.data[i] = av;
        b.data[i] = mean_p.data[i] - av * mean_i.data[i];
    }
    let ma = box_blur(&a, r);
    let mb = box_blur(&b, r);
    FPlane { w, h, data: (0..n).map(|i| ma.data[i] * guide.data[i] + mb.data[i]).collect() }
}

/// Colour guided filter (He et al. §3.5): guide is an RGB image (three 0..1
/// planes). Respects colour edges such as hair against similar-luma
/// backgrounds much better than the grey version.
pub fn guided_filter_color(g: &[FPlane; 3], src: &FPlane, r: usize, eps: f32) -> FPlane {
    let (w, h) = (src.w, src.h);
    let n = w * h;
    let mul = |a: &FPlane, b: &FPlane| FPlane { w, h, data: a.data.par_iter().zip(b.data.par_iter()).map(|(x, y)| x * y).collect() };
    let m: Vec<FPlane> = g.iter().map(|c| box_blur(c, r)).collect();
    let mp = box_blur(src, r);
    let mip: Vec<FPlane> = g.iter().map(|c| box_blur(&mul(c, src), r)).collect();
    let pairs = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];
    let var: Vec<FPlane> = pairs
        .iter()
        .map(|&(i, j)| {
            let mut v = box_blur(&mul(&g[i], &g[j]), r);
            for k in 0..n {
                v.data[k] -= m[i].data[k] * m[j].data[k];
            }
            v
        })
        .collect();
    let mut ab: Vec<[f32; 4]> = vec![[0.0; 4]; n];
    ab.par_iter_mut().enumerate().for_each(|(k, out)| {
        let cov = [
            mip[0].data[k] - m[0].data[k] * mp.data[k],
            mip[1].data[k] - m[1].data[k] * mp.data[k],
            mip[2].data[k] - m[2].data[k] * mp.data[k],
        ];
        let (s00, s01, s02, s11, s12, s22) = (
            var[0].data[k] + eps,
            var[1].data[k],
            var[2].data[k],
            var[3].data[k] + eps,
            var[4].data[k],
            var[5].data[k] + eps,
        );
        // inverse of symmetric 3x3
        let i00 = s11 * s22 - s12 * s12;
        let i01 = s02 * s12 - s01 * s22;
        let i02 = s01 * s12 - s02 * s11;
        let i11 = s00 * s22 - s02 * s02;
        let i12 = s01 * s02 - s00 * s12;
        let i22 = s00 * s11 - s01 * s01;
        let det = s00 * i00 + s01 * i01 + s02 * i02;
        let id = if det.abs() < 1e-20 { 0.0 } else { 1.0 / det };
        let a0 = (cov[0] * i00 + cov[1] * i01 + cov[2] * i02) * id;
        let a1 = (cov[0] * i01 + cov[1] * i11 + cov[2] * i12) * id;
        let a2 = (cov[0] * i02 + cov[1] * i12 + cov[2] * i22) * id;
        let b = mp.data[k] - a0 * m[0].data[k] - a1 * m[1].data[k] - a2 * m[2].data[k];
        *out = [a0, a1, a2, b];
    });
    let planes: Vec<FPlane> = (0..4).map(|c| FPlane { w, h, data: ab.iter().map(|v| v[c]).collect() }).collect();
    let mb: Vec<FPlane> = planes.iter().map(|p| box_blur(p, r)).collect();
    FPlane {
        w,
        h,
        data: (0..n)
            .into_par_iter()
            .map(|k| mb[0].data[k] * g[0].data[k] + mb[1].data[k] * g[1].data[k] + mb[2].data[k] * g[2].data[k] + mb[3].data[k])
            .collect(),
    }
}

/// Split straight RGBA into three 0..1 planes, optionally downsampling by an
/// integer factor (area average).
pub fn rgb_planes(rgba: &[u8], w: usize, h: usize, factor: usize) -> [FPlane; 3] {
    let f = factor.max(1);
    let (ow, oh) = (w.div_ceil(f), h.div_ceil(f));
    // One output row at a time (in parallel); within a row the source pixels are summed in the
    // same order as a plain scan, so the result does not depend on the number of threads.
    let rows: Vec<[Vec<f32>; 3]> = (0..oh)
        .into_par_iter()
        .map(|oy| {
            let mut acc = [vec![0f32; ow], vec![0f32; ow], vec![0f32; ow]];
            let mut cnt = vec![0f32; ow];
            for y in oy * f..((oy + 1) * f).min(h) {
                let line = &rgba[y * w * 4..(y + 1) * w * 4];
                for (x, px) in line.chunks_exact(4).enumerate() {
                    let o = x / f;
                    for c in 0..3 {
                        acc[c][o] += px[c] as f32 / 255.0;
                    }
                    cnt[o] += 1.0;
                }
            }
            for ch in &mut acc {
                for (v, n) in ch.iter_mut().zip(cnt.iter()) {
                    *v /= n.max(1.0);
                }
            }
            acc
        })
        .collect();
    let mut out = [FPlane::new(ow, oh), FPlane::new(ow, oh), FPlane::new(ow, oh)];
    for (oy, row) in rows.iter().enumerate() {
        for c in 0..3 {
            out[c].data[oy * ow..(oy + 1) * ow].copy_from_slice(&row[c]);
        }
    }
    out
}

pub fn downsample_plane(p: &[u8], w: usize, h: usize, factor: usize) -> FPlane {
    let f = factor.max(1);
    let (ow, oh) = (w.div_ceil(f), h.div_ceil(f));
    let mut out = FPlane::new(ow, oh);
    out.data.par_chunks_mut(ow).enumerate().for_each(|(oy, acc)| {
        let mut cnt = vec![0f32; ow];
        for y in oy * f..((oy + 1) * f).min(h) {
            for (x, v) in p[y * w..(y + 1) * w].iter().enumerate() {
                acc[x / f] += *v as f32 / 255.0;
                cnt[x / f] += 1.0;
            }
        }
        for (v, n) in acc.iter_mut().zip(cnt.iter()) {
            *v /= n.max(1.0);
        }
    });
    out
}

/// Fast guided-filter refinement of an alpha matte at full resolution.
/// The filter runs on a downsampled working copy (≤ ~2.5 MP) and only the
/// uncertain band around edges is replaced, so solid interiors and clean
/// backgrounds stay exactly 0/255.
pub fn refine_alpha(rgba: &[u8], alpha: &[u8], w: usize, h: usize, radius_px: f32, eps: f32) -> Vec<u8> {
    let target = 2_500_000f32;
    let factor = (((w * h) as f32 / target).sqrt().ceil() as usize).max(1);
    let g = rgb_planes(rgba, w, h, factor);
    let a = downsample_plane(alpha, w, h, factor);
    let r = ((radius_px / factor as f32).round() as usize).max(1);
    let q = guided_filter_color(&g, &a, r, eps);
    // uncertain band: dilate(alpha>8) & erode(alpha>247) difference, widened
    let band_r = (radius_px.ceil() as i32).max(2);
    let hi = morph_u8(alpha, w, h, band_r);
    let lo = morph_u8(alpha, w, h, -band_r);
    let mut out = alpha.to_vec();
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let fy = (y as f32 + 0.5) / factor as f32 - 0.5;
        for x in 0..w {
            let i = y * w + x;
            if hi[i] < 4 || lo[i] > 251 {
                continue; // certain background / foreground
            }
            let fx = (x as f32 + 0.5) / factor as f32 - 0.5;
            let v = q.sample_bilinear(fx, fy).clamp(0.0, 1.0);
            row[x] = (v * 255.0 + 0.5) as u8;
        }
    });
    out
}

/// Approximate fast foreground colour estimation (Forte & Pitié 2021,
/// "blur-fusion"). Rewrites the RGB of semi-transparent pixels so that the
/// original background colour no longer bleeds into the edges (halo / fringe
/// removal). Fully opaque and fully transparent pixels are left untouched.
pub fn decontaminate(rgba: &mut [u8], alpha: &[u8], w: usize, h: usize, radius: f32) {
    if !alpha.iter().any(|&a| a > 0 && a < 255) {
        return;
    }
    let passes: [f32; 2] = [radius.max(4.0) * 6.0, radius.max(2.0)];
    // estimate F and B with progressively smaller kernels
    let n = w * h;
    let img: Vec<[f32; 3]> = (0..n).map(|i| [rgba[i * 4] as f32 / 255.0, rgba[i * 4 + 1] as f32 / 255.0, rgba[i * 4 + 2] as f32 / 255.0]).collect();
    let a: Vec<f32> = alpha.iter().map(|&v| v as f32 / 255.0).collect();
    let mut f = img.clone();
    let mut b = img.clone();
    for &r in &passes {
        let ri = r.round().max(1.0) as usize;
        let blur3 = |src: &Vec<[f32; 3]>, wgt: &dyn Fn(usize) -> f32| -> (Vec<[f32; 3]>, FPlane) {
            let wp = FPlane { w, h, data: (0..n).map(|i| wgt(i)).collect() };
            let mut chans = Vec::with_capacity(3);
            for c in 0..3 {
                let p = FPlane { w, h, data: (0..n).map(|i| src[i][c] * wp.data[i]).collect() };
                chans.push(box_blur(&box_blur(&p, ri), ri));
            }
            let wb = box_blur(&box_blur(&wp, ri), ri);
            ((0..n).map(|i| [chans[0].data[i], chans[1].data[i], chans[2].data[i]]).collect(), wb)
        };
        let (fb, fa) = blur3(&f, &|i| a[i]);
        let (bb, ba) = blur3(&b, &|i| 1.0 - a[i]);
        let mut nf = f.clone();
        let mut nb = b.clone();
        for i in 0..n {
            let fh = if fa.data[i] > 1e-5 { [fb[i][0] / fa.data[i], fb[i][1] / fa.data[i], fb[i][2] / fa.data[i]] } else { f[i] };
            let bh = if ba.data[i] > 1e-5 { [bb[i][0] / ba.data[i], bb[i][1] / ba.data[i], bb[i][2] / ba.data[i]] } else { b[i] };
            let al = a[i];
            for c in 0..3 {
                let resid = img[i][c] - al * fh[c] - (1.0 - al) * bh[c];
                nf[i][c] = (fh[c] + al * resid).clamp(0.0, 1.0);
                nb[i][c] = (bh[c] + (1.0 - al) * resid).clamp(0.0, 1.0);
            }
        }
        f = nf;
        b = nb;
    }
    for i in 0..n {
        let av = alpha[i];
        if av == 0 || av == 255 {
            continue;
        }
        for c in 0..3 {
            rgba[i * 4 + c] = (f[i][c] * 255.0 + 0.5) as u8;
        }
    }
}

/// Bilinear upscale of a 0..1 plane to `nw`×`nh`, returned as u8.
pub fn upscale_plane(p: &FPlane, nw: usize, nh: usize) -> Vec<u8> {
    let sx = p.w as f32 / nw as f32;
    let sy = p.h as f32 / nh as f32;
    let mut out = vec![0u8; nw * nh];
    out.par_chunks_mut(nw).enumerate().for_each(|(y, row)| {
        let fy = (y as f32 + 0.5) * sy - 0.5;
        for (x, v) in row.iter_mut().enumerate() {
            let fx = (x as f32 + 0.5) * sx - 0.5;
            *v = (p.sample_bilinear(fx, fy).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn box_blur_preserves_constant() {
        let p = FPlane { w: 7, h: 5, data: vec![0.5; 35] };
        let b = box_blur(&p, 2);
        assert!(b.data.iter().all(|v| (v - 0.5).abs() < 1e-6));
    }
    #[test]
    fn morph_grows_and_shrinks() {
        let mut d = vec![0u8; 25];
        d[12] = 255;
        let g = morph_u8(&d, 5, 5, 1);
        assert_eq!(g.iter().filter(|&&v| v == 255).count(), 9);
        let s = morph_u8(&g, 5, 5, -1);
        assert_eq!(s.iter().filter(|&&v| v == 255).count(), 1);
    }
    #[test]
    fn decontaminate_removes_background_tint() {
        // left half red fg alpha 1, right half green bg alpha 0, middle column 50% blend
        let (w, h) = (21usize, 5usize);
        let mut rgba = vec![0u8; w * h * 4];
        let mut alpha = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let (c, a) = if x < 10 { ([255, 0, 0], 255) } else if x == 10 { ([128, 127, 0], 128) } else { ([0, 255, 0], 0) };
                rgba[i * 4..i * 4 + 3].copy_from_slice(&c);
                rgba[i * 4 + 3] = 255;
                alpha[i] = a;
            }
        }
        decontaminate(&mut rgba, &alpha, w, h, 3.0);
        let i = (2 * w + 10) * 4;
        assert!(rgba[i] > 200 && rgba[i + 1] < 60, "edge colour {:?}", &rgba[i..i + 3]);
    }
}
