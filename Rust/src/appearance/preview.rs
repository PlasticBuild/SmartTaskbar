use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::{env, fs, os::windows::ffi::OsStrExt};
use windows_sys::Win32::{
    Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Registry::*,
    },
    UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW},
};

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let f = |shift: u32| {
        let x = ((a >> shift) & 255) as f32;
        let y = ((b >> shift) & 255) as f32;
        (x + (y - x) * t.clamp(0., 1.)).round() as u8
    };
    rgb(f(0), f(8), f(16))
}
#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub dark: bool,
    pub text: u32,
    pub card: u32,
    page: u32,
    muted: u32,
    line: u32,
    accent: u32,
}
impl Palette {
    unsafe fn load() -> Self {
        unsafe {
            let mut value = 1u32;
            let mut size = 4;
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").as_ptr(),
                wide("AppsUseLightTheme").as_ptr(),
                RRF_RT_REG_DWORD,
                null_mut(),
                &mut value as *mut _ as _,
                &mut size,
            );
            if value == 0 {
                Self {
                    dark: true,
                    page: rgb(22, 24, 27),
                    card: rgb(36, 39, 44),
                    text: rgb(241, 243, 247),
                    muted: rgb(176, 185, 199),
                    line: rgb(63, 69, 78),
                    accent: rgb(126, 197, 241),
                }
            } else {
                Self {
                    dark: false,
                    page: rgb(242, 245, 247),
                    card: rgb(255, 255, 255),
                    text: rgb(26, 36, 48),
                    muted: rgb(80, 95, 111),
                    line: rgb(208, 216, 224),
                    accent: rgb(14, 91, 143),
                }
            }
        }
    }
}
pub(super) struct Paint {
    pub palette: Palette,
    pub brush: HBRUSH,
    pub normal: HFONT,
    heading: HFONT,
    title: HFONT,
    icons: Vec<HICON>,
    icon_refresh: Option<Instant>,
    pending_icons: Option<Receiver<IconBatch>>,
}

// HICONs are process-wide. The owning batch either transfers its icons to the
// UI thread or destroys all of them when a closed window drops its receiver.
struct IconBatch(Vec<usize>);
impl Drop for IconBatch {
    fn drop(&mut self) {
        for h in self.0.drain(..) {
            unsafe {
                DestroyIcon(h as HICON);
            }
        }
    }
}
impl Paint {
    pub unsafe fn new(dpi: i32) -> Self {
        unsafe {
            let palette = Palette::load();
            let brush = CreateSolidBrush(palette.card);
            let mut p = Self {
                palette,
                brush,
                normal: null_mut(),
                heading: null_mut(),
                title: null_mut(),
                icons: Vec::new(),
                icon_refresh: None,
                pending_icons: None,
            };
            p.fonts(dpi);
            p
        }
    }
    pub fn refresh_icons(&mut self) -> bool {
        if self.pending_icons.is_some()
            || self
                .icon_refresh
                .is_some_and(|last| last.elapsed().as_secs() < 4)
        {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("pinned-app-icons".into())
            .spawn(move || unsafe {
                let initialized = CoInitializeEx(
                    null(),
                    (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
                ) >= 0;
                let icons = if initialized {
                    let icons = load_icons();
                    CoUninitialize();
                    icons
                } else {
                    Vec::new()
                };
                let _ = tx.send(IconBatch(icons.into_iter().map(|h| h as usize).collect()));
            });
        if worker.is_err() {
            return false;
        }
        self.pending_icons = Some(rx);
        self.icon_refresh = Some(Instant::now());
        true
    }
    pub unsafe fn receive_icons(&mut self) -> bool {
        let Some(rx) = self.pending_icons.as_ref() else {
            return true;
        };
        let next = match rx.try_recv() {
            Err(TryRecvError::Empty) => return false,
            reply => reply.ok(),
        };
        self.pending_icons = None;
        unsafe {
            if let Some(mut icons) = next
                && !icons.0.is_empty()
            {
                let handles = icons.0.drain(..).map(|h| h as HICON).collect();
                for owned in std::mem::replace(&mut self.icons, handles) {
                    DestroyIcon(owned);
                }
            }
        }
        true
    }
    pub unsafe fn theme(&mut self, hwnd: HWND) {
        unsafe {
            self.palette = Palette::load();
            let dark = self.palette.dark as i32;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &dark as *const _ as _,
                4,
            );
            if !self.brush.is_null() {
                DeleteObject(self.brush);
            }
            self.brush = CreateSolidBrush(self.palette.card);
        }
    }
    pub unsafe fn fonts(&mut self, dpi: i32) {
        unsafe {
            let font = |sz: i32, weight: i32| {
                CreateFontW(
                    -sz * dpi / 96,
                    0,
                    0,
                    0,
                    weight,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    0,
                    wide("Segoe UI").as_ptr(),
                )
            };
            for &old in &[self.normal, self.heading, self.title] {
                if !old.is_null() {
                    DeleteObject(old);
                }
            }
            self.normal = font(14, 400);
            self.heading = font(17, 600);
            self.title = font(26, 600);
        }
    }
}
impl Drop for Paint {
    fn drop(&mut self) {
        unsafe {
            for &h in &[self.normal, self.heading, self.title, self.brush] {
                if !h.is_null() {
                    DeleteObject(h);
                }
            }
            for &icon in &self.icons {
                DestroyIcon(icon);
            }
        }
    }
}

unsafe fn load_icons() -> Vec<HICON> {
    unsafe {
        let mut icons = Vec::new();
        if let Some(base) = env::var_os("APPDATA") {
            let pins = PathBuf::from(base)
                .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");
            if let Ok(entries) = fs::read_dir(pins) {
                let mut links: Vec<_> = entries
                    .flatten()
                    .map(|i| i.path())
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")))
                    .collect();
                links.sort();
                for path in links.iter().take(12) {
                    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                    let mut info: SHFILEINFOW = mem::zeroed();
                    if SHGetFileInfoW(
                        path.as_ptr(),
                        0,
                        &mut info,
                        mem::size_of_val(&info) as u32,
                        SHGFI_ICON | SHGFI_LARGEICON,
                    ) != 0
                        && !info.hIcon.is_null()
                    {
                        icons.push(info.hIcon);
                    }
                }
            }
        }
        if icons.is_empty() {
            // Packaged pins may have no .lnk. Use the application icons of visible windows.
            let mut apps = RunningIcons {
                icons: Vec::new(),
                sources: Vec::new(),
            };
            EnumWindows(Some(running_icon), &mut apps as *mut _ as isize);
            icons = apps.icons;
        }
        icons
    }
}
struct RunningIcons {
    icons: Vec<HICON>,
    sources: Vec<usize>,
}
unsafe extern "system" fn running_icon(hwnd: HWND, data: LPARAM) -> i32 {
    unsafe {
        let apps = &mut *(data as *mut RunningIcons);
        if IsWindowVisible(hwnd) == 0
            || !GetWindow(hwnd, GW_OWNER).is_null()
            || GetWindowTextLengthW(hwnd) <= 0
            || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0
        {
            return 1;
        }
        let mut raw = GetClassLongPtrW(hwnd, GCLP_HICON);
        if raw == 0 {
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                ICON_BIG as usize,
                0,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                30,
                &mut raw,
            );
        }
        if raw != 0 && !apps.sources.contains(&raw) {
            let owned = CopyIcon(raw as HICON);
            if !owned.is_null() {
                apps.icons.push(owned);
                apps.sources.push(raw);
            }
        }
        (apps.icons.len() < 10) as i32
    }
}

pub(super) struct Geometry {
    pub bar: Box2,
    pub activation: Box2,
    pub icons: Vec<Box2>,
    pub menu_box: Box2,
    pub media: Option<Box2>,
}
pub(super) fn geometry(s: &State) -> Geometry {
    arrange(
        s.page.scene(),
        &s.draft,
        s.paint.icons.len(),
        s.demo.shown,
        s.demo.open,
    )
}

fn arrange(scene: Box2, draft: &Settings, pins: usize, shown: f32, opened: f32) -> Geometry {
    use crate::policy::{
        Edge, NativeBar, Rect, activation_band, anchored_start, island_rect, joined_start_face,
        slide_offset,
    };
    let display = Rect::new(scene.x, scene.y, scene.x + scene.w, scene.y + scene.h);
    let step = (scene.w / 12).clamp(17, 33);
    let count = (pins + 1).min(((scene.w - 55) / step).max(1) as usize);
    let thickness = (step + 7).clamp(24, 40);
    let native = NativeBar {
        display,
        edge: Edge::Bottom,
        bounds: Rect::new(
            display.left,
            display.bottom - thickness,
            display.right,
            display.bottom,
        ),
    };
    // The preview uses the native policy at the desktop's displayed scale.
    let dpi = (scene.w * 96 / 1200).clamp(24, 96) as u32;
    let content = count as i32 * step + 43;
    let resting = if draft.island {
        island_rect(native, content, draft.alignment, dpi)
    } else {
        native.bounds
    };
    let menu = anchored_start(native, resting, (scene.h / 2).clamp(72, 165), dpi);
    let joined = if draft.island {
        joined_start_face(native.edge, resting, menu)
    } else {
        resting
    };
    let lerp = |a: i32, b: i32| a + ((b - a) as f32 * opened).round() as i32;
    let left = lerp(resting.left, joined.left);
    let right = lerp(resting.right, joined.right);
    let dy = slide_offset(native.edge, thickness, shown.into()).y;
    let bar = Box2::at(left, resting.top + dy, right - left, resting.height());
    let icon_step = step.min((bar.w - 42) / count as i32).max(7);
    let row = count as i32 * icon_step + 41;
    let start = if draft.island {
        bar.x + 4
    } else {
        bar.x + (bar.w - row) / 2 + 4
    };
    let icons = (0..count)
        .map(|n| {
            Box2::at(
                start + n as i32 * icon_step,
                bar.y + (bar.h - icon_step) / 2,
                icon_step,
                icon_step,
            )
        })
        .collect();
    let band = activation_band(native, Some(resting), draft.zone, dpi);
    let to_box = |r: Rect| Box2::at(r.left, r.top, r.width(), r.height());
    let media = if draft.media_player && shown >= 0.99 && opened == 0.0 {
        crate::media::placement(
            native,
            draft.island.then_some(resting),
            draft.media_side,
            dpi,
        )
        .map(|p| to_box(p.bounds))
    } else {
        None
    };
    Geometry {
        bar,
        icons,
        activation: to_box(band),
        menu_box: to_box(menu),
        media,
    }
}

struct Canvas {
    dc: HDC,
    dpi: i32,
    scroll: i32,
}
impl Canvas {
    fn px(&self, n: i32) -> i32 {
        n * self.dpi / 96
    }
    fn rect(&self, r: Box2) -> RECT {
        RECT {
            left: self.px(r.x),
            top: self.px(r.y - self.scroll),
            right: self.px(r.x + r.w),
            bottom: self.px(r.y + r.h - self.scroll),
        }
    }
    unsafe fn fill(&self, r: Box2, c: u32) {
        unsafe {
            let b = GetStockObject(DC_BRUSH) as HBRUSH;
            SetDCBrushColor(self.dc, c);
            FillRect(self.dc, &self.rect(r), b);
        }
    }
    unsafe fn round(&self, r: Box2, rad: i32, bg: u32, edge: u32) {
        unsafe {
            let a = self.rect(r);
            let old_b = SelectObject(self.dc, GetStockObject(DC_BRUSH));
            let old_p = SelectObject(self.dc, GetStockObject(DC_PEN));
            SetDCBrushColor(self.dc, bg);
            SetDCPenColor(self.dc, edge);
            let d = self.px(rad * 2).max(1);
            RoundRect(self.dc, a.left, a.top, a.right, a.bottom, d, d);
            SelectObject(self.dc, old_p);
            SelectObject(self.dc, old_b);
        }
    }
    unsafe fn text(&self, r: Box2, value: &str, font: HFONT, color: u32) {
        unsafe {
            let mut r = self.rect(r);
            let text = wide(value);
            let old = SelectObject(self.dc, font);
            SetTextColor(self.dc, color);
            DrawTextW(
                self.dc,
                text.as_ptr(),
                (text.len() - 1) as i32,
                &mut r,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(self.dc, old);
        }
    }
    unsafe fn icon(&self, r: Box2, icon: HICON) {
        unsafe {
            let a = self.rect(r);
            DrawIconEx(
                self.dc,
                a.left,
                a.top,
                icon,
                a.right - a.left,
                a.bottom - a.top,
                0,
                null_mut(),
                DI_NORMAL,
            );
        }
    }
}

pub(super) unsafe fn paint(hwnd: HWND, s: &State) {
    unsafe {
        let mut ps: PAINTSTRUCT = mem::zeroed();
        let screen = BeginPaint(hwnd, &mut ps);
        if screen.is_null() {
            return;
        }
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        let dc = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, bounds.right.max(1), bounds.bottom.max(1));
        if dc.is_null() || bmp.is_null() {
            if !dc.is_null() {
                DeleteDC(dc);
            }
            if !bmp.is_null() {
                DeleteObject(bmp);
            }
            EndPaint(hwnd, &ps);
            return;
        }
        let previous = SelectObject(dc, bmp);
        SetBkMode(dc, TRANSPARENT as i32);
        let c = Canvas {
            dc,
            dpi: s.dpi,
            scroll: s.scroll,
        };
        let palette = s.paint.palette;
        SetDCBrushColor(dc, palette.page);
        FillRect(dc, &bounds, GetStockObject(DC_BRUSH) as HBRUSH);
        let p = s.page;
        c.text(
            Box2::at(p.preview.x, 12, p.width - 35, 38),
            "Taskbar appearance",
            s.paint.title,
            palette.text,
        );
        for (card, caption) in [
            (p.preview, "Live preview"),
            (p.settings, "Appearance & behavior"),
        ] {
            c.round(card, 9, palette.card, palette.line);
            c.text(
                Box2::at(card.x + 14, card.y + 6, card.w - 28, 32),
                caption,
                s.paint.heading,
                palette.text,
            );
        }
        let settings_y = p.settings.y + 9;
        let sx = p.settings.x + 15;
        let sw = p.settings.w - 30;
        for (y, value) in [
            (128, format!("{} px", s.draft.radius)),
            (174, format!("{}%", s.draft.transparency)),
            (220, format!("{} px", s.draft.hover_lift)),
            (266, format!("{} px", s.draft.click_depth)),
        ] {
            c.text(
                Box2::at(sx + sw - 48, settings_y + y, 48, 18),
                &value,
                s.paint.normal,
                palette.muted,
            );
        }
        let notice = if !s.status.is_empty() {
            s.status.as_str()
        } else if s.old != s.draft {
            "Unapplied changes"
        } else {
            ""
        };
        c.text(
            Box2::at(p.preview.x + 120, p.footer, (p.width - 290).max(20), 32),
            notice,
            s.paint.normal,
            palette.muted,
        );
        draw_scene(&c, s);
        BitBlt(screen, 0, 0, bounds.right, bounds.bottom, dc, 0, 0, SRCCOPY);
        SelectObject(dc, previous);
        DeleteObject(bmp);
        DeleteDC(dc);
        EndPaint(hwnd, &ps);
    }
}

unsafe fn draw_scene(c: &Canvas, s: &State) {
    unsafe {
        let scene = s.page.scene();
        let r = c.rect(scene);
        let saved = SaveDC(c.dc);
        IntersectClipRect(c.dc, r.left, r.top, r.right, r.bottom);
        let p = s.paint.palette;
        let high = if p.dark {
            rgb(37, 82, 110)
        } else {
            rgb(150, 206, 223)
        };
        let low = if p.dark {
            rgb(22, 42, 65)
        } else {
            rgb(92, 150, 181)
        };
        for y in (0..scene.h).step_by(3) {
            c.fill(
                Box2::at(scene.x, scene.y + y, scene.w, 3),
                mix(high, low, y as f32 / scene.h as f32),
            );
        }
        let app = if s.demo.full {
            scene
        } else {
            Box2::at(
                scene.x + scene.w / 12,
                scene.y + scene.h / 9,
                scene.w * 2 / 3,
                scene.h * 3 / 5,
            )
        };
        c.round(app, if s.demo.full { 0 } else { 7 }, p.card, p.line);
        c.fill(
            Box2::at(app.x + 1, app.y + 1, app.w - 2, 24),
            mix(p.card, p.page, 0.6),
        );
        c.text(
            Box2::at(app.x + 10, app.y + 2, app.w - 41, 19),
            "Preview window",
            s.paint.normal,
            p.muted,
        );
        c.fill(
            Box2::at(app.x + 9, app.y + 34, (app.w * 5 / 9).max(18), 4),
            mix(p.card, p.accent, 0.36),
        );
        for i in 0..3 {
            c.fill(
                Box2::at(app.x + 9, app.y + 48 + i * 10, app.w * (7 - i) / 10, 3),
                mix(p.card, p.muted, 0.32),
            );
        }
        let g = geometry(s);
        let glass = mix(p.card, low, s.draft.transparency as f32 / 100.);
        if s.demo.open > 0.0 {
            let menu = g.menu_box;
            let height = (menu.h as f32 * s.demo.open).ceil() as i32;
            let clip = c.rect(Box2::at(menu.x, menu.y + menu.h - height, menu.w, height));
            let nested = SaveDC(c.dc);
            IntersectClipRect(c.dc, clip.left, clip.top, clip.right, clip.bottom);
            c.round(menu, s.draft.radius as i32, glass, p.line);
            c.text(
                Box2::at(menu.x + 10, menu.y + 6, menu.w - 20, 24),
                "Pinned apps",
                s.paint.normal,
                p.text,
            );
            for (i, &icon) in s
                .paint
                .icons
                .iter()
                .take(((menu.w - 20) / 35).max(0) as usize)
                .enumerate()
            {
                c.icon(
                    Box2::at(menu.x + 12 + i as i32 * 35, menu.y + 40, 23, 23),
                    icon,
                );
            }
            RestoreDC(c.dc, nested);
        }
        if s.demo.shown > 0.0 {
            c.round(
                g.bar,
                if s.draft.island {
                    s.draft.radius as i32
                } else {
                    0
                },
                glass,
                p.line,
            );
            for (index, b) in g.icons.iter().enumerate() {
                let hover = s.demo.hot == Some(index);
                let movement = if !hover {
                    0
                } else if s.demo.down {
                    s.draft.click_depth as i32
                } else {
                    -(s.draft.hover_lift as i32)
                };
                let span = (b.w * 2 / 3).max(9)
                    + if hover && !s.demo.down {
                        (s.draft.hover_lift as i32 + 1) / 2
                    } else {
                        0
                    };
                let icon = Box2::at(
                    b.x + (b.w - span) / 2,
                    b.y + (b.h - span) / 2 + movement,
                    span,
                    span,
                );
                if index == 0 {
                    let half = (span - 2) / 2;
                    for dy in [0, half + 2] {
                        for dx in [0, half + 2] {
                            c.fill(
                                Box2::at(icon.x + dx, icon.y + dy, half, half),
                                rgb(48, 169, 232),
                            );
                        }
                    }
                } else {
                    c.icon(icon, s.paint.icons[index - 1]);
                }
            }
            let tx = g.bar.x + g.bar.w - 34;
            let ty = g.bar.y + g.bar.h / 2;
            for i in 0..3 {
                c.fill(Box2::at(tx + i * 4, ty + 3 - i * 2, 2, i * 2 + 3), p.text);
            }
            c.round(Box2::at(tx + 16, ty - 2, 12, 7), 2, glass, p.text);
            c.fill(Box2::at(tx + 18, ty, 7, 3), p.text);
            if let Some(player) = g.media {
                c.round(player, (s.draft.radius as i32).max(3), glass, p.line);
                let thumbnail = (player.h - 4).min(18);
                let art = Box2::at(player.x + 3, player.y + 2, thumbnail, thumbnail);
                c.round(art, 2, mix(glass, p.accent, 0.25), p.line);
                c.text(
                    Box2::at(art.x + 2, art.y, art.w, art.h),
                    "♪",
                    s.paint.normal,
                    p.text,
                );
                let start = art.x + art.w + 4;
                let width = player.x + player.w - start - 4;
                c.text(
                    Box2::at(start, player.y, width, player.h - 3),
                    "‹  ▶  ›",
                    s.paint.normal,
                    p.text,
                );
                c.fill(Box2::at(start, player.y + player.h - 3, width, 1), p.line);
                c.fill(
                    Box2::at(start, player.y + player.h - 3, width / 2, 1),
                    p.accent,
                );
            }
        } else if s.demo.full {
            c.fill(
                Box2::at(
                    g.activation.x,
                    g.activation.y + g.activation.h - 2,
                    g.activation.w,
                    2,
                ),
                p.accent,
            );
        }
        RestoreDC(c.dc, saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_reveals_at_final_location_and_island_meets_it() {
        let scene = Box2::at(13, 112, 500, 326);
        for alignment in [Alignment::Left, Alignment::Center, Alignment::Right] {
            let settings = Settings {
                island: true,
                alignment,
                ..Settings::default()
            };
            let first = arrange(scene, &settings, 1, 1.0, 0.01);
            let last = arrange(scene, &settings, 1, 1.0, 1.0);
            assert_eq!(
                (first.menu_box.x, first.menu_box.y, first.menu_box.w),
                (last.menu_box.x, last.menu_box.y, last.menu_box.w)
            );
            assert_eq!(last.menu_box.y + last.menu_box.h, last.bar.y);
            assert_eq!((last.menu_box.x, last.menu_box.w), (last.bar.x, last.bar.w));
            assert!(
                last.icons
                    .iter()
                    .all(|i| i.x >= last.bar.x && i.x + i.w <= last.bar.x + last.bar.w)
            );
        }
    }
    #[test]
    fn full_width_and_activation_zone_are_independent() {
        let scene = Box2::at(12, 15, 618, 420);
        let settings = Settings {
            island: false,
            zone: Zone::Center,
            ..Settings::default()
        };
        let g = arrange(scene, &settings, 12, 0.0, 0.0);
        assert_eq!(g.bar.w, scene.w);
        assert_eq!(g.bar.y, scene.y + scene.h);
        assert_eq!(g.activation.x, scene.x + scene.w / 4);
        assert_eq!(g.activation.w, scene.w - 2 * (scene.w / 4));
    }
}
