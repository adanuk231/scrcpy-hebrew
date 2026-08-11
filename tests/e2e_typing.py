# -*- coding: utf-8 -*-
"""
End-to-end test: launch the tool, switch the host layout, type on the real
Windows keyboard, and read back what the phone actually received.

    set SCRCPY_HEBREW_ALLOW_INJECTED=1
    python tests/e2e_typing.py <serial> ["abc "] [langid, default 040D]

It drives Chrome's omnibox on the device and reads the field with uiautomator,
so the phone must be unlocked and have Chrome installed.
"""
import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)
import scrcpy_hebrew as S                                  # noqa: E402

user32, kernel32 = S.user32, S.kernel32
WM_INPUTLANGCHANGEREQUEST = 0x0050


def adb(serial, *args):
    return S.adb(serial, *args)


# ------------------------------------------------------------ host input ----

def find_scrcpy_window(timeout=30):
    found = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(hwnd, _):
        if user32.IsWindowVisible(hwnd) and S.exe_of_window(hwnd) == "scrcpy.exe":
            if user32.GetWindowTextLengthW(hwnd):
                found.append(hwnd)
                return False
        return True

    deadline = time.time() + timeout
    while time.time() < deadline:
        found.clear()
        user32.EnumWindows(cb, 0)
        if found:
            return found[0]
        time.sleep(0.5)
    return None


def focus(hwnd):
    user32.ShowWindow(hwnd, 9)                             # SW_RESTORE
    cur = kernel32.GetCurrentThreadId()
    for _ in range(12):
        fg = user32.GetForegroundWindow()
        fg_tid = user32.GetWindowThreadProcessId(fg, None) if fg else 0
        if fg_tid:
            user32.AttachThreadInput(cur, fg_tid, True)
        user32.BringWindowToTop(hwnd)
        user32.SetForegroundWindow(hwnd)
        user32.SetActiveWindow(hwnd)
        if fg_tid:
            user32.AttachThreadInput(cur, fg_tid, False)
        time.sleep(0.2)
        if user32.GetForegroundWindow() == hwnd:
            return True
    return False


def set_layout(hwnd, langid):
    hkl = user32.LoadKeyboardLayoutW("%08X" % langid, 0x00000080)
    user32.PostMessageW(hwnd, WM_INPUTLANGCHANGEREQUEST, 0, hkl)
    time.sleep(0.4)
    tid = user32.GetWindowThreadProcessId(hwnd, None)
    return user32.GetKeyboardLayout(tid) & 0xFFFF


def tap_key(vk):
    scan = user32.MapVirtualKeyW(vk, 0)      # uhid mode needs a real scancode
    for flags in (0, S.KEYEVENTF_KEYUP):
        arr = (S.INPUT * 1)()
        arr[0].type = 1
        arr[0].ki = S.KEYBDINPUT(wVk=vk, wScan=scan, dwFlags=flags, time=0,
                                 dwExtraInfo=0)
        user32.SendInput(1, arr, ctypes.sizeof(S.INPUT))
        time.sleep(0.02)
    time.sleep(0.08)


# ---------------------------------------------------------- device probes ---

URL_BAR = 'resource-id="com.android.chrome:id/url_bar"'


def dump(serial):
    adb(serial, "shell", "uiautomator dump /sdcard/e2e.xml")
    return adb(serial, "shell", "cat /sdcard/e2e.xml")


def node_attr(xml, marker, attr):
    idx = xml.find(marker)
    if idx < 0:
        return None
    node = xml[xml.rfind("<node", 0, idx):xml.find(">", idx)]
    key = '%s="' % attr
    at = node.find(key)
    return node[at + len(key):node.find('"', at + len(key))] if at >= 0 else None


def open_omnibox(serial):
    adb(serial, "shell", "am start -n com.android.chrome/com.google.android."
                         "apps.chrome.Main -a android.intent.action.VIEW "
                         "-d about:blank")
    time.sleep(5)
    bounds = node_attr(dump(serial), URL_BAR, "bounds")
    if not bounds:
        print("FAIL: Chrome's omnibox not found on the device")
        return False
    n = [int(v) for v in bounds.replace("[", " ").replace("]", " ")
         .replace(",", " ").split()]
    adb(serial, "shell", "input tap %d %d" % ((n[0] + n[2]) // 2,
                                              (n[1] + n[3]) // 2))
    time.sleep(2)
    return True


def read_url_bar(serial):
    for attempt in range(4):
        text = node_attr(dump(serial), URL_BAR, "text")
        if text is not None:
            return text
        time.sleep(1.2)
    return None


# ----------------------------------------------------------------- test -----

def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    serial = sys.argv[1]
    keys = sys.argv[2] if len(sys.argv) > 2 else "abc "
    langid = int(sys.argv[3], 16) if len(sys.argv) > 3 else 0x040D

    if os.environ.get("SCRCPY_HEBREW_ALLOW_INJECTED") != "1":
        print("set SCRCPY_HEBREW_ALLOW_INJECTED=1 first, or the daemon will "
              "ignore the synthetic keystrokes this test sends")
        return 2

    print("== mode for %s: %s" % (serial, S.probe_mode(serial)))
    if not open_omnibox(serial):
        return 1

    launcher = subprocess.Popen(
        [sys.executable, os.path.join(ROOT, "scrcpy_hebrew.py"), "launch",
         "-s", serial, "--window-x", "100", "--window-y", "100",
         "--window-width", "320", "--window-height", "640", "--stay-awake"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        encoding="utf-8", errors="replace")

    hwnd = find_scrcpy_window()
    if not hwnd:
        print("FAIL: no scrcpy window")
        launcher.kill()
        return 1
    time.sleep(6)                                # let the daemon probe the phone

    if not focus(hwnd):
        print("ABORT: could not focus scrcpy, refusing to inject keys blindly")
        launcher.kill()
        return 1
    got = set_layout(hwnd, langid)
    if user32.GetForegroundWindow() != hwnd:
        focus(hwnd)
        got = user32.GetKeyboardLayout(
            user32.GetWindowThreadProcessId(hwnd, None)) & 0xFFFF
    print("== host layout 0x%04X (wanted 0x%04X)" % (got, langid))
    if got != langid or user32.GetForegroundWindow() != hwnd:
        print("ABORT: layout or focus is not what we asked for")
        launcher.kill()
        return 1

    time.sleep(2.5)                              # let a uhid language sync land
    before = read_url_bar(serial)
    focus(hwnd)
    print("== field before: %r" % before)
    print("== typing %r" % keys)
    for ch in keys:
        if user32.GetForegroundWindow() != hwnd:
            print("   ! scrcpy lost focus before %r" % ch)
            focus(hwnd)
        tap_key(0x20 if ch == " " else ord(ch.upper()))
        time.sleep(0.15)
    time.sleep(3.5)
    print("== field after : %r" % read_url_bar(serial))

    subprocess.call(["taskkill", "/F", "/IM", "scrcpy.exe"],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        launcher.wait(timeout=10)
    except Exception:
        launcher.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
