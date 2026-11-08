//! Free physical memory, used to warn before the memory-hungry best-quality
//! model is started on a PC that does not have enough.

/// Available physical memory in bytes, if it can be determined.
/// `CUTOUT_FAKE_AVAILABLE_MEM_MB` overrides it (tests).
pub fn available_memory() -> Option<u64> {
    if let Some(v) = std::env::var("CUTOUT_FAKE_AVAILABLE_MEM_MB").ok().and_then(|v| v.parse::<u64>().ok()) {
        return Some(v * 1_000_000);
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut s = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if unsafe { GlobalMemoryStatusEx(&mut s) }.is_ok() {
            return Some(s.ullAvailPhys);
        }
    }
    None
}

/// False when the user turned off "Show animations in Windows" (the system reduce-motion setting).
pub fn animations_enabled() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
        let mut on: i32 = 1;
        let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut i32 as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_ok();
        return !ok || on != 0;
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("CUTOUT_REDUCE_MOTION").is_none()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fake_value_is_honoured() {
        std::env::set_var("CUTOUT_FAKE_AVAILABLE_MEM_MB", "1234");
        assert_eq!(super::available_memory(), Some(1_234_000_000));
        std::env::remove_var("CUTOUT_FAKE_AVAILABLE_MEM_MB");
    }
}
