//! Records the writer's pen strokes straight from the digitizer.
//!
//! Because we keep the exact paths that were written, we can later replay
//! them with the eraser tool (the vanishing) and render them to an image
//! for the vision model — no framebuffer access needed at all.

use crate::events::*;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct Point {
    /// Raw wacom coordinates.
    pub x: i32,
    pub y: i32,
    pub pressure: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Stroke {
    pub points: Vec<Point>,
}

pub struct CaptureState {
    pub strokes: Vec<Stroke>,
    current: Vec<Point>,
    pub last_pen_event: Instant,
    /// True while the pen (or its eraser end) is in hover range of the screen.
    pub pen_near: bool,
}

pub struct Shared {
    pub state: Mutex<CaptureState>,
    /// Set while we are injecting events ourselves, so the reader thread
    /// discards the echo of our own strokes.
    pub injecting: AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Shared {
            state: Mutex::new(CaptureState {
                strokes: Vec::new(),
                current: Vec::new(),
                last_pen_event: Instant::now(),
                pen_near: false,
            }),
            injecting: AtomicBool::new(false),
        }
    }

    pub fn take_strokes(&self) -> Vec<Stroke> {
        let mut st = self.state.lock().unwrap();
        st.current.clear();
        std::mem::take(&mut st.strokes)
    }

    pub fn has_ink(&self) -> bool {
        let st = self.state.lock().unwrap();
        !st.strokes.is_empty()
    }

    /// True while the writer's pen is at (or hovering near) the page, or
    /// was within the last moment. The invisible hand must stay out of
    /// the way: injecting while a real pen is down makes xochitl draw
    /// wild zigzags between the two positions.
    pub fn pen_busy(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.pen_near || st.last_pen_event.elapsed().as_secs_f32() < 0.3
    }

    /// True when there is ink and the pen has been fully away from the
    /// screen for `pause_secs`.
    pub fn writer_paused(&self, pause_secs: f32) -> bool {
        let st = self.state.lock().unwrap();
        !st.strokes.is_empty()
            && !st.pen_near
            && st.last_pen_event.elapsed().as_secs_f32() > pause_secs
    }
}

pub fn spawn_pen_reader(path: PathBuf, shared: Arc<Shared>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("pen reader: cannot open {}: {e}", path.display());
                return;
            }
        };
        let mut buf = [0u8; EVENT_SIZE];
        let mut cur = Point {
            x: 0,
            y: 0,
            pressure: 0,
        };
        let mut pen_down = false;
        let mut rubber = false;
        let mut have_pos = false;

        loop {
            if file.read_exact(&mut buf).is_err() {
                eprintln!("pen reader: device read failed, stopping");
                return;
            }
            let ev = Event::parse(&buf);

            // Drain but ignore the echo of our own injected events.
            if shared.injecting.load(Ordering::Relaxed) {
                continue;
            }

            match (ev.type_, ev.code) {
                (EV_ABS, ABS_X) => {
                    cur.x = ev.value;
                    have_pos = true;
                }
                (EV_ABS, ABS_Y) => {
                    cur.y = ev.value;
                    have_pos = true;
                }
                (EV_ABS, ABS_PRESSURE) => cur.pressure = ev.value,
                (EV_KEY, BTN_TOOL_PEN) | (EV_KEY, BTN_TOOL_RUBBER) => {
                    if ev.code == BTN_TOOL_RUBBER {
                        rubber = ev.value == 1;
                    }
                    let mut st = shared.state.lock().unwrap();
                    st.pen_near = ev.value == 1;
                    st.last_pen_event = Instant::now();
                }
                (EV_KEY, BTN_TOUCH) => {
                    let mut st = shared.state.lock().unwrap();
                    st.last_pen_event = Instant::now();
                    if ev.value == 1 {
                        pen_down = true;
                        st.current.clear();
                    } else {
                        pen_down = false;
                        // Even a single-point tap is real ink (i-dots,
                        // periods) — keep it, or it survives the vanishing.
                        if !st.current.is_empty() && !rubber {
                            let pts = std::mem::take(&mut st.current);
                            st.strokes.push(Stroke { points: pts });
                        } else {
                            st.current.clear();
                        }
                    }
                }
                (EV_SYN, SYN_REPORT) => {
                    if pen_down && have_pos && !rubber {
                        let mut st = shared.state.lock().unwrap();
                        st.current.push(cur);
                        st.last_pen_event = Instant::now();
                    }
                }
                (EV_ABS, ABS_DISTANCE) => {
                    // Hovering counts as activity: the writer is still there.
                    let mut st = shared.state.lock().unwrap();
                    st.last_pen_event = Instant::now();
                }
                _ => {}
            }
        }
    })
}
