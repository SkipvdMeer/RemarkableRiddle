//! The invisible hand: injects pen and eraser events by writing raw
//! input_event structs to the pen device node. This is how the answer gets
//! written in real ink, and how ink is unwritten again.
//!
//! No uinput module, no xochitl hooks — the kernel delivers our events to
//! xochitl exactly as if a hand were moving the marker.

use crate::capture::Shared;
use crate::events::*;
use crate::geom::{densify, screen_to_wacom};
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const PRESSURE_WRITE: i32 = 2200;
const PRESSURE_ERASE: i32 = 3800;
const HOVER_DISTANCE: i32 = 70;
const TILT_X: i32 = -2300;
const TILT_Y: i32 = 2300;
/// Max distance between injected points, in screen pixels.
const STEP_PX: f32 = 6.0;

/// xochitl consumes pen input at display-frame granularity, so state
/// transitions (tool select, touch down, lift) need to be spaced further
/// apart than the ~60 Hz frame time or they get merged — a missed lift
/// welds all the letters into one long scrawl.
const SETTLE_TOOL: Duration = Duration::from_millis(60);
const SETTLE_HOVER: Duration = Duration::from_millis(18);
const SETTLE_LIFT: Duration = Duration::from_millis(18);

pub struct Pen {
    file: File,
    shared: Arc<Shared>,
    frames_sent: u64,
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    (dx * dx + dy * dy).sqrt()
}

/// The point at index `i`, shifted `amount` px perpendicular to the local
/// direction of the polyline.
fn offset_perp(pts: &[(f32, f32)], i: usize, amount: f32) -> (f32, f32) {
    let a = pts[i.saturating_sub(1)];
    let b = pts[(i + 1).min(pts.len() - 1)];
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.001 {
        return (pts[i].0, pts[i].1 + amount);
    }
    (pts[i].0 - dy / len * amount, pts[i].1 + dx / len * amount)
}

/// Extrapolate a polyline a few pixels past its first and last points.
fn extend_ends(poly: &[(f32, f32)], by: f32) -> Vec<(f32, f32)> {
    if poly.len() < 2 {
        return poly.to_vec();
    }
    let mut out = Vec::with_capacity(poly.len() + 2);
    let ext = |from: (f32, f32), to: (f32, f32)| -> (f32, f32) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        (to.0 + dx / len * by, to.1 + dy / len * by)
    };
    out.push(ext(poly[1], poly[0]));
    out.extend_from_slice(poly);
    out.push(ext(poly[poly.len() - 2], poly[poly.len() - 1]));
    out
}

/// One smooth, forward-only erasing pass: the eraser glides along the
/// stroke while weaving across it in a gentle sine, so the whole ink band
/// is covered in a single motion. The erase front moves steadily from the
/// first point of the stroke to the last and never doubles back — on the
/// page it reads as the ink sinking away in the order it was written.
fn weave_polyline(pts: &[(f32, f32)], amplitude: f32, wavelength: f32) -> Vec<(f32, f32)> {
    let mut path = Vec::with_capacity(pts.len());
    let mut dist = 0.0f32;
    for i in 0..pts.len() {
        if i > 0 {
            let (dx, dy) = (pts[i].0 - pts[i - 1].0, pts[i].1 - pts[i - 1].1);
            dist += (dx * dx + dy * dy).sqrt();
        }
        let a = amplitude * (dist / wavelength * std::f32::consts::TAU).sin();
        path.push(offset_perp(pts, i, a));
    }
    path
}

/// RAII guard: marks the capture thread's "ignore everything, that's us"
/// flag for the duration of an injection.
struct InjectGuard(Arc<Shared>);
impl Drop for InjectGuard {
    fn drop(&mut self) {
        // Give xochitl a moment to finish consuming our events before the
        // capture thread starts listening to the writer again.
        thread::sleep(Duration::from_millis(150));
        self.0.injecting.store(false, Ordering::Relaxed);
    }
}

impl Pen {
    pub fn open(path: &Path, shared: Arc<Shared>) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .with_context(|| format!("opening pen device {} for writing", path.display()))?;
        Ok(Pen {
            file,
            shared,
            frames_sent: 0,
        })
    }

    fn frame(&mut self, events: &[Event]) -> Result<()> {
        let mut buf = Vec::with_capacity((events.len() + 1) * EVENT_SIZE);
        for ev in events {
            buf.extend_from_slice(&ev.encode());
        }
        buf.extend_from_slice(&Event::new(EV_SYN, SYN_REPORT, 0).encode());
        self.file.write_all(&buf)?;

        // Breathe every so often so xochitl can drain its event queue; if
        // it overflows, the kernel drops events and pen lifts get lost.
        self.frames_sent += 1;
        if self.frames_sent % 96 == 0 {
            thread::sleep(Duration::from_millis(30));
        }
        Ok(())
    }

    /// Trace polylines (screen coordinates) with the given tool.
    fn trace(
        &mut self,
        polys: &[Vec<(f32, f32)>],
        tool: u16,
        pressure: i32,
        pace: Duration,
        step_px: f32,
    ) -> Result<()> {
        // Wait our turn: never begin injecting while the writer's pen is
        // at the page. Two hands on one quill draws wild zigzags between
        // the two positions (and we cannot see the real pen mid-injection
        // — its events are indistinguishable from our own echo).
        while self.shared.pen_busy() {
            thread::sleep(Duration::from_millis(60));
        }
        self.shared.injecting.store(true, Ordering::Relaxed);
        let _guard = InjectGuard(self.shared.clone());

        let first = match polys.iter().find(|p| !p.is_empty()) {
            Some(p) => p[0],
            None => return Ok(()),
        };
        let (fx, fy) = screen_to_wacom(first.0, first.1);

        // Announce the tool once, hovering at a real position, and let
        // xochitl register it before any ink flows. Tool sessions are kept
        // to one per trace: rapid rubber/pen toggling reads like eraser-end
        // taps to xochitl 3.x and makes the toolbar dance.
        self.frame(&[
            Event::new(EV_KEY, tool, 1),
            Event::new(EV_ABS, ABS_DISTANCE, HOVER_DISTANCE),
            Event::new(EV_ABS, ABS_X, fx),
            Event::new(EV_ABS, ABS_Y, fy),
            Event::new(EV_ABS, ABS_TILT_X, TILT_X),
            Event::new(EV_ABS, ABS_TILT_Y, TILT_Y),
        ])?;
        thread::sleep(SETTLE_TOOL);

        for poly in polys {
            if poly.is_empty() {
                continue;
            }
            let path = densify(poly, step_px);
            let (x0, y0) = screen_to_wacom(path[0].0, path[0].1);

            // Approach in hover, then touch down. The tool bit is repeated
            // here: the kernel filters it out if unchanged, and it restores
            // the state if xochitl lost it.
            self.frame(&[
                Event::new(EV_KEY, tool, 1),
                Event::new(EV_ABS, ABS_DISTANCE, HOVER_DISTANCE),
                Event::new(EV_ABS, ABS_X, x0),
                Event::new(EV_ABS, ABS_Y, y0),
                Event::new(EV_ABS, ABS_TILT_X, TILT_X),
                Event::new(EV_ABS, ABS_TILT_Y, TILT_Y),
            ])?;
            thread::sleep(SETTLE_HOVER);
            self.frame(&[
                Event::new(EV_ABS, ABS_DISTANCE, 0),
                Event::new(EV_KEY, BTN_TOUCH, 1),
                Event::new(EV_ABS, ABS_PRESSURE, pressure),
                Event::new(EV_ABS, ABS_X, x0),
                Event::new(EV_ABS, ABS_Y, y0),
                Event::new(EV_ABS, ABS_TILT_X, TILT_X),
                Event::new(EV_ABS, ABS_TILT_Y, TILT_Y),
            ])?;
            thread::sleep(SETTLE_HOVER);

            for &(px, py) in &path[1..] {
                let (wx, wy) = screen_to_wacom(px, py);
                self.frame(&[
                    Event::new(EV_ABS, ABS_X, wx),
                    Event::new(EV_ABS, ABS_Y, wy),
                    Event::new(EV_ABS, ABS_PRESSURE, pressure),
                    Event::new(EV_ABS, ABS_TILT_X, TILT_X),
                    Event::new(EV_ABS, ABS_TILT_Y, TILT_Y),
                ])?;
                thread::sleep(pace);
            }

            // Lift off, and stay lifted long enough to be seen.
            self.frame(&[
                Event::new(EV_ABS, ABS_PRESSURE, 0),
                Event::new(EV_KEY, BTN_TOUCH, 0),
                Event::new(EV_ABS, ABS_DISTANCE, HOVER_DISTANCE),
            ])?;
            thread::sleep(SETTLE_LIFT);
        }

        // Leave proximity entirely.
        self.frame(&[Event::new(EV_KEY, tool, 0)])?;
        thread::sleep(SETTLE_TOOL);
        Ok(())
    }

    /// Write ink: used for the diary's reply and test patterns.
    pub fn write_polylines(&mut self, polys: &[Vec<(f32, f32)>], pace: Duration) -> Result<()> {
        self.trace(polys, BTN_TOOL_PEN, PRESSURE_WRITE, pace, STEP_PX)
    }

    pub fn tap_screen(&mut self, at: (f32, f32)) -> Result<()> {
        self.trace(
            &[vec![at]],
            BTN_TOOL_PEN,
            PRESSURE_WRITE,
            Duration::ZERO,
            STEP_PX,
        )
    }

    /// Unwrite ink along the given screen-space polylines. Every stroke is
    /// retraced once, start to end, in the order it was written; strokes
    /// that sit close together (the letters of a word) chain into a single
    /// unbroken glide, and the whole page shares one tool session — so the
    /// ink vanishes with the same fluency it appeared with.
    pub fn erase_polylines(&mut self, polys: &[Vec<(f32, f32)>], pace: Duration) -> Result<()> {
        const ERASE_STEP_PX: f32 = 9.0;
        const EXTEND_PX: f32 = 14.0;
        const RESAMPLE_PX: f32 = 3.0;
        // Amplitude stays *within* the eraser's radius: the tip is never
        // farther from the written path than the rubber reaches, so tight
        // curve apexes (letter tops and bottoms) cannot slip through the
        // weave the way a wider swing lets them.
        const AMPLITUDE_PX: f32 = 8.0;
        const WAVELENGTH_PX: f32 = 14.0;
        /// A single-point tap (i-dot, period) becomes a short dash.
        const DOT_HALF_PX: f32 = 5.0;
        /// Strokes whose gap is below this glide into each other pen-down;
        /// beyond it (word gaps, new lines) the eraser lifts like the quill.
        const MERGE_GAP_PX: f32 = 70.0;
        let vanish_pace = pace.max(Duration::from_millis(1));

        let mut paths: Vec<Vec<(f32, f32)>> = Vec::new();
        for poly in polys.iter().filter(|p| !p.is_empty()) {
            let base = if poly.len() == 1 {
                let (x, y) = poly[0];
                vec![(x - DOT_HALF_PX, y), (x + DOT_HALF_PX, y)]
            } else {
                crate::geom::resample(poly, RESAMPLE_PX)
            };
            // xochitl inks a little past the captured tips (the pressure
            // ramp around touch-down/lift), so overshoot both ends.
            let pts = extend_ends(&base, EXTEND_PX);
            if pts.len() < 2 {
                continue;
            }
            match paths.last_mut() {
                Some(prev) if dist(*prev.last().unwrap(), pts[0]) < MERGE_GAP_PX => {
                    prev.extend(pts);
                }
                _ => paths.push(pts),
            }
        }

        // Densify after merging so the sine is sampled finely everywhere —
        // fast pen segments and stroke-to-stroke transits alike.
        let passes: Vec<Vec<(f32, f32)>> = paths
            .iter()
            .map(|p| weave_polyline(&densify(p, RESAMPLE_PX), AMPLITUDE_PX, WAVELENGTH_PX))
            .collect();

        if passes.is_empty() {
            return Ok(());
        }
        self.trace(
            &passes,
            BTN_TOOL_RUBBER,
            PRESSURE_ERASE,
            vanish_pace,
            ERASE_STEP_PX,
        )
    }

    /// Wipe a rectangular region with one continuous serpentine sweep: a
    /// full-height eraser column glides from the left edge to the right,
    /// never lifting, so everything in the band vanishes behind a curtain
    /// moving steadily across the page.
    pub fn erase_sweep(&mut self, region: crate::geom::Rect) -> Result<()> {
        /// Adjacent columns overlap within the eraser's radius, so the
        /// sweep leaves no gaps between passes.
        const COLUMN_SPACING_PX: f32 = 11.0;
        const ERASE_STEP_PX: f32 = 12.0;
        /// The wipe front crosses the page at this constant speed whatever
        /// the height of the band — a three-line answer vanishes at the
        /// same visible pace as a one-line question. Point pacing is
        /// derived from it: taller columns get faster points.
        const FRONT_PX_PER_SEC: f32 = 420.0;

        if region.right <= region.left || region.bottom <= region.top {
            return Ok(());
        }
        let pts_per_col = ((region.bottom - region.top) / ERASE_STEP_PX).ceil().max(1.0);
        let col_secs = COLUMN_SPACING_PX / FRONT_PX_PER_SEC;
        let vanish_pace = Duration::from_secs_f32((col_secs / pts_per_col).clamp(0.0002, 0.005));
        let cols = ((region.right - region.left) / COLUMN_SPACING_PX).ceil() as usize;
        let mut path = Vec::with_capacity((cols + 1) * 2);
        for i in 0..=cols {
            let x = (region.left + i as f32 * COLUMN_SPACING_PX).min(region.right);
            if i % 2 == 0 {
                path.push((x, region.top));
                path.push((x, region.bottom));
            } else {
                path.push((x, region.bottom));
                path.push((x, region.top));
            }
        }
        self.trace(
            &[path],
            BTN_TOOL_RUBBER,
            PRESSURE_ERASE,
            vanish_pace,
            ERASE_STEP_PX,
        )
    }
}
