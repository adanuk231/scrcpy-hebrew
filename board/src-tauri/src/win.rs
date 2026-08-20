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
    fn GetClientRect(hwnd: isize, rect: *mut Rect) -> i32;
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

#[link(name = "shell32")]
extern "system" {
    fn SHAppBarMessage(message: u32, data: *mut AppBarData) -> usize;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(hwnd: isize, attr: u32, out: *mut Rect, size: u32) -> i32;
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
const SW_SHOW: i32 = 5;
const SW_SHOWNA: i32 = 8;

const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;

const SPI_GETWORKAREA: u32 = 0x0030;
const WM_CLOSE: u32 = 0x0010;
const VK_LBUTTON: i32 = 0x01;
const VK_CONTROL: i32 = 0x11;

// ------------------------------------------------------------- finding -----

struct Hunt {
    prefix: String,
    found: Vec<(isize, String)>,
    first_only: bool,
}

struct ExeHunt {
    exe: String,
    skip_pid: u32,
    found: isize,
}

extern "system" fn exe_hunt_cb(hwnd: isize, param: isize) -> i32 {
    unsafe {
        let hunt = &mut *(param as *mut ExeHunt);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == hunt.skip_pid {
            return 1;
        }
        let mut buf = [0u16; 8];
        if GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) == 0 {
            return 1;                      // no title: not somebody's main window
        }
        if owner_exe(hwnd) == hunt.exe {
            hunt.found = hwnd;
            return 0;
        }
    }
    1
}

/// Another copy of us, already running - hidden windows included, which is
/// exactly the case that matters when the first copy started into the tray.
pub fn window_of_other_instance(exe: &str, our_pid: u32) -> Option<isize> {
    let mut hunt = ExeHunt { exe: exe.to_string(), skip_pid: our_pid, found: 0 };
    unsafe { EnumWindows(exe_hunt_cb, &mut hunt as *mut ExeHunt as isize) };
    if hunt.found == 0 { None } else { Some(hunt.found) }
}

pub fn show_and_activate(hwnd: isize) {
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
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

/// What the window *looks* like it occupies.
///
/// GetWindowRect answers with the extended bounds, which on Windows 10 include
/// an invisible resize border of about seven pixels a side. Snap two windows
/// so those rectangles touch and you get a visible gap of fourteen; snap them
/// so they look flush and the rectangles overlap. Every piece of magnetism
/// here works in these coordinates instead.
pub fn visible_rect(hwnd: isize) -> Option<Rect> {
    const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
    let mut r = Rect::default();
    let ok = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r,
                              std::mem::size_of::<Rect>() as u32)
    };
    if ok == 0 && r.w() > 0 && r.h() > 0 {
        Some(r)
    } else {
        rect_of(hwnd)
    }
}

/// Put the *visible* top-left corner where asked, border allowed for.
pub fn move_visible_to(hwnd: isize, x: i32, y: i32) {
    let (outer, inner) = match (rect_of(hwnd), visible_rect(hwnd)) {
        (Some(o), Some(i)) => (o, i),
        _ => return,
    };
    move_to(hwnd, x - (inner.left - outer.left), y - (inner.top - outer.top));
}

/// The picture itself, without frame or title bar.
pub fn client_size(hwnd: isize) -> Option<(i32, i32)> {
    let mut r = Rect::default();
    if unsafe { GetClientRect(hwnd, &mut r) } == 0 || r.w() <= 0 || r.h() <= 0 {
        return None;
    }
    Some((r.w(), r.h()))
}

/// How much bigger the window looks than the picture inside it.
pub fn frame_extra(hwnd: isize) -> (i32, i32) {
    match (visible_rect(hwnd), client_size(hwnd)) {
        (Some(v), Some((cw, ch))) => (v.w() - cw, v.h() - ch),
        _ => (0, 0),
    }
}

/// Move and size in visible coordinates, border allowed for.
pub fn place_visible(hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
    let (outer, inner) = match (rect_of(hwnd), visible_rect(hwnd)) {
        (Some(o), Some(i)) => (o, i),
        _ => return,
    };
    let pad_x = inner.left - outer.left;
    let pad_y = inner.top - outer.top;
    let pad_w = outer.w() - inner.w();
    let pad_h = outer.h() - inner.h();
    place(hwnd, x - pad_x, y - pad_y, w + pad_w, h + pad_h);
}

pub fn work_area() -> Rect {
    let mut r = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
    unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r, 0) };
    r
}

pub fn mouse_down() -> bool {
    (unsafe { GetAsyncKeyState(VK_LBUTTON) } as u16 & 0x8000) != 0
}

/// Held down while dragging a phone, this takes it out of its group.
pub fn ctrl_down() -> bool {
    (unsafe { GetAsyncKeyState(VK_CONTROL) } as u16 & 0x8000) != 0
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


// ---------------------------------------------------- drags, from Windows --

const EVENT_SYSTEM_MOVESIZESTART: u32 = 0x000A;
const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;

type WinEventProc = extern "system" fn(isize, u32, isize, i32, i32, u32, u32);

#[repr(C)]
#[derive(Default)]
struct Msg {
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt_x: i32,
    pt_y: i32,
    private: u32,
}

#[link(name = "user32")]
extern "system" {
    fn SetWinEventHook(min: u32, max: u32, module: isize, callback: WinEventProc,
                       pid: u32, tid: u32, flags: u32) -> isize;
    fn UnhookWinEvent(hook: isize) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
}

/// Which window is being dragged, and when the drag ends, straight from the
/// shell. Guessing this from the foreground window plus the mouse button is
/// what it replaces - that guess is wrong in every case where the user grabs a
/// window the board did not think was interesting yet.
pub enum Drag {
    Started(isize),
    Ended(isize),
}

static DRAGS: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<Drag>>> =
    std::sync::OnceLock::new();

extern "system" fn drag_proc(_hook: isize, event: u32, hwnd: isize, id_object: i32,
                             id_child: i32, _thread: u32, _time: u32) {
    if id_object != 0 || id_child != 0 || hwnd == 0 {
        return;                            // a scrollbar or a caret, not a window
    }
    if let Some(tx) = DRAGS.get() {
        let message = if event == EVENT_SYSTEM_MOVESIZESTART {
            Drag::Started(hwnd)
        } else {
            Drag::Ended(hwnd)
        };
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(message);
        }
    }
}

pub fn watch_drags() -> std::sync::mpsc::Receiver<Drag> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = DRAGS.set(std::sync::Mutex::new(tx));
    std::thread::spawn(|| unsafe {
        let hook = SetWinEventHook(
            EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND, 0, drag_proc, 0, 0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
        let mut msg = Msg::default();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWinEvent(hook);
    });
    rx
}

// ------------------------------------------------------- where the tray is --

#[repr(C)]
#[derive(Default)]
struct AppBarData {
    cb_size: u32,
    hwnd: isize,
    callback: u32,
    edge: u32,
    rc: Rect,
    lparam: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Default)]
struct MonitorInfo {
    cb_size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[link(name = "user32")]
extern "system" {
    fn MonitorFromPoint(point: Point, flags: u32) -> isize;
    fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    fn FindWindowExW(parent: isize, after: isize, class: *const u16, title: *const u16) -> isize;
    fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
}

const ABM_GETTASKBARPOS: u32 = 0x0000_0005;
const MONITOR_DEFAULTTONEAREST: u32 = 2;

/// Which side of its screen the taskbar - and so the tray - lives on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// The usable area of the screen a point is on, not of the primary screen.
pub fn work_area_at(x: i32, y: i32) -> Rect {
    let mut info = MonitorInfo::default();
    info.cb_size = std::mem::size_of::<MonitorInfo>() as u32;
    let monitor = unsafe { MonitorFromPoint(Point { x, y }, MONITOR_DEFAULTTONEAREST) };
    if monitor != 0 && unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
        return info.work;
    }
    work_area()
}

/// Where the taskbar is, straight from the shell. Only used when the tray
/// icon will not say where it is itself.
pub fn taskbar() -> Option<(Rect, Edge)> {
    let mut data = AppBarData::default();
    data.cb_size = std::mem::size_of::<AppBarData>() as u32;
    if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) } == 0 {
        return None;
    }
    let edge = match data.edge {
        0 => Edge::Left,
        1 => Edge::Top,
        2 => Edge::Right,
        _ => Edge::Bottom,
    };
    Some((data.rc, edge))
}

/// Which side of `area` the strip at `bar` is sitting on. The taskbar is
/// exactly the part of the screen that is not work area, so its own rectangle
/// answers this without any guessing about docking.
pub fn edge_of(bar: &Rect, area: &Rect) -> Edge {
    if bar.top >= area.bottom {
        Edge::Bottom
    } else if bar.bottom <= area.top {
        Edge::Top
    } else if bar.right <= area.left {
        Edge::Left
    } else if bar.left >= area.right {
        Edge::Right
    } else {
        // an auto-hiding taskbar overlaps the work area; fall back to whichever
        // side of the screen it is closest to
        let to_bottom = (area.bottom - bar.bottom).abs();
        let to_top = (bar.top - area.top).abs();
        let to_left = (bar.left - area.left).abs();
        let to_right = (area.right - bar.right).abs();
        let least = to_bottom.min(to_top).min(to_left).min(to_right);
        if least == to_left {
            Edge::Left
        } else if least == to_right {
            Edge::Right
        } else if least == to_top {
            Edge::Top
        } else {
            Edge::Bottom
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    let mut v: Vec<u16> = text.encode_utf16().collect();
    v.push(0);
    v
}

/// The notification area itself, asked for by name.
///
/// Not "the right hand end of the taskbar": on a right-to-left Windows - a
/// Hebrew one, for instance - the tray is at the *left* end, and a panel
/// anchored to the right would open at the other side of the screen from the
/// icon it belongs to.
pub fn tray_area() -> Option<Rect> {
    let shell = unsafe { FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null()) };
    if shell == 0 {
        return None;
    }
    let notify = unsafe {
        FindWindowExW(shell, 0, wide("TrayNotifyWnd").as_ptr(), std::ptr::null())
    };
    if notify == 0 {
        return rect_of(shell);
    }
    rect_of(notify)
}
