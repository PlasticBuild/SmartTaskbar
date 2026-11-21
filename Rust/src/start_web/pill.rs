use crate::policy::Rect;
use std::{mem::size_of, ptr::null_mut, slice};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT, SIZE},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow},
};

pub(super) fn paint(hwnd: HWND, bounds: Rect, dpi: i32, dark: bool) -> bool {
    unsafe {
        let width = bounds.width();
        let height = bounds.height();
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return false;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(dc);
            return false;
        }
        let old_bitmap = SelectObject(dc, bitmap);
        PatBlt(dc, 0, 0, width, height, BLACKNESS);
        let font = CreateFontW(
            -14 * dpi / 96,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            DEFAULT_PITCH as u32,
            windows_sys::w!("Segoe UI"),
        );
        let old_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00ff_ffff);
        let mut text = RECT {
            right: width,
            bottom: height,
            ..Default::default()
        };
        DrawTextW(
            dc,
            windows_sys::w!("Web"),
            -1,
            &mut text,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        GdiFlush();
        let pixels = slice::from_raw_parts_mut(bits.cast::<u32>(), (width * height) as usize);
        rasterize(pixels, width, height, dpi, dark);
        let origin = POINT {
            x: bounds.left,
            y: bounds.top,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let ok = UpdateLayeredWindow(
            hwnd,
            null_mut(),
            &origin,
            &size,
            dc,
            &POINT::default(),
            0,
            &blend,
            ULW_ALPHA,
        ) != 0;
        SelectObject(dc, old_font);
        DeleteObject(font);
        SelectObject(dc, old_bitmap);
        DeleteObject(bitmap);
        DeleteDC(dc);
        ok
    }
}

fn rasterize(pixels: &mut [u32], width: i32, height: i32, dpi: i32, dark: bool) {
    let radius = height as f32 / 2.0;
    let stroke = dpi as f32 / 96.0;
    let fill = if dark { 0.065 } else { 0.045 };
    let outline = if dark { 0.15 } else { 0.12 };
    for (i, pixel) in pixels.iter_mut().enumerate() {
        let x = (i % width as usize) as f32 + 0.5;
        let y = (i / width as usize) as f32 + 0.5;
        let dx = x - x.clamp(radius, width as f32 - radius);
        let depth = radius - dx.hypot(y - radius);
        let edge = (depth + 0.5).clamp(0.0, 1.0);
        let inner = (depth - stroke + 0.5).clamp(0.0, 1.0);
        let glyph = (*pixel & 255) as f32 / 255.0;
        let alpha = (inner * fill + (edge - inner) * outline) * (1.0 - glyph) + glyph;
        let alpha = (alpha * 255.0).round() as u32;
        let rgb = if dark { alpha * 0x0001_0101 } else { 0 };
        *pixel = (alpha << 24) | rgb;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_preserves_wallpaper_and_paints_a_readable_label() {
        let mut pixels = vec![0; 64 * 31];
        let center = 15 * 64 + 32;
        pixels[center] = 0x00ff_ffff;
        rasterize(&mut pixels, 64, 31, 96, true);
        assert_eq!(pixels[0], 0);
        assert_eq!(pixels[center], 0xffff_ffff);
        assert!((10..30).contains(&(pixels[center + 1] >> 24)));
    }
}
