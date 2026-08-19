# board

A small control strip for the phones. It does not show them.

scrcpy already gives you the best window there is for a phone: the screen,
resize handles, and nothing else, sitting on the desktop like any other window.
The board leaves that completely alone. What it adds is everything scrcpy has
no room for once you have more than one phone out.

```
board/src-tauri/target/release/scrcpy-board.exe
```

## What is on a card

One card per phone that adb can see, in whatever order you drag them into.

**Badges** say what the phone actually is: Android version, `uhid` or `paste`
(the same probe [scrcpy-hebrew](../README.md) uses to decide how Hebrew will be
typed), battery.

**Toggles that need a reconnect.** scrcpy has no runtime control channel for its
options, so changing one of these kills the process and starts it again with new
arguments, about a second. The card says so while it happens.

| control | what it passes |
|---|---|
| audio: off | `--no-audio` |
| audio: to pc | `--audio-source=output`, the `REMOTE_SUBMIX` route |
| audio: shared | `--audio-source=playback --audio-dup` |
| size | `--max-size` |
| fps | `--max-fps` |
| awake | `--stay-awake` |
| dark | `--turn-screen-off` |
| touches | `--show-touches` |
| look | `--no-control`, mirror without touching |
| skin | `--window-borderless` plus rounded corners |

The audio row is drawn from what the phone can really do, not from what scrcpy
accepts:

* below Android 11 there is no audio forwarding at all, so both routes are dead
  and the card says why;
* on 11 and 12 only the submix route exists, and it **takes the sound off the
  handset** - that is inherent to `REMOTE_SUBMIX`, not a scrcpy decision;
* from 13 the playback route plus `--audio-dup` keeps the phone audible, and
  apps can opt out of being captured.

**Buttons that do not need anything.** Back, home, recents, volume,
notifications, power. These go over adb rather than through scrcpy's shortcuts,
so they are instant, they do not care which window has focus, and they still
work on a phone you are mirroring with `look` on.

## Magnet

With `magnet` lit, phone windows click onto each other and onto the edges of the
screen when you let go of one, and everything already stuck to a window travels
with it while you drag. Two phones side by side behave like one slab.

Once an edge joins, the other axis is allowed a much longer reach to line up -
so shoulder-to-shoulder phones end up flush rather than stepped.

`line them up` puts every running phone in a row, in card order. Dragging a card
to a new position does the same thing, which is the whole point: the cards are
the phones.

## What it does to the rest of the setup

The board starts the Hebrew daemon itself, and writes
`%LOCALAPPDATA%\scrcpy-hebrew\windows.json`, a plain window-handle to serial
map. The daemon reads it instead of guessing the phone from a window title,
which matters the moment a window is titled anything other than its serial.

Each phone's scrcpy output goes to `%LOCALAPPDATA%\scrcpy-hebrew\board-<serial>.log`.
No console windows anywhere.

## Building

Needs Rust (stable, MSVC) and nothing else - no npm, no node, the frontend is
three static files.

```
cd board/src-tauri
cargo build --release
```

The result is one exe. `cargo tauri build` will wrap it in an installer if you
ever want one.

## Checking it without clicking anything

The buttons are driven by the same functions as this, so it exercises the real
path:

```
scrcpy-board.exe selftest <serial>   # one phone: start, park, close
scrcpy-board.exe selftest all        # every phone, and assert they parked flush
scrcpy-board.exe selftest keep       # every phone, left running
scrcpy-board.exe selftest readopt    # pick up phones left running, then close
```

`keep` followed by `readopt` from a second process is the proof that phones
outlive the board and are found again afterwards.

## Notes from building it

* **scrcpy is often a shim.** Chocolatey's `scrcpy.exe` is a 29 KB launcher that
  spawns the real binary, so the pid you get back does not own the window. The
  board finds windows by a unique title it passes in, then renames the window to
  the phone's name.
* **A window handle is not an identity.** Windows recycles them, so a handle
  written down in one session can belong to a stranger's window in the next.
  Anything re-adopted is checked against the owning process being `scrcpy.exe`
  before the board is allowed to touch it, let alone close it.
* **Reparenting works, and was still the wrong idea.** An earlier version adopted
  the scrcpy windows as children of one host window: it renders, it is stable,
  and `AttachThreadInput` plus `SetFocus` is what makes the keyboard reach a
  child that belongs to another process (without it the child never sees a
  keystroke). `win::adopt` is still in the tree for a future dock mode. But a
  phone in a frame is worse than a phone in a window.
