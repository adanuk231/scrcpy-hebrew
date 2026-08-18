# -*- coding: utf-8 -*-
"""
scrcpy-hebrew - Hebrew (and any non-Latin) keyboard input for scrcpy on Windows.

scrcpy converts your keystrokes into Android KeyEvents using the device's
KeyCharacterMap, which only covers ASCII. Every Hebrew, Arabic, Russian, Greek
or Persian key therefore dies in the log with:

    [server] WARN: Could not inject char u+05d1

There is no single fix that works on every phone, so this tool probes the device
and picks the one that does. Latin input is never touched either way.

  uhid  - runs scrcpy with --keyboard=uhid, so the phone sees a real USB
          keyboard and does the character mapping itself. Non-Latin input then
          works natively: real keystrokes, no buffering, no clipboard. Needs the
          phone to have a hardware-keyboard layout bound to a matching IME
          subtype (Settings -> Physical keyboard). The daemon keeps the phone's
          hardware language in step with the Windows layout, so Alt+Shift on the
          PC still switches languages.

  paste - a low-level keyboard hook swallows the non-Latin keystrokes, buffers
          them per word, and hands each word to the device through scrcpy's
          clipboard (Ctrl+V, which syncs the host clipboard to the device
          first). Used when the phone cannot map the keys itself.

Two findings worth knowing, both verified on real devices:

  * Samsung phones reject scrcpy's clipboard write outright, with
    "SecurityException: Given calling package android does not match caller's
    uid 2000" thrown from SemClipboardManager. On those, only uhid works.
  * scrcpy's MOD+v (Alt+v) pastes by *injecting the clipboard as key events* -
    the very thing that cannot represent non-ASCII. Ctrl+V is the one that
    syncs the real device clipboard. This tool uses Ctrl+V, and strips
    --legacy-paste from your arguments because it turns Ctrl+V back into key
    injection.

Usage
-----
    scrcpy_hebrew.py launch [-s SERIAL] [scrcpy args...]   scrcpy + daemon
    scrcpy_hebrew.py daemon                                daemon only
    scrcpy_hebrew.py probe  [SERIAL]                       report the mode

Requires Windows, Python 3.8+, and scrcpy 2.4+ on PATH (or set SCRCPY_EXE).
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import queue
import re
import shutil
import subprocess
import sys
import threading
import time
import tkinter as tk

# ---------------------------------------------------------------- tuning ----

FLUSH_ON_SPACE = True          # paste mode: deliver word by word as you type
IDLE_FLUSH_SEC = 1.0           # paste mode: deliver a partial word after silence
CLIPBOARD_SETTLE_SEC = 0.05
CLIPBOARD_RESTORE_SEC = 0.30
REVERSE_PREVIEW = True         # Tk has no bidi, so reverse RTL text for display
TARGET_EXES = ("scrcpy.exe",)
MARK = 0x48454252              # tags our own synthetic input so we ignore it

ALLOW_INJECTED = os.environ.get("SCRCPY_HEBREW_ALLOW_INJECTED") == "1"
DEBUG = os.environ.get("SCRCPY_HEBREW_DEBUG") == "1"

LOG_DIR = os.path.join(os.environ.get("LOCALAPPDATA", os.path.expanduser("~")),
                       "scrcpy-hebrew")
LOG_PATH = os.path.join(LOG_DIR, "daemon.log")


def scrcpy_exe():
    return os.environ.get("SCRCPY_EXE") or shutil.which("scrcpy") or "scrcpy.exe"


def log(msg):
    try:
        os.makedirs(LOG_DIR, exist_ok=True)
        with open(LOG_PATH, "a", encoding="utf-8") as fh:
            fh.write("%s  %s\n" % (time.strftime("%H:%M:%S"), msg))
    except Exception:
        pass


def dlog(msg):
    if DEBUG:
        log(msg)


# ------------------------------------------------------------- win32 glue ---

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
ULONG_PTR = ctypes.c_ulonglong if ctypes.sizeof(ctypes.c_void_p) == 8 else ctypes.c_ulong


class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [("vkCode", wt.DWORD), ("scanCode", wt.DWORD),
                ("flags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ULONG_PTR)]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD),
                ("time", wt.DWORD), ("dwExtraInfo", ULONG_PTR)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wt.DWORD), ("ki", KEYBDINPUT),
                ("padding", ctypes.c_byte * 8)]


LRESULT = ctypes.c_longlong if ctypes.sizeof(ctypes.c_void_p) == 8 else ctypes.c_long
HOOKPROC = ctypes.CFUNCTYPE(LRESULT, ctypes.c_int, wt.WPARAM, wt.LPARAM)

user32.SetWindowsHookExW.restype = wt.HHOOK
user32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, wt.HINSTANCE, wt.DWORD]
user32.CallNextHookEx.restype = LRESULT
user32.CallNextHookEx.argtypes = [wt.HHOOK, ctypes.c_int, wt.WPARAM, wt.LPARAM]
user32.GetForegroundWindow.restype = wt.HWND
user32.GetKeyboardLayout.restype = wt.HKL
user32.GetKeyboardLayout.argtypes = [wt.DWORD]
user32.GetKeyboardLayoutList.argtypes = [ctypes.c_int, ctypes.POINTER(wt.HKL)]
user32.SendInput.argtypes = [wt.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
user32.ToUnicodeEx.argtypes = [wt.UINT, wt.UINT, ctypes.c_char_p,
                               ctypes.c_wchar_p, ctypes.c_int, wt.UINT, wt.HKL]
user32.GetClipboardData.restype = wt.HANDLE
user32.SetClipboardData.restype = wt.HANDLE
user32.SetClipboardData.argtypes = [wt.UINT, wt.HANDLE]
kernel32.GlobalAlloc.restype = wt.HGLOBAL
kernel32.GlobalLock.restype = wt.LPVOID
kernel32.GlobalLock.argtypes = [wt.HGLOBAL]
kernel32.GlobalUnlock.argtypes = [wt.HGLOBAL]
kernel32.OpenProcess.restype = wt.HANDLE

WH_KEYBOARD_LL = 13
WM_KEYDOWN, WM_SYSKEYDOWN = 0x0100, 0x0104
LLKHF_INJECTED = 0x10
CF_UNICODETEXT = 13
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_SCANCODE = 0x0008
KEYEVENTF_EXTENDEDKEY = 0x0001

VK_BACK, VK_SPACE = 0x08, 0x20
VK_SHIFT, VK_CONTROL, VK_MENU, VK_CAPITAL = 0x10, 0x11, 0x12, 0x14
VK_LWIN, VK_RWIN = 0x5B, 0x5C
REPLAY_KEYS = {0x0D, 0x09, 0x1B,                       # Enter Tab Esc
               0x21, 0x22, 0x23, 0x24,                 # PgUp PgDn End Home
               0x25, 0x26, 0x27, 0x28,                 # arrows
               0x2D, 0x2E}                             # Insert Delete
EXTENDED_KEYS = {0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2D, 0x2E}

SC_LCTRL, SC_LSHIFT, SC_SPACE, SC_V = 0x1D, 0x2A, 0x39, 0x2F
LOCALE_SISO639LANGNAME = 0x0059


def held(vk):
    return bool(user32.GetAsyncKeyState(vk) & 0x8000)


def modifiers_held():
    return held(VK_SHIFT) or held(VK_CONTROL) or held(VK_MENU) or \
        held(VK_LWIN) or held(VK_RWIN)


_exe_cache = {}


def exe_of_window(hwnd):
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if not pid.value:
        return ""
    hit = _exe_cache.get(pid.value)
    if hit is not None:
        return hit
    name = ""
    h = kernel32.OpenProcess(0x1000, False, pid.value)   # QUERY_LIMITED_INFO
    if h:
        buf = ctypes.create_unicode_buffer(512)
        size = wt.DWORD(512)
        if kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            name = os.path.basename(buf.value).lower()
        kernel32.CloseHandle(h)
    if len(_exe_cache) > 256:
        _exe_cache.clear()
    _exe_cache[pid.value] = name
    return name


def window_title(hwnd):
    n = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(hwnd, buf, n + 1)
    return buf.value


# ------------------------------------------------------ keyboard layouts ----

def lang_tag(hkl):
    """ISO 639-1 code of a keyboard layout, e.g. 'he', 'ar', 'ru', 'en'."""
    buf = ctypes.create_unicode_buffer(16)
    kernel32.GetLocaleInfoW(hkl & 0xFFFF, LOCALE_SISO639LANGNAME, buf, 16)
    return buf.value.lower()


_latin_cache = {}


def layout_is_latin(hkl):
    """A layout is 'Latin' if its home-row keys produce plain ASCII."""
    key = hkl & 0xFFFF
    if key in _latin_cache:
        return _latin_cache[key]
    latin = True
    state = (ctypes.c_ubyte * 256)()
    out = ctypes.create_unicode_buffer(8)
    for vk in (0x41, 0x53, 0x44, 0x51):                  # A S D Q
        scan = user32.MapVirtualKeyW(vk, 0)
        n = user32.ToUnicodeEx(vk, scan, ctypes.cast(state, ctypes.c_char_p),
                               out, 8, 0, hkl)
        if n > 0 and out.value and ord(out.value[0]) > 0x7F:
            latin = False
            break
    _latin_cache[key] = latin
    return latin


def installed_non_latin_tags():
    n = user32.GetKeyboardLayoutList(0, None)
    arr = (wt.HKL * n)()
    user32.GetKeyboardLayoutList(n, arr)
    return {lang_tag(h) for h in arr if not layout_is_latin(h)}


def char_for(vk, scan, hkl):
    """Translate a keystroke through a layout, honouring live shift/caps."""
    state = (ctypes.c_ubyte * 256)()
    if held(VK_SHIFT):
        state[VK_SHIFT] = 0x80
    if user32.GetKeyState(VK_CAPITAL) & 1:
        state[VK_CAPITAL] = 0x01
    out = ctypes.create_unicode_buffer(8)
    n = user32.ToUnicodeEx(vk, scan, ctypes.cast(state, ctypes.c_char_p),
                           out, 8, 0, hkl)
    if n <= 0:
        return ""
    ch = out.value[:n]
    if len(ch) != 1 or ord(ch) < 0x20 or ord(ch) == 0x7F:
        return ""
    return ch


# ------------------------------------------------------------------ adb -----

def adb(serial, *args, timeout=20):
    cmd = ["adb"] + (["-s", serial] if serial else []) + list(args)
    try:
        return subprocess.run(cmd, capture_output=True, text=True,
                              encoding="utf-8", errors="replace",
                              timeout=timeout,
                              creationflags=0x08000000).stdout   # NO_WINDOW
    except Exception as exc:
        dlog("adb %r failed: %r" % (args, exc))
        return ""


def connected_devices():
    """{serial: model}"""
    devices = {}
    for line in adb(None, "devices", "-l").splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 2 and parts[1] == "device":
            model = next((p[6:] for p in parts[2:] if p.startswith("model:")), "")
            devices[parts[0]] = model
    return devices


NAMES_FILES = (
    os.path.join(LOG_DIR, "device-names.json"),
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "device-names.json"),
)

_NAME_CACHE = {}


def name_overrides():
    """{serial: 'Samsung S10'} - your own name for a phone, if you gave it one."""
    for path in NAMES_FILES:
        try:
            with open(path, encoding="utf-8") as fh:
                data = json.load(fh)
        except Exception:
            continue
        if isinstance(data, dict):
            return dict((str(k), str(v).strip()) for k, v in data.items()
                        if str(v).strip())
    return {}


def _clean(value):
    value = (value or "").strip()
    return "" if value.lower() in ("", "null", "unknown") else value


def device_name(serial, model=""):
    """The phone as a human calls it - 'Samsung S10', not 'SM_G973F'.

    device-names.json wins, then the name set on the phone itself, then the
    marketing name the vendor ships, and only then the raw model code.
    """
    if not serial:
        return _clean(model) or "device"
    override = name_overrides().get(serial)
    if override:
        return override
    if serial in _NAME_CACHE:
        return _NAME_CACHE[serial]

    name = _clean(adb(serial, "shell", "settings", "get", "global",
                      "device_name"))
    if not name:
        for prop in ("ro.product.marketname", "ro.product.vendor.marketname",
                     "ro.vendor.product.display"):
            name = _clean(adb(serial, "shell", "getprop", prop))
            if name:
                break
    if not name:
        vendor = _clean(adb(serial, "shell", "getprop",
                            "ro.product.manufacturer")).title()
        board = (_clean(adb(serial, "shell", "getprop", "ro.product.model"))
                 or _clean(model).replace("_", " "))
        name = (vendor + " " + board).strip() if board else vendor
    name = name or _clean(model).replace("_", " ") or serial
    _NAME_CACHE[serial] = name
    return name


def window_title_for(serial, devices):
    """device_name(), plus the serial only when two phones share a name."""
    name = device_name(serial, devices.get(serial, ""))
    clashes = [other for other in devices
               if other != serial and device_name(other, devices[other]) == name]
    return "%s [%s]" % (name, serial) if clashes else name


SUBTYPE_RE = re.compile(r"mLanguageTag=([A-Za-z-]+).*?:\s*(\S*keyboard_layout_\S+)")


def hardware_layouts(serial):
    """{'he': '<...>/keyboard_layout_hebrew', 'en': '.../english_us', ...}

    These are the hardware-keyboard layouts the phone has bound to an IME
    subtype - i.e. the languages it can actually type on a physical keyboard.
    """
    out = {}
    for line in adb(serial, "shell", "dumpsys input").splitlines():
        m = SUBTYPE_RE.search(line)
        if m:
            out.setdefault(m.group(1).split("-")[0].lower(), m.group(2))
    return out


def current_hardware_layout(serial):
    m = re.search(r"CurrentKeyboardLayout=(\S+)",
                  adb(serial, "shell", "dumpsys input"))
    return m.group(1) if m else None


def probe_mode(serial, tags=None):
    """'uhid' if the phone can type one of our non-Latin languages itself."""
    tags = tags or installed_non_latin_tags()
    have = hardware_layouts(serial)
    return "uhid" if (tags & set(have)) else "paste"


# ------------------------------------------------------------- clipboard ----

def _open_clipboard(tries=25):
    for _ in range(tries):
        if user32.OpenClipboard(None):
            return True
        time.sleep(0.02)
    return False


def get_clipboard_text():
    if not _open_clipboard():
        return None
    try:
        h = user32.GetClipboardData(CF_UNICODETEXT)
        if not h:
            return None
        p = kernel32.GlobalLock(h)
        if not p:
            return None
        try:
            return ctypes.wstring_at(p)
        finally:
            kernel32.GlobalUnlock(h)
    finally:
        user32.CloseClipboard()


def set_clipboard_text(text):
    if not _open_clipboard():
        return False
    try:
        user32.EmptyClipboard()
        data = text + "\0"
        size = len(data) * 2
        h = kernel32.GlobalAlloc(0x0042, size)           # MOVEABLE | ZEROINIT
        if not h:
            return False
        p = kernel32.GlobalLock(h)
        ctypes.memmove(p, ctypes.create_unicode_buffer(data), size)
        kernel32.GlobalUnlock(h)
        return bool(user32.SetClipboardData(CF_UNICODETEXT, h))
    finally:
        user32.CloseClipboard()


# ----------------------------------------------------------- synthetic in ---

def _send(items):
    arr = (INPUT * len(items))()
    for i, (scan, vk, flags) in enumerate(items):
        arr[i].type = 1
        arr[i].ki = KEYBDINPUT(wVk=vk, wScan=scan, dwFlags=flags, time=0,
                               dwExtraInfo=MARK)
    user32.SendInput(len(items), arr, ctypes.sizeof(INPUT))


def send_combo(mod_scans, key_scan):
    _send([(s, 0, KEYEVENTF_SCANCODE) for s in mod_scans] +
          [(key_scan, 0, KEYEVENTF_SCANCODE)])
    time.sleep(0.03)
    _send([(key_scan, 0, KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP)] +
          [(s, 0, KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP)
           for s in reversed(mod_scans)])


def send_paste():
    """Ctrl+V - scrcpy syncs the host clipboard to the device, then forwards it.

    Deliberately not MOD+v: in scrcpy 3.x that injects the clipboard as key
    events, which is exactly what cannot represent non-ASCII characters.
    """
    send_combo([SC_LCTRL], SC_V)


def send_vk(vk):
    scan = user32.MapVirtualKeyW(vk, 0)                  # uhid needs a scancode
    flags = KEYEVENTF_EXTENDEDKEY if vk in EXTENDED_KEYS else 0
    _send([(scan, vk, flags)])
    time.sleep(0.01)
    _send([(scan, vk, flags | KEYEVENTF_KEYUP)])


def send_lang_toggle():
    """Shift+Space - Android's hardware-keyboard language switch."""
    send_combo([SC_LSHIFT], SC_SPACE)


# ------------------------------------------------------------- the daemon ---

class Daemon:
    def __init__(self):
        self.lock = threading.RLock()
        self.buf = []
        self.suppressed = set()
        self.jobs = queue.Queue()
        self.inflight = 0
        self.last_key = 0.0
        self.status = ""
        self.status_at = 0.0
        self.stop = threading.Event()
        self.modes = {}                  # serial -> 'uhid' | 'paste'
        self.layouts = {}                # serial -> {tag: layout id}
        self.devices = {}                # serial -> model
        self.names = {}                  # serial -> 'Samsung S10'
        self.sync_want = {}              # serial -> tag
        self.sync_lock = threading.Lock()
        self.tags = installed_non_latin_tags()

    # ---- background: everything that talks to adb ----------------------
    def housekeeping(self):
        """adb is far too slow to call from inside the keyboard hook."""
        while not self.stop.is_set():
            found = connected_devices()
            if found:
                self.devices = found
                self.names = dict((serial, device_name(serial, model))
                                  for serial, model in found.items())
            for serial in list(self.devices):
                if serial not in self.modes:
                    self.layouts[serial] = hardware_layouts(serial)
                    self.modes[serial] = ("uhid" if self.tags &
                                          set(self.layouts[serial]) else "paste")
                    log("device %s (%s) -> %s mode, hardware layouts: %s"
                        % (serial, self.devices.get(serial, "?"),
                           self.modes[serial],
                           ",".join(sorted(self.layouts[serial])) or "none"))
            self.stop.wait(15)

    def serial_for(self, hwnd):
        title = window_title(hwnd)
        for serial in self.devices:
            if serial in title:
                return serial
        for serial, name in self.names.items():
            if name and name.lower() in title.lower():
                return serial
        for serial, model in self.devices.items():
            if model and model.replace("_", " ") in title.replace("_", " "):
                return serial
        return next(iter(self.devices)) if len(self.devices) == 1 else None

    def foreground(self):
        """(hwnd, serial, mode, tag) - tag is '' for a Latin layout."""
        hwnd = user32.GetForegroundWindow()
        if not hwnd or exe_of_window(hwnd) not in TARGET_EXES:
            return None, None, "", ""
        tid = user32.GetWindowThreadProcessId(hwnd, None)
        hkl = user32.GetKeyboardLayout(tid)
        tag = "" if layout_is_latin(hkl) else lang_tag(hkl)
        serial = self.serial_for(hwnd)
        return hwnd, serial, self.modes.get(serial, ""), tag

    def note(self, text):
        self.status = text
        self.status_at = time.time()

    # ---- paste mode ----------------------------------------------------
    def pending(self):
        with self.lock:
            return bool(self.buf) or self.inflight > 0 or not self.jobs.empty()

    def preview(self):
        with self.lock:
            return "".join(self.buf)

    def flush(self, hwnd):
        with self.lock:
            if not self.buf:
                return
            text = "".join(self.buf)
            self.buf = []
            self.inflight += 1
        self.jobs.put(("text", text, hwnd))

    def replay(self, vk, hwnd):
        with self.lock:
            self.inflight += 1
        self.jobs.put(("key", vk, hwnd))

    def worker(self):
        while not self.stop.is_set():
            try:
                kind, payload, hwnd = self.jobs.get(timeout=0.2)
            except queue.Empty:
                continue
            try:
                if not self._wait_for_focus(hwnd):
                    self.note("dropped (focus lost)")
                    log("dropped %r, scrcpy window lost focus" % (payload,))
                    continue
                if kind == "text":
                    self._deliver(payload)
                else:
                    send_vk(payload)
            except Exception as exc:
                log("worker error: %r" % (exc,))
            finally:
                with self.lock:
                    self.inflight -= 1

    def _wait_for_focus(self, hwnd, timeout=1.5):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if user32.GetForegroundWindow() == hwnd and not modifiers_held():
                return True
            time.sleep(0.03)
        return user32.GetForegroundWindow() == hwnd

    def _deliver(self, text):
        saved = get_clipboard_text()
        if not set_clipboard_text(text):
            self.note("clipboard busy")
            log("could not set clipboard for %r" % text)
            return
        time.sleep(CLIPBOARD_SETTLE_SEC)
        dlog("pasting %r" % text)
        send_paste()
        time.sleep(CLIPBOARD_RESTORE_SEC)
        if saved is not None and saved != text:
            set_clipboard_text(saved)

    # ---- uhid mode: keep the phone's language in step with Windows ------
    def request_sync(self, serial, tag, hwnd):
        with self.lock:
            if self.sync_want.get(serial) == tag:
                return
            self.sync_want[serial] = tag
        threading.Thread(target=self._sync, args=(serial, tag, hwnd),
                         daemon=True).start()

    def _sync(self, serial, tag, hwnd):
        want = self.layouts.get(serial, {}).get(tag or "en")
        if not want:
            return
        if not self.sync_lock.acquire(blocking=False):
            return
        try:
            for attempt in range(3):
                current = current_hardware_layout(serial)
                if current is None:
                    log("%s: cannot read the hardware layout" % serial)
                    return
                if current == want:
                    if attempt:
                        self.note((tag or "en").upper())
                    return
                if user32.GetForegroundWindow() != hwnd or modifiers_held():
                    return                   # user moved on; do not stray
                dlog("%s: toggling hardware language (want %s)" % (serial, want))
                send_lang_toggle()
                time.sleep(0.6)
            log("%s: hardware language did not settle on %s" % (serial, want))
        finally:
            self.sync_lock.release()

    # ---- the hook -------------------------------------------------------
    def on_key(self, msg, kb):
        if kb.dwExtraInfo == MARK:
            return False
        if (kb.flags & LLKHF_INJECTED) and not ALLOW_INJECTED:
            return False

        vk, scan = kb.vkCode, kb.scanCode
        if msg not in (WM_KEYDOWN, WM_SYSKEYDOWN):
            if vk in self.suppressed:
                self.suppressed.discard(vk)
                return True
            return False

        hwnd, serial, mode, tag = self.foreground()
        if not hwnd:
            return False

        if mode == "uhid":
            if serial:                    # the phone maps the keys itself
                self.request_sync(serial, tag, hwnd)
            return False

        if mode != "paste" or not tag:
            return False                  # unprobed or Latin: never swallow

        if held(VK_CONTROL) or held(VK_MENU) or held(VK_LWIN) or held(VK_RWIN):
            self.flush(hwnd)              # let scrcpy/device shortcuts through
            return False

        if vk == VK_BACK:
            with self.lock:
                popped = self.buf.pop() if self.buf else None
            if popped is not None:
                self.suppressed.add(vk)
                return True
            if self.pending():
                self.replay(vk, hwnd)
                self.suppressed.add(vk)
                return True
            return False

        if vk in REPLAY_KEYS:
            if self.pending():
                self.flush(hwnd)
                self.replay(vk, hwnd)
                self.suppressed.add(vk)
                return True
            return False

        tid = user32.GetWindowThreadProcessId(hwnd, None)
        ch = char_for(vk, scan, user32.GetKeyboardLayout(tid))
        if not ch:
            if self.pending():
                self.flush(hwnd)
            return False

        # ASCII only needs intercepting to stay in order behind queued text
        if ord(ch) < 0x80 and not self.pending():
            return False

        with self.lock:
            self.buf.append(ch)
            self.last_key = time.time()
        dlog("captured %r (vk=0x%02X) buffer=%r" % (ch, vk, self.preview()))
        self.suppressed.add(vk)
        if FLUSH_ON_SPACE and vk == VK_SPACE:
            self.flush(hwnd)
        return True

    def idle_tick(self):
        with self.lock:
            stale = self.buf and (time.time() - self.last_key) > IDLE_FLUSH_SEC
        if stale:
            hwnd, _, mode, _ = self.foreground()
            if hwnd and mode == "paste":
                self.flush(hwnd)


daemon = Daemon()
_proc_ref = []


def hook_thread():
    @HOOKPROC
    def proc(nCode, wParam, lParam):
        if nCode == 0:
            kb = ctypes.cast(lParam, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
            try:
                if daemon.on_key(wParam, kb):
                    return 1
            except Exception as exc:
                log("hook error: %r" % (exc,))
        return user32.CallNextHookEx(None, nCode, wParam, lParam)

    _proc_ref.append(proc)
    handle = user32.SetWindowsHookExW(WH_KEYBOARD_LL, proc, None, 0)
    if not handle:
        log("SetWindowsHookExW failed: %s" % ctypes.get_last_error())
        return
    log("hook installed")
    msg = wt.MSG()
    while not daemon.stop.is_set():
        if user32.PeekMessageW(ctypes.byref(msg), None, 0, 0, 1):
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))
        else:
            time.sleep(0.005)
    user32.UnhookWindowsHookEx(handle)


def scrcpy_running():
    class PROCENTRY32W(ctypes.Structure):
        _fields_ = [("dwSize", wt.DWORD), ("cntUsage", wt.DWORD),
                    ("th32ProcessID", wt.DWORD), ("th32DefaultHeapID", ULONG_PTR),
                    ("th32ModuleID", wt.DWORD), ("cntThreads", wt.DWORD),
                    ("th32ParentProcessID", wt.DWORD),
                    ("pcPriClassBase", ctypes.c_long),
                    ("dwFlags", wt.DWORD), ("szExeFile", ctypes.c_wchar * 260)]

    snap = kernel32.CreateToolhelp32Snapshot(0x00000002, 0)
    if snap == wt.HANDLE(-1).value:
        return True
    entry = PROCENTRY32W()
    entry.dwSize = ctypes.sizeof(PROCENTRY32W)
    try:
        ok = kernel32.Process32FirstW(snap, ctypes.byref(entry))
        while ok:
            if entry.szExeFile.lower() in TARGET_EXES:
                return True
            ok = kernel32.Process32NextW(snap, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snap)
    return False


class Overlay:
    """A small pill under the scrcpy window: what is buffered, or the language."""

    def __init__(self, root):
        self.root = root
        root.overrideredirect(True)
        root.attributes("-topmost", True)
        root.attributes("-alpha", 0.93)
        root.configure(bg="#10131a")
        self.label = tk.Label(root, text="", font=("Segoe UI", 12),
                              bg="#10131a", fg="#e8ecf5", padx=10, pady=4)
        self.label.pack()
        self.visible = False
        root.withdraw()

    def update(self):
        hwnd, _, mode, tag = daemon.foreground()
        text = daemon.preview()
        if daemon.status and (time.time() - daemon.status_at) > 2.5:
            daemon.status = ""

        if not (hwnd and (text or daemon.status or (tag and mode == "paste"))):
            if self.visible:
                self.root.withdraw()
                self.visible = False
            return

        shown = text[::-1] if (REVERSE_PREVIEW and text) else text
        caption = shown or tag.upper()
        if daemon.status:
            caption = ("%s   [%s]" % (caption, daemon.status) if text
                       else daemon.status)
        self.label.config(text=caption,
                          fg="#ffd479" if daemon.status else "#e8ecf5")
        rect = wt.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(rect))
        self.root.update_idletasks()
        w = max(self.root.winfo_reqwidth(), 60)
        h = self.root.winfo_reqheight()
        x = rect.left + (rect.right - rect.left - w) // 2
        y = rect.bottom + 6
        if y + h > self.root.winfo_screenheight():
            y = rect.bottom - h - 12
        self.root.geometry("+%d+%d" % (x, y))
        if not self.visible:
            self.root.deiconify()
            self.root.attributes("-topmost", True)
            self.visible = True


def run_daemon():
    kernel32.CreateMutexW(None, True, "scrcpy-hebrew-daemon")
    if kernel32.GetLastError() == 183:                   # ERROR_ALREADY_EXISTS
        return 0

    log("daemon starting, non-Latin layouts installed: %s"
        % (",".join(sorted(daemon.tags)) or "none"))
    threading.Thread(target=daemon.housekeeping, daemon=True).start()
    threading.Thread(target=hook_thread, daemon=True).start()
    threading.Thread(target=daemon.worker, daemon=True).start()

    root = tk.Tk()
    root.title("scrcpy-hebrew")
    overlay = Overlay(root)
    # the launcher starts us *before* scrcpy, and scrcpy needs a moment to push
    # its server and open a window - so never quit until we have seen one
    state = {"gone_since": 0.0, "seen": False, "started": time.time()}

    def tick():
        daemon.idle_tick()
        # follow the Windows layout proactively, so the first keystroke after
        # Alt+Shift already lands in the right language
        hwnd, serial, mode, tag = daemon.foreground()
        if hwnd and serial and mode == "uhid":
            daemon.request_sync(serial, tag, hwnd)
        try:
            overlay.update()
        except Exception as exc:
            dlog("overlay: %r" % exc)
        if scrcpy_running():
            state["seen"] = True
            state["gone_since"] = 0.0
        elif state["seen"]:
            if not state["gone_since"]:
                state["gone_since"] = time.time()
            elif time.time() - state["gone_since"] > 8:
                log("no scrcpy left, exiting")
                daemon.stop.set()
                root.destroy()
                return
        elif time.time() - state["started"] > 120:
            log("scrcpy never showed up, exiting")
            daemon.stop.set()
            root.destroy()
            return
        root.after(90, tick)

    root.after(200, tick)
    root.mainloop()
    return 0


# ---------------------------------------------------------------- launch ----

def run_launch(argv):
    serial, args, skip = None, [], False
    for i, a in enumerate(argv):
        if skip:
            skip = False
            continue
        if a == "-s" and i + 1 < len(argv):
            serial = argv[i + 1]
            skip = True
            args += ["-s", serial]
        elif a.startswith("--serial="):
            serial = a.split("=", 1)[1]
            args.append(a)
        elif a == "--legacy-paste":
            pass                  # would turn Ctrl+V back into key injection
        else:
            args.append(a)

    devices = connected_devices()
    if not serial and len(devices) == 1:
        serial = next(iter(devices))
        args += ["-s", serial]

    mode = probe_mode(serial) if serial else "paste"
    if mode == "uhid" and not any(a.startswith("--keyboard") for a in args):
        args.append("--keyboard=uhid")
    if serial and not any(a.startswith("--window-title") for a in args):
        args.append("--window-title=%s" % window_title_for(serial, devices))

    print("scrcpy-hebrew: %s -> %s mode" % (serial or "?", mode))

    here = os.path.dirname(os.path.abspath(__file__))
    pythonw = os.path.join(os.path.dirname(sys.executable), "pythonw.exe")
    subprocess.Popen([pythonw if os.path.exists(pythonw) else sys.executable,
                      os.path.join(here, "scrcpy_hebrew.py"), "daemon"],
                     creationflags=0x08000008)          # NO_WINDOW | DETACHED
    return subprocess.call([scrcpy_exe()] + args)


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "daemon"
    if cmd == "daemon":
        return run_daemon()
    if cmd == "launch":
        return run_launch(sys.argv[2:])
    if cmd == "probe":
        tags = installed_non_latin_tags()
        print("non-Latin layouts installed on this PC: %s"
              % (", ".join(sorted(tags)) or "none"))
        rest = [a for a in sys.argv[2:] if a != "-s"]
        targets = rest or list(connected_devices())
        for serial in targets:
            have = hardware_layouts(serial)
            print("%-22s %-16s %-8s hardware layouts: %s"
                  % (serial, device_name(serial), probe_mode(serial, tags),
                     ", ".join(sorted(have)) or "none"))
        return 0
    print(__doc__)
    return 1


if __name__ == "__main__":
    sys.exit(main())
