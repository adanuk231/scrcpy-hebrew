//! Where each phone was left, so it opens there next time.
//!
//! A phone that opens in the top left corner every time is a phone you move
//! every time. This is written down beside the logs, keyed by serial, and
//! merged rather than replaced: stopping one phone must not lose the spot of
//! another that is not out at the moment.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::win;

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Spot {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// The window height it was started with. A phone keeps the size you
    /// dragged it to for as long as that setting stays put; move the slider
    /// and the explicit choice wins, because you have just made it.
    #[serde(default)]
    pub asked: u32,
}

fn path() -> PathBuf {
    crate::local_dir().join("places.json")
}

pub fn all() -> HashMap<String, Spot> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn of(serial: &str) -> Option<Spot> {
    all().remove(serial).filter(|s| s.w > 0 && s.h > 0)
}

fn edit(change: impl FnOnce(&mut HashMap<String, Spot>)) {
    let mut known = all();
    change(&mut known);
    let dir = crate::local_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string_pretty(&known) {
        let _ = std::fs::write(path(), text);
    }
}

/// Write down where the phones that are out are sitting now.
pub fn keep(seen: &HashMap<String, win::Rect>) {
    edit(|known| {
        for (serial, r) in seen {
            let spot = known.entry(serial.clone()).or_default();
            spot.x = r.left;
            spot.y = r.top;
            spot.w = r.w();
            spot.h = r.h();
        }
    });
}

/// Note what was asked for at the moment a phone was started.
pub fn asked_for(serial: &str, height: u32) {
    edit(|known| {
        known.entry(serial.to_string()).or_default().asked = height;
    });
}
