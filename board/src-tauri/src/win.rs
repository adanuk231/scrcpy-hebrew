// Some of this is a toolkit the UI reaches for only in one mode or another.
#![allow(dead_code)]

//! The win32 surface the board needs: find the scrcpy windows, read and move
//! them, and hand the keyboard to one. Declared by hand rather than pulled
//! from the `windows` crate - it is a handful of calls, and this way nothing
//! has to agree with tauri about crate versions.

#[link(name = "user32")]
extern "system" {
    fn SetParent(child: isize, parent: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
    fn MoveWindow(hwnd: isize, x: i32, y: i32, w: i32, h: i32, repaint: i32) -> i32;
    fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    fn EnumWindows(cb: extern "system" fn(isize, isize) -> i32, param: isize) -> i32;
    fn GetWindowTextW(hwnd: isize, buf: *mut u16, max: i32) -> i32;
    fn IsWindow(hwnd: isize) -> i32;
    fn IsWindowVisible(hwnd: isize) -> i32;
    fn IsIconic(hwnd: isize) -> i32;
    fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
    fn SetFocus(hwnd: isize) -> isize;
    fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
    fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
    fn AttachThreadInput(attach: u32, attach_to: u32, join: i32) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn SystemParametersInfoW(action: u32, param: u32, data: *mut Rect, ini: u32) -> i32;
    fn SetWindowRgn(hwnd: isize, rgn: isize, redraw: i32) -> i32;
    fn GetForegroundWindow() -> isize;
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn SetWindowTextW(hwnd: isize, text: *const u16) -> i32;
    fn PostMessageW(hwnd: isize, msg: u32, w: usize, l: isize) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateRoundRectRgn(l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn QueryFullProcessImageNameW(h: isize, flags: u32, buf: *mut u16, size: *mut u32) -> i32;
}

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

/// Which executable owns this window. Window handles are recycled, so a handle
/// remembered from an earlier run can belong to somebody else entirely by the
/// time we read it back - this is what keeps the board from closing a
/// stranger's window.
pub fn owner_exe(hwnd: isize) -> String {
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == 0 {
        return String::new();
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return String::new();
    }
    let mut buf = [0u16; 512];
    let mut size = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..size as usize]);
    path.rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn w(&self) -> i32 {
        self.right - self.left
    }
    pub fn h(&self) -> i32 {
        self.bottom - self.top
    }
}

const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;

const WS_CHILD: isize = 0x4000_0000;
const WS_POPUP: isize = 0x8000_0000u32 as isize;
const WS_CAPTION: isize = 0x00C0_0000;
const WS_THICKFRAME: isize = 0x0004_0000;
const WS_SYSMENU: isize = 0x0008_0000;
const WS_MINIMIZEBOX: isize = 0x0002_0000;
const WS_MAXIMIZEBOX: isize = 0x0001_0000;

const WS_EX_APPWINDOW: isize = 0x0004_0000;
const WS_EX_WINDOWEDGE: isize = 0x0000_0100;
const WS_EX_CLIENTEDGE: isize = 0x0000_0200;
const WS_EX_DLGMODALFRAME: isize = 0x0000_0001;

const SW_HIDE: i32 = 0;
const SW_SHOWNA: i32 = 8;

const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;

const SPI_GETWORKAREA: u32 = 0x0030;
const WM_CLOSE: u32 = 0x0010;
const VK_LBUTTON: i32 = 0x01;

// ------------------------------------------------------------- finding -----

struct Hunt {
    prefix: String,
    found: Vec<(isize, String)>,
    first_only: bool,
}

extern "system" fn hunt_cb(hwnd: isize, param: isize) -> i32 {
    unsafe {
        let hunt = &mut *(param as *mut Hunt);
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 {
            let title = String::from_utf16_lossy(&buf[..n as usize]);
            if title.starts_with(&hunt.prefix) {
                hunt.found.push((hwnd, title));
                if hunt.first_only {
                    return 0;
                }
            }
        }
    }
    1
}

/// scrcpy is often reached through a shim (chocolatey installs one), so the
/// process we spawned is not the process that owns the window. The title we
/// gave it is the reliable handle.
pub fn find_by_title(title: &str) -> Option<isize> {
    let mut hunt = Hunt { prefix: title.to_string(), found: Vec::new(), first_only: true };
    unsafe { EnumWindows(hunt_cb, &mut hunt as *mut Hunt as isize) };
    hunt.found
        .into_iter()
        .find(|(_, t)| t == title)
        .map(|(hwnd, _)| hwnd)
}

pub fn find_all_with_prefix(prefix: &str) -> Vec<(isize, String)> {
    let mut hunt = Hunt { prefix: prefix.to_string(), found: Vec::new(), first_only: false };
    unsafe { EnumWindows(hunt_cb, &mut hunt as *mut Hunt as isize) };
    hunt.found
}

pub fn is_window(hwnd: isize) -> bool {
    unsafe { IsWindow(hwnd) != 0 }
}

pub fn is_minimised(hwnd: isize) -> bool {
    unsafe { IsIconic(hwnd) != 0 }
}

pub fn rect_of(hwnd: isize) -> Option<Rect> {
    let mut r = Rect::default();
    if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
        return None;
    }
    Some(r)
}

pub fn work_area() -> Rect {
    let mut r = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
    unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r, 0) };
    r
}

pub fn mouse_down() -> bool {
    (unsafe { GetAsyncKeyState(VK_LBUTTON) } as u16 & 0x8000) != 0
}

pub fn foreground() -> isize {
    unsafe { GetForegroundWindow() }
}

// -------------------------------------------------------------- moving -----

pub fn move_to(hwnd: isize, x: i32, y: i32) {
    unsafe {
        SetWindowPos(hwnd, 0, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn place(hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
    unsafe { MoveWindow(hwnd, x, y, w.max(1), h.max(1), 1) };
}

pub fn show(hwnd: isize, visible: bool) {
    unsafe { ShowWindow(hwnd, if visible { SW_SHOWNA } else { SW_HIDE }) };
}

pub fn raise(hwnd: isize) {
    unsafe {
        SetWindowPos(hwnd, 0, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Round the phone's corners for skin mode. The region belongs to the window,
/// so it has to be reapplied every time the window is resized.
pub fn round_corners(hwnd: isize, radius: i32) {
    if let Some(r) = rect_of(hwnd) {
        let rgn = unsafe { CreateRoundRectRgn(0, 0, r.w() + 1, r.h() + 1, radius, radius) };
        if rgn != 0 {
            unsafe { SetWindowRgn(hwnd, rgn, 1) };
        }
    }
}

pub fn square_corners(hwnd: isize) {
    unsafe { SetWindowRgn(hwnd, 0, 1) };
}

/// Strip the frame off a window we did not start borderless.
pub fn chromeless(hwnd: isize, on: bool) {
    unsafe {
        let mut style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if on {
            style &= !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
        } else {
            style |= WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        }
        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
        SetWindowPos(hwnd, 0, 0, 0, 0, 0,
                     SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

// ------------------------------------------------------------ keyboard -----

pub fn thread_of(hwnd: isize) -> u32 {
    unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) }
}

pub fn current_thread() -> u32 {
    unsafe { GetCurrentThreadId() }
}

/// Only needed when a scrcpy window is a *child* of ours: keys follow the
/// focus of the foreground thread's input queue, and a reparented window from
/// another process is not in it. Free-standing windows need none of this.
pub fn join_input(other: u32, join: bool) -> bool {
    let ours = current_thread();
    if ours == other || other == 0 {
        return true;
    }
    unsafe { AttachThreadInput(ours, other, if join { 1 } else { 0 }) != 0 }
}

/// The title we start scrcpy with has to be unique so we can find the window;
/// once we have it, the phone should be called what the phone is called.
/// Ask scrcpy to close the way clicking its X would, so it gets to put the
/// phone's screen back on and clean up after itself.
pub fn ask_to_close(hwnd: isize) {
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
}

pub fn rename(hwnd: isize, text: &str) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    unsafe { SetWindowTextW(hwnd, wide.as_ptr()) };
}

pub fn activate(hwnd: isize) {
    unsafe { SetForegroundWindow(hwnd) };
}

pub fn focus(hwnd: isize) {
    unsafe { SetFocus(hwnd) };
}

/// Kept from the embedded experiment: adopt a window as a child of `host`.
/// Not on the default path - the phones are nicer as ordinary windows - but
/// this is what a future dock mode needs.
#[allow(dead_code)]
pub fn adopt(hwnd: isize, host: isize) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let style = (style | WS_CHILD)
            & !WS_POPUP
            & !WS_CAPTION
            & !WS_THICKFRAME
            & !WS_SYSMENU
            & !WS_MINIMIZEBOX
            & !WS_MAXIMIZEBOX;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE)
            & !WS_EX_APPWINDOW
            & !WS_EX_WINDOWEDGE
            & !WS_EX_CLIENTEDGE
            & !WS_EX_DLGMODALFRAME;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
        SetParent(hwnd, host);
        SetWindowPos(hwnd, 0, 0, 0, 0, 0,
                     SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}
