//! Application state and the editor model — the Windows counterpart of the
//! macOS `EditorModel`: every user-visible workflow goes through here.

use crate::canvas::CanvasState;
use crate::codec::{self, Format};
use crate::engine::{Doc, Image, LayerInfo, State, TextProps};
use crate::models::{self, RemovalMode};
use crate::platform::{clipboard, paths};
use crate::settings::{Appearance, Settings};
use crate::theme::{self, Palette};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, TextureHandle};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ tools ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Move,
    RectSelect,
    EllipseSelect,
    Lasso,
    PolygonLasso,
    MagicWand,
    QuickSelect,
    MagicEraser,
    Eraser,
    Restore,
    Brush,
    Fill,
    Eyedropper,
    Text,
    Shape,
    Crop,
    Hand,
    Zoom,
}

impl Tool {
    pub const ALL: [Tool; 18] = [
        Tool::Move,
        Tool::RectSelect,
        Tool::EllipseSelect,
        Tool::Lasso,
        Tool::PolygonLasso,
        Tool::MagicWand,
        Tool::QuickSelect,
        Tool::MagicEraser,
        Tool::Eraser,
        Tool::Restore,
        Tool::Brush,
        Tool::Fill,
        Tool::Eyedropper,
        Tool::Text,
        Tool::Shape,
        Tool::Crop,
        Tool::Hand,
        Tool::Zoom,
    ];

    pub const GROUPS: [(&'static str, &'static [Tool]); 3] = [
        ("Select", &[Tool::Move, Tool::RectSelect, Tool::EllipseSelect, Tool::Lasso, Tool::PolygonLasso, Tool::MagicWand, Tool::QuickSelect]),
        ("Retouch", &[Tool::MagicEraser, Tool::Eraser, Tool::Restore, Tool::Brush, Tool::Fill, Tool::Eyedropper]),
        ("Create", &[Tool::Text, Tool::Shape, Tool::Crop, Tool::Hand, Tool::Zoom]),
    ];

    pub fn id(self) -> &'static str {
        match self {
            Tool::Move => "move",
            Tool::RectSelect => "rectSelect",
            Tool::EllipseSelect => "ellipseSelect",
            Tool::Lasso => "lasso",
            Tool::PolygonLasso => "polygonLasso",
            Tool::MagicWand => "magicWand",
            Tool::QuickSelect => "quickSelect",
            Tool::MagicEraser => "magicEraser",
            Tool::Eraser => "eraser",
            Tool::Restore => "restore",
            Tool::Brush => "brush",
            Tool::Fill => "fill",
            Tool::Eyedropper => "eyedropper",
            Tool::Text => "text",
            Tool::Shape => "shape",
            Tool::Crop => "crop",
            Tool::Hand => "hand",
            Tool::Zoom => "zoom",
        }
    }

    pub fn from_id(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.id() == s)
    }

    pub fn title(self) -> &'static str {
        match self {
            Tool::Move => "Move & Transform",
            Tool::RectSelect => "Rectangle Select",
            Tool::EllipseSelect => "Ellipse Select",
            Tool::Lasso => "Lasso",
            Tool::PolygonLasso => "Polygon Lasso",
            Tool::MagicWand => "Magic Wand",
            Tool::QuickSelect => "Quick Selection",
            Tool::MagicEraser => "Magic Eraser",
            Tool::Eraser => "Eraser",
            Tool::Restore => "Restore Brush",
            Tool::Brush => "Brush",
            Tool::Fill => "Paint Bucket",
            Tool::Eyedropper => "Eyedropper",
            Tool::Text => "Text",
            Tool::Shape => "Shapes",
            Tool::Crop => "Crop",
            Tool::Hand => "Hand",
            Tool::Zoom => "Zoom",
        }
    }

    pub fn icon(self) -> char {
        use crate::icons::*;
        match self {
            Tool::Move => MOVE,
            Tool::RectSelect => SQUARE_DASHED,
            Tool::EllipseSelect => CIRCLE_DASHED,
            Tool::Lasso => LASSO,
            Tool::PolygonLasso => PENTAGON,
            Tool::MagicWand => WAND,
            Tool::QuickSelect => BRUSH,
            Tool::MagicEraser => WAND_SPARKLES,
            Tool::Eraser => ERASER,
            Tool::Restore => UNDO_2,
            Tool::Brush => PAINTBRUSH,
            Tool::Fill => PAINT_BUCKET,
            Tool::Eyedropper => PIPETTE,
            Tool::Text => TYPE,
            Tool::Shape => SHAPES,
            Tool::Crop => CROP,
            Tool::Hand => HAND,
            Tool::Zoom => ZOOM_IN,
        }
    }

    pub fn shortcut_char(self) -> char {
        match self {
            Tool::Move => 'V',
            Tool::RectSelect => 'M',
            Tool::EllipseSelect => 'O',
            Tool::Lasso => 'L',
            Tool::PolygonLasso => 'P',
            Tool::MagicWand => 'W',
            Tool::QuickSelect => 'Q',
            Tool::MagicEraser => 'E',
            Tool::Eraser => 'X',
            Tool::Restore => 'R',
            Tool::Brush => 'B',
            Tool::Fill => 'G',
            Tool::Eyedropper => 'I',
            Tool::Text => 'T',
            Tool::Shape => 'U',
            Tool::Crop => 'C',
            Tool::Hand => 'H',
            Tool::Zoom => 'Z',
        }
    }

    pub fn key(self) -> Key {
        match self {
            Tool::Move => Key::V,
            Tool::RectSelect => Key::M,
            Tool::EllipseSelect => Key::O,
            Tool::Lasso => Key::L,
            Tool::PolygonLasso => Key::P,
            Tool::MagicWand => Key::W,
            Tool::QuickSelect => Key::Q,
            Tool::MagicEraser => Key::E,
            Tool::Eraser => Key::X,
            Tool::Restore => Key::R,
            Tool::Brush => Key::B,
            Tool::Fill => Key::G,
            Tool::Eyedropper => Key::I,
            Tool::Text => Key::T,
            Tool::Shape => Key::U,
            Tool::Crop => Key::C,
            Tool::Hand => Key::H,
            Tool::Zoom => Key::Z,
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Tool::Move => "Drag to move the layer. Drag corners to scale, the top handle to rotate. Click a layer's pixels to select it.",
            Tool::RectSelect | Tool::EllipseSelect => "Drag to select. Shift adds, Alt subtracts.",
            Tool::Lasso => "Drag around an area to select it. Shift adds, Alt subtracts.",
            Tool::PolygonLasso => "Click to add points; double-click or press Enter to close. Esc cancels.",
            Tool::MagicWand => "Click a colour to select similar connected pixels (ΔE colour distance).",
            Tool::QuickSelect => "Paint over an object; the selection grows to its edges. Alt subtracts.",
            Tool::MagicEraser => "Click a colour to erase it. Respects anti-aliased edges; erased pixels can be restored.",
            Tool::Eraser => "Paint to hide pixels (non-destructive).",
            Tool::Restore => "Paint to bring back erased or removed pixels.",
            Tool::Brush => "Paint with the current colour.",
            Tool::Fill => "Click to fill an area of similar colour.",
            Tool::Eyedropper => "Click to pick a colour.",
            Tool::Text => "Click to add text.",
            Tool::Shape => "Drag to draw a shape on a new layer.",
            Tool::Crop => "Drag the crop area, then press Enter.",
            Tool::Hand => "Drag to pan. Hold Space with any tool.",
            Tool::Zoom => "Click to zoom in, Alt-click to zoom out.",
        }
    }

    pub fn is_brush(self) -> bool {
        matches!(self, Tool::Eraser | Tool::Restore | Tool::Brush | Tool::QuickSelect)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelMode {
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl SelMode {
    pub const ALL: [SelMode; 4] = [SelMode::Replace, SelMode::Add, SelMode::Subtract, SelMode::Intersect];
    pub fn id(self) -> &'static str {
        match self {
            SelMode::Replace => "replace",
            SelMode::Add => "add",
            SelMode::Subtract => "subtract",
            SelMode::Intersect => "intersect",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            SelMode::Replace => "Replace",
            SelMode::Add => "Add",
            SelMode::Subtract => "Subtract",
            SelMode::Intersect => "Intersect",
        }
    }
    pub fn icon(self) -> char {
        match self {
            SelMode::Replace => crate::icons::SQUARE,
            SelMode::Add => crate::icons::SQUARE_PLUS,
            SelMode::Subtract => crate::icons::SQUARE_MINUS,
            SelMode::Intersect => crate::icons::SQUARE_DOT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyAction {
    Copy,
    CopyAndClear,
    CopyAndClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    None,
    Refine,
    Adjust,
}

// ----------------------------------------------------------------- sheets ---

pub struct ResizeSheet {
    pub canvas: bool,
    pub width: i64,
    pub height: i64,
    pub keep_aspect: bool,
    pub anchor: usize,
}

pub struct TextSheet {
    pub layer: Option<i64>,
    pub point: (f64, f64),
    pub text: String,
    pub align: String,
    pub font_filter: String,
}

pub enum Sheet {
    ModelSetup,
    Resize(ResizeSheet),
    Text(TextSheet),
    JpegWarning { action: CopyAction, dont_ask: bool },
    /// Best Quality needs ≈10 GB of free memory; offer alternatives when there is less.
    LowMemory { mode: RemovalMode, avail_gb: f64 },
    Settings,
    About,
    Shortcuts,
}

pub struct Toast {
    pub message: String,
    pub icon: char,
    pub error: bool,
    pub born: Instant,
    pub ttl: Duration,
}

pub struct Busy {
    pub title: String,
    pub cancellable: bool,
}

pub struct RemovalJob {
    pub removal: models::Removal,
    pub layer: i64,
    pub started: Instant,
    pub w: u32,
    pub h: u32,
}

pub enum ModelTask {
    Download { spec: &'static models::ModelSpec, rx: Receiver<Result<f64, Result<(), String>>>, cancel: Arc<AtomicBool>, progress: f64 },
    Install { rx: Receiver<Result<f64, Result<&'static models::ModelSpec, String>>>, progress: f64 },
}

#[derive(Default, Clone, PartialEq)]
pub struct RefineState {
    pub radius: f64,
    pub smooth: f64,
    pub feather: f64,
    pub contrast: f64,
    pub shift: f64,
    pub decontaminate: bool,
    pub active: bool,
}

impl RefineState {
    pub fn is_identity(&self) -> bool {
        self.radius == 0.0 && self.smooth == 0.0 && self.feather == 0.0 && self.contrast == 0.0 && self.shift == 0.0 && !self.decontaminate
    }
    pub fn params(&self) -> Value {
        json!({"radius": self.radius, "smooth": self.smooth, "feather": self.feather, "contrast": self.contrast, "shift": self.shift, "decontaminate": self.decontaminate})
    }
}

#[derive(Default, Clone, PartialEq)]
pub struct AdjustState {
    pub values: HashMap<&'static str, f64>,
    pub invert: bool,
    pub gray: bool,
}

impl AdjustState {
    pub fn payload(&self) -> Value {
        let mut d = serde_json::Map::new();
        for (k, v) in &self.values {
            d.insert((*k).to_string(), json!(v));
        }
        d.insert("invert".into(), json!(self.invert));
        d.insert("grayscale".into(), json!(self.gray));
        json!({"adjustments": Value::Object(d)})
    }
}

// ----------------------------------------------------------------- actions ---

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open,
    OpenProject,
    RestoreSession,
    ClearWorkspace,
    SaveProject,
    SaveProjectAs,
    Export(Format, f64),
    Undo,
    Redo,
    Copy(CopyAction, Option<Format>),
    Paste,
    PasteNewImage,
    SelectAll,
    Deselect,
    InvertSelection,
    DeleteSelection,
    KeepOnlySelection,
    RemoveBg(RemovalMode),
    RefineEdges,
    Compare,
    CropToSelection,
    Trim,
    ResizeImage,
    CanvasSize,
    Rotate(bool),
    Flip(&'static str),
    ManageModels,
    DuplicateLayer,
    DeleteLayer,
    MergeDown,
    Flatten,
    LayerOrder(&'static str),
    RestoreAll,
    ApplyMask,
    RasterizeText,
    ZoomIn,
    ZoomOut,
    Fit,
    Actual,
    ToggleTools,
    ToggleLayers,
    Settings,
    About,
    Shortcuts,
    Quit,
}

fn sc(m: Modifiers, k: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(m, k)
}

/// Keyboard shortcuts. Ctrl on Windows corresponds to ⌘ on the Mac, Alt to ⌥.
pub fn shortcuts() -> Vec<(KeyboardShortcut, Action)> {
    let c = Modifiers::COMMAND;
    let cs = Modifiers::COMMAND | Modifiers::SHIFT;
    let ca = Modifiers::COMMAND | Modifiers::ALT;
    let cas = Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT;
    let mut v = vec![
        (sc(cas, Key::C), Action::CanvasSize),
        (sc(ca, Key::C), Action::Copy(CopyAction::CopyAndClose, None)),
        (sc(cs, Key::C), Action::Copy(CopyAction::CopyAndClear, None)),
        (sc(ca, Key::V), Action::PasteNewImage),
        (sc(cs, Key::V), Action::Paste),
        (sc(ca, Key::I), Action::ResizeImage),
        (sc(ca, Key::T), Action::ToggleTools),
        (sc(ca, Key::L), Action::ToggleLayers),
        (sc(c, Key::O), Action::Open),
        (sc(cs, Key::O), Action::OpenProject),
        (sc(cs, Key::Backspace), Action::ClearWorkspace),
        (sc(c, Key::S), Action::SaveProject),
        (sc(cs, Key::S), Action::SaveProjectAs),
        (sc(cs, Key::E), Action::Export(Format::Png, 1.0)),
        (sc(cs, Key::Z), Action::Redo),
        (sc(c, Key::Y), Action::Redo),
        (sc(c, Key::Z), Action::Undo),
        (sc(c, Key::A), Action::SelectAll),
        (sc(c, Key::D), Action::Deselect),
        (sc(cs, Key::I), Action::InvertSelection),
        (sc(cs, Key::B), Action::RemoveBg(RemovalMode::Fast)),
        (sc(c, Key::B), Action::RemoveBg(RemovalMode::Best)),
        (sc(c, Key::R), Action::Rotate(true)),
        (sc(c, Key::L), Action::Rotate(false)),
        (sc(c, Key::J), Action::DuplicateLayer),
        (sc(c, Key::E), Action::MergeDown),
        (sc(cs, Key::CloseBracket), Action::LayerOrder("top")),
        (sc(cs, Key::OpenBracket), Action::LayerOrder("bottom")),
        (sc(c, Key::CloseBracket), Action::LayerOrder("up")),
        (sc(c, Key::OpenBracket), Action::LayerOrder("down")),
        (sc(c, Key::Plus), Action::ZoomIn),
        (sc(c, Key::Equals), Action::ZoomIn),
        (sc(c, Key::Minus), Action::ZoomOut),
        (sc(c, Key::Num0), Action::Fit),
        (sc(c, Key::Num1), Action::Actual),
        (sc(c, Key::Comma), Action::Settings),
        (sc(Modifiers::NONE, Key::Backslash), Action::Compare),
        (sc(Modifiers::NONE, Key::F1), Action::Shortcuts),
    ];
    // Most specific (most modifiers) first so Ctrl+Shift+Z is not eaten by Ctrl+Z.
    v.sort_by_key(|(s, _)| std::cmp::Reverse([s.modifiers.command, s.modifiers.shift, s.modifiers.alt].iter().filter(|b| **b).count()));
    v
}

// --------------------------------------------------------------------- app ---

pub struct App {
    pub ctx: egui::Context,
    pub doc: Doc,
    pub settings: Settings,
    saved_settings: Settings,
    pub tool: Tool,
    pub sel_mode: SelMode,
    pub toast: Option<Toast>,
    pub busy: Option<Busy>,
    pub compare_on: bool,
    pub drop_targeted: bool,
    pub sheet: Option<Sheet>,
    pub panel: PanelKind,
    pub selected_layers: BTreeSet<i64>,
    pub project_path: Option<PathBuf>,
    pub outline: Vec<f32>,
    last_sel_key: (bool, Option<crate::engine::IRect>, i64),
    pub thumbs: HashMap<i64, (i64, TextureHandle)>,
    pub canvas: CanvasState,
    pub removal: Option<RemovalJob>,
    pub refine: RefineState,
    pub adjust: AdjustState,
    pub palette: Palette,
    pub icon_tex: Option<TextureHandle>,
    pub wordmark: Option<(TextureHandle, egui::Vec2)>,
    pub model_task: Option<ModelTask>,
    pub model_msg: Option<String>,
    pub font_list: Option<Vec<String>>,
    pub active_backend: Option<String>,
    pub quit: bool,
    pub renaming: Option<(i64, String)>,
    pub layer_drag: Option<i64>,
    pub opacity_editing: bool,
    pub first_frame: Option<Instant>,
    pub startup_probe: Option<PathBuf>,
    pub reduce_motion: bool,
    pub parent: Option<crate::platform::ParentWindow>,
    pub script: Option<crate::selftest::Script>,
    pub shot_slot: Arc<std::sync::Mutex<Option<Arc<egui::ColorImage>>>>,
    #[cfg(windows)]
    v_was_down: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Vec<PathBuf>) -> App {
        let ctx = cc.egui_ctx.clone();
        theme::install_fonts(&ctx);
        theme::apply(&ctx);
        let settings = Settings::load();
        let tool = Tool::from_id(&settings.tool).unwrap_or(Tool::Move);
        let mut app = App {
            ctx: ctx.clone(),
            doc: Doc::new(),
            saved_settings: settings.clone(),
            settings,
            tool,
            sel_mode: SelMode::Replace,
            toast: None,
            busy: None,
            compare_on: false,
            drop_targeted: false,
            sheet: None,
            panel: PanelKind::None,
            selected_layers: BTreeSet::new(),
            project_path: None,
            outline: vec![],
            last_sel_key: (false, None, -1),
            thumbs: HashMap::new(),
            canvas: CanvasState::new(),
            removal: None,
            refine: RefineState::default(),
            adjust: AdjustState::default(),
            palette: Palette::get(true),
            icon_tex: None,
            wordmark: None,
            model_task: None,
            model_msg: None,
            font_list: None,
            active_backend: None,
            quit: false,
            renaming: None,
            layer_drag: None,
            opacity_editing: false,
            first_frame: None,
            startup_probe: std::env::var_os("CUTOUT_STARTUP_PROBE").map(PathBuf::from),
            reduce_motion: false,
            parent: crate::platform::ParentWindow::from_creation(cc),
            script: crate::selftest::Script::from_env(),
            shot_slot: Arc::new(std::sync::Mutex::new(None)),
            #[cfg(windows)]
            v_was_down: false,
        };
        app.apply_appearance();
        app.reduce_motion = !crate::platform::sysinfo::animations_enabled();
        if app.reduce_motion {
            ctx.all_styles_mut(|s| s.animation_time = 0.0);
        }
        if !open.is_empty() {
            app.open_paths(&open, false);
        }
        app
    }

    pub fn is_empty(&self) -> bool {
        self.doc.state.is_empty()
    }

    pub fn state(&self) -> &State {
        &self.doc.state
    }

    pub fn dark(&self) -> bool {
        self.palette.dark
    }

    pub fn apply_appearance(&mut self) {
        let pref = match self.settings.appearance {
            Appearance::System => egui::ThemePreference::System,
            Appearance::Light => egui::ThemePreference::Light,
            Appearance::Dark => egui::ThemePreference::Dark,
        };
        self.ctx.set_theme(pref);
    }

    pub fn save_settings_if_changed(&mut self) {
        self.settings.tool = self.tool.id().to_string();
        if self.settings != self.saved_settings {
            self.settings.save();
            self.saved_settings = self.settings.clone();
        }
    }

    // ------------------------------------------------------------ engine ---

    /// Push pending pixels to the canvas and refresh UI state.
    pub fn refresh(&mut self, full: bool) {
        self.canvas_update_from_engine(full);
        self.doc.refresh_state();
        let st = &self.doc.state;
        let key = (st.has_selection, st.selection_bounds, st.version);
        if key != self.last_sel_key || full {
            self.outline = if st.has_selection { self.doc.selection_outline() } else { vec![] };
        }
        self.last_sel_key = key;
        let ids: BTreeSet<i64> = self.doc.state.layers.iter().map(|l| l.id).collect();
        self.selected_layers.retain(|i| ids.contains(i));
        self.ctx.request_repaint();
    }

    /// Run an engine command and refresh. Errors appear as a toast.
    pub fn run(&mut self, cmd: &str, args: Value) -> Option<Value> {
        self.run_ex(cmd, args, true, false)
    }

    pub fn run_ex(&mut self, cmd: &str, args: Value, refresh: bool, silent: bool) -> Option<Value> {
        match self.doc.run(cmd, args) {
            Ok(r) => {
                if refresh {
                    self.refresh(false);
                }
                Some(r)
            }
            Err(e) => {
                if !silent {
                    self.show_toast(&e, crate::icons::TRIANGLE_ALERT, true);
                }
                if refresh {
                    self.refresh(false);
                }
                None
            }
        }
    }

    pub fn show_toast(&mut self, message: &str, icon: char, error: bool) {
        self.toast = Some(Toast { message: message.to_string(), icon, error, born: Instant::now(), ttl: Duration::from_millis(if error { 3500 } else { 1800 }) });
        self.ctx.request_repaint();
    }

    pub fn error_toast(&mut self, message: &str) {
        self.show_toast(message, crate::icons::TRIANGLE_ALERT, true);
    }

    pub fn thumbnail(&mut self, layer: &LayerInfo) -> Option<egui::TextureId> {
        let key = self.doc.state.version.wrapping_add(layer.id);
        if let Some((k, t)) = self.thumbs.get(&layer.id) {
            if *k == key {
                return Some(t.id());
            }
        }
        let img = self.doc.thumbnail(layer.id, 96)?;
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.data);
        let tex = self.ctx.load_texture(format!("thumb-{}", layer.id), ci, egui::TextureOptions::LINEAR);
        let id = tex.id();
        self.thumbs.insert(layer.id, (key, tex));
        Some(id)
    }

    // ------------------------------------------------------------ import ---

    /// A native file dialog owned by (modal to) the main window.
    pub fn file_dialog(&self) -> rfd::FileDialog {
        let d = rfd::FileDialog::new();
        match &self.parent {
            Some(p) => d.set_parent(p),
            None => d,
        }
    }

    pub fn open_dialog(&mut self) {
        let mut exts: Vec<&str> = codec::READABLE_EXTENSIONS.to_vec();
        exts.push("cutout");
        let files = self.file_dialog().set_title("Choose images or a CutOut project").add_filter("Images and CutOut projects", &exts).pick_files();
        if let Some(f) = files {
            let as_layers = !self.is_empty();
            self.open_paths(&f, as_layers);
        }
    }

    pub fn open_paths(&mut self, paths: &[PathBuf], as_layers: bool) {
        let mut first = !as_layers;
        for p in paths {
            if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("cutout")).unwrap_or(false) {
                self.open_project(p, false);
                first = false;
                continue;
            }
            match codec::decode_file(p) {
                Ok(d) => {
                    let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Image".into());
                    if let Err(e) = self.import_image(d.image, &name, !first) {
                        self.error_toast(&e);
                    } else {
                        if d.downscaled {
                            self.show_toast("Very large image was downscaled to fit 120 MP", crate::icons::MAXIMIZE, false);
                        }
                        first = false;
                    }
                }
                Err(e) => self.error_toast(&e),
            }
        }
    }

    pub fn import_image(&mut self, img: Image, name: &str, as_layer: bool) -> Result<(), String> {
        let was_empty = self.is_empty();
        self.doc.import(img, name, as_layer && !was_empty)?;
        if was_empty || !as_layer {
            self.project_path = None;
        }
        self.refresh(was_empty || !as_layer);
        if was_empty || !as_layer {
            self.canvas.zoom_to_fit(false);
        }
        Ok(())
    }

    /// Ctrl+V: paste as a new layer (or a new document when empty).
    pub fn paste(&mut self, as_new_document: bool) {
        match clipboard::read() {
            Ok(items) => {
                for (i, item) in items.into_iter().enumerate() {
                    if let Err(e) = self.import_image(item.image, &item.name, !(as_new_document && i == 0)) {
                        self.error_toast(&e);
                        return;
                    }
                }
                if !self.is_empty() && !as_new_document && self.state().layers.len() > 1 {
                    self.show_toast("Pasted as a new layer", crate::icons::COPY_PLUS, false);
                }
            }
            Err(e) => self.show_toast(&e, crate::icons::CLIPBOARD, true),
        }
    }

    // ------------------------------------------------------------- copy ---

    pub fn copy(&mut self, action: CopyAction, format: Option<Format>, force: bool) {
        let Some(img) = (if self.is_empty() { None } else { self.doc.export(false) }) else {
            self.show_toast("Nothing to copy yet", crate::icons::CLIPBOARD, true);
            return;
        };
        let fmt = format.unwrap_or_else(|| Format::from_clip(self.settings.clipboard_format));
        if fmt == Format::Jpeg && !force && img.has_transparency() && !self.settings.jpeg_no_warn {
            self.sheet = Some(Sheet::JpegWarning { action, dont_ask: false });
            return;
        }
        if let Err(e) = clipboard::write(&img, fmt) {
            // Never clear or close if the clipboard write failed.
            self.show_toast(&format!("Copy failed: {e}"), crate::icons::TRIANGLE_ALERT, true);
            return;
        }
        match action {
            CopyAction::Copy => self.show_toast(&format!("Copied {} to clipboard", fmt.label()), crate::icons::CIRCLE_CHECK, false),
            CopyAction::CopyAndClear => {
                self.clear_workspace(true);
                self.show_toast("Copied — workspace cleared", crate::icons::CIRCLE_CHECK, false);
            }
            CopyAction::CopyAndClose => {
                self.save_recovery();
                self.quit = true;
            }
        }
    }

    // -------------------------------------------------------- workspace ---

    pub fn clear_workspace(&mut self, save_recovery: bool) {
        if save_recovery {
            self.save_recovery();
        }
        self.cancel_removal();
        let _ = self.doc.run("clear", json!({}));
        self.project_path = None;
        self.compare_on = false;
        self.canvas.set_compare(false, None);
        self.panel = PanelKind::None;
        self.thumbs.clear();
        self.selected_layers.clear();
        self.refine = RefineState::default();
        self.adjust = AdjustState::default();
        self.refresh(true);
    }

    pub fn has_recovery(&self) -> bool {
        paths::recovery_file().is_file()
    }

    /// Quietly keep the last workspace so Copy & Clear / Copy & Close and
    /// quitting never lose work without asking.
    pub fn save_recovery(&mut self) {
        if self.is_empty() || !(self.state().modified || self.project_path.is_none()) {
            return;
        }
        let p = paths::recovery_file();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = self.doc.run("save_project", json!({"path": p.to_string_lossy(), "recovery": true}));
    }

    pub fn restore_last_session(&mut self) {
        if self.has_recovery() {
            let p = paths::recovery_file();
            self.open_project(&p, true);
        }
    }

    // ---------------------------------------------------------- projects ---

    pub fn open_project(&mut self, path: &std::path::Path, recovery: bool) {
        match self.doc.run("open_project", json!({"path": path.to_string_lossy()})) {
            Ok(_) => {
                self.project_path = if recovery { None } else { Some(path.to_path_buf()) };
                self.thumbs.clear();
                self.refresh(true);
                self.canvas.zoom_to_fit(false);
            }
            Err(e) => self.error_toast(&e),
        }
    }

    pub fn open_project_dialog(&mut self) {
        if let Some(p) = self.file_dialog().add_filter("CutOut project", &["cutout"]).pick_file() {
            self.open_project(&p, false);
        }
    }

    pub fn suggested_name(&self) -> String {
        if let Some(p) = &self.project_path {
            return p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        }
        let n = self.state().layers.first().map(|l| l.name.clone()).unwrap_or_else(|| "Image".into());
        if n.is_empty() { "CutOut Image".into() } else { n }
    }

    pub fn save_project(&mut self, save_as: bool) {
        if self.is_empty() {
            return;
        }
        let mut path = self.project_path.clone();
        if path.is_none() || save_as {
            let p = self.file_dialog().add_filter("CutOut project", &["cutout"]).set_file_name(format!("{}.cutout", sanitize(&self.suggested_name()))).save_file();
            match p {
                Some(mut p) => {
                    if p.extension().is_none() {
                        p.set_extension("cutout");
                    }
                    path = Some(p)
                }
                None => return,
            }
        }
        let Some(path) = path else { return };
        match self.doc.run("save_project", json!({"path": path.to_string_lossy()})) {
            Ok(_) => {
                let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.project_path = Some(path);
                self.refresh(false);
                self.show_toast(&format!("Saved {name}"), crate::icons::CIRCLE_CHECK, false);
            }
            Err(e) => self.error_toast(&format!("Save failed: {e}")),
        }
    }

    pub fn export(&mut self, format: Format, scale: f64) {
        if self.is_empty() {
            return;
        }
        let Some(mut img) = self.doc.export(false) else { return };
        if (scale - 1.0).abs() > f64::EPSILON {
            let w = ((img.width as f64 * scale).round() as u32).max(1);
            let h = ((img.height as f64 * scale).round() as u32).max(1);
            match Doc::resize(&img, w, h) {
                Some(r) => img = r,
                None => return,
            }
        }
        let suffix = if format == Format::Png { " (cutout)" } else { "" };
        let name = format!("{}{}.{}", sanitize(&self.suggested_name()), suffix, format.ext());
        let filter: &[&str] = if format == Format::Png { &["png"] } else { &["jpg", "jpeg"] };
        let Some(mut path) = self.file_dialog().add_filter(format.label(), filter).set_file_name(name).save_file() else { return };
        if path.extension().is_none() {
            path.set_extension(format.ext());
        }
        match codec::encode(&img, format) {
            Ok(bytes) => match write_atomic(&path, &bytes) {
                Ok(()) => {
                    let n = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    self.show_toast(&format!("Exported {n}"), crate::icons::DOWNLOAD, false)
                }
                Err(e) => self.error_toast(&format!("Export failed: {e}")),
            },
            Err(_) => self.error_toast("Could not encode the image"),
        }
    }

    // -------------------------------------------------------------- undo ---

    pub fn undo(&mut self) {
        if self.doc.run("undo", json!({})).is_ok() {
            self.refresh(false);
        }
    }

    pub fn redo(&mut self) {
        if self.doc.run("redo", json!({})).is_ok() {
            self.refresh(false);
        }
    }

    // ------------------------------------------------ background removal ---

    pub fn is_removing(&self) -> bool {
        self.removal.is_some()
    }

    pub fn remove_background(&mut self, mode: RemovalMode) {
        self.remove_background_ex(mode, false);
    }

    /// `force` skips the free-memory warning ("Continue anyway").
    pub fn remove_background_ex(&mut self, mode: RemovalMode, force: bool) {
        if self.is_empty() || self.removal.is_some() {
            return;
        }
        let Some(layer) = self.state().active_layer().cloned().or_else(|| self.state().layers.last().cloned()) else { return };
        let found = match mode {
            RemovalMode::Best => models::best_spec(&self.settings.best_model),
            RemovalMode::Fast => models::fast_spec(),
        };
        let Some((spec, path)) = found else {
            self.sheet = Some(Sheet::ModelSetup);
            return;
        };
        if !force && spec.id != models::BIREFNET_LITE.id {
            if let Some(avail) = crate::platform::sysinfo::available_memory() {
                if avail < 9_500_000_000 {
                    self.sheet = Some(Sheet::LowMemory { mode, avail_gb: avail as f64 / 1e9 });
                    return;
                }
            }
        }
        let Some(pixels) = self.doc.layer_pixels(layer.id) else { return };
        let (w, h) = (pixels.width, pixels.height);
        self.busy = Some(Busy { title: if mode == RemovalMode::Best { "Removing background…".into() } else { "Removing background (Fast)…".into() }, cancellable: true });
        let removal = models::start_removal(pixels, *spec, path, self.settings.use_directml);
        self.removal = Some(RemovalJob { removal, layer: layer.id, started: Instant::now(), w, h });
        self.ctx.request_repaint();
    }

    pub fn cancel_removal(&mut self) {
        if let Some(j) = self.removal.take() {
            j.removal.cancel();
        }
        self.busy = None;
    }

    fn poll_removal(&mut self) {
        let Some(job) = &self.removal else { return };
        let msg = job.removal.rx.try_recv();
        let Ok(res) = msg else {
            self.ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        let job = self.removal.take().unwrap();
        self.busy = None;
        match res {
            Ok((alpha, backend)) => {
                self.active_backend = Some(backend);
                self.canvas_prepare_reveal();
                match self.doc.set_layer_mask(job.layer, alpha, job.w, job.h, "Remove Background") {
                    Ok(()) => {
                        self.refresh(false);
                        let secs = job.started.elapsed().as_secs_f64();
                        self.show_toast(&format!("Background removed in {secs:.1} s"), crate::icons::SPARKLES, false);
                    }
                    Err(e) => self.error_toast(&e),
                }
            }
            Err(e) => {
                if e != "Cancelled" {
                    self.error_toast(&e);
                }
            }
        }
    }

    // ------------------------------------------------------------ layers ---

    pub fn select_layer(&mut self, id: i64, extend: bool) {
        if extend {
            if !self.selected_layers.remove(&id) {
                self.selected_layers.insert(id);
            }
        } else {
            self.selected_layers = BTreeSet::from([id]);
        }
        self.run("select_layer", json!({"id": id}));
    }

    pub fn merge_selected_or_down(&mut self) {
        if self.selected_layers.len() >= 2 {
            let ids: Vec<i64> = self.selected_layers.iter().copied().collect();
            self.run("merge_layers", json!({"ids": ids}));
            self.selected_layers.clear();
        } else if let Some(a) = self.state().active {
            self.run("merge_down", json!({"id": a}));
        }
    }

    pub fn set_opacity(&mut self, id: i64, v: f64, begin: bool) {
        if begin {
            self.run_ex("checkpoint", json!({"label": "Layer Opacity"}), false, false);
        }
        self.run("set_layer", json!({"id": id, "opacity": v, "record": false}));
    }

    // -------------------------------------------------------------- text ---

    pub fn commit_text(&mut self, props: TextProps, layer: Option<i64>, point: (f64, f64)) {
        let Some(img) = crate::text::render(&props) else {
            self.error_toast("That text is too large to render.");
            return;
        };
        let res = match layer {
            Some(id) => self.doc.eng.update_text_layer(id, props, img.data, img.width, img.height).map(|_| id),
            None => self.doc.eng.add_text_layer(props, img.data, img.width, img.height, point.0, point.1),
        };
        match res {
            Ok(id) => {
                self.selected_layers = BTreeSet::from([id]);
                self.refresh(false);
            }
            Err(e) => self.error_toast(&e),
        }
    }

    pub fn open_text_sheet(&mut self, layer: Option<i64>, point: (f64, f64)) {
        let mut sheet = TextSheet { layer, point, text: String::new(), align: "left".into(), font_filter: String::new() };
        if let Some(id) = layer {
            if let Some(t) = self.state().layers.iter().find(|l| l.id == id).and_then(|l| l.text.clone()) {
                sheet.text = t.text.clone();
                sheet.align = if t.align.is_empty() { "left".into() } else { t.align.clone() };
                self.settings.tools.font_name = t.font;
                self.settings.tools.font_size = t.size;
                self.settings.tools.bold = t.bold;
                self.settings.tools.italic = t.italic;
                self.settings.tools.color = [t.color[0] as f32 / 255.0, t.color[1] as f32 / 255.0, t.color[2] as f32 / 255.0, t.color[3] as f32 / 255.0];
            }
        }
        if self.font_list.is_none() {
            self.font_list = Some(crate::text::families());
        }
        self.sheet = Some(Sheet::Text(sheet));
    }

    // ----------------------------------------------------------- actions ---

    pub fn enabled(&self, a: &Action) -> bool {
        let empty = self.is_empty();
        let st = self.state();
        match a {
            Action::RestoreSession => self.has_recovery(),
            Action::ClearWorkspace | Action::SaveProject | Action::SaveProjectAs | Action::Export(..) | Action::Copy(..) | Action::SelectAll | Action::InvertSelection | Action::Compare | Action::Trim | Action::ResizeImage | Action::CanvasSize | Action::Rotate(_) | Action::Flip(_) | Action::DuplicateLayer | Action::Flatten | Action::LayerOrder(_) | Action::RestoreAll | Action::ApplyMask | Action::RefineEdges | Action::ZoomIn | Action::ZoomOut | Action::Fit | Action::Actual | Action::ToggleTools | Action::ToggleLayers => !empty,
            Action::Undo => st.can_undo,
            Action::Redo => st.can_redo,
            Action::Deselect | Action::DeleteSelection | Action::KeepOnlySelection | Action::CropToSelection => st.has_selection,
            Action::RemoveBg(_) => !empty && !self.is_removing(),
            Action::DeleteLayer | Action::MergeDown => st.layers.len() > 1,
            Action::RasterizeText => st.active_layer().map(|l| l.is_text).unwrap_or(false),
            _ => true,
        }
    }

    pub fn perform(&mut self, a: Action) {
        if !self.enabled(&a) {
            return;
        }
        match a {
            Action::Open => self.open_dialog(),
            Action::OpenProject => self.open_project_dialog(),
            Action::RestoreSession => self.restore_last_session(),
            Action::ClearWorkspace => self.clear_workspace(true),
            Action::SaveProject => self.save_project(false),
            Action::SaveProjectAs => self.save_project(true),
            Action::Export(f, s) => self.export(f, s),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Copy(c, f) => self.copy(c, f, false),
            Action::Paste => self.paste(false),
            Action::PasteNewImage => self.paste(true),
            Action::SelectAll => {
                self.run("select_all", json!({}));
            }
            Action::Deselect => {
                self.run("deselect", json!({}));
            }
            Action::InvertSelection => {
                self.run("invert_selection", json!({}));
            }
            Action::DeleteSelection => {
                self.run("delete_selection", json!({}));
            }
            Action::KeepOnlySelection => {
                self.run("mask_from_selection", json!({}));
            }
            Action::RemoveBg(m) => self.remove_background(m),
            Action::RefineEdges => {
                self.settings.show_tools = true;
                self.panel = PanelKind::Refine;
            }
            Action::Compare => {
                self.compare_on = !self.compare_on;
                self.apply_compare();
            }
            Action::CropToSelection => {
                self.run("crop_to_selection", json!({}));
                self.canvas.zoom_to_fit(true);
            }
            Action::Trim => {
                self.run("trim", json!({}));
                self.canvas.zoom_to_fit(true);
            }
            Action::ResizeImage | Action::CanvasSize => {
                let canvas = a == Action::CanvasSize;
                let (w, h) = (self.state().width as i64, self.state().height as i64);
                self.sheet = Some(Sheet::Resize(ResizeSheet { canvas, width: w, height: h, keep_aspect: true, anchor: 4 }));
            }
            Action::Rotate(cw) => {
                self.run("rotate_canvas", json!({"direction": if cw { "cw" } else { "ccw" }}));
                self.canvas.zoom_to_fit(true);
            }
            Action::Flip(axis) => {
                self.run("flip_canvas", json!({"axis": axis}));
            }
            Action::ManageModels => self.sheet = Some(Sheet::ModelSetup),
            Action::DuplicateLayer => {
                self.run("duplicate_layer", json!({}));
            }
            Action::DeleteLayer => {
                self.run("delete_layer", json!({}));
            }
            Action::MergeDown => self.merge_selected_or_down(),
            Action::Flatten => {
                self.run("flatten", json!({}));
            }
            Action::LayerOrder(dir) => {
                if let Some(id) = self.state().active {
                    self.run("move_layer", json!({"id": id, "direction": dir}));
                }
            }
            Action::RestoreAll => {
                self.run("clear_mask", json!({}));
            }
            Action::ApplyMask => {
                self.run("apply_mask", json!({}));
            }
            Action::RasterizeText => {
                self.run("rasterize_layer", json!({}));
            }
            Action::ZoomIn => self.canvas.zoom_step(true),
            Action::ZoomOut => self.canvas.zoom_step(false),
            Action::Fit => self.canvas.zoom_to_fit(true),
            Action::Actual => self.canvas.set_zoom_device(1.0, None, true),
            Action::ToggleTools => self.settings.show_tools = !self.settings.show_tools,
            Action::ToggleLayers => self.settings.show_layers = !self.settings.show_layers,
            Action::Settings => self.sheet = Some(Sheet::Settings),
            Action::About => self.sheet = Some(Sheet::About),
            Action::Shortcuts => self.sheet = Some(Sheet::Shortcuts),
            Action::Quit => self.quit = true,
        }
    }

    pub fn set_tool(&mut self, t: Tool) {
        if self.tool != t {
            self.tool = t;
            self.canvas_tool_changed();
        }
    }

    // ------------------------------------------------------------ frame ---

    fn handle_global_input(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.sheet.is_some() {
            return;
        }
        let mut todo: Vec<Action> = vec![];
        ctx.input_mut(|i| {
            for (s, a) in shortcuts() {
                if i.consume_shortcut(&s) {
                    todo.push(a);
                }
            }
            let mut paste = false;
            let mut copy = false;
            for e in &i.events {
                match e {
                    egui::Event::Copy => copy = true,
                    egui::Event::Paste(_) | egui::Event::PasteImage(_) => paste = true,
                    _ => {}
                }
            }
            if copy {
                todo.push(Action::Copy(CopyAction::Copy, None));
            }
            if paste {
                todo.push(Action::Paste);
            }
        });
        for a in todo {
            self.perform(a);
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let (hovered, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
        self.drop_targeted = hovered;
        if !dropped.is_empty() {
            let mut paths: Vec<PathBuf> = vec![];
            for f in dropped {
                let p = f.path().to_path_buf();
                if p.is_file() {
                    paths.push(p);
                } else if let Ok(bytes) = f.bytes() {
                    if let Ok(d) = codec::decode_bytes(&bytes) {
                        let was_empty = self.is_empty();
                        let _ = self.import_image(d.image, "Dropped Image", !was_empty);
                    }
                }
            }
            if !paths.is_empty() {
                let as_layers = !self.is_empty();
                self.open_paths(&paths, as_layers);
            }
        }
    }

    fn poll_model_task(&mut self) {
        let mut finished: Option<Result<String, String>> = None;
        match &mut self.model_task {
            Some(ModelTask::Download { rx, progress, spec, .. }) => {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Ok(p) => *progress = p,
                        Err(r) => finished = Some(r.map(|_| format!("{} downloaded and verified.", spec.title))),
                    }
                }
            }
            Some(ModelTask::Install { rx, progress }) => {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Ok(p) => *progress = p,
                        Err(r) => finished = Some(r.map(|s| format!("{} installed.", s.title))),
                    }
                }
            }
            None => {}
        }
        if let Some(r) = finished {
            self.model_task = None;
            match r {
                Ok(m) => self.model_msg = Some(m),
                Err(e) => {
                    if e != "Cancelled" {
                        self.model_msg = Some(e)
                    }
                }
            }
        }
        if self.model_task.is_some() {
            self.ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub fn start_download(&mut self, spec: &'static models::ModelSpec) {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now();
            let r = models::download(spec, &mut |p| {
                if last.elapsed() > Duration::from_millis(100) {
                    last = Instant::now();
                    let _ = tx.send(Ok(p));
                    ctx.request_repaint();
                }
                !c2.load(Ordering::SeqCst)
            });
            let _ = tx.send(Err(r));
            ctx.request_repaint();
        });
        self.model_msg = None;
        self.model_task = Some(ModelTask::Download { spec, rx, cancel, progress: 0.0 });
    }

    pub fn start_install(&mut self, src: PathBuf) {
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now();
            let r = models::install_from_file(&src, &mut |p| {
                if last.elapsed() > Duration::from_millis(100) {
                    last = Instant::now();
                    let _ = tx.send(Ok(p));
                    ctx.request_repaint();
                }
                true
            });
            let _ = tx.send(Err(r));
            ctx.request_repaint();
        });
        self.model_msg = None;
        self.model_task = Some(ModelTask::Install { rx, progress: 0.0 });
    }

    /// One frame of application logic (called before drawing).
    pub fn frame_logic(&mut self, ctx: &egui::Context) {
        let dark = ctx.global_style().visuals.dark_mode;
        if dark != self.palette.dark || self.first_frame.is_none() {
            self.palette = Palette::get(dark);
            self.canvas.theme_changed();
        }
        if self.first_frame.is_none() {
            self.first_frame = Some(Instant::now());
            crate::text::preload();
        }
        let shot = self.shot_slot.lock().unwrap().take();
        if let Some(img) = shot {
            self.script_screenshot_received(&img);
        }
        self.script_tick(ctx);
        self.poll_removal();
        self.poll_model_task();
        self.handle_drops(ctx);
        self.handle_global_input(ctx);
        if let Some(t) = &self.toast {
            if t.born.elapsed() > t.ttl {
                self.toast = None;
            } else {
                ctx.request_repaint_after(t.ttl - t.born.elapsed());
            }
        }
        if self.quit {
            self.quit = false;
            self.save_recovery();
            self.settings.save();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_recovery();
            self.settings.save();
        }
    }

    fn remember_window(&mut self, ctx: &egui::Context) {
        let (inner, outer, maximized, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect, v.outer_rect, v.maximized.unwrap_or(false), v.minimized.unwrap_or(false))
        });
        if minimized {
            return;
        }
        self.settings.maximized = maximized;
        if !maximized {
            if let (Some(i), Some(o)) = (inner, outer) {
                let w = [o.min.x, o.min.y, i.width(), i.height()];
                if w.iter().all(|v| v.is_finite()) && w[2] >= 700.0 && w[3] >= 460.0 && self.settings.window != Some(w) {
                    self.settings.window = Some(w);
                }
            }
        }
    }

    pub fn finish_frame(&mut self, ctx: &egui::Context) {
        if self.script.is_none() {
            self.remember_window(ctx);
        }
        // Quietly persist preferences once the user stops dragging.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.save_settings_if_changed();
        }
        if let Some(path) = &self.startup_probe {
            if let Some(t0) = self.first_frame {
                if ctx.cumulative_frame_nr() >= 3 {
                    let ms = t0.elapsed().as_millis();
                    let _ = std::fs::write(path, format!("{{\"first_frames_ms\": {ms}}}\n"));
                    self.startup_probe = None;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    #[cfg(windows)]
    pub fn poll_paste_key(&mut self, raw: &mut egui::RawInput) {
        // egui-winit turns Ctrl+V into a Paste event only for text/bitmap clipboards; a file
        // copied in Explorer would otherwise do nothing, so detect the key edge ourselves.
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let down = unsafe { GetAsyncKeyState(0x56) } as u16 & 0x8000 != 0;
        let edge = down && !self.v_was_down;
        self.v_was_down = down;
        let key = |vk: i32| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0;
        let command_only = key(0x11) && !key(0x10) && !key(0x12); // Ctrl without Shift / Alt
        let focused = raw.viewports.get(&raw.viewport_id).and_then(|v| v.focused).unwrap_or(true);
        let has_paste = raw.events.iter().any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_)));
        if edge && !has_paste && focused && command_only {
            raw.events.push(egui::Event::Paste(String::new()));
        }
    }
}

pub fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() { "CutOut".into() } else { s }
}

/// Write via a temporary file and rename so a crash never leaves a truncated export.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("out")));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::ui::root(self, ui, frame);
        let ctx = ui.ctx().clone();
        self.finish_frame(&ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.palette.canvas.to_normalized_gamma_f32()
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, _raw: &mut egui::RawInput) {
        self.script_inject(_raw);
        #[cfg(windows)]
        self.poll_paste_key(_raw);
    }
}
