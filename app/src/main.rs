//! CutOut for Windows — a Rust desktop shell over `cutout-core`, the same
//! engine that powers the macOS app. (It also runs on macOS/Linux for
//! development; the shipping targets are Windows 10/11 x64 and ARM64.)

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod canvas;
mod codec;
mod engine;
mod flat_background;
mod icons;
mod models;
mod selftest;
mod platform;
mod settings;
mod text;
mod theme;
mod tiles;
mod ui;

use eframe::egui;
use std::path::PathBuf;

fn app_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../../assets/icon/CutOut-512.png")).ok()?.to_rgba8();
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

/// Restrict where Windows searches for DLLs (system folder and the application folder only),
/// so a DLL dropped in the current directory or the user-writable PATH can never be loaded.
#[cfg(windows)]
fn harden_dll_search() {
    use windows::Win32::System::LibraryLoader::{SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32};
    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_APPLICATION_DIR);
    }
}

fn main() -> eframe::Result {
    #[cfg(windows)]
    harden_dll_search();
    // Files passed on the command line (e.g. from "Open with" or a file association).
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let open: Vec<PathBuf> = args.iter().map(PathBuf::from).filter(|p| p.is_file()).collect();
    #[cfg(windows)]
    let reset = args.iter().any(|arg| arg == "--reset-window-position");
    #[cfg(windows)]
    let launch_monitor = platform::window::launch_monitor();
    // File associations still open separate documents. A launch without a file reuses CutOut.
    #[cfg(windows)]
    let instance = if open.is_empty() && std::env::var_os("CUTOUT_SCRIPT").is_none()
        && std::env::var_os("CUTOUT_STARTUP_PROBE").is_none()
    {
        match platform::window::Instance::acquire(reset) {
            Ok(instance) => instance,
            Err(()) => return Ok(()),
        }
    } else {
        None
    };
    let settings = settings::Settings::load();
    #[cfg(not(windows))]
    let size = settings.window.map(|w| [w[2].max(700.0), w[3].max(460.0)]).unwrap_or([1180.0, 780.0]);
    let mut vp = egui::ViewportBuilder::default().with_title("CutOut").with_drag_and_drop(true);
    #[cfg(windows)]
    {
        // Create a modest, normal window for wgpu. Win32 places it before eframe shows it.
        vp = vp.with_inner_size([800.0, 560.0]);
    }
    #[cfg(not(windows))]
    {
        vp = vp.with_inner_size(size).with_min_inner_size([700.0, 460.0]);
        if let Some(w) = settings.window {
            vp = vp.with_position([w[0], w[1]]);
        }
        if settings.maximized {
            vp = vp.with_maximized(true);
        }
    }
    if let Some(i) = app_icon() {
        vp = vp.with_icon(i);
    }
    let options = eframe::NativeOptions {
        viewport: vp,
        renderer: eframe::Renderer::Wgpu,
        centered: cfg!(not(windows)) && settings.window.is_none(),
        ..Default::default()
    };
    let result = eframe::run_native("CutOut", options, Box::new(|cc| {
        #[cfg(windows)]
        platform::window::initialize(cc, &settings, launch_monitor, reset, instance.as_ref());
        Ok(Box::new(app::App::new(cc, open, settings)))
    }));
    if let Err(e) = &result {
        // A GUI-subsystem executable has no console, so say what went wrong in a dialog.
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("CutOut could not start")
            .set_description(format!("The graphics system could not be initialised.\n\n{e}\n\nUpdate your graphics driver, or run CutOut on a PC with DirectX 12 or Vulkan support."))
            .show();
    }
    result
}
