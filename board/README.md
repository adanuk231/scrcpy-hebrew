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

Drag the panel somewhere else and it stays there. Its contents change height
all the time - a longer note, a different phone - and none of that is a reason
to yank it back to the corner. It goes back to the tray when it is put away and
opened again, however it is opened, and on a cold start. If it ends up hanging
off the screen it is nudged back on, no further.

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

## The phones have no title bar

A phone window is the picture and nothing else. What a title bar was for now
floats over the phone you are pointing at: a handle to drag it by, rounded
corners on and off, show this phone on the board, and stop mirroring. Icons
only, on a window with no background at all - the strip is transparent, so the
only thing on screen is the marks, which carry their own shadow to stay
readable over any picture.

**Point at a phone and the icons appear.** Not click: a phone you have not
clicked on has no title bar either, and clicking it to bring up the icons would
tap whatever is on the screen underneath. Whatever is under the pointer wins,
then the phone being dragged, then the one with the keyboard - and moving the
pointer onto the icons themselves keeps them where they are. They park
off-screen a moment after you leave, so crossing a gap on the way to them does
not make them flicker. The strip never takes the focus, so a phone can be moved
without your editor losing it.

One strip, not one per phone. You can only point at one at a time, and a second
webview hanging over every phone is a lot of machinery for a row of four icons.
It sits just above the phone, or just inside the top if the phone is against
the top of the screen.

The icons are five: drag, rounded corners, drag to resize, show this phone on
the board, stop.

**The board moves and resizes the phone itself.** The tidy way is to ask the window to
start the move loop it would start from its own title bar - `WM_NCLBUTTONDOWN`
with `HTCAPTION` - but that does not survive being asked from another process:
the loop wants the mouse capture, and whether it gets it depends on who is in
the foreground and on what the webview did with the button that is already
held down. It worked once and then never again. So a thread follows the cursor
while the button is down, moves the window, and posts the same two events the
shell would have posted - which leaves the group travelling, ctrl peeling one
off and the snap on landing all working off the one path they already used.

Resizing goes the same way, and the reason is worth the paragraph, because
every tidy-looking route to it is a trap.

**The phone is not started borderless.** `--window-borderless` gives a window
SDL considers not resizable, and SDL then answers `WM_NCCALCSIZE` by pinning
the client area to the size the window was born at. Resize that window from
outside and it works, on paper: the frame grows, `GetWindowRect` reports the
new size, and the picture inside stays exactly as it was, so what you get is a
crop. Putting `WS_THICKFRAME` back to earn a resize border makes it worse -
min and max track sizes are only enforced on a window that has a sizing
border, and SDL answers that message with the same number twice, so the window
snaps back to its birth size. Asked for 300x660, it came back 273x600.

Started as an ordinary window it is resizable as far as SDL is concerned, and
stays that way even after the board takes the title bar, the sizing border and
the system menu off it: SDL's idea of the window is its own. What is left is
one rectangle that Windows, DWM and the client area all agree on - measured at
243x539, then at 380x830 after a resize, all three the same both times, with
the picture scaling to match.

The `aspect` selftest checks that agreement at every step, because this fails
silently: a window that resizes around a picture that does not.

With no sizing border there is nothing for the mouse to grab, which is why the
resize icon exists and drags a corner by hand. It moves the window sixty times
a second rather than at the speed of the drag: every size change asks the phone
for a fresh frame at a new shape, and until it arrives the last one is stretched
to fit.

How sharp that stays as the window grows is not about the window at all - it is
how many pixels the phone is sending. `size` is `native` by default, which is
the phone's own resolution and as good as it gets; set it to 720 and a window
any bigger than that is stretching what it has. The tests that only measure a
window use 720 because it starts faster, but the branch that leaves phones
running for real use starts them the way the app does.

## Magnet

Always on, nothing to switch.

* **Nothing overlaps.** A phone dropped on top of another is pushed out the
  shortest way - so you can aim one at another and let go, and it lands against
  the nearest side.
* **A group travels together.** Drag a phone and everything stuck to it comes
  along. Stuck means touching, worked out fresh at the moment you pick it up.
* **Ctrl to take one out.** Hold ctrl while dragging and that phone leaves its
  group, so you can pull one back out of a row without the row coming with it.
  Pressing ctrl part way through a drag works too - it lets go on the spot.
* Once an edge joins, the other axis gets a much longer reach to line up, so
  shoulder-to-shoulder phones end up flush rather than stepped.

### Resizing

* **A phone resizes its group with it.** Pull one taller and the row comes with
  it, each phone keeping its own shape, and the row closes up again.
* **Ctrl resizes only the one you are pulling**, and it stays in the group - so
  one phone can be a different size from the rest without leaving the slab.
* **No black bars.** The picture has a fixed shape, so the window is put back to
  that shape when you let go. The edge you dragged is the one you meant, so it
  decides and the other follows: pull the bottom down and it gets taller *and*
  wider. Only a corner, where you moved both, is fitted inside what you drew.

The shape comes from scrcpy's own log rather than from the window - a window
that has been pulled out of shape has black bars in it, and measuring that
would just preserve them. scrcpy prints `Texture: 1080x2280`, and prints it
again when the phone rotates.

### Where a phone opens

Where you left it. Position and size are written down per serial as soon as a
phone stops moving, from the same loop that runs the magnet - so it covers
dragging, resizing, lining up, and the board being killed outright. Next time
it opens there.

The remembered box is checked before it is used: pulled back onto a monitor
that still exists, then dropped the way a dragged phone is dropped, so it never
lands on top of a phone that is already out. A phone the board has never seen
still parks to the right of whatever is there.

The size is the one you dragged it to, unless the height in settings has been
changed since - that is the more recent instruction, so it wins. The shape is
always the picture's, so a phone that has been turned on its side is not
squeezed back upright.

`line them up` puts every running phone in a row, in tab order, **where they
already are**: the row forms at the top left corner of the ground they already
cover, so tidying a row does not also carry it off to the corner of the screen.
Dragging a tab sideways reorders and re-lays them out.

Two things make this work that are worth writing down. A drag announces itself
- either the shell says so (`EVENT_SYSTEM_MOVESIZESTART`, for a resize, which
is still a real move loop) or the board says so because it is the one moving
the window - so nothing has to be inferred from the mouse button and the
foreground window. And every measurement
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
scrcpy-board.exe selftest hover      # which phone is under a point, and hand-made drags
scrcpy-board.exe selftest remember   # a phone opens where it was last left
scrcpy-board.exe selftest lineup     # lining a row up leaves it where it stands
scrcpy-board.exe selftest aspect     # pull a phone out of shape, then put it back
scrcpy-board.exe selftest resizegroup # resize one of a pair, check the row follows
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
