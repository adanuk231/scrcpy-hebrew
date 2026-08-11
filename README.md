# scrcpy-hebrew

Hebrew - and Arabic, Russian, Greek, Persian, any non-Latin layout - typing that
actually works in [scrcpy](https://github.com/Genymobile/scrcpy) on Windows.

If you have ever mirrored an Android phone, switched your keyboard to Hebrew and
watched the log fill with this while nothing appears on the device:

```
[server] WARN: Could not inject char u+05d1
[server] WARN: Could not inject char u+05d0
```

that is what this fixes. Latin typing is left completely alone.

## Why scrcpy cannot do it

By default scrcpy turns each keystroke into an Android `KeyEvent` by looking the
character up in the device's `KeyCharacterMap`. That map only covers ASCII, so
any character outside it has no keycode to be sent as, and the server logs
`Could not inject char`.

There is no single workaround that holds on every phone, so this tool probes the
device and picks the one that does.

### `uhid` mode - the good one

scrcpy is started with `--keyboard=uhid`, which creates a real USB HID keyboard
on the phone (visible in `getevent` as an input device named `scrcpy`). Raw
scancodes are sent and **the phone** maps them to characters, so scrcpy's
ASCII-only path is bypassed entirely. Hebrew is then just... typing. No
buffering, no clipboard, no latency, works in every app.

This needs the phone to have a hardware-keyboard layout bound to a matching IME
subtype - on most phones that already exists once the language is added to the
on-screen keyboard. `Shift+Space` switches the phone's hardware language, and
the daemon presses it for you whenever you change the Windows layout, so
`Alt+Shift` on the PC keeps working the way you expect.

### `paste` mode - the fallback

For phones that cannot map the keys themselves, a low-level Windows keyboard
hook swallows the non-Latin keystrokes before scrcpy sees them, buffers them per
word, and delivers each word through scrcpy's clipboard. A small pill under the
scrcpy window shows what is buffered.

## Two things worth knowing

Both were found the hard way, with raw `getevent` captures and `-Vdebug` logs:

* **Samsung phones reject scrcpy's clipboard write outright.** Setting the
  device clipboard throws from inside Samsung's own framework:

  ```
  java.lang.SecurityException: Given calling package android does not match caller's uid 2000
      at com.samsung.android.content.clipboard.SemClipboardManager.isUPSMode
      at com.genymobile.scrcpy.wrappers.ClipboardManager.setText
  ```

  So on Samsung, every clipboard-based trick is dead on arrival and `uhid` is
  the only route. (This is also why people end up adding `--legacy-paste`, which
  makes it worse - see below.)

* **`MOD+v` is not the Unicode-safe paste.** In scrcpy 3.x, `MOD+v` (Alt+v)
  *injects the clipboard as key events* - the exact mechanism that cannot
  represent non-ASCII. `Ctrl+V` is the one that syncs the real device clipboard
  first. This tool uses `Ctrl+V`, and strips `--legacy-paste` from your
  arguments because that flag turns `Ctrl+V` back into key injection too.

## Install

Requires Windows, Python 3.8+ (tkinter, which ships with the standard
installer), and scrcpy 2.4 or newer on `PATH` - `--keyboard=uhid` was added in
2.4. `adb` must be on `PATH` as well.

```
git clone https://github.com/adanuk231/scrcpy-hebrew
cd scrcpy-hebrew
python scrcpy_hebrew.py probe          # see what your phones can do
```

No dependencies to install - it is one file of stdlib `ctypes`.

## Use

Run scrcpy through the launcher instead of directly, with all your usual scrcpy
arguments:

```
scrcpy-he.cmd -s SERIAL --stay-awake --window-width 400
```

Then switch your Windows keyboard to Hebrew and type. That is the whole thing.

To point an existing desktop shortcut at it, change the shortcut's target from
`scrcpy.exe` to the full path of `scrcpy-he.cmd` and keep the arguments as they
are (drop `--legacy-paste` if it is there).

`probe` tells you which mode each connected phone will use:

```
> python scrcpy_hebrew.py probe
non-Latin layouts installed on this PC: he
RF8M30D1YMN            uhid     hardware layouts: en, he
ONQ8MVAEFMEYOJQK       paste    hardware layouts: none
```

If a phone reports `paste` and you would rather have the native `uhid` path, add
the language to the phone's on-screen keyboard, then start scrcpy once with
`--keyboard=uhid` and set the layout under
**Settings → General management → Physical keyboard → scrcpy**. Probe again and
it will flip to `uhid`.

### Notes

* In `uhid` mode the phone believes a hardware keyboard is attached, so its
  on-screen keyboard stays hidden while scrcpy runs. `MOD+k` opens the keyboard
  settings on the device.
* In `paste` mode the host clipboard is used as the delivery channel and
  restored afterwards; a clipboard manager will see each word go by.
* The daemon is a singleton, serves every scrcpy window at once, and exits by
  itself a few seconds after the last one closes.
* Logs: `%LOCALAPPDATA%\scrcpy-hebrew\daemon.log`. Set
  `SCRCPY_HEBREW_DEBUG=1` for per-keystroke detail.

## Verified on

| Device | Android | Mode | Result |
|---|---|---|---|
| Samsung SM-G973F (S10) | 11 | `uhid` | `abc` on a Hebrew layout produced `שנב` in Chrome |
| Xiaomi M2006C3MG (Redmi 9C) | 10 | `paste` | Hebrew words delivered via `Ctrl+V`, `Device clipboard set` |

scrcpy 3.3.1, Windows 10, Python 3.12.

## Tests

`tests/e2e_typing.py` is the harness used above: it opens Chrome on the phone,
launches the tool, switches the host layout, types on the real Windows keyboard
through `SendInput`, and reads the field back with `uiautomator`.

```
set SCRCPY_HEBREW_ALLOW_INJECTED=1
python tests/e2e_typing.py <serial> "abc "
```

(The daemon ignores synthetic keystrokes by default - that is how it avoids
re-processing its own injections - so the tests set that variable to drive it.)

## License

MIT
