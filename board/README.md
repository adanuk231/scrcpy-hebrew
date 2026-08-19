# board

A small control strip for the phones. It does not show them.

scrcpy already gives you the best window there is for a phone: the screen,
resize handles, and nothing else, sitting on the desktop like any other window.
The board leaves that completely alone. What it adds is everything scrcpy has
no room for once you have more than one phone out.

```
board/src-tauri/target/release/scrcpy-board.exe
```

## One set of controls

The board is a tray panel, not a window you leave lying about: it opens beside
the tray icon, sizes itself to what is in it, and goes back with Escape or the
X. Where "beside the tray" is gets asked rather than assumed - the taskbar can
sit on any of the four edges, on any monitor, and on a right-to-left Windows
the notification area is at the *left* end of it.

There is one controller, not one per phone. The strip of phones along the
bottom decides which phone it points at; the number keys pick one too. Behind
the sliders button on the right - which stays put while the phones scroll - is
everything that is not about a particular phone.

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

Always on, nothing to switch.

* **Nothing overlaps.** A phone dropped on top of another is pushed out the
  shortest way - so you can aim one at another and let go, and it lands against
  the nearest side.
* **A group travels together.** Drag a phone and everything stuck to it comes
  along. Stuck means touching, worked out fresh at the moment you pick it up.
* **Throw one to get it out.** Move a phone hard enough and it lets go of its
  group mid-drag, which is how you pull one back out of a row without dragging
  the row with it.
* Once an edge joins, the other axis gets a much longer reach to line up, so
  shoulder-to-shoulder phones end up flush rather than stepped.

`line them up` puts every running phone in a row, in tab order; dragging a tab
sideways reorders and re-lays them out.

Two things make this work that are worth writing down. Windows tells us when a
drag starts and ends (`EVENT_SYSTEM_MOVESIZESTART`), so nothing has to be
inferred from the mouse button and the foreground window. And every measurement
is in *visible* coordinates: `GetWindowRect` returns the extended bounds, which
on Windows 10 include about seven invisible pixels of resize border a side, so
snapping those rectangles flush leaves a fourteen pixel gap on screen and
"touching" stops being true the moment you move a window by hand. The visible
bounds come from `DwmGetWindowAttribute`.

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

## Tray, and starting with Windows

The window is not the app. Closing it only puts it away - the phones keep
running and the tray icon stays - and the tray menu has *Show the board*,
*Start at login* and *Quit (phones stay up)*. Left-clicking the icon
toggles the window. The `-` chip in the header does the same thing as closing.

*Start at login* writes one string to
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`:

```
"<path to>\scrcpy-board.exe" --hidden
```

No installer, no scheduled task, no elevation, and turning it off deletes the
value. `--hidden` means signing in gives you the tray icon rather than a window
in the way. It can also be set without the UI:

```
scrcpy-board.exe autostart on
scrcpy-board.exe autostart off
```

Whichever copy of the exe you run that from is the copy that gets registered,
and the toggle reads back as off if the registered path is not this exe - so
moving the exe and forgetting is visible rather than silent.

**One copy only.** Started at login and then double-clicked is the ordinary way
to end up with two tray icons and two magnet loops fighting over the same
windows. A named mutex makes the second launch bring the first board forward
and exit instead.

(If you ever force-kill the board, Windows leaves a dead tray icon behind until
you sweep the mouse over it. Quitting from the tray menu removes it properly.)

## Checking it without clicking anything

The buttons are driven by the same functions as this, so it exercises the real
path:

```
scrcpy-board.exe selftest <serial>   # one phone: start, park, close
scrcpy-board.exe selftest all        # every phone, and assert they parked flush
scrcpy-board.exe selftest keep       # every phone, left running
scrcpy-board.exe selftest readopt    # pick up phones left running, then close
scrcpy-board.exe selftest autostart  # the Run key round-trips and restores
scrcpy-board.exe selftest drags      # what the shell reports while you drag a phone
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
