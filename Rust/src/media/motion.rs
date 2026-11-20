//! The visual copy covers the compositor's layout jump. The real shell hit region
//! is held by the normal recovery lease and is restored as soon as the motion ends.
use crate::{
    policy::{NativeBar, Point, Rect, SIDE_INSET_DIP},
    settings::Alignment,
};
use std::{
    ptr::{null, null_mut},
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

pub const DURATION: u64 = 220;

pub fn expected_face(
    bar: NativeBar,
    face: Rect,
    reserve: i32,
    dpi: u32,
    alignment: Alignment,
) -> Rect {
    let px = |dip| dip * dpi as i32 / 96;
    let d = bar.display;
    if bar.edge.horizontal() {
        let outer_room = d.width() - face.width();
        let leading = if reserve == 0 {
            match alignment {
                Alignment::Left => px(SIDE_INSET_DIP),
                Alignment::Center => outer_room / 2,
                Alignment::Right => outer_room - px(SIDE_INSET_DIP),
            }
        } else {
            (outer_room + px(reserve)) / 2
        };
        let x = d.left + leading.clamp(0, outer_room.max(0));
        Rect::new(x, face.top, x + face.width(), face.bottom)
    } else {
        let room = d.height() - face.height();
        let y = d.top + ((room + px(reserve)) / 2).clamp(0, room.max(0));
        Rect::new(face.left, y, face.right, y + face.height())
    }
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.0);
        }
    }
}

pub struct Reflow {
    pub from: Rect,
    pub requested_at: u64,
    bitmap: Bitmap,
    layer: HWND,
    freeze: Option<FrozenStrip>,
    pub started: Option<u64>,
    pub clipped: bool,
}

struct FrozenStrip {
    hwnd: HWND,
    _bitmap: Bitmap,
}

impl Drop for FrozenStrip {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

impl Reflow {
    pub fn capture(tray: HWND, face: Rect, expected: Rect, radius: i32, now: u64) -> Option<Self> {
        if face.width() < 2
            || face.height() < 2
            || i64::from(face.width()) * i64::from(face.height()) > 400_000
        {
            return None;
        }
        // A menu or other window over the rail is not transition artwork.
        let center = POINT {
            x: face.left + face.width() / 2,
            y: face.top + face.height() / 2,
        };
        if unsafe { GetAncestor(WindowFromPhysicalPoint(center), GA_ROOT) } != tray {
            return None;
        }
        let bitmap = snapshot(face)?;
        let layer = create_layer(&bitmap, face, radius)?;
        // Cover the old and new areas while Explorer rebuilds its clip. Otherwise
        // the real bar can flash at the destination before the animation starts.
        let whole = Rect::new(
            face.left.min(expected.left),
            face.top.min(expected.top),
            face.right.max(expected.right),
            face.bottom.max(expected.bottom),
        );
        let freeze =
            if whole != face && i64::from(whole.width()) * i64::from(whole.height()) <= 400_000 {
                snapshot(whole).and_then(|bitmap| {
                    let hwnd = create_layer(&bitmap, whole, 0)?;
                    let freeze = FrozenStrip {
                        hwnd,
                        _bitmap: bitmap,
                    };
                    if unsafe {
                        SetWindowPos(
                            hwnd,
                            HWND_TOPMOST,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        )
                    } == 0
                    {
                        return None;
                    }
                    unsafe {
                        UpdateWindow(hwnd);
                    }
                    Some(freeze)
                })
            } else {
                None
            };
        Some(Self {
            from: face,
            requested_at: now,
            bitmap,
            layer,
            freeze,
            started: None,
            clipped: false,
        })
    }

    pub fn show(&mut self, point: Point) -> bool {
        unsafe {
            let ok = SetWindowPos(
                self.layer,
                HWND_TOPMOST,
                point.x,
                point.y,
                self.from.width(),
                self.from.height(),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) != 0;
            if ok {
                UpdateWindow(self.layer);
            }
            ok
        }
    }

    pub fn release_freeze(&mut self) {
        self.freeze = None;
    }
}

impl Drop for Reflow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.layer);
        }
        // Keep the bitmap alive through the layer's final window messages.
        let _ = self.bitmap.0;
    }
}

pub fn position(from: Rect, final_face: Rect, elapsed: u64) -> Point {
    let t = (elapsed as f64 / DURATION as f64).min(1.0);
    let progress = t * t * (3.0 - 2.0 * t);
    let step = |a, b| a + (f64::from(b - a) * progress).round() as i32;
    Point {
        x: step(from.left, final_face.left),
        y: step(from.top, final_face.top),
    }
}

fn snapshot(r: Rect) -> Option<Bitmap> {
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, r.width(), r.height());
        let success = if !dc.is_null() && !bitmap.is_null() {
            let old = SelectObject(dc, bitmap);
            let success = BitBlt(
                dc,
                0,
                0,
                r.width(),
                r.height(),
                screen,
                r.left,
                r.top,
                SRCCOPY,
            ) != 0;
            SelectObject(dc, old);
            success
        } else {
            false
        };
        if !dc.is_null() {
            DeleteDC(dc);
        }
        ReleaseDC(null_mut(), screen);
        if bitmap.is_null() {
            return None;
        }
        let bitmap = Bitmap(bitmap);
        success.then_some(bitmap)
    }
}

unsafe extern "system" fn paint(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCHITTEST => HTTRANSPARENT as LRESULT,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let bitmap = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HBITMAP;
                if !dc.is_null() && !bitmap.is_null() {
                    let mem = CreateCompatibleDC(dc);
                    if !mem.is_null() {
                        let old = SelectObject(mem, bitmap);
                        let r = ps.rcPaint;
                        BitBlt(
                            dc,
                            r.left,
                            r.top,
                            r.right - r.left,
                            r.bottom - r.top,
                            mem,
                            r.left,
                            r.top,
                            SRCCOPY,
                        );
                        SelectObject(mem, old);
                        DeleteDC(mem);
                    }
                }
                EndPaint(hwnd, &ps);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn create_layer(bitmap: &Bitmap, face: Rect, radius: i32) -> Option<HWND> {
    static CLASS: OnceLock<u16> = OnceLock::new();
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = *CLASS.get_or_init(|| {
            let name: Vec<u16> = "SmartTaskbar.Rust.MediaMotion"
                .encode_utf16()
                .chain([0])
                .collect();
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(paint),
                hInstance: instance,
                lpszClassName: name.as_ptr(),
                ..Default::default()
            })
        });
        if class == 0 {
            return None;
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            class as usize as *const u16,
            null(),
            WS_POPUP,
            face.left,
            face.top,
            face.width(),
            face.height(),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, bitmap.0 as isize);
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        let region = if radius == 0 {
            null_mut()
        } else {
            CreateRoundRectRgn(0, 0, face.width() + 1, face.height() + 1, radius, radius)
        };
        if !region.is_null() && SetWindowRgn(hwnd, region, 0) == 0 {
            DeleteObject(region);
        }
        Some(hwnd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reflow_has_intermediate_positions_on_either_axis() {
        let before = Rect::new(100, 100, 500, 140);
        for after in [Rect::new(180, 100, 580, 140), Rect::new(100, 30, 500, 70)] {
            assert_eq!(
                position(before, after, 0),
                Point {
                    x: before.left,
                    y: before.top
                }
            );
            assert_eq!(
                position(before, after, DURATION / 2),
                Point {
                    x: (before.left + after.left) / 2,
                    y: (before.top + after.top) / 2
                }
            );
            assert_eq!(
                position(before, after, DURATION),
                Point {
                    x: after.left,
                    y: after.top
                }
            );
        }
    }
}
