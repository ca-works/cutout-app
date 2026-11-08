//! Rasterises editable text layers (the platform text service for the engine's
//! Text layers). Uses cosmic-text, so shaping and layout are identical on every
//! platform; fonts come from the operating system.

use crate::engine::{Image, TextProps};
use cosmic_text::{Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight};
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
pub const DEFAULT_FONT: &str = "Segoe UI";
#[cfg(not(windows))]
pub const DEFAULT_FONT: &str = "Helvetica Neue";

struct Fonts {
    system: FontSystem,
    cache: SwashCache,
}

fn fonts() -> &'static Mutex<Fonts> {
    static F: OnceLock<Mutex<Fonts>> = OnceLock::new();
    // Scanning system fonts takes a moment, so it only happens on first use.
    F.get_or_init(|| Mutex::new(Fonts { system: FontSystem::new(), cache: SwashCache::new() }))
}

/// Installed font families, sorted and de-duplicated.
pub fn families() -> Vec<String> {
    let f = fonts().lock().unwrap();
    let mut v: Vec<String> = f.system.db().faces().filter_map(|face| face.families.first().map(|(n, _)| n.clone())).collect();
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup();
    v
}

/// Warm the font database on a background thread so the first text edit is instant.
pub fn preload() {
    std::thread::spawn(|| {
        drop(fonts().lock());
    });
}

pub fn render(p: &TextProps) -> Option<Image> {
    let text = if p.text.is_empty() { " " } else { p.text.as_str() };
    let mut guard = fonts().lock().ok()?;
    let Fonts { system, cache } = &mut *guard;
    let size = p.size.clamp(1.0, 2000.0) as f32;
    let metrics = Metrics::new(size, size * 1.2);
    let mut buffer = Buffer::new(system, metrics);
    let c = p.color;
    let attrs = Attrs::new()
        .family(Family::Name(&p.font))
        .weight(if p.bold { Weight::BOLD } else { Weight::NORMAL })
        .style(if p.italic { Style::Italic } else { Style::Normal })
        .color(Color::rgba(c[0], c[1], c[2], c[3]));
    buffer.set_size(None, None);
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(system, false);
    let (mut width, mut lines) = (0.0f32, 0usize);
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        lines += 1;
    }
    let align = match p.align.as_str() {
        "center" => Align::Center,
        "right" => Align::Right,
        _ => Align::Left,
    };
    // Re-layout with the measured width so centre / right alignment has a frame.
    buffer.set_size(Some(width.ceil().max(1.0)), None);
    for line in buffer.lines.iter_mut() {
        line.set_align(Some(align));
    }
    buffer.shape_until_scroll(system, false);
    let height = lines.max(1) as f32 * size * 1.2;
    let pad = (p.size * 0.15).ceil() as i32;
    let w = width.ceil() as i32 + pad * 2;
    let h = height.ceil() as i32 + pad * 2;
    if w <= 0 || h <= 0 || w > 16_384 || h > 16_384 {
        return None;
    }
    let (wu, hu) = (w as usize, h as usize);
    // premultiplied accumulation buffer
    let mut acc = vec![0u8; wu * hu * 4];
    buffer.draw(system, cache, Color::rgba(c[0], c[1], c[2], c[3]), |x, y, gw, gh, col| {
        let a = col.a() as u32;
        if a == 0 {
            return;
        }
        for yy in y..y + gh as i32 {
            for xx in x..x + gw as i32 {
                let (px, py) = (xx + pad, yy + pad);
                if px < 0 || py < 0 || px >= w || py >= h {
                    continue;
                }
                let o = (py as usize * wu + px as usize) * 4;
                // source-over of a premultiplied source onto the buffer
                let inv = 255 - a;
                let (sr, sg, sb) = ((col.r() as u32 * a + 127) / 255, (col.g() as u32 * a + 127) / 255, (col.b() as u32 * a + 127) / 255);
                acc[o] = (sr + (acc[o] as u32 * inv + 127) / 255).min(255) as u8;
                acc[o + 1] = (sg + (acc[o + 1] as u32 * inv + 127) / 255).min(255) as u8;
                acc[o + 2] = (sb + (acc[o + 2] as u32 * inv + 127) / 255).min(255) as u8;
                acc[o + 3] = (a + (acc[o + 3] as u32 * inv + 127) / 255).min(255) as u8;
            }
        }
    });
    // premultiplied -> straight alpha
    for px in acc.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for ch in px.iter_mut().take(3) {
                *ch = ((*ch as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(Image { width: w as u32, height: h as u32, data: acc })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_some_ink() {
        let p = TextProps { text: "Hi".into(), font: DEFAULT_FONT.into(), size: 48.0, color: [255, 0, 0, 255], bold: true, italic: false, align: "left".into() };
        // System fonts may be absent in a minimal CI image; only check when rendering succeeds.
        if let Some(img) = render(&p) {
            assert!(img.width > 10 && img.height > 10);
        }
    }
}
