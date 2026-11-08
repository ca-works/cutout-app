//! The composited document as a grid of 512 px textures, so a brush stroke only
//! re-uploads the tiles it touches (the same scheme as the macOS CALayer grid).

use eframe::egui::{self, Color32, ColorImage, Context, Pos2, Rect, TextureFilter, TextureHandle, TextureOptions, TextureWrapMode};

pub const TILE: usize = 512;

pub fn options(nearest: bool) -> TextureOptions {
    TextureOptions {
        magnification: if nearest { TextureFilter::Nearest } else { TextureFilter::Linear },
        minification: TextureFilter::Linear,
        wrap_mode: TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(TextureFilter::Linear),
    }
}

pub struct TileSet {
    pub w: usize,
    pub h: usize,
    cols: usize,
    rows: usize,
    tiles: Vec<TextureHandle>,
    pub nearest: bool,
}

fn tile_image(data: &[u8], w: usize, x: usize, y: usize, tw: usize, th: usize) -> ColorImage {
    let mut buf = Vec::with_capacity(tw * th * 4);
    for row in 0..th {
        let s = ((y + row) * w + x) * 4;
        buf.extend_from_slice(&data[s..s + tw * 4]);
    }
    // The engine's display buffer is premultiplied RGBA — exactly what egui blends.
    ColorImage::from_rgba_premultiplied([tw, th], &buf)
}

impl TileSet {
    /// Build every tile from a premultiplied RGBA buffer of `w`×`h`.
    pub fn new(ctx: &Context, name: &str, data: &[u8], w: usize, h: usize, nearest: bool) -> TileSet {
        let cols = w.div_ceil(TILE);
        let rows = h.div_ceil(TILE);
        let mut tiles = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                let (x, y) = (c * TILE, r * TILE);
                let (tw, th) = (TILE.min(w - x), TILE.min(h - y));
                let img = tile_image(data, w, x, y, tw, th);
                tiles.push(ctx.load_texture(format!("{name}-{r}-{c}"), img, options(nearest)));
            }
        }
        TileSet { w, h, cols, rows, tiles, nearest }
    }

    /// Re-upload the tiles intersecting `dirty` (x, y, w, h in image pixels).
    pub fn update(&mut self, data: &[u8], dirty: (i32, i32, i32, i32)) {
        let (dx, dy, dw, dh) = dirty;
        if dw <= 0 || dh <= 0 {
            return;
        }
        let c0 = (dx.max(0) as usize) / TILE;
        let c1 = (((dx + dw - 1).max(0)) as usize / TILE).min(self.cols - 1);
        let r0 = (dy.max(0) as usize) / TILE;
        let r1 = (((dy + dh - 1).max(0)) as usize / TILE).min(self.rows - 1);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let (x, y) = (c * TILE, r * TILE);
                let (tw, th) = (TILE.min(self.w - x), TILE.min(self.h - y));
                let img = tile_image(data, self.w, x, y, tw, th);
                self.tiles[r * self.cols + c].set(img, options(self.nearest));
            }
        }
    }

    /// Switch between smooth and nearest-neighbour magnification (re-uploads every tile).
    pub fn set_nearest(&mut self, data: &[u8], nearest: bool) {
        if self.nearest == nearest {
            return;
        }
        self.nearest = nearest;
        self.update(data, (0, 0, self.w as i32, self.h as i32));
    }

    /// Paint at `origin` (screen position of image pixel 0,0) with `scale` points per pixel.
    /// Tile edges are snapped to the physical pixel grid so no seams appear.
    pub fn paint(&self, painter: &egui::Painter, origin: Pos2, scale: f32, tint: Color32) {
        let ppp = painter.ctx().pixels_per_point();
        let snap = |v: f32| (v * ppp).round() / ppp;
        let clip = painter.clip_rect();
        for r in 0..self.rows {
            for c in 0..self.cols {
                let (x, y) = (c * TILE, r * TILE);
                let (tw, th) = (TILE.min(self.w - x), TILE.min(self.h - y));
                let min = Pos2::new(snap(origin.x + x as f32 * scale), snap(origin.y + y as f32 * scale));
                let max = Pos2::new(snap(origin.x + (x + tw) as f32 * scale), snap(origin.y + (y + th) as f32 * scale));
                let rect = Rect::from_min_max(min, max);
                if !rect.intersects(clip) {
                    continue;
                }
                painter.image(self.tiles[r * self.cols + c].id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
            }
        }
    }
}
