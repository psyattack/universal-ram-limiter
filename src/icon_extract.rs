//! Extract the icon from an .exe file on Windows.
//!
//! Two flavours are exposed:
//!
//! * [`extract_exe_icon`] — pulls the first icon out of an executable and
//!   writes it to a temporary `.ico` file for `nwg::ImageList`.
//! * [`extract_exe_icon_rgba`] — returns the icon as a `(width, height, RGBA)`
//!   tuple (kept around for any future GPU-texture-based UI).

use std::path::{Path, PathBuf};

/// Derive the process name (without .exe) from a full path.
pub fn process_name_from_path(exe_path: &str) -> String {
    Path::new(exe_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Try to extract the application icon from `exe_path` and save it as a
/// temporary `.ico` file. Returns the path to the temp ico on success.
#[cfg(target_os = "windows")]
pub fn extract_exe_icon(exe_path: &str) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::shared::windef::HICON;
    use winapi::um::shellapi::ExtractIconExW;
    use winapi::um::winuser::DestroyIcon;

    let wide_path: Vec<u16> = OsStr::new(exe_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut large_icon: HICON = std::ptr::null_mut();
        let extracted = ExtractIconExW(
            wide_path.as_ptr(),
            0,
            &mut large_icon,
            std::ptr::null_mut(),
            1,
        );
        if extracted == 0 || large_icon.is_null() {
            return None;
        }

        let hash = simple_hash(exe_path);
        let tmp_dir = std::env::temp_dir().join("ram_limiter_icons");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let ico_path = tmp_dir.join(format!("{hash:016x}.ico"));

        let ok = save_hicon_to_file(large_icon, &ico_path);
        DestroyIcon(large_icon);

        if ok {
            Some(ico_path)
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn extract_exe_icon(_exe_path: &str) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "windows")]
fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Save an HICON to an .ico file. Writes a single-image ICO from the bitmap
/// data behind the icon; sufficient for `nwg::ImageList::add_icon_from_filename`.
#[cfg(target_os = "windows")]
unsafe fn save_hicon_to_file(icon: winapi::shared::windef::HICON, path: &Path) -> bool {
    use std::io::Write;
    use winapi::um::wingdi::*;
    use winapi::um::winuser::{GetDC, GetIconInfo, ReleaseDC, ICONINFO};

    let mut info: ICONINFO = std::mem::zeroed();
    if GetIconInfo(icon, &mut info) == 0 {
        return false;
    }

    let hdc = GetDC(std::ptr::null_mut());
    let mut bmp: BITMAP = std::mem::zeroed();
    if GetObjectW(
        info.hbmColor as *mut _,
        std::mem::size_of::<BITMAP>() as i32,
        &mut bmp as *mut _ as *mut _,
    ) == 0
    {
        ReleaseDC(std::ptr::null_mut(), hdc);
        DeleteObject(info.hbmColor as _);
        DeleteObject(info.hbmMask as _);
        return false;
    }

    let w = bmp.bmWidth;
    let h = bmp.bmHeight;
    let bpp = 32i32;
    let row_size = ((w * bpp + 31) / 32) * 4;
    let img_size = (row_size * h) as usize;

    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = bpp as u16;
    bi.bmiHeader.biCompression = BI_RGB;

    let mut color_bits = vec![0u8; img_size];
    let got = GetDIBits(
        hdc,
        info.hbmColor,
        0,
        h as u32,
        color_bits.as_mut_ptr() as *mut _,
        &mut bi,
        DIB_RGB_COLORS,
    );

    let mask_row = ((w + 31) / 32) * 4;
    let mask_size = (mask_row * h) as usize;
    let mut mask_bits = vec![0xFFu8; mask_size];
    let mut mbi: BITMAPINFO = std::mem::zeroed();
    mbi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    mbi.bmiHeader.biWidth = w;
    mbi.bmiHeader.biHeight = h;
    mbi.bmiHeader.biPlanes = 1;
    mbi.bmiHeader.biBitCount = 1;
    mbi.bmiHeader.biCompression = BI_RGB;
    if !info.hbmMask.is_null() {
        GetDIBits(
            hdc,
            info.hbmMask,
            0,
            h as u32,
            mask_bits.as_mut_ptr() as *mut _,
            &mut mbi,
            DIB_RGB_COLORS,
        );
    }

    ReleaseDC(std::ptr::null_mut(), hdc);
    DeleteObject(info.hbmColor as _);
    DeleteObject(info.hbmMask as _);

    if got == 0 {
        return false;
    }

    let mut out = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let icon_dir_size = 6 + 16;
    let bmp_header_size = 40u32;
    let data_size = bmp_header_size + img_size as u32 + mask_size as u32;

    let _ = out.write_all(&0u16.to_le_bytes());
    let _ = out.write_all(&1u16.to_le_bytes());
    let _ = out.write_all(&1u16.to_le_bytes());
    let icon_w = if w >= 256 { 0u8 } else { w as u8 };
    let icon_h = if h >= 256 { 0u8 } else { h as u8 };
    let _ = out.write_all(&[icon_w]);
    let _ = out.write_all(&[icon_h]);
    let _ = out.write_all(&[0u8]);
    let _ = out.write_all(&[0u8]);
    let _ = out.write_all(&1u16.to_le_bytes());
    let _ = out.write_all(&(bpp as u16).to_le_bytes());
    let _ = out.write_all(&data_size.to_le_bytes());
    let _ = out.write_all(&(icon_dir_size as u32).to_le_bytes());
    let _ = out.write_all(&bmp_header_size.to_le_bytes());
    let _ = out.write_all(&w.to_le_bytes());
    let _ = out.write_all(&(h * 2).to_le_bytes());
    let _ = out.write_all(&1u16.to_le_bytes());
    let _ = out.write_all(&(bpp as u16).to_le_bytes());
    let _ = out.write_all(&0u32.to_le_bytes());
    let _ = out.write_all(&(img_size as u32 + mask_size as u32).to_le_bytes());
    let _ = out.write_all(&0u32.to_le_bytes());
    let _ = out.write_all(&0u32.to_le_bytes());
    let _ = out.write_all(&0u32.to_le_bytes());
    let _ = out.write_all(&0u32.to_le_bytes());
    let _ = out.write_all(&color_bits);
    let _ = out.write_all(&mask_bits);

    true
}
