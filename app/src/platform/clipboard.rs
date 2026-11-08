//! Clipboard integration. CutOut reads the clipboard only when the user pastes
//! and never monitors it.
//!
//! Windows publishes a copied image in three formats, in this order:
//!
//! 1. the registered **"PNG"** format (straight alpha; read by PowerPoint, Word,
//!    Outlook, Teams, Slack, browsers, Figma, Photoshop, Paint, …),
//! 2. **CF_DIBV5** — 32-bit BGRA with an alpha mask and an sRGB colour space
//!    (Paint.NET, GIMP, Affinity, many .NET/Win32 apps),
//! 3. **CF_DIB** — the same pixels flattened on white (legacy apps that do not
//!    understand alpha show white instead of black).
//!
//! The data is placed on the clipboard eagerly (no delayed rendering), so it
//! stays available after CutOut exits — which Copy & Close relies on.

use crate::codec;
use crate::engine::Image;

pub struct Item {
    pub image: Image,
    pub name: String,
    pub downscaled: bool,
}

#[cfg(windows)]
pub use win::{read, write};
#[cfg(not(windows))]
pub use dev::{read, write};

// ---------------------------------------------------------------- Windows ---

#[cfg(windows)]
mod win {
    use super::*;
    use crate::codec::Format;
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;
    const CF_DIBV5: u32 = 17;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn register(name: &str) -> u32 {
        let w = wide(name);
        unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) }
    }

    /// Holds the clipboard open; closes on drop.
    struct Open;

    impl Open {
        fn new() -> Result<Open, String> {
            for _ in 0..20 {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Open);
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err("The clipboard is in use by another program. Try again.".into())
        }
    }

    impl Drop for Open {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    fn set_bytes(format: u32, bytes: &[u8]) -> Result<(), String> {
        unsafe {
            let h: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| format!("Out of memory: {e}"))?;
            let p = GlobalLock(h);
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                return Err("Could not lock clipboard memory.".into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
            let _ = GlobalUnlock(h);
            // On success the system owns the memory.
            if SetClipboardData(format, Some(HANDLE(h.0))).is_err() {
                let _ = GlobalFree(Some(h));
                return Err("The clipboard rejected the image.".into());
            }
        }
        Ok(())
    }

    fn get_bytes(format: u32) -> Option<Vec<u8>> {
        unsafe {
            if IsClipboardFormatAvailable(format).is_err() {
                return None;
            }
            let h = GetClipboardData(format).ok()?;
            let hg = HGLOBAL(h.0);
            let size = GlobalSize(hg);
            let p = GlobalLock(hg);
            if p.is_null() || size == 0 {
                return None;
            }
            let v = std::slice::from_raw_parts(p as *const u8, size).to_vec();
            let _ = GlobalUnlock(hg);
            Some(v)
        }
    }

    fn le32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    /// 124-byte BITMAPV5HEADER + bottom-up BGRA pixels with straight alpha.
    pub(super) fn dibv5(img: &Image) -> Vec<u8> {
        let (w, h) = (img.width as usize, img.height as usize);
        let mut v = Vec::with_capacity(124 + w * h * 4);
        le32(&mut v, 124);
        le32(&mut v, img.width);
        le32(&mut v, img.height); // positive: bottom-up (widest compatibility)
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&32u16.to_le_bytes());
        le32(&mut v, 3); // BI_BITFIELDS
        le32(&mut v, (w * h * 4) as u32);
        le32(&mut v, 0);
        le32(&mut v, 0);
        le32(&mut v, 0);
        le32(&mut v, 0);
        le32(&mut v, 0x00FF_0000); // R
        le32(&mut v, 0x0000_FF00); // G
        le32(&mut v, 0x0000_00FF); // B
        le32(&mut v, 0xFF00_0000); // A
        le32(&mut v, 0x7352_4742); // LCS_sRGB ('sRGB')
        v.extend_from_slice(&[0u8; 36]); // endpoints
        le32(&mut v, 0);
        le32(&mut v, 0);
        le32(&mut v, 0); // gamma
        le32(&mut v, 4); // LCS_GM_IMAGES
        le32(&mut v, 0);
        le32(&mut v, 0);
        le32(&mut v, 0);
        debug_assert_eq!(v.len(), 124);
        for row in (0..h).rev() {
            for px in img.data[row * w * 4..(row + 1) * w * 4].chunks_exact(4) {
                v.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        v
    }

    /// 40-byte BITMAPINFOHEADER + bottom-up 32-bit BGRX, flattened on white.
    pub(super) fn dib_flat(img: &Image) -> Vec<u8> {
        let flat = img.flattened([255, 255, 255]);
        let (w, h) = (flat.width as usize, flat.height as usize);
        let mut v = Vec::with_capacity(40 + w * h * 4);
        le32(&mut v, 40);
        le32(&mut v, flat.width);
        le32(&mut v, flat.height);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&32u16.to_le_bytes());
        le32(&mut v, 0); // BI_RGB
        le32(&mut v, (w * h * 4) as u32);
        for _ in 0..4 {
            le32(&mut v, 0);
        }
        for row in (0..h).rev() {
            for px in flat.data[row * w * 4..(row + 1) * w * 4].chunks_exact(4) {
                v.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        v
    }

    pub fn write(img: &Image, format: Format) -> Result<(), String> {
        // Prepare everything before touching the clipboard so a failure never empties it.
        let png_fmt = register("PNG");
        let main = codec::encode(img, format)?;
        let (dib5, dib) = if format == Format::Jpeg {
            let flat = img.flattened([255, 255, 255]);
            (dibv5(&flat), dib_flat(&flat))
        } else {
            (dibv5(img), dib_flat(img))
        };
        let _open = Open::new()?;
        unsafe { EmptyClipboard() }.map_err(|e| format!("Could not clear the clipboard: {e}"))?;
        match format {
            Format::Png => set_bytes(png_fmt, &main)?,
            Format::Jpeg => {
                set_bytes(register("JFIF"), &main)?;
            }
        }
        set_bytes(CF_DIBV5, &dib5)?;
        set_bytes(CF_DIB, &dib)?;
        // Verify the data is really there.
        let verify = if format == Format::Png { png_fmt } else { register("JFIF") };
        match get_bytes(verify) {
            Some(b) if b.len() >= main.len() => Ok(()),
            _ => Err("The clipboard could not be verified.".into()),
        }
    }

    fn files() -> Vec<std::path::PathBuf> {
        unsafe {
            if IsClipboardFormatAvailable(CF_HDROP).is_err() {
                return vec![];
            }
            let Ok(h) = GetClipboardData(CF_HDROP) else { return vec![] };
            let drop = HDROP(h.0 as *mut c_void);
            let n = DragQueryFileW(drop, u32::MAX, None);
            let mut out = Vec::new();
            for i in 0..n.min(16) {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let got = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                out.push(std::path::PathBuf::from(String::from_utf16_lossy(&buf[..got])));
            }
            out
        }
    }

    /// Parse a CF_DIB / CF_DIBV5 block (24/32-bit; other depths go through the BMP decoder).
    pub(super) fn parse_dib(b: &[u8]) -> Option<Image> {
        let u32at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
        let hsize = u32at(0)? as usize;
        if hsize < 40 || b.len() < hsize {
            return None;
        }
        let w = u32at(4)? as i32;
        let hraw = u32at(8)? as i32;
        let bpp = u16::from_le_bytes([*b.get(14)?, *b.get(15)?]);
        let comp = u32at(16)?;
        let clr_used = u32at(32)? as usize;
        let (w, h, top_down) = (w.unsigned_abs() as usize, hraw.unsigned_abs() as usize, hraw < 0);
        if w == 0 || h == 0 || w > 32_768 || h > 32_768 {
            return None;
        }
        if !(bpp == 24 || bpp == 32) || !(comp == 0 || comp == 3) {
            // Palette / exotic formats: let the BMP decoder handle a synthesized file.
            let mut f = Vec::with_capacity(14 + b.len());
            f.extend_from_slice(b"BM");
            f.extend_from_slice(&((14 + b.len()) as u32).to_le_bytes());
            f.extend_from_slice(&[0, 0, 0, 0]);
            let palette = if bpp <= 8 { (if clr_used > 0 { clr_used } else { 1 << bpp }) * 4 } else { 0 };
            f.extend_from_slice(&((14 + hsize + palette) as u32).to_le_bytes());
            f.extend_from_slice(b);
            return super::codec::decode_bytes(&f).ok().map(|d| d.image);
        }
        let (mut rm, mut gm, mut bm, mut am) = (0x00FF_0000u32, 0x0000_FF00u32, 0x0000_00FFu32, 0u32);
        let mut off = hsize;
        if comp == 3 {
            if hsize >= 56 {
                rm = u32at(40)?;
                gm = u32at(44)?;
                bm = u32at(48)?;
                am = u32at(52)?;
            } else {
                rm = u32at(40)?;
                gm = u32at(44)?;
                bm = u32at(48)?;
                off += 12;
            }
        } else if hsize >= 56 {
            am = u32at(52)?; // V4/V5 may carry alpha even with BI_RGB
        }
        off += clr_used * 4;
        let bytes_pp = (bpp / 8) as usize;
        let stride = (w * bytes_pp + 3) & !3;
        if b.len() < off + stride * h {
            return None;
        }
        let shift = |m: u32| if m == 0 { 0 } else { m.trailing_zeros() };
        let ext = |v: u32, m: u32| -> u8 {
            if m == 0 {
                return 0;
            }
            let bits = (m >> shift(m)).count_ones().min(8);
            let x = (v & m) >> shift(m);
            if bits >= 8 { x as u8 } else { ((x * 255) / ((1u32 << bits) - 1)) as u8 }
        };
        let mut data = vec![0u8; w * h * 4];
        let mut any_alpha = false;
        for y in 0..h {
            let src_row = if top_down { y } else { h - 1 - y };
            let row = &b[off + src_row * stride..off + src_row * stride + w * bytes_pp];
            for x in 0..w {
                let px = &row[x * bytes_pp..x * bytes_pp + bytes_pp];
                let o = (y * w + x) * 4;
                if bytes_pp == 3 {
                    data[o] = px[2];
                    data[o + 1] = px[1];
                    data[o + 2] = px[0];
                    data[o + 3] = 255;
                } else {
                    let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                    data[o] = ext(v, rm);
                    data[o + 1] = ext(v, gm);
                    data[o + 2] = ext(v, bm);
                    let a = if am != 0 { ext(v, am) } else { 255 };
                    data[o + 3] = a;
                    any_alpha |= a != 0;
                }
            }
        }
        if bytes_pp == 4 && !any_alpha {
            for p in data.chunks_exact_mut(4) {
                p[3] = 255;
            }
        }
        Some(Image { width: w as u32, height: h as u32, data })
    }

    pub fn read() -> Result<Vec<Item>, String> {
        let _open = Open::new()?;
        let mut items = Vec::new();
        for p in files() {
            if let Ok(d) = codec::decode_file(&p) {
                let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Pasted Image".into());
                items.push(Item { image: d.image, name, downscaled: d.downscaled });
            }
        }
        if !items.is_empty() {
            return Ok(items);
        }
        for name in ["PNG", "image/png", "JFIF", "image/jpeg", "GIF", "image/webp"] {
            if let Some(b) = get_bytes(register(name)) {
                if let Ok(d) = codec::decode_bytes(&b) {
                    return Ok(vec![Item { image: d.image, name: "Pasted Image".into(), downscaled: d.downscaled }]);
                }
            }
        }
        for fmt in [CF_DIBV5, CF_DIB] {
            if let Some(b) = get_bytes(fmt) {
                if let Some(img) = parse_dib(&b) {
                    return Ok(vec![Item { image: img, name: "Pasted Image".into(), downscaled: false }]);
                }
            }
        }
        Err("The clipboard doesn't contain an image.".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn sample() -> Image {
            // 2×2: opaque red, half-transparent green / fully transparent, opaque blue
            Image { width: 2, height: 2, data: vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 0, 0, 0, 0, 255, 255] }
        }

        #[test]
        fn dibv5_roundtrip_keeps_alpha() {
            let img = sample();
            let back = parse_dib(&dibv5(&img)).expect("parse");
            assert_eq!((back.width, back.height), (2, 2));
            assert_eq!(back.data, img.data);
        }

        #[test]
        fn legacy_dib_is_flattened_on_white() {
            let back = parse_dib(&dib_flat(&sample())).expect("parse");
            assert_eq!(&back.data[8..12], &[255, 255, 255, 255]);
        }

        // Needs an interactive desktop session with a clipboard (true on GitHub's
        // Windows runners and on a normal Windows PC).
        #[test]
        #[ignore = "uses the real system clipboard"]
        fn real_clipboard_roundtrip() {
            let img = sample();
            write(&img, Format::Png).expect("write");
            let items = read().expect("read");
            assert_eq!(items[0].image.data, img.data, "PNG round trip keeps alpha exactly");
            let _open = Open::new().unwrap();
            assert!(get_bytes(CF_DIBV5).is_some() && get_bytes(CF_DIB).is_some());
        }
    }
}

// ------------------------------------------------------------- Development --

#[cfg(not(windows))]
mod dev {
    //! Development stand-in so the UI can be exercised on macOS/Linux. The
    //! shipping Windows build uses the implementation above.
    use super::*;
    use crate::codec::Format;

    pub fn write(img: &Image, format: Format) -> Result<(), String> {
        let src = if format == Format::Jpeg { img.flattened([255, 255, 255]) } else { img.clone() };
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        cb.set_image(arboard::ImageData { width: src.width as usize, height: src.height as usize, bytes: std::borrow::Cow::Borrowed(&src.data) }).map_err(|e| e.to_string())
    }

    pub fn read() -> Result<Vec<Item>, String> {
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        if let Ok(files) = cb.get().file_list() {
            let mut items = Vec::new();
            for p in files.into_iter().take(16) {
                if let Ok(d) = codec::decode_file(&p) {
                    let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Pasted Image".into());
                    items.push(Item { image: d.image, name, downscaled: d.downscaled });
                }
            }
            if !items.is_empty() {
                return Ok(items);
            }
        }
        match cb.get_image() {
            Ok(i) => Ok(vec![Item { image: Image { width: i.width as u32, height: i.height as u32, data: i.bytes.into_owned() }, name: "Pasted Image".into(), downscaled: false }]),
            Err(_) => Err("The clipboard doesn't contain an image.".into()),
        }
    }
}
