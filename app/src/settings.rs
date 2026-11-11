//! Persistent preferences (the macOS app keeps the same keys in UserDefaults).

use crate::platform::paths;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];
    pub fn title(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClipFormat {
    #[default]
    Png,
    Jpeg,
}

impl ClipFormat {
    pub fn label(self) -> &'static str {
        match self {
            ClipFormat::Png => "PNG",
            ClipFormat::Jpeg => "JPEG",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            ClipFormat::Png => "png",
            ClipFormat::Jpeg => "jpg",
        }
    }
}

/// Per-tool options, persisted between sessions (same fields as the macOS `ToolSettings`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSettings {
    pub brush_size: f64,
    pub brush_hardness: f64,
    pub brush_opacity: f64,
    pub eraser_size: f64,
    pub eraser_hardness: f64,
    pub restore_size: f64,
    pub restore_hardness: f64,
    pub restore_strength: f64,
    pub eraser_strength: f64,
    pub tolerance: f64,
    pub contiguous: bool,
    pub antialias: bool,
    pub sample_all: bool,
    pub feather: f64,
    pub erase_feather: f64,
    pub quick_size: f64,
    pub quick_edge: f64,
    pub fill_opacity: f64,
    pub shape_kind: String,
    pub shape_stroke: f64,
    pub shape_filled: bool,
    pub shape_corner: f64,
    pub font_name: String,
    pub font_size: f64,
    pub bold: bool,
    pub italic: bool,
    pub color: [f32; 4],
    pub fill_color: [f32; 4],
}

impl Default for ToolSettings {
    fn default() -> Self {
        ToolSettings {
            brush_size: 40.0,
            brush_hardness: 0.8,
            brush_opacity: 1.0,
            eraser_size: 40.0,
            eraser_hardness: 0.7,
            restore_size: 40.0,
            restore_hardness: 0.7,
            restore_strength: 1.0,
            eraser_strength: 1.0,
            tolerance: 10.0,
            contiguous: true,
            antialias: true,
            sample_all: false,
            feather: 0.0,
            erase_feather: 0.0,
            quick_size: 30.0,
            quick_edge: 0.5,
            fill_opacity: 1.0,
            shape_kind: "rect".into(),
            shape_stroke: 6.0,
            shape_filled: false,
            shape_corner: 0.0,
            font_name: crate::text::DEFAULT_FONT.into(),
            font_size: 72.0,
            bold: true,
            italic: false,
            color: [0.0, 0.48, 1.0, 1.0],
            fill_color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

fn rgba8(c: [f32; 4]) -> [u8; 4] {
    [0, 1, 2, 3].map(|i| (c[i].clamp(0.0, 1.0) * 255.0).round() as u8)
}

impl ToolSettings {
    pub fn color_rgba(&self) -> [u8; 4] {
        rgba8(self.color)
    }
    pub fn fill_rgba(&self) -> [u8; 4] {
        rgba8(self.fill_color)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub tool: String,
    pub tools: ToolSettings,
    pub appearance: Appearance,
    pub show_tools: bool,
    pub show_layers: bool,
    pub clipboard_format: ClipFormat,
    pub jpeg_no_warn: bool,
    pub best_model: String,
    /// Use the DirectML GPU provider for the AI model when this build has it (experimental).
    pub use_directml: bool,
    pub sec_background: bool,
    pub sec_selection: bool,
    pub sec_image: bool,
    pub sec_adjust: bool,
    pub window: Option<[f32; 4]>,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            tool: "move".into(),
            tools: ToolSettings::default(),
            appearance: Appearance::System,
            show_tools: false,
            show_layers: false,
            clipboard_format: ClipFormat::Png,
            jpeg_no_warn: false,
            best_model: "birefnet-general-v2".into(),
            use_directml: true,
            sec_background: true,
            sec_selection: true,
            sec_image: false,
            sec_adjust: false,
            window: None,
            maximized: false,
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read(paths::settings_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = paths::settings_file();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            // atomic replace so a crash never leaves a truncated file
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}
