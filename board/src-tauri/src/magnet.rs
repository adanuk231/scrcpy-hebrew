//! Magnetism between the phone windows. Always on.
//!
//! Phones never sit on top of each other: one dropped over another is pushed
//! out to whichever side is nearest and left flush against it. Whatever is
//! stuck to a phone travels with it - unless you hold ctrl, which takes the
//! one you are dragging out of its group and leaves the rest where they are.
//!
//! All of this works in *visible* coordinates. `GetWindowRect` reports the
//! extended bounds, which on Windows 10 include an invisible resize border of
//! about seven pixels a side: snap two windows so those rectangles touch and
//! you see a fourteen pixel gap, snap them so they look flush and the
//! rectangles overlap. That mismatch is also why a group would quietly stop
//! forming once windows had been moved by hand.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::win;
use crate::{publish_windows, Board, Pedal};

/// how close two edges have to be before they click together
const SNAP: i32 = 26;
/// how close counts as already stuck together
const TOUCHING: i32 = 8;
/// how far a window will travel to line up with one it has just joined
const ALIGN: i32 = 90;
const TICK: Duration = Duration::from_millis(30);

fn spans(a1: i32, a2: i32, b1: i32, b2: i32) -> bool {
    a1.min(a2) < b1.max(b2) && b1.min(b2) < a1.max(a2)
}

fn over(a: &win::Rect, b: &win::Rect) -> bool {
    spans(a.left, a.right, b.left, b.right) && spans(a.top, a.bottom, b.top, b.bottom)
}

fn shifted(r: &win::Rect, dx: i32, dy: i32) -> win::Rect {
    win::Rect {
        left: r.left + dx,
        top: r.top + dy,
        right: r.right + dx,
        bottom: r.bottom + dy,
    }
}

/// Are these two stuck to each other right now?
pub fn adjacent(a: &win::Rect, b: &win::Rect) -> bool {
    let side_by_side = spans(a.top, a.bottom, b.top, b.bottom)
        && ((a.right - b.left).abs() <= TOUCHING || (b.right - a.left).abs() <= TOUCHING);
    let stacked = spans(a.left, a.right, b.left, b.right)
        && ((a.bottom - b.top).abs() <= TOUCHING || (b.bottom - a.top).abs() <= TOUCHING);
    side_by_side || stacked
}

/// Everything stuck to `seed`, directly or through another phone.
pub fn cluster(seed: &str, rects: &HashMap<String, win::Rect>) -> HashSet<String> {
    let mut group: HashSet<String> = HashSet::new();
    group.insert(seed.to_string());
    loop {
        let mut grew = false;
        for (serial, rect) in rects {
            if group.contains(serial) {
                continue;
            }
            if group
                .iter()
                .any(|m| rects.get(m).map_or(false, |r| adjacent(r, rect)))
            {
                group.insert(serial.clone());
                grew = true;
            }
        }
        if !grew {
            return group;
        }
    }
}

/// The shortest way out of everything it is sitting on top of.
fn separate(moving: &win::Rect, others: &[win::Rect]) -> (i32, i32) {
    let mut dx = 0;
    let mut dy = 0;
    for _ in 0..8 {
        let now = shifted(moving, dx, dy);
        let hit = match others.iter().find(|o| over(&now, o)) {
            Some(o) => o,
            None => break,
        };
        // four ways out: to its left, to its right, above it, below it
        let ways = [
            (hit.left - now.right, 0),
            (hit.right - now.left, 0),
            (0, hit.top - now.bottom),
            (0, hit.bottom - now.top),
        ];
        let (px, py) = ways
            .iter()
            .min_by_key(|(x, y)| x.abs() + y.abs())
            .copied()
            .unwrap_or((0, 0));
        if px == 0 && py == 0 {
            break;
        }
        dx += px;
        dy += py;
    }
    (dx, dy)
}

fn choose(candidates: &[(i32, bool)], join_limit: i32, align_limit: i32) -> (i32, bool) {
    let mut best: Option<(i32, bool)> = None;
    for (gap, is_join) in candidates {
        let limit = if *is_join { join_limit } else { align_limit };
        if gap.abs() > limit {
            continue;
        }
        let better = match best {
            None => true,
            Some((chosen, chosen_join)) => {
                (*is_join && !chosen_join)
                    || (*is_join == chosen_join && gap.abs() < chosen.abs())
            }
        };
        if better {
            best = Some((*gap, *is_join));
        }
    }
    best.unwrap_or((0, false))
}

/// The offset that clicks `moving` onto one of `others`, or onto the edge of
/// the screen, if anything is close enough.
fn snap_offset(moving: &win::Rect, others: &[win::Rect], area: &win::Rect) -> (i32, i32) {
    // two kinds of candidate: a join, where an edge meets an edge, and an
    // alignment, where two windows merely line up. Joins win, and once one has
    // happened the alignment on the other axis gets a longer reach, so phones
    // put shoulder to shoulder end up flush rather than stepped.
    let mut xs: Vec<(i32, bool)> = Vec::new();
    let mut ys: Vec<(i32, bool)> = Vec::new();

    for o in others {
        if spans(moving.top, moving.bottom, o.top, o.bottom) {
            xs.push((o.left - moving.right, true));
            xs.push((o.right - moving.left, true));
            ys.push((o.top - moving.top, false));
            ys.push((o.bottom - moving.bottom, false));
        }
        if spans(moving.left, moving.right, o.left, o.right) {
            ys.push((o.top - moving.bottom, true));
            ys.push((o.bottom - moving.top, true));
            xs.push((o.left - moving.left, false));
            xs.push((o.right - moving.right, false));
        }
    }
    xs.push((area.left - moving.left, false));
    xs.push((area.right - moving.right, false));
    ys.push((area.top - moving.top, false));
    ys.push((area.bottom - moving.bottom, false));

    let (dx, joined_x) = choose(&xs, SNAP, SNAP);
    let (dy, joined_y) = choose(&ys, SNAP, if joined_x { ALIGN } else { SNAP });
    if joined_y && !joined_x {
        let (dx, _) = choose(&xs, SNAP, ALIGN);
        return (dx, dy);
    }
    (dx, dy)
}

/// Where a dropped phone belongs: out of everyone's way first, then clicked
/// onto whatever it landed beside.
fn settle(moving: &win::Rect, others: &[win::Rect], area: &win::Rect) -> (i32, i32) {
    let (mut dx, mut dy) = separate(moving, others);
    let (sx, sy) = snap_offset(&shifted(moving, dx, dy), others, area);
    dx += sx;
    dy += sy;
    // an alignment can in principle slide it back onto somebody, so one more pass
    let (fx, fy) = separate(&shifted(moving, dx, dy), others);
    (dx + fx, dy + fy)
}

pub fn geometry(pedals: &HashMap<String, Pedal>) -> HashMap<String, win::Rect> {
    pedals
        .iter()
        .filter(|(_, p)| win::is_window(p.hwnd) && !win::is_minimised(p.hwnd))
        .filter_map(|(s, p)| win::visible_rect(p.hwnd).map(|r| (s.clone(), r)))
        .collect()
}

/// Line them up in the given order, flush, starting at the top left.
pub fn arrange(pedals: &HashMap<String, Pedal>, order: &[String]) {
    let area = win::work_area();
    let mut x = area.left + 24;
    let y = area.top + 24;
    for serial in order {
        if let Some(pedal) = pedals.get(serial) {
            if let Some(r) = win::visible_rect(pedal.hwnd) {
                win::move_visible_to(pedal.hwnd, x, y);
                x += r.w();
            }
        }
    }
}

/// The largest box of the right shape that fits inside what was dragged out.
///
/// The picture has a fixed shape, so a window of any other shape is padding.
/// Fitting inside rather than around means the window only ever ends up
/// smaller than you dragged it, never surprising you by growing.
fn fit_to_aspect(w: i32, h: i32, aspect: f64) -> (i32, i32) {
    let by_width = ((h as f64) * aspect).round() as i32;
    if by_width <= w {
        (by_width.max(80), h.max(80))
    } else {
        let by_height = ((w as f64) / aspect).round() as i32;
        (w.max(80), by_height.max(80))
    }
}

/// Put a phone at this visible size, keeping whichever edges were not dragged.
///
/// The edge you pulled is the one you meant, so it decides and the other
/// follows. Drag the bottom down and the window gets taller *and* wider;
/// only a corner, where you moved both, is fitted inside what you drew.
fn resize_one(hwnd: isize, aspect: f64, want: &win::Rect, from: &win::Rect) -> win::Rect {
    let (extra_w, extra_h) = win::frame_extra(hwnd);
    let want_w = (want.w() - extra_w).max(80);
    let want_h = (want.h() - extra_h).max(80);
    let moved_w = want.w() != from.w();
    let moved_h = want.h() != from.h();
    let (cw, ch) = if moved_w && !moved_h {
        (want_w, ((want_w as f64) / aspect).round().max(80.0) as i32)
    } else if moved_h && !moved_w {
        (((want_h as f64) * aspect).round().max(80.0) as i32, want_h)
    } else {
        fit_to_aspect(want_w, want_h, aspect)
    };
    let (w, h) = (cw + extra_w, ch + extra_h);
    // the edge that moved is the one under the cursor, so hold the other one
    let x = if want.left != from.left { want.right - w } else { want.left };
    let y = if want.top != from.top { want.bottom - h } else { want.top };
    win::place_visible(hwnd, x, y, w, h);
    win::Rect { left: x, top: y, right: x + w, bottom: y + h }
}

/// A phone has been resized. Take the black bars off it, and unless ctrl was
/// held, put the rest of its group to the same height (or width) and close the
/// row up again so it stays one slab.
pub fn resize_done(pedals: &HashMap<String, Pedal>, serial: &str, from: &win::Rect,
               group: &HashSet<String>, solo: bool) {
    let pedal = match pedals.get(serial) {
        Some(p) => p,
        None => return,
    };
    let now = match win::visible_rect(pedal.hwnd) {
        Some(r) => r,
        None => return,
    };
    let aspect = crate::video_aspect(serial)
        .unwrap_or(now.w() as f64 / now.h().max(1) as f64);
    let settled = resize_one(pedal.hwnd, aspect, &now, from);
    if solo || group.is_empty() {
        // grown sideways, it may be sitting on somebody now
        let others: Vec<win::Rect> = pedals
            .iter()
            .filter(|(s, _)| s.as_str() != serial)
            .filter_map(|(_, p)| win::visible_rect(p.hwnd))
            .collect();
        let (dx, dy) = settle(&settled, &others, &win::work_area());
        if dx != 0 || dy != 0 {
            win::move_visible_to(pedal.hwnd, settled.left + dx, settled.top + dy);
        }
        return;
    }

    // a row or a column? whichever way the group already lies
    let mates: Vec<(&String, win::Rect)> = group
        .iter()
        .filter_map(|s| pedals.get(s).and_then(|p| win::visible_rect(p.hwnd)).map(|r| (s, r)))
        .collect();
    let row = mates.iter().all(|(_, r)| spans(r.top, r.bottom, from.top, from.bottom));
    let column = !row
        && mates.iter().all(|(_, r)| spans(r.left, r.right, from.left, from.right));
    if !row && !column {
        return;
    }

    let mut line: Vec<(isize, f64, win::Rect)> = Vec::new();
    for (s, r) in &mates {
        if let Some(p) = pedals.get(*s) {
            let a = crate::video_aspect(s).unwrap_or(r.w() as f64 / r.h().max(1) as f64);
            line.push((p.hwnd, a, *r));
        }
    }
    line.push((pedal.hwnd, aspect, settled));

    if row {
        line.sort_by_key(|(_, _, r)| r.left);
        let left = line.iter().map(|(_, _, r)| r.left).min().unwrap_or(settled.left);
        let mut x = left;
        for (hwnd, a, _) in &line {
            let (extra_w, extra_h) = win::frame_extra(*hwnd);
            let ch = settled.h() - extra_h;
            let cw = ((ch as f64) * a).round() as i32;
            win::place_visible(*hwnd, x, settled.top, cw + extra_w, ch + extra_h);
            x += cw + extra_w;
        }
    } else {
        line.sort_by_key(|(_, _, r)| r.top);
        let top = line.iter().map(|(_, _, r)| r.top).min().unwrap_or(settled.top);
        let mut y = top;
        for (hwnd, a, _) in &line {
            let (extra_w, extra_h) = win::frame_extra(*hwnd);
            let cw = settled.w() - extra_w;
            let ch = ((cw as f64) / a).round() as i32;
            win::place_visible(*hwnd, settled.left, y, cw + extra_w, ch + extra_h);
            y += ch + extra_h;
        }
    }
}

/// Windows says when a drag starts and ends. The polling in between only
/// carries the group along and measures how hard the phone is being thrown.
pub fn run(app: tauri::AppHandle, drags: std::sync::mpsc::Receiver<win::Drag>) {
    let mut dragging: Option<String> = None;
    let mut group: HashSet<String> = HashSet::new();
    let mut anchor = (0, 0);
    let mut solo = false;
    let mut was_visible = true;
    let mut grabbed: Option<win::Rect> = None;

    loop {
        std::thread::sleep(TICK);

        // put away and brought back means back to its corner by the tray,
        // however it was brought back - the icon, the menu, or running the exe
        // again. Moving it by hand is what makes it stay where it is put, and
        // that only lasts as long as it is on screen.
        if let Some(window) = app.get_webview_window("main") {
            let visible = window.is_visible().unwrap_or(false);
            if visible && !was_visible {
                crate::park_over_tray(&app);
            }
            was_visible = visible;
        }

        let board = app.state::<Board>();
        let mut pedals = board.pedals.lock().unwrap();

        let lost: Vec<String> = pedals
            .iter()
            .filter(|(_, p)| !win::is_window(p.hwnd))
            .map(|(s, _)| s.clone())
            .collect();
        if !lost.is_empty() {
            for serial in &lost {
                pedals.remove(serial);
                if dragging.as_deref() == Some(serial.as_str()) {
                    dragging = None;
                    group.clear();
                }
            }
            publish_windows(&pedals);
            let _ = app.emit("pedals-changed", &lost);
        }

        let rects = geometry(&pedals);

        while let Ok(event) = drags.try_recv() {
            match event {
                win::Drag::Started(hwnd) => {
                    let ours = pedals.iter().find(|(_, p)| p.hwnd == hwnd).map(|(s, _)| s.clone());
                    if let Some(serial) = ours {
                        // ctrl at the moment of grabbing means take this one
                        // out on its own and leave the row alone
                        solo = win::ctrl_down();
                        group = if solo {
                            HashSet::new()
                        } else {
                            let mut c = cluster(&serial, &rects);
                            c.remove(&serial);
                            c
                        };
                        anchor = rects.get(&serial).map(|r| (r.left, r.top)).unwrap_or((0, 0));
                        grabbed = rects.get(&serial).copied();
                        dragging = Some(serial);
                    }
                }
                win::Drag::Ended(hwnd) => {
                    let ours = pedals.iter().find(|(_, p)| p.hwnd == hwnd).map(|(s, _)| s.clone());
                    let is_ours = ours.as_deref() == dragging.as_deref();
                    if let (Some(serial), true) = (ours, is_ours) {
                        let resized = match (grabbed, win::visible_rect(pedals[&serial].hwnd)) {
                            (Some(was), Some(now)) => was.w() != now.w() || was.h() != now.h(),
                            _ => false,
                        };
                        if resized {
                            let from = grabbed.unwrap_or_default();
                            resize_done(&pedals, &serial, &from, &group, solo);
                            dragging = None;
                            group.clear();
                            solo = false;
                            grabbed = None;
                            continue;
                        }
                        if let Some(now) = win::visible_rect(pedals[&serial].hwnd) {
                            let others: Vec<win::Rect> = rects
                                .iter()
                                .filter(|(s, _)| **s != serial && !group.contains(*s))
                                .map(|(_, r)| *r)
                                .collect();
                            let (dx, dy) = settle(&now, &others, &win::work_area());
                            if dx != 0 || dy != 0 {
                                win::move_visible_to(pedals[&serial].hwnd, now.left + dx, now.top + dy);
                                for mate in group.iter() {
                                    if let (Some(r), Some(p)) = (rects.get(mate), pedals.get(mate)) {
                                        win::move_visible_to(p.hwnd, r.left + dx, r.top + dy);
                                    }
                                }
                            }
                        }
                        dragging = None;
                        group.clear();
                        solo = false;
                        grabbed = None;
                    }
                }
            }
        }

        if let Some(serial) = dragging.clone() {
            // ctrl part way through a drag counts too, so you can start
            // moving a pair and then decide to peel one off
            if !solo && win::ctrl_down() {
                solo = true;
                if !group.is_empty() {
                    group.clear();
                    let _ = app.emit("detached", &serial);
                }
            }
            if let Some(now) = rects.get(&serial) {
                let (dx, dy) = (now.left - anchor.0, now.top - anchor.1);
                if dx != 0 || dy != 0 {
                    for mate in group.iter() {
                        if let (Some(r), Some(p)) = (rects.get(mate), pedals.get(mate)) {
                            win::move_visible_to(p.hwnd, r.left + dx, r.top + dy);
                        }
                    }
                    anchor = (now.left, now.top);
                }
            }
        }
    }
}
