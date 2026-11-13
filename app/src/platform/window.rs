//! Windows placement uses physical desktop coordinates and current monitor work areas.

use crate::settings::Settings;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};
use windows::core::{BOOL, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

const DEFAULT_SIZE: [f32; 2] = [1180.0, 780.0];
const SUBCLASS: usize = 1;
const FIRST_SHOW: u32 = WM_APP + 1;
const ACTIVATE: u32 = WM_APP + 2;
const RECOVER: u32 = WM_APP + 3;

fn hwnd(window: &impl HasWindowHandle) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as _)),
        _ => None,
    }
}

pub fn launch_monitor() -> HMONITOR {
    unsafe {
        let mut cursor = POINT::default();
        if GetCursorPos(&mut cursor).is_ok() {
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONULL);
            if !monitor.is_invalid() {
                return monitor;
            }
        }
        let foreground = GetForegroundWindow();
        if !foreground.is_invalid() {
            return MonitorFromWindow(foreground, MONITOR_DEFAULTTOPRIMARY);
        }
        MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY)
    }
}

#[derive(Clone, Copy)]
struct Display {
    handle: HMONITOR,
    work: RECT,
    dpi: u32,
    primary: bool,
}

fn displays() -> Vec<Display> {
    unsafe extern "system" fn collect(
        handle: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool()
            && width(info.rcWork) > 0
            && height(info.rcWork) > 0
        {
            // Used to migrate legacy points. New coordinates are saved directly in physical pixels.
            let (mut dpi, mut y) = (96, 96);
            let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut y) };
            let list = unsafe { &mut *(data.0 as *mut Vec<Display>) };
            list.push(Display {
                handle,
                work: info.rcWork,
                dpi: dpi.max(96),
                primary: info.dwFlags & 1 != 0,
            });
        }
        true.into()
    }

    let mut list = Vec::new();
    // EnumDisplayMonitors invokes the callback synchronously on this thread.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut list as *mut _ as isize),
        )
    };
    list
}

fn width(rect: RECT) -> i32 {
    rect.right.saturating_sub(rect.left)
}
fn height(rect: RECT) -> i32 {
    rect.bottom.saturating_sub(rect.top)
}

fn overlap(rect: RECT, work: RECT) -> i64 {
    let x = rect
        .right
        .min(work.right)
        .saturating_sub(rect.left.max(work.left))
        .max(0);
    let y = rect
        .bottom
        .min(work.bottom)
        .saturating_sub(rect.top.max(work.top))
        .max(0);
    i64::from(x) * i64::from(y)
}

fn caption_accessible(rect: RECT, display: Display) -> bool {
    let work = display.work;
    // GetWindowRect includes an invisible resize border. Allow that border at a snapped edge.
    let border = (8 * display.dpi / 96) as i32;
    rect.top >= work.top - border
        && rect.top.saturating_add((30 * display.dpi / 96) as i32) <= work.bottom
        && rect.right > work.left
        && rect.right <= work.right + border
        && overlap(rect, work) >= i64::from(width(rect)) * i64::from(height(rect)) / 2
}

fn fit(rect: RECT, work: RECT, center: bool) -> RECT {
    let w = width(rect).clamp(1, width(work));
    let h = height(rect).clamp(1, height(work));
    let (x, y) = if center {
        (
            work.left + (width(work) - w) / 2,
            work.top + (height(work) - h) / 2,
        )
    } else {
        (
            rect.left.clamp(work.left, work.right - w),
            rect.top.clamp(work.top, work.bottom - h),
        )
    };
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

fn client_size(settings: &Settings) -> [f32; 2] {
    settings
        .window
        .filter(|rect| rect.iter().all(|v| v.is_finite()) && rect[2] > 0.0 && rect[3] > 0.0)
        .map_or(DEFAULT_SIZE, |rect| [rect[2], rect[3]])
}

fn desired_rect(hwnd: HWND, display: Display, origin: [i32; 2], size: [f32; 2]) -> RECT {
    let scale = display.dpi as f32 / 96.0;
    let mut rect = RECT {
        right: (size[0] * scale).round().clamp(1.0, 100_000.0) as i32,
        bottom: (size[1] * scale).round().clamp(1.0, 100_000.0) as i32,
        ..Default::default()
    };
    unsafe {
        let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
        let ex_style = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
        let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, display.dpi);
    }
    RECT {
        left: origin[0],
        top: origin[1],
        right: origin[0].saturating_add(width(rect)),
        bottom: origin[1].saturating_add(height(rect)),
    }
}

fn place(hwnd: HWND, rect: RECT) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            width(rect),
            height(rect),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn preferred(list: &[Display], monitor: HMONITOR) -> Option<Display> {
    list.iter()
        .find(|d| d.handle == monitor)
        .or_else(|| list.iter().find(|d| d.primary))
        .or(list.first())
        .copied()
}

fn restore(hwnd: HWND, settings: &Settings, monitor: HMONITOR) -> bool {
    let list = displays();
    let Some(fallback) = preferred(&list, monitor) else {
        return false;
    };
    let size = client_size(settings);
    let candidate = |d: Display| {
        let origin = saved_origin(settings, d.dpi)?;
        // Match the monitor containing the saved title bar before deciding whether it fits.
        let title = RECT {
            left: origin[0],
            top: origin[1],
            right: origin[0].saturating_add(1),
            bottom: origin[1].saturating_add(24),
        };
        let rect = desired_rect(hwnd, d, origin, size);
        let title_on_display = overlap(title, d.work) > 0 || caption_accessible(rect, d);
        title_on_display.then_some((rect, d))
    };
    let saved = list
        .iter()
        .copied()
        .filter_map(candidate)
        .max_by_key(|(r, d)| overlap(*r, d.work));
    let (rect, display) = match saved {
        Some(pair) => pair,
        None => (desired_rect(hwnd, fallback, [0, 0], size), fallback),
    };
    if saved.is_some()
        && caption_accessible(rect, display)
        && width(rect) <= width(display.work)
        && height(rect) <= height(display.work)
    {
        place(hwnd, rect);
    } else {
        place(hwnd, fit(rect, display.work, saved.is_none()));
    }
    saved.is_some()
}

fn saved_origin(settings: &Settings, dpi: u32) -> Option<[i32; 2]> {
    settings.window_origin_px.or_else(|| {
        let rect = settings
            .window
            .filter(|r| r.iter().all(|v| v.is_finite()))?;
        let scale = dpi as f32 / 96.0;
        Some([
            (rect[0] * scale).round() as i32,
            (rect[1] * scale).round() as i32,
        ])
    })
}

fn recover(hwnd: HWND) {
    unsafe {
        // Minimized windows keep their state until a user restores them.
        if IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            return;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let list = displays();
        let display = list.iter().copied().max_by_key(|d| overlap(rect, d.work));
        let Some(display) = display else { return };
        let intersects = overlap(rect, display.work) > 0;
        if IsZoomed(hwnd).as_bool() {
            if !intersects {
                let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
                restore(hwnd, &Settings::default(), launch_monitor());
            }
        } else if !intersects {
            if let Some(fallback) = preferred(&list, launch_monitor()) {
                place(hwnd, fit(rect, fallback.work, true));
            }
        } else if !caption_accessible(rect, display)
            || width(rect) > width(display.work)
            || height(rect) > height(display.work)
        {
            place(hwnd, fit(rect, display.work, false));
        }
    }
}

pub fn remember(window: &impl HasWindowHandle, settings: &mut Settings) {
    let Some(hwnd) = hwnd(window) else { return };
    unsafe {
        if IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            return;
        }
        settings.maximized = IsZoomed(hwnd).as_bool();
        if settings.maximized {
            return;
        }
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        if GetWindowRect(hwnd, &mut outer).is_ok()
            && GetClientRect(hwnd, &mut client).is_ok()
            && width(client) > 0
            && height(client) > 0
        {
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            settings.window_origin_px = Some([outer.left, outer.top]);
            settings.window = Some([
                outer.left as f32 / scale,
                outer.top as f32 / scale,
                width(client) as f32 / scale,
                height(client) as f32 / scale,
            ]);
        }
    }
}

pub struct Instance {
    mutex: HANDLE,
    property: HSTRING,
}

impl Instance {
    /// The session-local mutex serializes ordinary launches, including while wgpu starts.
    /// An isolated settings profile has its own instance (portable installs and tests).
    pub fn acquire(reset: bool) -> Result<Option<Self>, ()> {
        let mut hash = DefaultHasher::new();
        super::paths::config_dir()
            .to_string_lossy()
            .to_lowercase()
            .hash(&mut hash);
        let name = format!("CutOut.13785F18.{:016x}", hash.finish());
        let Ok(mutex) =
            (unsafe { CreateMutexW(None, false, &HSTRING::from(format!("Local\\{name}"))) })
        else {
            return Ok(None);
        };
        let instance = Self {
            mutex,
            property: HSTRING::from(name),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match unsafe { WaitForSingleObject(mutex, 50) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => return Ok(Some(instance)),
                WAIT_TIMEOUT => {}
                _ => return Ok(None),
            }
            if instance.activate(reset) || Instant::now() >= deadline {
                return Err(());
            }
        }
    }

    fn activate(&self, reset: bool) -> bool {
        struct Search<'a> {
            property: &'a HSTRING,
            hwnd: HWND,
        }
        unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
            let search = unsafe { &mut *(data.0 as *mut Search<'_>) };
            if unsafe { GetPropW(hwnd, search.property) }.is_invalid() {
                return true.into();
            }
            search.hwnd = hwnd;
            false.into()
        }
        let mut search = Search {
            property: &self.property,
            hwnd: HWND::default(),
        };
        unsafe {
            let _ = EnumWindows(Some(find), LPARAM(&mut search as *mut _ as isize));
            if search.hwnd.is_invalid() {
                return false;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(search.hwnd, Some(&mut pid));
            if pid != 0 {
                let _ = AllowSetForegroundWindow(pid);
            }
            PostMessageW(
                Some(search.hwnd),
                ACTIVATE,
                WPARAM(usize::from(reset)),
                LPARAM(0),
            )
            .is_ok()
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
        }
    }
}

pub fn initialize(
    window: &impl HasWindowHandle,
    settings: &Settings,
    monitor: HMONITOR,
    reset: bool,
    instance: Option<&Instance>,
) {
    let Some(hwnd) = hwnd(window) else { return };
    let valid = if reset {
        restore(hwnd, &Settings::default(), monitor)
    } else {
        restore(hwnd, settings, monitor)
    };
    let maximize = valid && settings.maximized;
    unsafe {
        if SetWindowSubclass(
            hwnd,
            Some(window_proc),
            SUBCLASS,
            if maximize { 2 } else { 1 },
        )
        .as_bool()
        {
            if let Some(instance) = instance {
                // A non-null marker is sufficient. Windows removes the property when the window is destroyed.
                let _ = SetPropW(hwnd, &instance.property, Some(HANDLE(hwnd.0)));
            }
        }
    }
}

fn activate(hwnd: HWND) {
    unsafe {
        let popup = GetLastActivePopup(hwnd);
        let target = if !popup.is_invalid() && IsWindowVisible(popup).as_bool() {
            popup
        } else {
            hwnd
        };
        if !SetForegroundWindow(target).as_bool() && GetForegroundWindow() != target {
            let _ = FlashWindowEx(&FLASHWINFO {
                cbSize: size_of::<FLASHWINFO>() as u32,
                hwnd,
                dwFlags: FLASHW_TRAY,
                uCount: 2,
                dwTimeout: 0,
            });
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    first_show: usize,
) -> LRESULT {
    // Repositioning or activating can synchronously send more messages. Defer until winit
    // has finished processing the original event and released its window-state lock.
    unsafe {
        match msg {
            WM_WINDOWPOSCHANGED if first_show != 0 && IsWindowVisible(hwnd).as_bool() => {
                let _ = SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS, 0);
                let _ = PostMessageW(Some(hwnd), FIRST_SHOW, WPARAM(first_show), LPARAM(0));
            }
            FIRST_SHOW => {
                if wp.0 == 2 {
                    let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
                } else if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                recover(hwnd);
                activate(hwnd);
                return LRESULT(0);
            }
            ACTIVATE => {
                if IsIconic(hwnd).as_bool() || (wp.0 != 0 && IsZoomed(hwnd).as_bool()) {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                if wp.0 != 0 {
                    restore(hwnd, &Settings::default(), launch_monitor());
                }
                if first_show == 0 {
                    if !IsWindowVisible(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                    recover(hwnd);
                    activate(hwnd);
                }
                return LRESULT(0);
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE | WM_EXITSIZEMOVE => {
                let _ = PostMessageW(Some(hwnd), RECOVER, WPARAM(0), LPARAM(0));
            }
            WM_SIZE if wp.0 == SIZE_RESTORED as usize && first_show == 0 => {
                let _ = PostMessageW(Some(hwnd), RECOVER, WPARAM(0), LPARAM(0));
            }
            RECOVER => {
                recover(hwnd);
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn centers_and_shrinks_within_offset_work_area() {
        let work = rect(-1600, -1200, 0, -40);
        assert_eq!(
            fit(rect(0, 0, 3000, 900), work, true),
            rect(-1600, -1070, 0, -170)
        );
        assert_eq!(
            fit(rect(100, 80, 600, 580), rect(60, 0, 800, 420), true),
            rect(180, 0, 680, 420)
        );
    }

    #[test]
    fn validates_caption_and_controls_on_current_monitor() {
        let display = Display {
            handle: HMONITOR::default(),
            work: rect(-1920, 40, 0, 1040),
            dpi: 144,
            primary: false,
        };
        assert!(caption_accessible(rect(-1800, 60, -800, 860), display));
        assert!(caption_accessible(rect(-1930, 30, -960, 1050), display));
        assert!(!caption_accessible(rect(-1800, 0, -800, 850), display));
        assert!(!caption_accessible(rect(-500, 60, 100, 860), display));
        assert!(!caption_accessible(rect(1920, 60, 2920, 860), display));
        assert_eq!(
            fit(rect(-500, 60, 100, 860), display.work, false),
            rect(-600, 60, 0, 860)
        );
    }

    #[test]
    fn migrates_legacy_origins_at_each_monitor_scale() {
        let mut settings = Settings {
            window: Some([-900.0, 100.0, 1180.0, 780.0]),
            ..Settings::default()
        };
        for (dpi, origin) in [
            (96, [-900, 100]),
            (120, [-1125, 125]),
            (144, [-1350, 150]),
            (192, [-1800, 200]),
        ] {
            assert_eq!(saved_origin(&settings, dpi), Some(origin));
        }
        settings.window_origin_px = Some([-1350, 150]);
        assert_eq!(saved_origin(&settings, 192), Some([-1350, 150]));
        assert_eq!(client_size(&settings), [1180.0, 780.0]);
    }
}
