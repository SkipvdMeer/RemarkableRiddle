//! Coordinate systems.
//!
//! Screen space: portrait 1404 x 1872 px, origin top-left, y down.
//! Wacom (pen digitizer) space on the reMarkable 2: ABS_X runs along the
//! *long* axis of the device with 0 at the bottom of the portrait screen,
//! ABS_Y along the short axis. The transform below matches ghostwriter's
//! known-good rM2 mapping. If `--test-draw` comes out rotated or mirrored
//! on a future firmware, this file is the only place to fix.

pub const SCREEN_W: f32 = 1404.0;
pub const SCREEN_H: f32 = 1872.0;
pub const WACOM_X_MAX: f32 = 20966.0;
pub const WACOM_Y_MAX: f32 = 15725.0;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub fn screen_to_wacom(sx: f32, sy: f32) -> (i32, i32) {
    let wx = (1.0 - sy / SCREEN_H) * WACOM_X_MAX;
    let wy = (sx / SCREEN_W) * WACOM_Y_MAX;
    (
        (wx.round() as i32).clamp(0, WACOM_X_MAX as i32),
        (wy.round() as i32).clamp(0, WACOM_Y_MAX as i32),
    )
}

pub fn wacom_to_screen(wx: i32, wy: i32) -> (f32, f32) {
    let sx = wy as f32 / WACOM_Y_MAX * SCREEN_W;
    let sy = (1.0 - wx as f32 / WACOM_X_MAX) * SCREEN_H;
    (sx, sy)
}

/// Insert intermediate points so no segment is longer than `max_step`.
/// Makes injected strokes move at a constant, human-ish speed and gives
/// xochitl a smooth polyline to ink.
pub fn densify(points: &[(f32, f32)], max_step: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(points.len() * 2);
    for (i, &p) in points.iter().enumerate() {
        if i == 0 {
            out.push(p);
            continue;
        }
        let prev = points[i - 1];
        let (dx, dy) = (p.0 - prev.0, p.1 - prev.1);
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > max_step {
            let steps = (dist / max_step).ceil() as usize;
            for s in 1..steps {
                let t = s as f32 / steps as f32;
                out.push((prev.0 + dx * t, prev.1 + dy * t));
            }
        }
        out.push(p);
    }
    out
}

/// Drop points closer than `min_dist` to the previous kept point. Captured
/// handwriting arrives at ~200 Hz — far denser than the eraser needs — and
/// replaying it raw floods xochitl's event queue until the kernel drops
/// events and pen state (like lifts) gets lost.
pub fn resample(points: &[(f32, f32)], min_dist: f32) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len() / 2 + 2);
    for &p in points {
        if let Some(&q) = out.last() {
            let (dx, dy) = (p.0 - q.0, p.1 - q.1);
            if (dx * dx + dy * dy).sqrt() < min_dist {
                continue;
            }
        }
        out.push(p);
    }
    if let Some(&last) = points.last() {
        if out.last() != Some(&last) {
            out.push(last);
        }
    }
    out
}

pub fn polylines_bbox(polys: &[Vec<(f32, f32)>]) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    for poly in polys {
        for &(x, y) in poly {
            r = Some(match r {
                None => Rect {
                    left: x,
                    top: y,
                    right: x,
                    bottom: y,
                },
                Some(b) => Rect {
                    left: b.left.min(x),
                    top: b.top.min(y),
                    right: b.right.max(x),
                    bottom: b.bottom.max(y),
                },
            });
        }
    }
    r
}
