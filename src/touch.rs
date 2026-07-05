//! Corner-tap detection on the touchscreen — the manual "send it" trigger.

use crate::events::*;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub fn parse(s: &str) -> Corner {
        match s.to_lowercase().replace('_', "-").as_str() {
            "top-left" => Corner::TopLeft,
            "bottom-right" => Corner::BottomRight,
            "bottom-left" => Corner::BottomLeft,
            _ => Corner::TopRight,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TouchConfig {
    pub corner: Corner,
    /// Corner hot-zone size as a fraction of each screen dimension.
    pub corner_size: f32,
    /// Raw axis maxima for the touch controller (rM2 defaults).
    pub max_x: f32,
    pub max_y: f32,
    /// The rM2 touch panel reports y with origin at the bottom of the
    /// portrait screen; flip to screen orientation.
    pub invert_x: bool,
    pub invert_y: bool,
}

impl Default for TouchConfig {
    fn default() -> Self {
        TouchConfig {
            corner: Corner::TopRight,
            corner_size: 0.12,
            max_x: 1403.0,
            max_y: 1871.0,
            invert_x: false,
            invert_y: true,
        }
    }
}

pub fn spawn_touch_reader(path: PathBuf, cfg: TouchConfig, tap_tx: Sender<()>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("touch reader: cannot open {}: {e}", path.display());
                return;
            }
        };
        let mut buf = [0u8; EVENT_SIZE];
        let (mut x, mut y) = (0i32, 0i32);
        let mut down_at: Option<(Instant, i32, i32)> = None;

        loop {
            if file.read_exact(&mut buf).is_err() {
                eprintln!("touch reader: device read failed, stopping");
                return;
            }
            let ev = Event::parse(&buf);
            match (ev.type_, ev.code) {
                (EV_ABS, ABS_MT_POSITION_X) => x = ev.value,
                (EV_ABS, ABS_MT_POSITION_Y) => y = ev.value,
                (EV_ABS, ABS_MT_TRACKING_ID) => {
                    if ev.value >= 0 {
                        down_at = Some((Instant::now(), x, y));
                    } else if let Some((t0, x0, y0)) = down_at.take() {
                        let quick = t0.elapsed().as_millis() < 500;
                        let steady = (x - x0).abs() < 40 && (y - y0).abs() < 40;
                        if quick && steady && in_corner(&cfg, x as f32, y as f32) {
                            let _ = tap_tx.send(());
                        }
                    }
                }
                _ => {}
            }
        }
    })
}

fn in_corner(cfg: &TouchConfig, x: f32, y: f32) -> bool {
    let mut nx = (x / cfg.max_x).clamp(0.0, 1.0);
    let mut ny = (y / cfg.max_y).clamp(0.0, 1.0);
    if cfg.invert_x {
        nx = 1.0 - nx;
    }
    if cfg.invert_y {
        ny = 1.0 - ny;
    }
    let s = cfg.corner_size;
    match cfg.corner {
        Corner::TopRight => nx > 1.0 - s && ny < s,
        Corner::TopLeft => nx < s && ny < s,
        Corner::BottomRight => nx > 1.0 - s && ny > 1.0 - s,
        Corner::BottomLeft => nx < s && ny > 1.0 - s,
    }
}
