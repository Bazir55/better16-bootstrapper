//! The progress window.
//!
//! It looks like the launcher always has: a borderless white box with a
//! thin grey edge, the logo, one line of text, a progress bar and a picture
//! button. Everything except the progress bar is painted by hand, which is
//! less code than fighting owner-drawn statics and buttons, and lets the
//! pictures scale on high-DPI screens (the @2x bitmaps are used there).
//!
//! The window lives on the main thread and the actual work runs on a second
//! one, which talks to the window only through PostMessage.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use crate::error::{Error, Result};
use crate::log::log;
use crate::win::{self, BOOL, HWND, Handle, wide};

type WndProc = unsafe extern "system" fn(HWND, u32, usize, isize) -> isize;

#[repr(C)]
struct WndClassEx {
    size: u32,
    style: u32,
    proc_: WndProc,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
    icon_small: Handle,
}

#[repr(C)]
struct Msg {
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[repr(C)]
struct PaintStruct {
    hdc: Handle,
    erase: BOOL,
    paint: Rect,
    restore: BOOL,
    inc_update: BOOL,
    reserved: [u8; 32],
}

#[repr(C)]
struct LogFont {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    char_set: u8,
    out_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face_name: [u16; 32],
}

#[repr(C)]
struct TrackMouseEventInfo {
    size: u32,
    flags: u32,
    hwnd: HWND,
    hover_time: u32,
}

#[repr(C)]
struct InitCommonControlsEx {
    size: u32,
    classes: u32,
}

win::import!("user32" {
    fn RegisterClassExW(class: *const WndClassEx) -> u16;
    fn CreateWindowExW(ex_style: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: Handle, instance: Handle, param: *mut c_void) -> HWND;
    fn DefWindowProcW(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn GetMessageW(msg: *mut Msg, hwnd: HWND, min: u32, max: u32) -> BOOL;
    fn TranslateMessage(msg: *const Msg) -> BOOL;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn PostMessageW(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> BOOL;
    fn SendMessageW(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn PostQuitMessage(code: i32);
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
    fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    fn GetDlgItem(hwnd: HWND, id: i32) -> HWND;
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn SetWindowLongW(hwnd: HWND, index: i32, value: i32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
    fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    fn LoadBitmapW(instance: Handle, name: *const u16) -> Handle;
    fn SetCursor(cursor: Handle) -> Handle;
    fn GetDC(hwnd: HWND) -> Handle;
    fn ReleaseDC(hwnd: HWND, dc: Handle) -> i32;
    fn BeginPaint(hwnd: HWND, ps: *mut PaintStruct) -> Handle;
    fn EndPaint(hwnd: HWND, ps: *const PaintStruct) -> BOOL;
    fn InvalidateRect(hwnd: HWND, rect: *const Rect, erase: BOOL) -> BOOL;
    fn FillRect(dc: Handle, rect: *const Rect, brush: Handle) -> i32;
    fn FrameRect(dc: Handle, rect: *const Rect, brush: Handle) -> i32;
    fn DrawTextW(dc: Handle, text: *const u16, len: i32, rect: *mut Rect, format: u32) -> i32;
    fn ScreenToClient(hwnd: HWND, point: *mut [i32; 2]) -> BOOL;
    fn TrackMouseEvent(info: *mut TrackMouseEventInfo) -> BOOL;
});

win::import!("gdi32" {
    fn CreateFontIndirectW(font: *const LogFont) -> Handle;
    fn GetDeviceCaps(dc: Handle, index: i32) -> i32;
    fn CreateSolidBrush(color: u32) -> Handle;
    fn CreateCompatibleDC(dc: Handle) -> Handle;
    fn CreateCompatibleBitmap(dc: Handle, w: i32, h: i32) -> Handle;
    fn SelectObject(dc: Handle, obj: Handle) -> Handle;
    fn DeleteObject(obj: Handle) -> BOOL;
    fn DeleteDC(dc: Handle) -> BOOL;
    fn BitBlt(dst: Handle, x: i32, y: i32, w: i32, h: i32, src: Handle, sx: i32, sy: i32, rop: u32) -> BOOL;
    fn StretchBlt(dst: Handle, x: i32, y: i32, w: i32, h: i32, src: Handle, sx: i32, sy: i32, sw: i32, sh: i32, rop: u32) -> BOOL;
    fn SetStretchBltMode(dc: Handle, mode: i32) -> i32;
    fn SetBrushOrgEx(dc: Handle, x: i32, y: i32, old: *mut [i32; 2]) -> BOOL;
    fn SetTextColor(dc: Handle, color: u32) -> u32;
    fn SetBkMode(dc: Handle, mode: i32) -> i32;
});

win::import!("comctl32" {
    fn InitCommonControlsEx(init: *const InitCommonControlsEx) -> BOOL;
});

const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const PBS_SMOOTH: u32 = 0x01;
const PBS_MARQUEE: u32 = 0x08;

const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCHITTEST: u32 = 0x0084;
const WM_KEYDOWN: u32 = 0x0100;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_USER: u32 = 0x0400;
const PBM_SETPOS: u32 = WM_USER + 2;
const PBM_SETRANGE32: u32 = WM_USER + 6;
const PBM_SETMARQUEE: u32 = WM_USER + 10;

const WM_APP: u32 = 0x8000;
const MSG_STATUS: u32 = WM_APP + 1;
const MSG_PROGRESS: u32 = WM_APP + 2;
const MSG_FAILED: u32 = WM_APP + 3;
const MSG_DONE: u32 = WM_APP + 4;
const MSG_HIDE_BUTTON: u32 = WM_APP + 5;
const MSG_SUCCESS: u32 = WM_APP + 6;

const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const TME_LEAVE: u32 = 2;
const VK_ESCAPE: usize = 0x1B;
const VK_RETURN: usize = 0x0D;
const SRCCOPY: u32 = 0x00CC_0020;
const HALFTONE: i32 = 4;
const TRANSPARENT: i32 = 1;
const DT_CENTER: u32 = 0x01;
const DT_VCENTER: u32 = 0x04;
const DT_SINGLELINE: u32 = 0x20;
const DT_NOPREFIX: u32 = 0x800;
const DT_END_ELLIPSIS: u32 = 0x8000;

const ID_PROGRESS: i32 = 101;
const MARQUEE: usize = usize::MAX;
const GWL_STYLE: i32 = -16;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const LOGPIXELSY: i32 = 90;
const ICC_PROGRESS_CLASS: u32 = 0x20;
const MB_ICONERROR: u32 = 0x10;
const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;

const WHITE: u32 = 0x00FF_FFFF;
const EDGE: u32 = 0x00B8_B8B8; // RGB(184, 184, 184)
const INK: u32 = 0x0019_1919; // RGB(25, 25, 25)

// Layout at 96 dpi; everything is scaled from these.
const WIDTH: i32 = 500;
const HEIGHT: i32 = 320;
const LOGO: (i32, i32, i32, i32) = (205, 70, 90, 90);
const TEXT: (i32, i32, i32, i32) = (5, 196, 490, 26);
const PROGRESS: (i32, i32, i32, i32) = (24, 242, 452, 20);
const BUTTON: (i32, i32, i32, i32) = (185, 265, 130, 44);
const SUCCESS_TITLE: (i32, i32, i32, i32) = (5, 192, 490, 30);
const SUCCESS_TEXT: (i32, i32, i32, i32) = (5, 222, 490, 22);

// Bitmap resource ids, as numbered in build.rs. The @2x copy is id + 1.
const BMP_LOGO: u16 = 101;
const BMP_CANCEL: u16 = 103;
const BMP_CANCEL_HOVER: u16 = 105;
const BMP_OK: u16 = 107;
const BMP_OK_HOVER: u16 = 109;

pub const EXIT_FAILED: u32 = 1;
pub const EXIT_CANCELLED: u32 = 2;

static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static STATE: AtomicPtr<State> = AtomicPtr::new(core::ptr::null_mut());
static CANCEL: AtomicBool = AtomicBool::new(false);
static CANCELLABLE: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, PartialEq)]
enum Button {
    None,
    Cancel,
    Ok,
}

/// Window state. Only ever touched on the window's thread.
struct State {
    dpi: i32,
    text: Vec<u16>,
    font: Handle,
    title_font: Handle,
    small_font: Handle,
    white: Handle,
    edge: Handle,
    logo: Handle,
    cancel: [Handle; 2],
    ok: [Handle; 2],
    /// Size of the loaded bitmaps relative to the 1x layout: 1 or 2.
    bitmap_scale: i32,
    button: Button,
    hot: bool,
    pressed: bool,
    tracking: bool,
    success: bool,
}

impl State {
    fn px(&self, v: i32) -> i32 {
        v * self.dpi / 96
    }

    fn rect(&self, r: (i32, i32, i32, i32)) -> Rect {
        Rect { left: self.px(r.0), top: self.px(r.1), right: self.px(r.0 + r.2), bottom: self.px(r.1 + r.3) }
    }
}

fn state<'a>() -> Option<&'a mut State> {
    // Single window, single thread: see State.
    unsafe { STATE.load(Ordering::Acquire).as_mut() }
}

fn window() -> HWND {
    WINDOW.load(Ordering::Acquire)
}

fn screen_dpi() -> i32 {
    unsafe {
        let dc = GetDC(core::ptr::null_mut());
        let dpi = GetDeviceCaps(dc, LOGPIXELSY);
        ReleaseDC(core::ptr::null_mut(), dc);
        if dpi > 0 { dpi } else { 96 }
    }
}

fn font(face: &str, points: i32, dpi: i32) -> Handle {
    let mut lf: LogFont = unsafe { core::mem::zeroed() };
    lf.height = -(points * dpi + 36) / 72;
    lf.weight = 400;
    lf.quality = 5; // ClearType where available, plain antialiasing on XP
    for (dst, src) in lf.face_name.iter_mut().zip(face.encode_utf16()) {
        *dst = src;
    }
    unsafe { CreateFontIndirectW(&lf) }
}

fn new_state() -> State {
    let dpi = screen_dpi();
    let bitmap_scale = if dpi > 96 { 2 } else { 1 };
    let load = |id: u16| unsafe {
        let id = if bitmap_scale == 2 { id + 1 } else { id };
        LoadBitmapW(win::GetModuleHandleW(core::ptr::null()), id as usize as *const u16)
    };
    State {
        dpi,
        text: wide(&alloc::format!("Connecting to {}...", crate::config::product().name)),
        font: font("Tahoma", 11, dpi),
        title_font: font("Tahoma", 15, dpi),
        small_font: font("Tahoma", 10, dpi),
        white: unsafe { CreateSolidBrush(WHITE) },
        edge: unsafe { CreateSolidBrush(EDGE) },
        logo: load(BMP_LOGO),
        cancel: [load(BMP_CANCEL), load(BMP_CANCEL_HOVER)],
        ok: [load(BMP_OK), load(BMP_OK_HOVER)],
        bitmap_scale,
        button: Button::Cancel,
        hot: false,
        pressed: false,
        tracking: false,
        success: false,
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let handles = [self.font, self.title_font, self.small_font, self.white, self.edge, self.logo];
        for h in handles.into_iter().chain(self.cancel).chain(self.ok) {
            if !h.is_null() {
                unsafe { DeleteObject(h) };
            }
        }
    }
}

/// Shows the window, runs `work` on a second thread and pumps messages
/// until it's done. Returns the process exit code.
pub fn run(work: fn() -> Result<()>) -> u32 {
    unsafe {
        let init = InitCommonControlsEx {
            size: core::mem::size_of::<InitCommonControlsEx>() as u32,
            classes: ICC_PROGRESS_CLASS,
        };
        InitCommonControlsEx(&init);

        let instance = win::GetModuleHandleW(core::ptr::null());
        let class_name = wide("Better16Launcher");
        let mut icon = LoadIconW(instance, 1 as *const u16);
        if icon.is_null() {
            icon = LoadIconW(core::ptr::null_mut(), 32512 as *const u16); // IDI_APPLICATION
        }
        let class = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: 0,
            proc_: window_proc,
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon,
            cursor: LoadCursorW(core::ptr::null_mut(), 32512 as *const u16), // IDC_ARROW
            background: core::ptr::null_mut(),
            menu_name: core::ptr::null(),
            class_name: class_name.as_ptr(),
            icon_small: icon,
        };
        RegisterClassExW(&class);

        let state = Box::new(new_state());
        let (w, h) = (state.px(WIDTH), state.px(HEIGHT));
        let progress = state.rect(PROGRESS);
        STATE.store(Box::into_raw(state), Ordering::Release);

        // WS_THICKFRAME with no non-client area (see WM_NCCALCSIZE) is what
        // gets a borderless window the system drop shadow on Vista and up.
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class_name.as_ptr(),
            wide(crate::config::product().name).as_ptr(),
            WS_POPUP | WS_THICKFRAME | WS_CLIPCHILDREN | WS_SYSMENU | WS_MINIMIZEBOX,
            (GetSystemMetrics(SM_CXSCREEN) - w) / 2,
            (GetSystemMetrics(SM_CYSCREEN) - h) / 2,
            w,
            h,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            instance,
            core::ptr::null_mut(),
        );
        if hwnd.is_null() {
            log!("could not create the window: {}", win::error_text(win::last_error()));
            return EXIT_FAILED;
        }
        WINDOW.store(hwnd, Ordering::Release);

        let bar = CreateWindowExW(
            0,
            wide("msctls_progress32").as_ptr(),
            core::ptr::null(),
            WS_CHILD | WS_VISIBLE | PBS_SMOOTH,
            progress.left,
            progress.top,
            progress.right - progress.left,
            progress.bottom - progress.top,
            hwnd,
            ID_PROGRESS as Handle,
            instance,
            core::ptr::null_mut(),
        );
        SendMessageW(bar, PBM_SETRANGE32, 0, 1000);

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);

        let thread = win::CreateThread(core::ptr::null_mut(), 0, worker, work as *mut c_void, 0, core::ptr::null_mut());
        if thread.is_null() {
            log!("could not start the worker thread");
            return EXIT_FAILED;
        }
        win::CloseHandle(thread);

        let mut msg: Msg = core::mem::zeroed();
        while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        msg.wparam as u32
    }
}

unsafe extern "system" fn worker(param: *mut c_void) -> u32 {
    let work: fn() -> Result<()> = unsafe { core::mem::transmute(param) };
    let _com = crate::shell::Com::init();
    match work() {
        Ok(()) => post(MSG_DONE, 0, 0),
        Err(Error::Cancelled) => {
            log!("cancelled");
            post(MSG_DONE, EXIT_CANCELLED as usize, 0)
        }
        Err(e) => {
            log!("failed: {e}");
            let text = alloc::format!("{e}\n\nDetails are in {}", crate::log::path());
            post(MSG_FAILED, 0, Box::into_raw(Box::new(text)) as isize)
        }
    }
    0
}

fn paint(hwnd: HWND, s: &State) {
    unsafe {
        let mut ps: PaintStruct = core::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let (w, h) = (s.px(WIDTH), s.px(HEIGHT));

        // Paint into a buffer first so status updates don't flicker.
        let buf = CreateCompatibleDC(dc);
        let bmp = CreateCompatibleBitmap(dc, w, h);
        let old_bmp = SelectObject(buf, bmp);

        let all = Rect { left: 0, top: 0, right: w, bottom: h };
        FillRect(buf, &all, s.white);
        FrameRect(buf, &all, s.edge);

        draw_bitmap(buf, s, s.logo, s.rect(LOGO));

        SetBkMode(buf, TRANSPARENT);
        SetTextColor(buf, INK);
        let flags = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS;
        if s.success {
            draw_text(buf, s.title_font, "BETTER16 IS SUCCESSFULLY INSTALLED!", s.rect(SUCCESS_TITLE), flags);
            draw_text(buf, s.small_font, "Click the 'Play' button on any game to join the action!", s.rect(SUCCESS_TEXT), flags);
        } else {
            let mut r = s.rect(TEXT);
            let old = SelectObject(buf, s.font);
            DrawTextW(buf, s.text.as_ptr(), -1, &mut r, flags);
            SelectObject(buf, old);
        }

        let pictures = match s.button {
            Button::None => None,
            Button::Cancel => Some(s.cancel),
            Button::Ok => Some(s.ok),
        };
        if let Some(pictures) = pictures {
            draw_bitmap(buf, s, pictures[s.hot as usize], s.rect(BUTTON));
        }

        BitBlt(dc, 0, 0, w, h, buf, 0, 0, SRCCOPY);
        SelectObject(buf, old_bmp);
        DeleteObject(bmp);
        DeleteDC(buf);
        EndPaint(hwnd, &ps);
    }
}

fn draw_text(dc: Handle, font: Handle, text: &str, mut r: Rect, flags: u32) {
    unsafe {
        let old = SelectObject(dc, font);
        DrawTextW(dc, wide(text).as_ptr(), -1, &mut r, flags);
        SelectObject(dc, old);
    }
}

fn draw_bitmap(dc: Handle, s: &State, bitmap: Handle, to: Rect) {
    if bitmap.is_null() {
        return;
    }
    unsafe {
        let src = CreateCompatibleDC(dc);
        let old = SelectObject(src, bitmap);
        let (w, h) = (to.right - to.left, to.bottom - to.top);
        // Source size, from the 1x layout size and which set was loaded.
        let (sw, sh) = (w * 96 / s.dpi * s.bitmap_scale, h * 96 / s.dpi * s.bitmap_scale);
        if sw == w && sh == h {
            BitBlt(dc, to.left, to.top, w, h, src, 0, 0, SRCCOPY);
        } else {
            SetStretchBltMode(dc, HALFTONE);
            SetBrushOrgEx(dc, 0, 0, core::ptr::null_mut());
            StretchBlt(dc, to.left, to.top, w, h, src, 0, 0, sw, sh, SRCCOPY);
        }
        SelectObject(src, old);
        DeleteDC(src);
    }
}

fn button_rect(s: &State) -> Option<Rect> {
    (s.button != Button::None).then(|| s.rect(BUTTON))
}

fn invalidate(hwnd: HWND, r: Option<Rect>) {
    unsafe {
        match r {
            Some(r) => InvalidateRect(hwnd, &r, 0),
            None => InvalidateRect(hwnd, core::ptr::null(), 0),
        };
    }
}

fn press_button(hwnd: HWND, s: &mut State) {
    match s.button {
        Button::Cancel => request_cancel(hwnd, s),
        Button::Ok => finish(hwnd, 0),
        Button::None => {}
    }
}

fn request_cancel(hwnd: HWND, s: &mut State) {
    if !CANCELLABLE.load(Ordering::Acquire) {
        return;
    }
    CANCEL.store(true, Ordering::Release);
    s.button = Button::None;
    s.hot = false;
    invalidate(hwnd, Some(s.rect(BUTTON)));
}

fn finish(hwnd: HWND, code: u32) {
    unsafe {
        PostQuitMessage(code as i32);
        DestroyWindow(hwnd);
    }
}

fn mouse_pos(lparam: isize) -> (i32, i32) {
    ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32)
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    let Some(s) = state() else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    unsafe {
        match msg {
            // The whole window is client area: no caption, no border.
            WM_NCCALCSIZE => 0,
            // Drag the window by anything that isn't the button. Returning
            // only these two also turns off resizing by the edges.
            WM_NCHITTEST => {
                let mut pt = [mouse_pos(lparam).0, mouse_pos(lparam).1];
                ScreenToClient(hwnd, &mut pt);
                match button_rect(s) {
                    Some(r) if r.contains(pt[0], pt[1]) => HTCLIENT,
                    _ => HTCAPTION,
                }
            }
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                paint(hwnd, s);
                0
            }
            WM_SETCURSOR if s.hot => {
                SetCursor(LoadCursorW(core::ptr::null_mut(), 32649 as *const u16)); // IDC_HAND
                1
            }
            WM_MOUSEMOVE => {
                let (x, y) = mouse_pos(lparam);
                let hot = button_rect(s).is_some_and(|r| r.contains(x, y));
                if hot != s.hot {
                    s.hot = hot;
                    invalidate(hwnd, button_rect(s));
                }
                if !s.tracking {
                    let mut t = TrackMouseEventInfo {
                        size: core::mem::size_of::<TrackMouseEventInfo>() as u32,
                        flags: TME_LEAVE,
                        hwnd,
                        hover_time: 0,
                    };
                    s.tracking = TrackMouseEvent(&mut t) != 0;
                }
                0
            }
            WM_MOUSELEAVE => {
                s.tracking = false;
                s.pressed = false;
                if s.hot {
                    s.hot = false;
                    invalidate(hwnd, button_rect(s));
                }
                0
            }
            WM_LBUTTONDOWN => {
                s.pressed = s.hot;
                0
            }
            WM_LBUTTONUP => {
                if s.pressed && s.hot {
                    press_button(hwnd, s);
                }
                s.pressed = false;
                0
            }
            WM_KEYDOWN if wparam == VK_ESCAPE || (wparam == VK_RETURN && s.button == Button::Ok) => {
                press_button(hwnd, s);
                0
            }
            // Closing from the taskbar is a cancel request; the worker
            // decides when it's safe to stop and then closes the window.
            WM_CLOSE => {
                if s.success {
                    finish(hwnd, 0);
                } else {
                    request_cancel(hwnd, s);
                }
                0
            }
            MSG_STATUS => {
                let text = Box::from_raw(lparam as *mut Vec<u16>);
                if !CANCEL.load(Ordering::Acquire) {
                    s.text = *text;
                    invalidate(hwnd, Some(s.rect(TEXT)));
                }
                0
            }
            MSG_PROGRESS => {
                let bar = GetDlgItem(hwnd, ID_PROGRESS);
                let style = GetWindowLongW(bar, GWL_STYLE) as u32;
                if wparam == MARQUEE {
                    if style & PBS_MARQUEE == 0 {
                        SetWindowLongW(bar, GWL_STYLE, (style | PBS_MARQUEE) as i32);
                        SendMessageW(bar, PBM_SETMARQUEE, 1, 30);
                    }
                } else {
                    if style & PBS_MARQUEE != 0 {
                        SendMessageW(bar, PBM_SETMARQUEE, 0, 0);
                        SetWindowLongW(bar, GWL_STYLE, (style & !PBS_MARQUEE) as i32);
                    }
                    SendMessageW(bar, PBM_SETPOS, wparam, 0);
                }
                0
            }
            MSG_HIDE_BUTTON => {
                if s.button == Button::Cancel {
                    s.button = Button::None;
                    s.hot = false;
                    invalidate(hwnd, Some(s.rect(BUTTON)));
                }
                0
            }
            MSG_SUCCESS => {
                s.success = true;
                s.button = Button::Ok;
                s.hot = false;
                ShowWindow(GetDlgItem(hwnd, ID_PROGRESS), SW_HIDE);
                invalidate(hwnd, None);
                0
            }
            MSG_FAILED => {
                let text = Box::from_raw(lparam as *mut String);
                win::message_box(hwnd, &text, crate::config::product().name, MB_ICONERROR);
                finish(hwnd, EXIT_FAILED);
                0
            }
            // After the success prompt the window stays until OK.
            MSG_DONE => {
                if !s.success {
                    finish(hwnd, wparam as u32);
                }
                0
            }
            WM_DESTROY => {
                WINDOW.store(core::ptr::null_mut(), Ordering::Release);
                let s = STATE.swap(core::ptr::null_mut(), Ordering::AcqRel);
                if !s.is_null() {
                    drop(Box::from_raw(s));
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn post(msg: u32, wparam: usize, lparam: isize) {
    let hwnd = window();
    let ok = !hwnd.is_null() && unsafe { PostMessageW(hwnd, msg, wparam, lparam) } != 0;
    if !ok && lparam != 0 {
        // Nobody will free it on the other side.
        unsafe {
            match msg {
                MSG_STATUS => drop(Box::from_raw(lparam as *mut Vec<u16>)),
                MSG_FAILED => drop(Box::from_raw(lparam as *mut String)),
                _ => {}
            }
        }
    }
}

pub fn status(text: &str) {
    log!("{text}");
    post(MSG_STATUS, 0, Box::into_raw(Box::new(wide(text))) as isize);
}

pub fn progress(done: u64, total: u64) {
    let permille = if total == 0 { 0 } else { (done.min(total) * 1000 / total) as usize };
    post(MSG_PROGRESS, permille, 0);
}

/// An animated bar, for steps that can't say how far along they are.
pub fn busy() {
    post(MSG_PROGRESS, MARQUEE, 0);
}

pub fn check_cancel() -> Result<()> {
    if CANCEL.load(Ordering::Acquire) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

/// From here on there's no Cancel: the install is past the point where
/// stopping would leave things half-registered.
pub fn disable_cancel() {
    CANCELLABLE.store(false, Ordering::Release);
    post(MSG_HIDE_BUTTON, 0, 0);
}

/// Swaps the progress for the "successfully installed" message and an OK
/// button. The window then stays up until OK is pressed.
pub fn success() {
    post(MSG_SUCCESS, 0, 0);
}
