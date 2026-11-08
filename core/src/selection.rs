//! Selections (canvas-space 8-bit coverage) and the colour-region tools:
//! Magic Wand / Magic Eraser / colour-range deletion / Paint Bucket and the
//! brush-driven Quick Selection.

use crate::color::{rgb_to_lab, ColorKey, Lab};
use crate::geometry::{rasterize_polygon, IRect};
use crate::image::Plane;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Combine {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

/// Combine a freshly computed coverage into an existing selection.
pub fn combine(existing: Option<&Plane>, new: Vec<u8>, w: u32, h: u32, mode: Combine) -> Plane {
    let out = match (mode, existing) {
        (Combine::Replace, _) | (Combine::Add, None) => new,
        (Combine::Subtract, None) | (Combine::Intersect, None) => vec![0; new.len()],
        (Combine::Add, Some(e)) => e.data.iter().zip(new.iter()).map(|(&a, &b)| a.max(b)).collect(),
        (Combine::Subtract, Some(e)) => e.data.iter().zip(new.iter()).map(|(&a, &b)| ((a as u32 * (255 - b as u32)) / 255) as u8).collect(),
        (Combine::Intersect, Some(e)) => e.data.iter().zip(new.iter()).map(|(&a, &b)| a.min(b)).collect(),
    };
    Plane::from_vec(w, h, out).expect("dims")
}

pub fn rect_coverage(w: u32, h: u32, r: IRect) -> Vec<u8> {
    let mut v = vec![0u8; w as usize * h as usize];
    let r = r.intersect(&IRect::new(0, 0, w as i32, h as i32));
    for y in r.y..r.bottom() {
        let row = &mut v[y as usize * w as usize..(y as usize + 1) * w as usize];
        row[r.x as usize..r.right() as usize].iter_mut().for_each(|p| *p = 255);
    }
    v
}

pub fn ellipse_coverage(w: u32, h: u32, x: f64, y: f64, rw: f64, rh: f64) -> Vec<u8> {
    let cx = x + rw / 2.0;
    let cy = y + rh / 2.0;
    let n = 128;
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            (cx + rw / 2.0 * t.cos(), cy + rh / 2.0 * t.sin())
        })
        .collect();
    rasterize_polygon(&pts, w, h, true)
}

pub fn polygon_coverage(w: u32, h: u32, pts: &[(f64, f64)]) -> Vec<u8> {
    rasterize_polygon(pts, w, h, true)
}

/// Per-pixel perceptual distance (ΔE2000-based, ×4 fixed point in u16) from
/// a seed colour across a straight-alpha RGBA image. Cached by the engine so
/// that tolerance changes re-threshold instantly.
pub fn distance_map(rgba: &[u8], key: ColorKey) -> Vec<u16> {
    rgba.par_chunks_exact(4)
        .map(|p| {
            let d = key.distance([p[0], p[1], p[2], p[3]]);
            (d * 4.0).round().min(u16::MAX as f32) as u16
        })
        .collect()
}

/// Turn a distance map into anti-aliased coverage.
///
/// * pixels with ΔE ≤ `tol` are fully selected;
/// * pixels in (`tol`, `tol + soft`] get a linear ramp — these are the
///   anti-aliased blend pixels between the clicked colour and its neighbours;
/// * with `contiguous`, only pixels 4-connected to the seed through fully
///   selected pixels are kept (ramp pixels are included but do not propagate).
pub fn threshold_region(dist: &[u16], w: u32, h: u32, seed: (u32, u32), tol: f32, soft: f32, contiguous: bool) -> Vec<u8> {
    let t4 = tol * 4.0;
    let s4 = soft.max(0.01) * 4.0;
    let cov = |d: u16| -> u8 {
        let d = d as f32;
        if d <= t4 {
            255
        } else if d < t4 + s4 {
            (255.0 * (1.0 - (d - t4) / s4)).round() as u8
        } else {
            0
        }
    };
    let n = (w * h) as usize;
    if !contiguous {
        return dist.par_iter().map(|&d| cov(d)).collect();
    }
    let mut out = vec![0u8; n];
    let (w, h) = (w as usize, h as usize);
    let si = seed.1 as usize * w + seed.0 as usize;
    if si >= n {
        return out;
    }
    let mut visited = vec![false; n];
    let mut q = VecDeque::new();
    visited[si] = true;
    q.push_back(si);
    while let Some(i) = q.pop_front() {
        let c = cov(dist[i]);
        out[i] = c;
        if c < 255 && i != si {
            continue; // edge pixel – include but don't spread
        }
        let (x, y) = (i % w, i / w);
        let mut push = |j: usize| {
            if !visited[j] {
                visited[j] = true;
                if dist[j] as f32 <= t4 + s4 {
                    q.push_back(j);
                }
            }
        };
        if x > 0 {
            push(i - 1);
        }
        if x + 1 < w {
            push(i + 1);
        }
        if y > 0 {
            push(i - w);
        }
        if y + 1 < h {
            push(i + w);
        }
    }
    out
}

/// Brush-driven Quick Selection.
///
/// Seeds are the pixels under the brush. A compact colour model of the seeds
/// (k-means in Lab) is grown outward with a region-growing pass that stops at
/// strong edges and colours that are far from every seed cluster, limited to
/// a neighbourhood around the stroke.
pub fn quick_select(rgba: &[u8], w: u32, h: u32, stroke: &[(f64, f64)], radius: f64, edge_sensitivity: f32) -> Vec<u8> {
    let (wu, hu) = (w as usize, h as usize);
    let n = wu * hu;
    let mut out = vec![0u8; n];
    if stroke.is_empty() {
        return out;
    }
    let r2 = radius * radius;
    // seed mask
    let mut seeds: Vec<usize> = Vec::new();
    let mut bx0 = f64::MAX;
    let mut by0 = f64::MAX;
    let mut bx1 = f64::MIN;
    let mut by1 = f64::MIN;
    for &(px, py) in stroke {
        bx0 = bx0.min(px);
        by0 = by0.min(py);
        bx1 = bx1.max(px);
        by1 = by1.max(py);
        let x0 = (px - radius).floor().max(0.0) as usize;
        let y0 = (py - radius).floor().max(0.0) as usize;
        let x1 = ((px + radius).ceil() as usize).min(wu.saturating_sub(1));
        let y1 = ((py + radius).ceil() as usize).min(hu.saturating_sub(1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f64 + 0.5 - px;
                let dy = y as f64 + 0.5 - py;
                if dx * dx + dy * dy <= r2 {
                    let i = y * wu + x;
                    if out[i] == 0 {
                        out[i] = 255;
                        seeds.push(i);
                    }
                }
            }
        }
    }
    if seeds.is_empty() {
        return out;
    }
    let lab_of = |i: usize| rgb_to_lab(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
    // k-means on a subsample of seed colours
    let step = (seeds.len() / 4000).max(1);
    let samples: Vec<Lab> = seeds.iter().step_by(step).map(|&i| lab_of(i)).collect();
    let k = samples.len().clamp(1, 6);
    let mut centers: Vec<Lab> = (0..k).map(|j| samples[j * samples.len() / k]).collect();
    let mut assign = vec![0usize; samples.len()];
    for _ in 0..6 {
        for (s, a) in samples.iter().zip(assign.iter_mut()) {
            *a = (0..k).min_by(|&p, &q| lab_d2(*s, centers[p]).partial_cmp(&lab_d2(*s, centers[q])).unwrap()).unwrap();
        }
        let mut acc = vec![(0f32, 0f32, 0f32, 0f32); k];
        for (s, &a) in samples.iter().zip(assign.iter()) {
            acc[a].0 += s.l;
            acc[a].1 += s.a;
            acc[a].2 += s.b;
            acc[a].3 += 1.0;
        }
        for (c, a) in centers.iter_mut().zip(acc.iter()) {
            if a.3 > 0.0 {
                *c = Lab { l: a.0 / a.3, a: a.1 / a.3, b: a.2 / a.3 };
            }
        }
    }
    let mut spread = vec![0f32; k];
    let mut cnt = vec![0f32; k];
    for (s, &a) in samples.iter().zip(assign.iter()) {
        spread[a] += lab_d2(*s, centers[a]).sqrt();
        cnt[a] += 1.0;
    }
    let thresh: Vec<f32> = (0..k).map(|j| (2.5 * spread[j] / cnt[j].max(1.0) + 6.0).clamp(7.0, 24.0)).collect();
    let pad = radius * 4.0 + 8.0;
    let lim = IRect::from_bounds(bx0 - pad, by0 - pad, bx1 + pad, by1 + pad).intersect(&IRect::new(0, 0, w as i32, h as i32));
    let edge_t = 0.18 * (1.0 - edge_sensitivity.clamp(0.0, 0.95));
    let luma = |i: usize| crate::color::luma(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
    let mut q: VecDeque<usize> = seeds.iter().copied().collect();
    let mut visited = out.iter().map(|&v| v > 0).collect::<Vec<bool>>();
    while let Some(i) = q.pop_front() {
        let (x, y) = ((i % wu) as i32, (i / wu) as i32);
        let li = luma(i);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < lim.x || ny < lim.y || nx >= lim.right() || ny >= lim.bottom() {
                continue;
            }
            let j = ny as usize * wu + nx as usize;
            if visited[j] {
                continue;
            }
            visited[j] = true;
            if rgba[j * 4 + 3] < 16 {
                continue;
            }
            if (luma(j) - li).abs() > edge_t {
                continue;
            }
            let lj = lab_of(j);
            let ok = (0..k).any(|c| crate::color::delta_e2000(lj, centers[c]) <= thresh[c]);
            if ok {
                out[j] = 255;
                q.push_back(j);
            }
        }
    }
    // close small holes and soften the boundary — only around the stroke
    let r = lim.inflate(4).intersect(&IRect::new(0, 0, w as i32, h as i32));
    let (rw, rh) = (r.w as usize, r.h as usize);
    let mut sub = vec![0u8; rw * rh];
    for y in 0..rh {
        let src = (y + r.y as usize) * wu + r.x as usize;
        sub[y * rw..(y + 1) * rw].copy_from_slice(&out[src..src + rw]);
    }
    let closed = crate::filters::morph_u8(&crate::filters::morph_u8(&sub, rw, rh, 1), rw, rh, -1);
    let soft = crate::filters::gaussian_u8(&closed, rw, rh, 0.6);
    for y in 0..rh {
        let dst = (y + r.y as usize) * wu + r.x as usize;
        out[dst..dst + rw].copy_from_slice(&soft[y * rw..(y + 1) * rw]);
    }
    out
}

#[inline]
fn lab_d2(a: Lab, b: Lab) -> f32 {
    (a.l - b.l).powi(2) + (a.a - b.a).powi(2) + (a.b - b.b).powi(2)
}

/// Marching-ants outline: axis-aligned boundary segments between selected
/// (≥128) and unselected pixels, merged into runs. Returns x0,y0,x1,y1 tuples.
pub fn outline_segments(sel: &Plane, max_segments: usize) -> Vec<f32> {
    let (w, h) = (sel.w as usize, sel.h as usize);
    let on = |x: isize, y: isize| -> bool {
        x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && sel.data[y as usize * w + x as usize] >= 128
    };
    let mut segs = Vec::new();
    // horizontal edges at y = 0..=h
    for y in 0..=h as isize {
        let mut run: Option<usize> = None;
        for x in 0..=w as isize {
            let edge = x < w as isize && on(x, y) != on(x, y - 1);
            match (edge, run) {
                (true, None) => run = Some(x as usize),
                (false, Some(s)) => {
                    segs.extend_from_slice(&[s as f32, y as f32, x as f32, y as f32]);
                    run = None;
                }
                _ => {}
            }
        }
        if segs.len() / 4 > max_segments {
            return Vec::new();
        }
    }
    for x in 0..=w as isize {
        let mut run: Option<usize> = None;
        for y in 0..=h as isize {
            let edge = y < h as isize && on(x, y) != on(x - 1, y);
            match (edge, run) {
                (true, None) => run = Some(y as usize),
                (false, Some(s)) => {
                    segs.extend_from_slice(&[x as f32, s as f32, x as f32, y as f32]);
                    run = None;
                }
                _ => {}
            }
        }
        if segs.len() / 4 > max_segments {
            return Vec::new();
        }
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&f(x, y));
            }
        }
        v
    }

    #[test]
    fn magic_wand_keeps_white_part_on_beige() {
        // beige background with a white square in the middle and a slightly
        // darker beige (shadow) gradient on the left.
        let (w, h) = (40, 40);
        let rgba = img(w, h, |x, y| {
            if (15..25).contains(&x) && (15..25).contains(&y) {
                [250, 250, 250, 255]
            } else if x < 5 {
                [224, 212, 189, 255]
            } else {
                [232, 220, 196, 255]
            }
        });
        let key = ColorKey::new([232, 220, 196, 255]);
        let d = distance_map(&rgba, key);
        let sel = threshold_region(&d, w, h, (2, 30), 6.0, 3.0, true);
        assert_eq!(sel[(20 * w + 20) as usize], 0, "white part must not be selected");
        assert_eq!(sel[(2 * w + 2) as usize], 255, "shadowed beige is selected");
        assert_eq!(sel[(30 * w + 35) as usize], 255);
    }

    #[test]
    fn contiguous_vs_global() {
        let (w, h) = (30, 10);
        // red | blue wall | red
        let rgba = img(w, h, |x, _| if (10..20).contains(&x) { [0, 0, 255, 255] } else { [255, 0, 0, 255] });
        let d = distance_map(&rgba, ColorKey::new([255, 0, 0, 255]));
        let c = threshold_region(&d, w, h, (0, 0), 5.0, 2.0, true);
        let g = threshold_region(&d, w, h, (0, 0), 5.0, 2.0, false);
        assert_eq!(c[25], 0);
        assert_eq!(g[25], 255);
        assert_eq!(c[15], 0);
    }

    #[test]
    fn antialiased_edge_gets_partial_coverage() {
        let (w, h) = (3, 1);
        // bg, 50% blend, fg
        let rgba = vec![232, 220, 196, 255, 141, 136, 124, 255, 50, 50, 50, 255];
        let d = distance_map(&rgba, ColorKey::new([232, 220, 196, 255]));
        let s = threshold_region(&d, w, h, (0, 0), 10.0, 40.0, true);
        assert_eq!(s[0], 255);
        assert!(s[1] > 0 && s[1] < 255, "blend coverage {}", s[1]);
        assert_eq!(s[2], 0);
    }

    #[test]
    fn quick_select_stays_inside_object() {
        let (w, h) = (60, 60);
        let rgba = img(w, h, |x, y| {
            let dx = x as f64 - 30.0;
            let dy = y as f64 - 30.0;
            if dx * dx + dy * dy < 15.0 * 15.0 { [200, 40, 40, 255] } else { [40, 160, 220, 255] }
        });
        let sel = quick_select(&rgba, w, h, &[(28.0, 30.0), (32.0, 30.0)], 3.0, 0.5);
        assert!(sel[30 * 60 + 40] > 200, "grew inside the disc");
        assert_eq!(sel[30 * 60 + 50], 0, "stopped at the edge");
        assert_eq!(sel[5 * 60 + 5], 0);
    }

    #[test]
    fn outline_of_square() {
        let p = Plane::from_vec(4, 4, rect_coverage(4, 4, IRect::new(1, 1, 2, 2))).unwrap();
        let s = outline_segments(&p, 1000);
        assert_eq!(s.len() / 4, 4);
    }

    #[test]
    fn combine_modes() {
        let a = Plane::from_vec(2, 1, vec![255, 0]).unwrap();
        assert_eq!(*combine(Some(&a), vec![0, 255], 2, 1, Combine::Add).data, vec![255, 255]);
        assert_eq!(*combine(Some(&a), vec![255, 255], 2, 1, Combine::Subtract).data, vec![0, 0]);
        assert_eq!(*combine(Some(&a), vec![255, 255], 2, 1, Combine::Intersect).data, vec![255, 0]);
    }
}
