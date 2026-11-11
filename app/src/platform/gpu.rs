//! Which graphics adapter the AI model would run on, and whether it is worth using.
//!
//! DirectML runs on any DirectX 12 adapter, but that does not make every adapter a good choice:
//! the network needs several gigabytes of video memory, and on an integrated GPU (which borrows
//! system memory and is far slower) a removal can run into the driver's time-out and reset the
//! display adapter for the whole desktop. Measured on the development PC (docs/WINDOWS_PERFORMANCE.md):
//! an RTX 4070 Laptop GPU (8 GB) takes 0.3 s; the integrated Radeon 780M sometimes finished in
//! 3.6 s and sometimes hit the time-out; the CPU path is reliable at 6.8 s. So the GPU is used only
//! when the adapter Windows ranks as high-performance has enough *dedicated* memory.

/// The adapter DirectML will pick (same ranking as `PerformancePreference::HighPerformance`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub dedicated_bytes: u64,
}

/// Dedicated video memory an adapter needs before CutOut runs the model on it. The model peaks at
/// about 5.3 GB on an 8 GB card; "6 GB" cards report a little under 6 GiB. Not validated on cards
/// with less than 8 GB (none was available), which is why the limit is conservative.
/// `CUTOUT_DML_MIN_VRAM_MB` overrides it (tests, troubleshooting).
pub fn min_dedicated_bytes() -> u64 {
    std::env::var("CUTOUT_DML_MIN_VRAM_MB").ok().and_then(|v| v.parse::<u64>().ok()).map(|mb| mb * 1024 * 1024).unwrap_or(5_500 * 1024 * 1024)
}

/// The high-performance hardware adapter, if there is one (cached for the life of the process).
pub fn high_performance_adapter() -> Option<Adapter> {
    static CACHE: std::sync::OnceLock<Option<Adapter>> = std::sync::OnceLock::new();
    CACHE.get_or_init(query).clone()
}

/// True when the model should run on the GPU on this PC.
pub fn suitable_for_model() -> bool {
    high_performance_adapter().map_or(false, |a| a.dedicated_bytes >= min_dedicated_bytes())
}

#[cfg(windows)]
fn query() -> Option<Adapter> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE};
    // SAFETY: plain COM calls on interfaces owned by this function; DXGI needs no COM apartment.
    unsafe {
        let factory: IDXGIFactory6 = CreateDXGIFactory1().ok()?;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(i, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue; // WARP / Basic Render Driver
            }
            let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            return Some(Adapter { name: String::from_utf16_lossy(&desc.Description[..end]), dedicated_bytes: desc.DedicatedVideoMemory as u64 });
        }
        None
    }
}

#[cfg(not(windows))]
fn query() -> Option<Adapter> {
    None
}
