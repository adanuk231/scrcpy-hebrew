//! Run at login, written straight into the user's Run key.
//!
//! No installer, no scheduled task, no elevation: one string under
//! HKCU that Windows reads at sign-in, and deleting it is all that
//! turning this off means.

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, access: u32,
                     out: *mut isize) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32,
                      data: *const u8, size: u32) -> i32;
    fn RegQueryValueExW(key: isize, name: *const u16, reserved: *mut u32,
                        kind: *mut u32, data: *mut u8, size: *mut u32) -> i32;
    fn RegDeleteValueW(key: isize, name: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_SZ: u32 = 1;
const ERROR_SUCCESS: i32 = 0;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "scrcpy-board";

fn wide(text: &str) -> Vec<u16> {
    let mut v: Vec<u16> = text.encode_utf16().collect();
    v.push(0);
    v
}

fn open(access: u32) -> Option<isize> {
    let sub = wide(RUN_KEY);
    let mut key: isize = 0;
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut key) };
    if rc == ERROR_SUCCESS {
        Some(key)
    } else {
        None
    }
}

/// What the Run key would have to say for this exe to be the one starting.
fn command() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    // --hidden so signing in gives you the tray icon, not a window in the way
    format!("\"{}\" --hidden", exe)
}

pub fn enabled() -> bool {
    let key = match open(KEY_QUERY_VALUE) {
        Some(k) => k,
        None => return false,
    };
    let name = wide(VALUE);
    let mut size: u32 = 0;
    let rc = unsafe {
        RegQueryValueExW(key, name.as_ptr(), std::ptr::null_mut(),
                         std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
    };
    if rc != ERROR_SUCCESS || size == 0 {
        unsafe { RegCloseKey(key) };
        return false;
    }
    let mut buf = vec![0u8; size as usize];
    let rc = unsafe {
        RegQueryValueExW(key, name.as_ptr(), std::ptr::null_mut(),
                         std::ptr::null_mut(), buf.as_mut_ptr(), &mut size)
    };
    unsafe { RegCloseKey(key) };
    if rc != ERROR_SUCCESS {
        return false;
    }
    let wide: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|c| *c != 0)
        .collect();
    let stored = String::from_utf16_lossy(&wide);
    // an entry pointing at an exe that has since moved is not this one
    stored.eq_ignore_ascii_case(&command())
}

pub fn set(on: bool) -> Result<(), String> {
    let key = open(KEY_SET_VALUE).ok_or("cannot open the Run key")?;
    let name = wide(VALUE);
    let rc = if on {
        let value = wide(&command());
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2)
        };
        unsafe {
            RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, bytes.as_ptr(), bytes.len() as u32)
        }
    } else {
        let rc = unsafe { RegDeleteValueW(key, name.as_ptr()) };
        // already gone is the state we wanted anyway
        if rc == 2 { ERROR_SUCCESS } else { rc }
    };
    unsafe { RegCloseKey(key) };
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("registry refused it (code {})", rc))
    }
}


// ------------------------------------------------------- one copy only ----

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(attrs: *const u8, owner: i32, name: *const u16) -> isize;
    fn GetLastError() -> u32;
}

const ERROR_ALREADY_EXISTS: u32 = 183;

/// Started at login *and* double-clicked is the normal way to end up with two
/// boards, two tray icons and two magnet loops fighting over the same windows.
/// The first copy keeps the mutex for the life of the process; later ones are
/// told to go away.
pub fn claim_only_copy() -> bool {
    let name = wide("scrcpy-board-single-instance");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle == 0 {
        return true;                      // cannot tell, so do not block startup
    }
    unsafe { GetLastError() != ERROR_ALREADY_EXISTS }
}
