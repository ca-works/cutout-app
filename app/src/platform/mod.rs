//! Operating-system integration. Everything outside this module is portable.

pub mod clipboard;
pub mod gpu;
pub mod paths;
pub mod sysinfo;
#[cfg(windows)]
pub mod window;

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, WindowHandle};

/// The main window, so native dialogs are modal to it (and cannot open behind it).
#[derive(Clone, Copy)]
pub struct ParentWindow {
    window: RawWindowHandle,
    display: RawDisplayHandle,
}

impl ParentWindow {
    pub fn from_creation(cc: &eframe::CreationContext<'_>) -> Option<ParentWindow> {
        let window = cc.window_handle().ok()?.as_raw();
        let display = cc.display_handle().ok()?.as_raw();
        Some(ParentWindow { window, display })
    }
}

impl HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the handle belongs to the application window, which outlives every dialog.
        Ok(unsafe { WindowHandle::borrow_raw(self.window) })
    }
}

impl HasDisplayHandle for ParentWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(unsafe { DisplayHandle::borrow_raw(self.display) })
    }
}

