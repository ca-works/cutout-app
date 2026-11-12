//! Model-free background mask for the Windows shell. No AI or platform imports.
//! Inspired by ENG Drawing Viewer's dominant-border / exterior-connected seeding,
//! but deliberately omits GrabCut, clustering and speck deletion to retain small text.
use std::collections::VecDeque;

pub fn mask(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let (w, h) = (width as usize, height as usize);
    let n = w.checked_mul(h).filter(|&n| n > 0).ok_or("Empty image")?;
    if n.checked_mul(4) != Some(rgba.len()) {
        return Err("Invalid RGBA buffer".into());
    }
    // Quantized border mode is robust against objects crossing a minority of the border.
    // Fully transparent RGB is undefined and must never influence colour estimation.
    let mut counts = [0u32; 4096];
    let mut sums = [[0u64; 3]; 4096];
    let mut sample = |i: usize| {
        let p = &rgba[i * 4..i * 4 + 4];
        if p[3] < 240 {
            return;
        }
        let bin = ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
        counts[bin] += 1;
        for c in 0..3 {
            sums[bin][c] += p[c] as u64;
        }
    };
    for x in 0..w {
        sample(x);
        if h > 1 {
            sample((h - 1) * w + x);
        }
    }
    for y in 1..h.saturating_sub(1) {
        sample(y * w);
        if w > 1 {
            sample(y * w + w - 1);
        }
    }
    let bin = (0..4096).max_by_key(|&i| counts[i]).unwrap();
    let mut out = vec![255; n];
    if counts[bin] == 0 {
        return Ok(out);
    } // Already cut out: don't guess from hidden RGB.
    let bg = sums[bin].map(|v| (v / counts[bin] as u64) as i16);
    let mut queue = VecDeque::new();
    let visit = |i: usize, out: &mut [u8], queue: &mut VecDeque<usize>| {
        if out[i] != 255 {
            return;
        }
        let p = &rgba[i * 4..i * 4 + 4];
        let d = (0..3)
            .map(|c| (p[c] as i16 - bg[c]).unsigned_abs())
            .max()
            .unwrap();
        if p[3] == 0 || d < 24 {
            // A narrow soft transition; no blur, resizing, RGB edits or alpha doubling.
            out[i] = if p[3] == 0 || d <= 6 {
                0
            } else {
                ((d - 6) * 255 / 18) as u8
            };
            queue.push_back(i);
        }
    };
    for x in 0..w {
        visit(x, &mut out, &mut queue);
        visit((h - 1) * w + x, &mut out, &mut queue);
    }
    for y in 0..h {
        visit(y * w, &mut out, &mut queue);
        visit(y * w + w - 1, &mut out, &mut queue);
    }
    while let Some(i) = queue.pop_front() {
        if i >= w {
            visit(i - w, &mut out, &mut queue);
        }
        if i + w < n {
            visit(i + w, &mut out, &mut queue);
        }
        if i % w > 0 {
            visit(i - 1, &mut out, &mut queue);
        }
        if i % w + 1 < w {
            visit(i + 1, &mut out, &mut queue);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene(bg: [u8; 4], fg: [u8; 4], w: u32, h: u32) -> Vec<u8> {
        let mut p = bg.repeat((w * h) as usize);
        for y in h / 4..h * 3 / 4 {
            for x in w / 4..w * 3 / 4 {
                p[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize].copy_from_slice(&fg);
            }
        }
        p
    }
    #[test]
    fn white_black_colored_and_dimensions() {
        for bg in [[255, 255, 255, 255], [0, 0, 0, 255], [40, 110, 190, 255]] {
            let fg = if bg[0] == 0 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            };
            let p = scene(bg, fg, 100, 80);
            let original = p.clone();
            let m = mask(&p, 100, 80).unwrap();
            assert_eq!(m.len(), 8000);
            assert_eq!(m[0], 0);
            assert_eq!(m[4050], 255);
            assert_eq!(p, original);
        }
    }
    #[test]
    fn thin_text_and_enclosed_background_color_survive() {
        let mut p = scene([255; 4], [0, 0, 0, 255], 40, 40);
        p[(20 * 40 + 20) * 4..(20 * 40 + 20) * 4 + 4].copy_from_slice(&[255; 4]);
        p[(4 * 40 + 4) * 4..(4 * 40 + 4) * 4 + 4].copy_from_slice(&[0, 0, 0, 255]);
        let m = mask(&p, 40, 40).unwrap();
        assert_eq!(m[20 * 40 + 20], 255);
        assert_eq!(m[4 * 40 + 4], 255);
    }
    #[test]
    fn similar_foreground_and_partial_alpha_preserved() {
        let p = scene([255; 4], [225, 225, 225, 128], 40, 40);
        let m = mask(&p, 40, 40).unwrap();
        assert_eq!(m[20 * 40 + 20], 255);
        assert_eq!(p[(20 * 40 + 20) * 4 + 3], 128);
    }
    #[test]
    fn transparent_border_does_not_guess_hidden_rgb() {
        let p = scene([255, 0, 0, 0], [255, 0, 0, 128], 40, 40);
        assert!(mask(&p, 40, 40).unwrap().iter().all(|&v| v == 255));
    }
    #[test]
    fn invalid_empty_and_overflow() {
        for (p, w, h) in [
            (vec![], 0, 0),
            (vec![0], 1, 1),
            (vec![], u32::MAX, u32::MAX),
        ] {
            assert!(mask(&p, w, h).is_err());
        }
    }
    #[test]
    fn large_image() {
        let p = scene([255; 4], [0, 0, 0, 255], 1920, 1080);
        let m = mask(&p, 1920, 1080).unwrap();
        assert_eq!(m.len(), 1920 * 1080);
        assert_eq!(m[540 * 1920 + 960], 255);
    }
    #[test]
    fn soft_edge() {
        let mut p = scene([255; 4], [0, 0, 0, 255], 40, 40);
        p[4..8].copy_from_slice(&[240, 240, 240, 255]);
        let m = mask(&p, 40, 40).unwrap();
        assert!(m[1] > 0 && m[1] < 255);
    }
}
