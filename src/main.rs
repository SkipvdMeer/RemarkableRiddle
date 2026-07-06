//! riddle — a Chamber of Secrets diary for the reMarkable 2.
//!
//! Write on the page. Pause. The ink sinks into the paper, the diary
//! considers, and an answer rises up in handwriting — lingers — and fades.

mod capture;
mod config;
mod device;
mod engine;
mod events;
mod geom;
mod pen;
mod render;
mod script;
mod touch;
mod xochitl;

use anyhow::{anyhow, Result};
use capture::Shared;
use clap::{Parser, Subcommand};
use config::Config;
use engine::{Engine, Reply};
use geom::{polylines_bbox, Rect, SCREEN_H, SCREEN_W};
use pen::Pen;
use script::Script;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use touch::{Corner, TouchConfig};

type Polys = Vec<Vec<(f32, f32)>>;
const OFFLINE_REPLY: &str = "I need internet to help you further.";

#[derive(Parser)]
#[command(
    name = "riddle",
    version,
    about = "Tom Riddle's diary for the reMarkable 2"
)]
struct Cli {
    /// Path to riddle.toml (defaults to ./riddle.toml if present)
    #[arg(long)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// The full experience (default): listen, vanish, answer, fade.
    Run,
    /// Draw an orientation test pattern into the open notebook.
    TestDraw,
    /// Scribble something; after the pause it gets unwritten. Repeats.
    TestErase,
    /// Write the given text on the page in the diary's hand, then fade it.
    TestScript { text: String },
    /// Capture handwriting and consult the diary, but draw/erase nothing.
    /// Saves riddle-page.png and prints the transcription and reply.
    DryRun,
    /// Print raw pen and touch events (for calibrating a new firmware).
    DebugInput,
    /// Print the screen coordinates of every pen tap — tap toolbar buttons
    /// to find the right force_pen_tool tap positions for riddle.toml.
    Calibrate,
    /// (Host-side) Render text in the diary's hand to a PNG file.
    Preview {
        text: String,
        #[arg(long, default_value = "preview.png")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(cli.config.as_deref())?;
    match cli.cmd.unwrap_or(Cmd::Run) {
        Cmd::Run => run(cfg),
        Cmd::TestDraw => test_draw(cfg),
        Cmd::TestErase => test_erase(cfg),
        Cmd::TestScript { text } => test_script(cfg, &text),
        Cmd::DryRun => dry_run(cfg),
        Cmd::DebugInput => debug_input(cfg),
        Cmd::Calibrate => calibrate(cfg),
        Cmd::Preview { text, out } => preview(cfg, &text, &out),
    }
}

struct Session {
    shared: Arc<Shared>,
    pen: Pen,
    taps: Receiver<()>,
}

fn open_session(cfg: &Config) -> Result<Session> {
    let devs = device::discover(cfg.pen_device.as_deref(), cfg.touch_device.as_deref())?;
    eprintln!(
        "pen: {} ({}), touch: {} ({})",
        devs.pen.display(),
        devs.pen_name,
        devs.touch.display(),
        devs.touch_name
    );
    let shared = Arc::new(Shared::new());
    capture::spawn_pen_reader(devs.pen.clone(), shared.clone());
    let (tap_tx, tap_rx) = mpsc::channel();
    let touch_cfg = TouchConfig {
        corner: Corner::parse(&cfg.corner),
        corner_size: cfg.corner_size,
        invert_x: cfg.touch_invert_x,
        invert_y: cfg.touch_invert_y,
        ..TouchConfig::default()
    };
    touch::spawn_touch_reader(devs.touch.clone(), touch_cfg, tap_tx);
    let pen = Pen::open(&devs.pen, shared.clone())?;
    Ok(Session {
        shared,
        pen,
        taps: tap_rx,
    })
}

/// Wait until the writer is done: pen away for the pause, or a corner tap.
fn wait_for_page(session: &Session, cfg: &Config) {
    loop {
        thread::sleep(Duration::from_millis(100));
        let tapped = cfg.tap_trigger && session.taps.try_recv().is_ok();
        if session.shared.has_ink() && (tapped || session.shared.writer_paused(cfg.pause_secs)) {
            return;
        }
    }
}

fn run(cfg: Config) -> Result<()> {
    let script = Script::load()?;
    let mut engine = Engine::new(
        cfg.model.clone(),
        cfg.engine_base_url.clone(),
        cfg.api_key()?,
        cfg.system_prompt()?,
    );
    let mut session = open_session(&cfg)?;
    let write_pace = Duration::from_millis(cfg.write_pace_ms);
    let erase_pace = Duration::from_millis(cfg.erase_pace_ms);

    println!("The diary lies open, listening. Write, and pause…");

    loop {
        wait_for_page(&session, &cfg);
        let strokes = session.shared.take_strokes();
        if !xochitl::active_notebook_allowed(&cfg).unwrap_or(false) {
            println!(
                "✒ ink noticed, but the open notebook is not under the '{}' folder — ignoring.",
                cfg.allowed_folder
            );
            continue;
        }
        let polys = render::strokes_to_polys(&strokes);
        let user_bbox = match polylines_bbox(&polys) {
            Some(b) => b,
            None => continue,
        };
        println!("✒ {} strokes. The ink sinks into the page…", strokes.len());

        let png = render::polylines_to_png(&polys)?;

        // Consult the mind on a worker thread *before* the vanishing
        // starts: the diary thinks while the ink sinks, so the answer is
        // often ready the moment the page is blank.
        let (tx, rx) = mpsc::channel();
        {
            let eng = engine.clone();
            let png = png.clone();
            thread::spawn(move || {
                let _ = tx.send(eng.converse(&png));
            });
        }
        // The writer's words vanish behind a curtain sweeping left to
        // right; the wipe band reaches above and below the writing so
        // stray ascenders and descenders can't survive it.
        let wipe = wipe_region(user_bbox);
        println!(
            "  wiping x {:.0}..{:.0}, y {:.0}..{:.0}",
            wipe.left, wipe.right, wipe.top, wipe.bottom
        );
        session.pen.erase_sweep(wipe)?;
        let dots_at = (
            user_bbox.left.clamp(cfg.margin_px, SCREEN_W - 300.0),
            (user_bbox.top + 20.0).clamp(80.0, SCREEN_H - 160.0),
        );
        select_pen_tool(&mut session.pen, &cfg)?;
        let (result, leftover_dots) =
            think_with_dots(&mut session.pen, rx, dots_at, write_pace, erase_pace)?;
        if !leftover_dots.is_empty() {
            session.pen.erase_polylines(&leftover_dots, erase_pace)?;
        }

        let (reply, remember_reply) = match result {
            Ok(r) => (r, true),
            Err(e) => {
                eprintln!("the mind faltered: {e:#}");
                (
                    Reply {
                        transcription: String::new(),
                        reply: OFFLINE_REPLY.into(),
                    },
                    false,
                )
            }
        };
        println!("You wrote: {}", reply.transcription);
        println!("It answers: {}", reply.reply);

        // Lay the answer out where the writer's words used to be.
        let text = truncate_words(&tidy_reply(&reply.reply), 240);
        let placed = place_reply(&script, &text, &cfg, user_bbox);

        session.pen.write_polylines(&placed, write_pace)?;

        if remember_reply {
            engine.remember(
                if reply.transcription.is_empty() {
                    "(illegible)".into()
                } else {
                    reply.transcription.clone()
                },
                reply.reply.clone(),
            );
        }

        let linger = cfg.linger_base_secs + cfg.linger_per_char_secs * text.len() as f32;
        thread::sleep(Duration::from_secs_f32(linger));
        if cfg.fade_reply {
            // The answer fades exactly like the question: a full-height
            // curtain sweeping left to right over the padded reply area.
            if let Some(bbox) = polylines_bbox(&placed) {
                session.pen.erase_sweep(wipe_region(bbox))?;
            }
        }
        // Discard anything the pen reader picked up while we were busy.
        let _ = session.shared.take_strokes();
        while session.taps.try_recv().is_ok() {}
        println!("The page is blank once more.\n");
    }
}

/// Draw up to three slow "thinking" dots while the engine answers. Returns
/// the result and the dots on the page (so the caller can wipe them).
fn think_with_dots(
    pen: &mut Pen,
    rx: Receiver<Result<Reply>>,
    at: (f32, f32),
    write_pace: Duration,
    _erase_pace: Duration,
) -> Result<(Result<Reply>, Polys)> {
    let mut dots: Polys = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(1200)) {
            Ok(r) => return Ok((r, dots)),
            Err(RecvTimeoutError::Disconnected) => {
                return Ok((Err(anyhow!("the mind went silent")), dots))
            }
            Err(RecvTimeoutError::Timeout) => {
                if dots.len() < 3 {
                    let dot = dot_polyline(at.0 + dots.len() as f32 * 36.0, at.1);
                    pen.write_polylines(std::slice::from_ref(&dot), write_pace)?;
                    dots.push(dot);
                }
            }
        }
    }
}

fn dot_polyline(cx: f32, cy: f32) -> Vec<(f32, f32)> {
    let r = 4.0;
    (0..=8)
        .map(|i| {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// One flowing paragraph: newlines and runs of whitespace collapse to
/// single spaces, so the quill never writes ragged gaps mid-sentence.
fn tidy_reply(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_words(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut out = String::new();
    for word in text.split_whitespace() {
        if out.len() + word.len() + 4 > max_chars {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.push_str("...");
    out
}

fn select_pen_tool(pen: &mut Pen, cfg: &Config) -> Result<()> {
    if !cfg.force_pen_tool {
        return Ok(());
    }
    let delay = Duration::from_millis(cfg.tool_tap_delay_ms);
    pen.tap_screen((cfg.tool_menu_tap_x, cfg.tool_menu_tap_y))?;
    thread::sleep(delay);
    pen.tap_screen((cfg.pen_tool_tap_x, cfg.pen_tool_tap_y))?;
    thread::sleep(delay);
    Ok(())
}

fn place_reply(script: &Script, text: &str, cfg: &Config, user_bbox: Rect) -> Polys {
    const MIN_REPLY_WIDTH: f32 = 300.0;
    const TOP_SAFE_PX: f32 = 80.0;
    const BOTTOM_SAFE_PX: f32 = 120.0;

    let left_min = cfg.margin_px.min(SCREEN_W - MIN_REPLY_WIDTH);
    let left_max = (SCREEN_W - cfg.margin_px - MIN_REPLY_WIDTH).max(left_min);
    let left = user_bbox.left.clamp(left_min, left_max);
    let width = (SCREEN_W - left - cfg.margin_px).max(MIN_REPLY_WIDTH);

    let block = script.layout(text, cfg.reply_size_px, (left, 0.0), width);
    let Some(bbox) = polylines_bbox(&block) else {
        return block;
    };

    let bottom_limit = SCREEN_H - BOTTOM_SAFE_PX;
    let desired_top = user_bbox.top.clamp(TOP_SAFE_PX, bottom_limit);
    let mut dy = desired_top - bbox.top;
    let overflow = bbox.bottom + dy - bottom_limit;
    if overflow > 0.0 {
        dy -= overflow;
    }
    if bbox.top + dy < TOP_SAFE_PX {
        dy = TOP_SAFE_PX - bbox.top;
    }

    block
        .iter()
        .map(|p| p.iter().map(|&(x, y)| (x, y + dy)).collect())
        .collect()
}

fn test_draw(cfg: Config) -> Result<()> {
    let script = Script::load()?;
    let mut session = open_session(&cfg)?;
    let pace = Duration::from_millis(cfg.write_pace_ms);
    println!("Open a notebook page. Drawing the test pattern in 3 seconds…");
    thread::sleep(Duration::from_secs(3));

    // A capital F is asymmetric on both axes — instant orientation check.
    let mut polys: Polys = vec![
        vec![(200.0, 200.0), (200.0, 520.0)],
        vec![(200.0, 200.0), (400.0, 200.0)],
        vec![(200.0, 360.0), (340.0, 360.0)],
    ];
    polys.extend(script.layout("the riddle", 60.0, (450.0, 800.0), 800.0));
    select_pen_tool(&mut session.pen, &cfg)?;
    session.pen.write_polylines(&polys, pace)?;
    println!("Expected: a capital F near the TOP-LEFT, and 'the riddle' mid-page in cursive.");
    println!("If it is rotated or mirrored, the transform in src/geom.rs needs flipping.");
    Ok(())
}

/// The curtain reaches past the writing on every side, so ascenders,
/// descenders and pressure tails cannot survive the wipe.
fn wipe_region(bbox: Rect) -> Rect {
    Rect {
        left: (bbox.left - 150.0).max(0.0),
        top: (bbox.top - 170.0).max(0.0),
        right: (bbox.right + 150.0).min(SCREEN_W),
        bottom: (bbox.bottom + 170.0).min(SCREEN_H),
    }
}

fn test_erase(cfg: Config) -> Result<()> {
    let mut session = open_session(&cfg)?;
    println!(
        "Scribble on the page. {}s after you lift the pen away (or on a corner tap), it will be unwritten with the same left-to-right sweep the diary uses. Ctrl-C to stop.",
        cfg.pause_secs
    );
    loop {
        wait_for_page(&session, &cfg);
        let strokes = session.shared.take_strokes();
        let polys = render::strokes_to_polys(&strokes);
        let Some(bbox) = polylines_bbox(&polys) else {
            continue;
        };
        let wipe = wipe_region(bbox);
        println!(
            "✒ sweeping {} strokes: x {:.0}..{:.0}, y {:.0}..{:.0}",
            strokes.len(),
            wipe.left,
            wipe.right,
            wipe.top,
            wipe.bottom
        );
        session.pen.erase_sweep(wipe)?;
        println!("Gone. Again?");
    }
}

fn test_script(cfg: Config, text: &str) -> Result<()> {
    let script = Script::load()?;
    let mut session = open_session(&cfg)?;
    let write_pace = Duration::from_millis(cfg.write_pace_ms);
    println!("Writing in 3 seconds…");
    thread::sleep(Duration::from_secs(3));
    let width = SCREEN_W - 2.0 * cfg.margin_px;
    let polys = script.layout(text, cfg.reply_size_px, (cfg.margin_px, 200.0), width);
    select_pen_tool(&mut session.pen, &cfg)?;
    session.pen.write_polylines(&polys, write_pace)?;
    let linger = cfg.linger_base_secs + cfg.linger_per_char_secs * text.len() as f32;
    println!("Lingering {linger:.1}s, then fading…");
    thread::sleep(Duration::from_secs_f32(linger));
    if let Some(bbox) = polylines_bbox(&polys) {
        session.pen.erase_sweep(wipe_region(bbox))?;
    }
    println!("Faded.");
    Ok(())
}

fn dry_run(cfg: Config) -> Result<()> {
    let engine = Engine::new(
        cfg.model.clone(),
        cfg.engine_base_url.clone(),
        cfg.api_key()?,
        cfg.system_prompt()?,
    );
    let session = open_session(&cfg)?;
    println!("Dry run: write something and pause. Nothing will be drawn or erased.");
    loop {
        wait_for_page(&session, &cfg);
        let strokes = session.shared.take_strokes();
        let polys = render::strokes_to_polys(&strokes);
        let png = render::polylines_to_png(&polys)?;
        std::fs::write("riddle-page.png", &png)?;
        println!(
            "✒ {} strokes captured → riddle-page.png; consulting the diary…",
            strokes.len()
        );
        match engine.converse(&png) {
            Ok(r) => {
                println!("transcription: {}", r.transcription);
                println!("reply:         {}", r.reply);
            }
            Err(e) => eprintln!("engine error: {e:#}"),
        }
    }
}

fn calibrate(cfg: Config) -> Result<()> {
    let devs = device::discover(cfg.pen_device.as_deref(), cfg.touch_device.as_deref())?;
    println!("Tap the screen with the pen; every tap prints its screen position.");
    println!("For force_pen_tool: tap the toolbar's writing-tool icon, then the");
    println!("Ballpoint entry in the menu that opens. Ctrl-C to stop.");
    use std::io::Read;
    let mut f = std::fs::File::open(&devs.pen)?;
    let mut buf = [0u8; events::EVENT_SIZE];
    let (mut x, mut y) = (0i32, 0i32);
    let mut down = false;
    let mut announced = false;
    while f.read_exact(&mut buf).is_ok() {
        let ev = events::Event::parse(&buf);
        match (ev.type_, ev.code) {
            (events::EV_ABS, events::ABS_X) => x = ev.value,
            (events::EV_ABS, events::ABS_Y) => y = ev.value,
            (events::EV_KEY, events::BTN_TOUCH) => {
                down = ev.value == 1;
                announced = false;
            }
            (events::EV_SYN, _) if down && !announced && (x, y) != (0, 0) => {
                let (sx, sy) = geom::wacom_to_screen(x, y);
                println!("pen tap at screen ({sx:.0}, {sy:.0})");
                announced = true;
            }
            _ => {}
        }
    }
    Ok(())
}

fn debug_input(cfg: Config) -> Result<()> {
    let devs = device::discover(cfg.pen_device.as_deref(), cfg.touch_device.as_deref())?;
    println!("pen:   {} ({})", devs.pen.display(), devs.pen_name);
    println!("touch: {} ({})", devs.touch.display(), devs.touch_name);
    for (label, path) in [("PEN", devs.pen.clone()), ("TOUCH", devs.touch.clone())] {
        thread::spawn(move || {
            use std::io::Read;
            let mut f = std::fs::File::open(&path).expect("open device");
            let mut buf = [0u8; events::EVENT_SIZE];
            while f.read_exact(&mut buf).is_ok() {
                let ev = events::Event::parse(&buf);
                if ev.type_ != events::EV_SYN {
                    println!("[{label}] {}", events::describe(&ev));
                }
            }
        });
    }
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn preview(cfg: Config, text: &str, out: &PathBuf) -> Result<()> {
    let script = Script::load()?;
    let width = SCREEN_W - 2.0 * cfg.margin_px;
    let polys = script.layout(text, cfg.reply_size_px, (cfg.margin_px, 80.0), width);
    let png = render::polylines_to_png(&polys)?;
    std::fs::write(out, png)?;
    println!("wrote {}", out.display());
    Ok(())
}
