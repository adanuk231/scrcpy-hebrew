#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! scrcpy board - a small control strip for phones that stay ordinary windows.
//!
//! The board never takes the picture away from scrcpy and never puts it in a
//! frame of its own: every phone is a normal scrcpy window you can move,
//! resize and alt-tab like anything else. What the board adds is the things
//! scrcpy has no room for - what each phone was started with, what it can
//! actually do, buttons that do not need a restart, and magnetism between the
//! windows so two phones can be handled as one slab.

mod magnet;
mod startup;
mod win;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::{Emitter, Manager, State};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED: u32 = 0x0000_0008;


// ---------------------------------------------------------------- state ----

struct Pedal {
    child: Option<Child>,
    hwnd: isize,
    skin: bool,
}

#[derive(Default)]
struct Board {
    pedals: Mutex<HashMap<String, Pedal>>,
    /// where the board last put its own window, so it can tell whether the
    /// window is still there or has been picked up and moved
    parked: Mutex<Option<(i32, i32)>>,
}

// ------------------------------------------------------------- plumbing ----

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {}", program, e))?;
    if !out.status.success() && out.stdout.is_empty() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// scrcpy_hebrew.py lives above the app; in a dev build the exe is buried
/// under target/, so walk up until it turns up.
fn engine() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SCRCPY_HEBREW") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    for root in roots {
        let mut here: &Path = &root;
        for _ in 0..7 {
            let candidate = here.join("scrcpy_hebrew.py");
            if candidate.is_file() {
                return Some(candidate);
            }
            match here.parent() {
                Some(up) => here = up,
                None => break,
            }
        }
    }
    None
}

fn python(windowless: bool) -> (String, Vec<String>) {
    // the py launcher is the reliable way in; pyw is its windowless twin
    let launcher = if windowless { "pyw" } else { "py" };
    let ok = Command::new(launcher)
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    if ok {
        return (launcher.to_string(), vec!["-3".to_string()]);
    }
    let fallback = if windowless { "pythonw" } else { "python" };
    (fallback.to_string(), vec![])
}

fn scrcpy_exe() -> String {
    if let Ok(p) = std::env::var("SCRCPY_EXE") {
        return p;
    }
    // the chocolatey shim works too, but the real binary keeps the pid and the
    // window in one process, which is one less thing to guess about
    let choco = "C:\\ProgramData\\chocolatey\\lib\\scrcpy\\tools\\scrcpy.exe";
    if Path::new(choco).is_file() {
        return choco.to_string();
    }
    "scrcpy".to_string()
}

fn local_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("scrcpy-hebrew")
}

/// The shape of the picture, straight out of scrcpy's log.
///
/// The window on screen is not a reliable answer: resize it by hand and it
/// keeps whatever shape you left it, black bars and all. scrcpy prints the
/// texture it is drawing - and prints it again when the phone rotates - so the
/// last one in the log is the truth.
pub fn video_aspect(serial: &str) -> Option<f64> {
    let text = std::fs::read_to_string(local_dir().join(format!("board-{}.log", serial))).ok()?;
    let mut found = None;
    for line in text.lines() {
        if let Some(rest) = line.split("Texture: ").nth(1) {
            let mut parts = rest.trim().split('x');
            if let (Some(w), Some(h)) = (parts.next(), parts.next()) {
                if let (Ok(w), Ok(h)) = (w.trim().parse::<f64>(), h.trim().parse::<f64>()) {
                    if w > 0.0 && h > 0.0 {
                        found = Some(w / h);
                    }
                }
            }
        }
    }
    found
}

/// Tell the Hebrew daemon which phone each window belongs to, so it never has
/// to guess from a window title.
fn publish_windows(pedals: &HashMap<String, Pedal>) {
    let map: HashMap<String, String> = pedals
        .iter()
        .filter(|(_, p)| p.hwnd != 0)
        .map(|(serial, p)| (p.hwnd.to_string(), serial.clone()))
        .collect();
    let dir = local_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string_pretty(&map) {
        if let Ok(mut fh) = std::fs::File::create(dir.join("windows.json")) {
            let _ = fh.write_all(text.as_bytes());
        }
    }
}

// ------------------------------------------------------------- commands ----

#[tauri::command]
fn probe() -> Result<serde_json::Value, String> {
    let script = engine().ok_or("scrcpy_hebrew.py not found next to the app")?;
    let (exe, pre) = python(false);
    let mut args: Vec<String> = pre;
    args.push(script.to_string_lossy().to_string());
    args.push("capabilities".to_string());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = run(&exe, &refs)?;
    serde_json::from_str(&out).map_err(|e| format!("{}: {}", e, out))
}

#[derive(Deserialize, Clone, Debug)]
struct Opts {
    audio: String,
    keyboard: String,
    max_size: u32,
    max_fps: u32,
    stay_awake: bool,
    screen_off: bool,
    show_touches: bool,
    view_only: bool,
    skin: bool,
    height: u32,
    name: String,
}

fn scrcpy_args(serial: &str, title: &str, o: &Opts) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-s".into(),
        serial.into(),
        format!("--window-title={}", title),
        format!("--window-height={}", o.height.max(240)),
        // born off-screen, so nothing flashes in the middle of the desktop
        "--window-x=-6000".into(),
        "--window-y=-6000".into(),
    ];
    if o.skin {
        a.push("--window-borderless".into());
    }
    if o.keyboard == "uhid" {
        a.push("--keyboard=uhid".into());
    }
    match o.audio.as_str() {
        "output" => a.push("--audio-source=output".into()),
        "dup" => {
            a.push("--audio-source=playback".into());
            a.push("--audio-dup".into());
        }
        _ => a.push("--no-audio".into()),
    }
    if o.max_size > 0 {
        a.push(format!("--max-size={}", o.max_size));
    }
    if o.max_fps > 0 {
        a.push(format!("--max-fps={}", o.max_fps));
    }
    if o.stay_awake {
        a.push("--stay-awake".into());
    }
    if o.screen_off {
        a.push("--turn-screen-off".into());
    }
    if o.show_touches {
        a.push("--show-touches".into());
    }
    if o.view_only {
        a.push("--no-control".into());
    }
    a
}

#[derive(Serialize)]
struct Started {
    serial: String,
    hwnd: String,
    args: Vec<String>,
}

/// Where a new phone should land: to the right of the phones already out, so
/// starting two in a row gives you a magnetised pair without touching them.
/// Visible coordinates, so "to the right of" means what it looks like.
fn parking_spot(rects: &[win::Rect], w: i32, h: i32) -> (i32, i32) {
    let area = win::work_area();
    let mut x = area.left + 24;
    let y = area.top + 24;
    for r in rects {
        if r.top < y + h && r.bottom > y {
            x = x.max(r.right);
        }
    }
    if x + w > area.right {
        x = area.left + 24;
    }
    (x, y)
}

#[tauri::command]
fn start(state: State<Board>, serial: String, opts: Opts) -> Result<Started, String> {
    start_inner(&state, serial, opts)
}

fn start_inner(board: &Board, serial: String, opts: Opts) -> Result<Started, String> {
    stop_inner(board, &serial);

    start_daemon();

    let title = format!("scrcpyboard-{}", serial);
    let args = scrcpy_args(&serial, &title, &opts);
    let dir = local_dir();
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join(format!("board-{}.log", serial));
    let sink = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let sink2 = sink.try_clone().map_err(|e| e.to_string())?;

    let mut child = Command::new(scrcpy_exe())
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::from(sink))
        .stderr(Stdio::from(sink2))
        .spawn()
        .map_err(|e| format!("could not start scrcpy: {}", e))?;

    let deadline = Instant::now() + Duration::from_secs(25);
    let mut hwnd = 0isize;
    while Instant::now() < deadline {
        if let Some(found) = win::find_by_title(&title) {
            hwnd = found;
            break;
        }
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    if hwnd == 0 {
        let _ = child.kill();
        let tail = std::fs::read_to_string(&log).unwrap_or_default();
        let tail = tail
            .lines()
            .filter(|l| !l.trim().is_empty())
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" / ");
        return Err(format!("scrcpy never opened a window. {}", tail));
    }

    // the unique title was only ever a handle; show the phone's name
    win::rename(hwnd, if opts.name.is_empty() { &serial } else { &opts.name });

    let size = win::visible_rect(hwnd).unwrap_or_default();
    let taken: Vec<win::Rect> = {
        let pedals = board.pedals.lock().unwrap();
        pedals.values().filter_map(|p| win::visible_rect(p.hwnd)).collect()
    };
    let (x, y) = parking_spot(&taken, size.w(), size.h());
    win::move_visible_to(hwnd, x, y);
    if opts.skin {
        win::round_corners(hwnd, 34);
    }
    win::show(hwnd, true);
    win::activate(hwnd);
    let mut pedals = board.pedals.lock().unwrap();
    pedals.insert(
        serial.clone(),
        Pedal { child: Some(child), hwnd, skin: opts.skin },
    );
    publish_windows(&pedals);

    Ok(Started { serial, hwnd: hwnd.to_string(), args })
}

fn stop_inner(board: &Board, serial: &str) {
    let mut pedals = board.pedals.lock().unwrap();
    if let Some(mut pedal) = pedals.remove(serial) {
        // ask first, so scrcpy turns the phone's screen back on and tidies up
        if win::is_window(pedal.hwnd) {
            win::ask_to_close(pedal.hwnd);
            let deadline = Instant::now() + Duration::from_millis(1800);
            while Instant::now() < deadline && win::is_window(pedal.hwnd) {
                std::thread::sleep(Duration::from_millis(60));
            }
        }
        if let Some(child) = pedal.child.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
    publish_windows(&pedals);
}

#[tauri::command]
fn stop(state: State<Board>, serial: String) -> Result<(), String> {
    stop_inner(&state, &serial);
    Ok(())
}

#[tauri::command]
fn running(state: State<Board>) -> Vec<String> {
    let pedals = state.pedals.lock().unwrap();
    pedals
        .iter()
        .filter(|(_, p)| win::is_window(p.hwnd))
        .map(|(serial, _)| serial.clone())
        .collect()
}

/// The one thing scrcpy cannot do on its own: put a rounded, frameless skin on
/// a window that is already running.
#[tauri::command]
fn set_skin(state: State<Board>, serial: String, on: bool) -> Result<(), String> {
    let mut pedals = state.pedals.lock().unwrap();
    let pedal = pedals.get_mut(&serial).ok_or("that phone is not running")?;
    win::chromeless(pedal.hwnd, on);
    if on {
        win::round_corners(pedal.hwnd, 34);
    } else {
        win::square_corners(pedal.hwnd);
    }
    pedal.skin = on;
    Ok(())
}

/// Line the phones up in the given order and stick them together.
#[tauri::command]
fn arrange(state: State<Board>, order: Vec<String>) {
    let pedals = state.pedals.lock().unwrap();
    magnet::arrange(&pedals, &order);
}

#[tauri::command]
fn focus_device(state: State<Board>, serial: String) {
    let pedals = state.pedals.lock().unwrap();
    if let Some(pedal) = pedals.get(&serial) {
        win::activate(pedal.hwnd);
    }
}

/// Live buttons. These deliberately go through adb rather than through
/// scrcpy's own shortcuts: no restart, no key-injection guesswork, and they
/// still work on a phone running with --no-control.
#[tauri::command]
fn action(serial: String, what: String) -> Result<String, String> {
    let key = match what.as_str() {
        "back" => "4",
        "home" => "3",
        "recents" => "187",
        "power" => "26",
        "volume_up" => "24",
        "volume_down" => "25",
        "wake" => "224",
        "sleep" => "223",
        "notifications" => "83",
        _ => return Err(format!("unknown action {}", what)),
    };
    run("adb", &["-s", &serial, "shell", "input", "keyevent", key])
}

#[tauri::command]
fn rotate(serial: String, locked: bool, landscape: bool) -> Result<String, String> {
    let accel = if locked { "0" } else { "1" };
    run(
        "adb",
        &["-s", &serial, "shell", "settings", "put", "system",
          "accelerometer_rotation", accel],
    )?;
    if locked {
        let value = if landscape { "1" } else { "0" };
        return run(
            "adb",
            &["-s", &serial, "shell", "settings", "put", "system",
              "user_rotation", value],
        );
    }
    Ok(String::new())
}

#[derive(Serialize)]
struct Status {
    battery: i32,
    charging: bool,
}

#[tauri::command]
fn status(serial: String) -> Result<Status, String> {
    let dump = run("adb", &["-s", &serial, "shell", "dumpsys", "battery"])?;
    let mut battery = -1;
    let mut charging = false;
    for line in dump.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("level:") {
            battery = rest.trim().parse().unwrap_or(-1);
        } else if let Some(rest) = line.strip_prefix("AC powered:") {
            charging |= rest.trim() == "true";
        } else if let Some(rest) = line.strip_prefix("USB powered:") {
            charging |= rest.trim() == "true";
        }
    }
    Ok(Status { battery, charging })
}

#[tauri::command]
fn open_logs() {
    let _ = Command::new("explorer")
        .arg(local_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

// ----------------------------------------------------------------- main ----

/// Windows left over from an earlier run of the board, or from a board that
/// was closed while the phones stayed out.
fn readopt(board: &Board) {
    let path = local_dir().join("windows.json");
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };
    let map: HashMap<String, String> = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(_) => return,
    };
    let mut pedals = board.pedals.lock().unwrap();
    for (hwnd, serial) in map {
        let hwnd: isize = match hwnd.parse() {
            Ok(h) => h,
            Err(_) => continue,
        };
        // a handle alone proves nothing: it may have been recycled since we
        // wrote it down, and closing a stranger's window would be unforgivable
        if !win::is_window(hwnd) || win::owner_exe(hwnd) != "scrcpy.exe" {
            continue;
        }
        pedals.insert(serial, Pedal { child: None, hwnd, skin: false });
    }
    publish_windows(&pedals);
}

fn start_daemon() {
    if let Some(script) = engine() {
        let (exe, pre) = python(true);
        let _ = Command::new(exe)
            .args(pre)
            .arg(script)
            .arg("daemon")
            .creation_flags(CREATE_NO_WINDOW | DETACHED)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// `scrcpy-board.exe selftest <what>` drives start, park, magnet and stop
/// through the same functions the buttons call - no window, no clicking.
///
///   selftest <serial>   one phone, start to stop
///   selftest all        every phone, and check they parked flush
///   selftest keep       every phone, left running
///   selftest readopt    pick up phones left running, then close them
fn test_opts(name: &str, keyboard: &str) -> Opts {
    Opts {
        audio: "off".into(),
        keyboard: keyboard.into(),
        max_size: 720,
        max_fps: 30,
        stay_awake: true,
        screen_off: false,
        show_touches: false,
        view_only: false,
        skin: false,
        height: 700,
        name: name.into(),
    }
}

fn devices() -> Vec<(String, String, String)> {
    let report = match probe() {
        Ok(r) => r,
        Err(e) => {
            println!("probe failed: {}", e);
            return Vec::new();
        }
    };
    report["devices"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|d| {
                    (
                        d["serial"].as_str().unwrap_or("").to_string(),
                        d["name"].as_str().unwrap_or("").to_string(),
                        d["keyboard"].as_str().unwrap_or("paste").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn selftest(what: String) -> i32 {
    let board = Board::default();

    if what.starts_with("aspect") {
        // pull a phone out of shape the way a free resize would, then let the
        // same code the mouse uses put it back
        let board = Board::default();
        let list = devices();
        let (serial, name, keyboard) = match list.first() {
            Some(d) => d.clone(),
            None => {
                println!("no phones");
                return 1;
            }
        };
        if let Err(e) = start_inner(&board, serial.clone(), test_opts(&name, &keyboard)) {
            println!("start FAILED: {}", e);
            return 1;
        }
        let hwnd = board.pedals.lock().unwrap()[&serial].hwnd;
        let aspect = video_aspect(&serial);
        println!("scrcpy says the picture is {:?} wide per tall", aspect);

        let before = win::visible_rect(hwnd).unwrap_or_default();
        let (cw, ch) = win::client_size(hwnd).unwrap_or((0, 0));
        println!("as opened: {}x{} of picture, {:.4} shape", cw, ch, cw as f64 / ch as f64);

        // drag the right edge far out, which is what makes the black bars
        win::place_visible(hwnd, before.left, before.top, before.w() + 260, before.h());
        let (cw, ch) = win::client_size(hwnd).unwrap_or((0, 0));
        println!("pulled out:  {}x{} of picture, {:.4} shape", cw, ch, cw as f64 / ch as f64);

        let group = std::collections::HashSet::new();
        magnet::resize_done(&board.pedals.lock().unwrap(), &serial, &before, &group, true);
        let (cw, ch) = win::client_size(hwnd).unwrap_or((0, 0));
        let shape = cw as f64 / ch.max(1) as f64;
        println!("put back:    {}x{} of picture, {:.4} shape", cw, ch, shape);

        let wanted = aspect.unwrap_or(shape);
        let off = (shape - wanted).abs();
        println!("off by {:.4}", off);
        stop_inner(&board, &serial);
        return if off < 0.02 { 0 } else { 1 };
    }

    if what == "resizegroup" {
        // two phones, side by side and touching, then one of them is dragged
        // taller: the other should come with it and the row should close up
        let board = Board::default();
        let list = devices();
        if list.len() < 2 {
            println!("needs two phones");
            return 1;
        }
        for (serial, name, keyboard) in &list {
            if let Err(e) = start_inner(&board, serial.clone(), test_opts(name, keyboard)) {
                println!("{} FAILED: {}", name, e);
                return 1;
            }
        }
        let pedals = board.pedals.lock().unwrap();
        let show = |label: &str| {
            for (serial, _, _) in &list {
                if let Some(r) = pedals.get(serial).and_then(|p| win::visible_rect(p.hwnd)) {
                    println!("  {:<10} {:<18} {},{} {}x{}", label, serial, r.left, r.top, r.w(), r.h());
                }
            }
        };
        show("before");

        let first = list[0].0.clone();
        let hwnd = pedals[&first].hwnd;
        let from = win::visible_rect(hwnd).unwrap_or_default();
        // drag its bottom edge down by 120, the way a mouse would
        win::place_visible(hwnd, from.left, from.top, from.w(), from.h() + 120);

        let group: std::collections::HashSet<String> =
            list[1..].iter().map(|(s, _, _)| s.clone()).collect();
        magnet::resize_done(&pedals, &first, &from, &group, false);
        show("after");

        let mut rects: Vec<win::Rect> = list
            .iter()
            .filter_map(|(s, _, _)| pedals.get(s).and_then(|p| win::visible_rect(p.hwnd)))
            .collect();
        rects.sort_by_key(|r| r.left);
        let same_height = rects.windows(2).all(|w| (w[0].h() - w[1].h()).abs() <= 2);
        let flush = rects.windows(2).all(|w| (w[0].right - w[1].left).abs() <= 2);
        let grew = rects.iter().any(|r| r.h() > from.h());
        println!("all the same height: {}", same_height);
        println!("still one slab:      {}", flush);
        println!("actually taller:     {}", grew);
        drop(pedals);
        for (serial, _, _) in &list {
            stop_inner(&board, serial);
        }
        return if same_height && flush && grew { 0 } else { 1 };
    }

    if what == "drags" {
        // the magnet takes its cue from the shell rather than from guessing,
        // so this is what it hears while you push a phone around
        println!("watching for 25 seconds - drag a phone window about");
        let drags = win::watch_drags();
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut seen = 0;
        while Instant::now() < deadline {
            while let Ok(event) = drags.try_recv() {
                seen += 1;
                match event {
                    win::Drag::Started(h) => println!(
                        "  grabbed   {:>10}  {}", h, win::owner_exe(h)),
                    win::Drag::Ended(h) => println!(
                        "  dropped   {:>10}  {:?}", h, win::visible_rect(h)),
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        println!("{} events", seen);
        return if seen > 0 { 0 } else { 1 };
    }

    if what == "autostart" {
        let before = startup::enabled();
        println!("enabled to begin with: {}", before);
        if let Err(e) = startup::set(true) {
            println!("could not enable: {}", e);
            return 1;
        }
        println!("after enabling: {}", startup::enabled());
        if let Err(e) = startup::set(false) {
            println!("could not disable: {}", e);
            return 1;
        }
        println!("after disabling: {}", startup::enabled());
        let _ = startup::set(before);
        println!("restored to: {}", startup::enabled());
        return 0;
    }

    if what == "readopt" {
        readopt(&board);
        let found: Vec<(String, isize)> = board
            .pedals
            .lock()
            .unwrap()
            .iter()
            .map(|(s, p)| (s.clone(), p.hwnd))
            .collect();
        println!("readopted {} phone(s): {:?}", found.len(), found);
        for (serial, _) in &found {
            stop_inner(&board, serial);
        }
        println!("closed them again");
        return if found.is_empty() { 1 } else { 0 };
    }

    if what == "all" || what == "keep" {
        let list = devices();
        if list.is_empty() {
            println!("no phones");
            return 1;
        }
        for (serial, name, keyboard) in &list {
            match start_inner(&board, serial.clone(), test_opts(name, keyboard)) {
                Ok(started) => println!("{} started, hwnd {}", name, started.hwnd),
                Err(e) => {
                    println!("{} FAILED: {}", name, e);
                    return 1;
                }
            }
        }
        let rects: Vec<(String, win::Rect)> = board
            .pedals
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(s, p)| win::visible_rect(p.hwnd).map(|r| (s.clone(), r)))
            .collect();
        for (serial, r) in &rects {
            println!("{} at {},{} {}x{}", serial, r.left, r.top, r.w(), r.h());
        }
        if rects.len() > 1 {
            let touching = magnet::adjacent(&rects[0].1, &rects[1].1);
            println!("parked flush against each other: {}", touching);
        }
        if what == "keep" {
            println!("left running");
            std::mem::forget(board);
            return 0;
        }
        for (serial, _) in &rects {
            stop_inner(&board, serial);
        }
        println!("all closed");
        return 0;
    }

    let serial = what;
    let opts = test_opts("Self Test", "uhid");
    match start_inner(&board, serial.clone(), opts) {
        Ok(started) => println!("start ok, hwnd {}", started.hwnd),
        Err(e) => {
            println!("start FAILED: {}", e);
            return 1;
        }
    }
    let hwnd = board.pedals.lock().unwrap().get(&serial).map(|p| p.hwnd).unwrap_or(0);
    println!("window alive: {}", win::is_window(hwnd));
    println!("rect: {:?}", win::visible_rect(hwnd));
    println!("published: {}", std::fs::read_to_string(local_dir().join("windows.json"))
        .unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" "));

    std::thread::sleep(Duration::from_secs(3));
    let survived = win::is_window(hwnd);
    println!("still there after 3s: {}", survived);

    stop_inner(&board, &serial);
    std::thread::sleep(Duration::from_millis(400));
    println!("gone after stop: {}", !win::is_window(hwnd));
    if survived { 0 } else { 1 }
}

// ------------------------------------------------------------ start-up ----

#[tauri::command]
fn autostart_state() -> bool {
    startup::enabled()
}

#[tauri::command]
fn set_autostart(on: bool) -> Result<bool, String> {
    startup::set(on)?;
    Ok(startup::enabled())
}

#[tauri::command]
fn hide_board(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

/// The page has just changed height. Put the panel back in its corner and
/// round the window itself off - a border-radius in CSS only paints inside a
/// square window, and the square shows.
#[tauri::command]
fn settled(app: tauri::AppHandle) {
    // a panel that has been carried off somewhere stays there. It goes back to
    // the tray corner when it is put away and opened again, and on a cold
    // start - not because its own contents changed height.
    if adrift(&app) {
        keep_on_screen(&app);
    } else {
        park_over_tray(&app);
    }
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                win::round_corners(h.hwnd.get(), 14);
            }
        }
    }
}

/// A panel that belongs to a tray icon should come up beside the tray.
///
/// Which means asking where the tray is rather than assuming: the taskbar can
/// be on any of the four edges, it can be on a second monitor, and on a
/// right-to-left Windows the notification area sits at the *left* end of it.
fn park_over_tray(app: &tauri::AppHandle) {
    const GAP: i32 = 12;
    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => return,
    };
    let size = match window.outer_size() {
        Ok(s) => s,
        Err(_) => return,
    };
    let (w, h) = (size.width as i32, size.height as i32);

    let tray = win::tray_area();
    let bar = win::taskbar().map(|(rect, _)| rect).or(tray);
    let anchor = tray.or(bar);

    let (cx, cy) = match anchor {
        Some(r) => ((r.left + r.right) / 2, (r.top + r.bottom) / 2),
        None => {
            let area = win::work_area();
            (area.right, area.bottom)
        }
    };
    let area = win::work_area_at(cx, cy);
    let edge = bar.map(|r| win::edge_of(&r, &area)).unwrap_or(win::Edge::Bottom);

    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
    let (x, y) = match edge {
        win::Edge::Bottom => (
            clamp(cx - w / 2, area.left + GAP, area.right - w - GAP),
            area.bottom - h - GAP,
        ),
        win::Edge::Top => (
            clamp(cx - w / 2, area.left + GAP, area.right - w - GAP),
            area.top + GAP,
        ),
        win::Edge::Left => (
            area.left + GAP,
            clamp(cy - h / 2, area.top + GAP, area.bottom - h - GAP),
        ),
        win::Edge::Right => (
            area.right - w - GAP,
            clamp(cy - h / 2, area.top + GAP, area.bottom - h - GAP),
        ),
    };
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    *app.state::<Board>().parked.lock().unwrap() = Some((x, y));
}

/// Has the window been moved since the board last placed it?
fn adrift(app: &tauri::AppHandle) -> bool {
    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => return false,
    };
    match (window.outer_position(), *app.state::<Board>().parked.lock().unwrap()) {
        (Ok(now), Some((x, y))) => (now.x - x).abs() > 4 || (now.y - y).abs() > 4,
        _ => true,
    }
}

/// Keep a window that is somewhere of its own choosing on the screen, without
/// dragging it back to the tray.
fn keep_on_screen(app: &tauri::AppHandle) {
    const GAP: i32 = 4;
    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => return,
    };
    let (pos, size) = match (window.outer_position(), window.outer_size()) {
        (Ok(p), Ok(s)) => (p, s),
        _ => return,
    };
    let (w, h) = (size.width as i32, size.height as i32);
    let area = win::work_area_at(pos.x + w / 2, pos.y + h / 2);
    let x = pos.x.min(area.right - w - GAP).max(area.left + GAP);
    let y = pos.y.min(area.bottom - h - GAP).max(area.top + GAP);
    if x != pos.x || y != pos.y {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

fn reveal(app: &tauri::AppHandle) {
    park_over_tray(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// The tray is the whole point of a strip you do not want in the way: the
/// phones keep running whether the window is up or not, so the window can
/// spend most of its life hidden.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show", "Show the board", true, None::<&str>)?;
    let login = CheckMenuItem::with_id(
        app, "login", "Start at login", true, startup::enabled(), None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit (phones stay up)", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show, &PredefinedMenuItem::separator(app)?, &login,
          &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let mut tray = TrayIconBuilder::with_id("board")
        .menu(&menu)
        .tooltip("scrcpy board")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => reveal(app),
            "login" => {
                let want = !startup::enabled();
                if let Err(e) = startup::set(want) {
                    eprintln!("start at login: {}", e);
                }
                let _ = app.emit("autostart-changed", startup::enabled());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                match app.get_webview_window("main") {
                    Some(window) if window.is_visible().unwrap_or(false) => {
                        let _ = window.hide();
                    }
                    _ => reveal(app),
                }
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "selftest" {
        std::process::exit(selftest(args[2].clone()));
    }
    if args.len() > 2 && args[1] == "autostart" {
        // whichever copy of the exe you run this from is the one registered
        let on = args[2] == "on";
        match startup::set(on) {
            Ok(()) => {
                println!("start at login: {}", if startup::enabled() { "on" } else { "off" });
                std::process::exit(0);
            }
            Err(e) => {
                println!("{}", e);
                std::process::exit(1);
            }
        }
    }
    let start_hidden = args.iter().any(|a| a == "--hidden");

    if !startup::claim_only_copy() {
        // a board is already up: bring that one forward rather than adding a
        // second tray icon and a second magnet loop
        let ours = std::process::id();
        if let Some(hwnd) = win::window_of_other_instance("scrcpy-board.exe", ours) {
            win::show_and_activate(hwnd);
        }
        std::process::exit(0);
    }

    tauri::Builder::default()
        .manage(Board::default())
        .setup(move |app| {
            readopt(&app.state::<Board>());
            start_daemon();
            build_tray(app.handle())?;
            if start_hidden {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            } else {
                park_over_tray(app.handle());
            }
            // the shell tells us when a drag starts and ends; the magnet loop
            // only has to carry the group and watch how hard it is thrown
            let drags = win::watch_drags();
            let handle = app.handle().clone();
            std::thread::spawn(move || magnet::run(handle, drags));
            Ok(())
        })
        // closing the window only puts it away: the phones are ordinary
        // windows and outlive the strip, and readopt() picks them up again
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            probe, start, stop, running, set_skin, arrange,
            focus_device, action, rotate, status, open_logs,
            autostart_state, set_autostart, hide_board, settled
        ])
        .run(tauri::generate_context!())
        .expect("board failed to start");
}
