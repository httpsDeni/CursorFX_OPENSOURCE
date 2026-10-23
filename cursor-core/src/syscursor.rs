//! Renderiza cursores padrao do sistema em pixels.
//! Implementacao original sobre API publica (LoadCursor + GetIconInfo + GetDIBits).

use super::cur::CurImage;

#[cfg(windows)]
use windows::Win32::Graphics::Gdi::HBITMAP;

#[cfg(windows)]
fn idc_for_reg(reg_name: &str) -> Option<windows::core::PCWSTR> {
    use windows::Win32::UI::WindowsAndMessaging::*;
    Some(match reg_name {
        "Arrow" => IDC_ARROW,
        "Help" => IDC_HELP,
        "AppStarting" => IDC_APPSTARTING,
        "Wait" => IDC_WAIT,
        "Crosshair" => IDC_CROSS,
        "IBeam" => IDC_IBEAM,
        "NWPen" => windows::core::PCWSTR(32661u16 as _), // IDC_PEN
        "No" => IDC_NO,
        "SizeNS" => IDC_SIZENS,
        "SizeWE" => IDC_SIZEWE,
        "SizeNWSE" => IDC_SIZENWSE,
        "SizeNESW" => IDC_SIZENESW,
        "SizeAll" => IDC_SIZEALL,
        "UpArrow" => IDC_UPARROW,
        "Hand" => IDC_HAND,
        _ => return None,
    })
}

/// Imagem do cursor padrao da funcao. None fora do Windows ou sem suporte.
#[cfg(windows)]
pub fn system_cursor_image(reg_name: &str) -> Option<CurImage> {
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetIconInfo, LoadCursorW, HICON, ICONINFO,
    };

    let hcur = unsafe { LoadCursorW(None, idc_for_reg(reg_name)?).ok()? };
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(HICON(hcur.0), &mut info).ok()? };

    let img = unsafe { render_icon_info(&info) };

    unsafe {
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask.into());
        }
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(info.hbmColor.into());
        }
    }
    img
}

#[cfg(not(windows))]
pub fn system_cursor_image(_reg_name: &str) -> Option<CurImage> {
    None
}

#[cfg(windows)]
unsafe fn render_icon_info(info: &windows::Win32::UI::WindowsAndMessaging::ICONINFO) -> Option<CurImage> {
    use windows::Win32::Graphics::Gdi::*;

    let mask_obj: HGDIOBJ = info.hbmMask.into();
    let mut bm_mask = BITMAP::default();
    if GetObjectW(
        mask_obj,
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm_mask as *mut _ as *mut std::ffi::c_void),
    ) == 0
    {
        return None;
    }
    if bm_mask.bmWidth <= 0 || bm_mask.bmHeight <= 0 || bm_mask.bmWidth > 256 {
        return None;
    }

    let has_color = !info.hbmColor.is_invalid();
    let (w, h) = if has_color {
        let color_obj: HGDIOBJ = info.hbmColor.into();
        let mut bm = BITMAP::default();
        if GetObjectW(
            color_obj,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut std::ffi::c_void),
        ) == 0
        {
            return None;
        }
        (bm.bmWidth as u32, bm.bmHeight as u32)
    } else {
        (bm_mask.bmWidth as u32, (bm_mask.bmHeight / 2) as u32)
    };
    if w == 0 || h == 0 || w > 256 || h > 256 {
        return None;
    }

    // AND mask: metade superior do hbmMask, 1bpp.
    let mask = read_mono_bitmap(info.hbmMask, w, h, 0)?;
    // XOR: colorido ou metade inferior 1bpp.
    let xor: Vec<u8> = if has_color {
        read_color_bitmap(info.hbmColor, w, h)?
    } else {
        let mono = read_mono_bitmap(info.hbmMask, w, h, h)?;
        let mut px = vec![0u8; (w * h * 4) as usize];
        for (i, bit) in mono.iter().enumerate() {
            let v = if *bit == 1 { 255 } else { 0 };
            px[i * 4..i * 4 + 3].copy_from_slice(&[v, v, v]);
            px[i * 4 + 3] = 255;
        }
        px
    };

    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut any_alpha = false;
    for i in 0..(w * h) as usize {
        let a = xor[i * 4 + 3];
        if a != 0 {
            any_alpha = true;
        }
        rgba[i * 4..i * 4 + 4].copy_from_slice(&xor[i * 4..i * 4 + 4]);
    }
    // `mask` ja vem descompactada (1 bit por pixel, linha a linha).
    if !any_alpha {
        for i in 0..(w * h) as usize {
            rgba[i * 4 + 3] = if mask[i] == 1 { 0 } else { 255 };
        }
    } else {
        for i in 0..(w * h) as usize {
            if mask[i] == 1 {
                rgba[i * 4 + 3] = 0;
            }
        }
    }
    Some(CurImage { width: w, height: h, rgba })
}

/// Le 1bpp de um HBITMAP, de cima para baixo, comecando na linha `skip`.
#[cfg(windows)]
unsafe fn read_mono_bitmap(
    hbm: HBITMAP,
    w: u32,
    h: u32,
    skip: u32,
) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::*;
    let hdc = CreateCompatibleDC(None);
    if hdc.is_invalid() {
        return None;
    }
    // 1bpp exige tabela com 2 cores; sem ela o driver escreve fora do buffer.
    #[repr(C)]
    struct Bmi1 {
        header: BITMAPINFOHEADER,
        colors: [RGBQUAD; 2],
    }
    let mut bmi = Bmi1 {
        header: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -((h + skip) as i32),
            biPlanes: 1,
            biBitCount: 1,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        colors: [RGBQUAD::default(), RGBQUAD::default()],
    };
    let row = ((w + 31) / 32 * 4) as usize;
    let mut bits = vec![0u8; row * (h + skip) as usize];
    let lines = GetDIBits(
        hdc,
        hbm,
        0,
        h + skip,
        Some(bits.as_mut_ptr() as *mut std::ffi::c_void),
        &mut bmi as *mut Bmi1 as *mut BITMAPINFO,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(hdc);
    if lines as u32 != h + skip {
        return None;
    }
    let start = row * skip as usize;
    let mut out = vec![0u8; (w * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let byte = bits[start + y * row + x / 8];
            out[y * w as usize + x] = (byte >> (7 - (x % 8))) & 1;
        }
    }
    Some(out)
}

/// Le colorido 32bpp de um HBITMAP, de cima para baixo, em RGBA.
#[cfg(windows)]
unsafe fn read_color_bitmap(hbm: HBITMAP, w: u32, h: u32) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::*;
    let hdc = CreateCompatibleDC(None);
    if hdc.is_invalid() {
        return None;
    }
    let mut bits = vec![0u8; (w * h * 4) as usize];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let lines = GetDIBits(
        hdc,
        hbm,
        0,
        h,
        Some(bits.as_mut_ptr() as *mut std::ffi::c_void),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(hdc);
    if lines as u32 != h {
        return None;
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        out[i * 4] = bits[i * 4 + 2];
        out[i * 4 + 1] = bits[i * 4 + 1];
        out[i * 4 + 2] = bits[i * 4];
        out[i * 4 + 3] = bits[i * 4 + 3];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_role_returns_none() {
        assert!(system_cursor_image("Xyz").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn renders_arrow() {
        let img = system_cursor_image("Arrow").expect("cursor do sistema");
        assert!(img.width > 0 && img.height > 0);
        assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
        assert!(img.rgba.iter().skip(3).step_by(4).any(|&a| a != 0));
    }
}
