# riddle

*"My name is Tom Riddle."*

A [Chamber of Secrets diary](https://harrypotter.fandom.com/wiki/Tom_Riddle%27s_diary) for the
**reMarkable 2**. Write on the page with your marker and pause: your ink sinks into the paper
behind a curtain sweeping left to right, the diary considers for a moment (three pulsing dots
of ink), and an answer rises up through the page in joined-up cursive — written by no visible
hand. It lingers just long enough to be read, then fades the same way it came. The page is
blank once more, but the diary remembers.

Inspired by [awwaiid/ghostwriter](https://github.com/awwaiid/ghostwriter), rebuilt around the
disappearing-ink séance and made robust for recent firmware (developed against **3.27.1.0**).

---

## Contents

- [How it works](#how-it-works)
- [Requirements](#requirements)
- [Build & deploy](#build--deploy)
- [API key](#api-key)
- [Running](#running)
  - [As a permanent service](#as-a-permanent-service)
  - [By hand](#by-hand)
- [The séance, step by step](#the-séance-step-by-step)
- [Configuration reference](#configuration-reference)
- [Subcommands](#subcommands)
- [First-time checks](#first-time-checks)
- [The handwriting](#the-handwriting)
- [The persona](#the-persona)
- [Troubleshooting](#troubleshooting)
- [Architecture & source map](#architecture--source-map)
- [Security & privacy notes](#security--privacy-notes)

---

## How it works

Everything happens through the Linux input layer — no framebuffer hacks, no kernel modules,
no xochitl patching, which is what usually breaks on firmware updates:

- **Capture** — your pen strokes are recorded straight from the Wacom digitizer
  (`/dev/input/event*`, discovered by device name). Even single-point taps (i-dots,
  periods) are kept.
- **The vanishing** — an eraser tool (`BTN_TOOL_RUBBER`, the same thing the Marker Plus
  eraser end sends) is injected as one continuous serpentine stroke: a full-height column
  gliding left to right across a padded band around your writing. The wipe front moves at a
  constant speed regardless of how tall the text block is. Real ink, really gone.
- **The mind** — the recorded strokes are rendered to a PNG in-process (no framebuffer
  access needed) and sent to an OpenAI vision model wearing a diary persona. The session
  transcript is kept in memory, so the diary remembers the conversation until it closes.
- **The hand** — the reply is turned into pen paths using a Hershey single-stroke script
  font, with consecutive letters chained into continuous cursive runs, and injected as pen
  events: genuinely *written*, word by word, as notebook ink.

## Requirements

- A **reMarkable 2** on recent firmware (3.x), awake, unlocked, connected to Wi-Fi.
- SSH access to the tablet (see below), same network as your computer for Wi-Fi deploys.
- An **OpenAI API key** with access to a vision model (`gpt-4o-mini` by default).
- Build host: anything Rust runs on; the instructions below are for macOS.

## Build & deploy

```sh
brew install rustup zig cargo-zigbuild
rustup default stable
rustup target add armv7-unknown-linux-musleabihf
./deploy.sh                    # USB (10.11.99.1), or: ./deploy.sh root@<tablet-ip>
```

The SSH password is on the tablet under **Settings → Help → Copyrights and licenses**
(bottom of the page, "GPLv3 Compliance").

`deploy.sh` cross-compiles a static ARM binary (zig as the linker — no toolchain images
needed), copies it to `/home/root/riddle/`, and installs `riddle.toml` and `.env` **only if
the tablet doesn't have them yet** — your on-device tuning and key are never overwritten.

> If riddle is installed as a systemd service (see below), stop it before copying a new
> binary over an old one: `systemctl stop riddle`, deploy, `systemctl start riddle`.

## API key

Copy `.env.example` to `.env` and put your OpenAI key in it:

```sh
cp .env.example .env           # then edit: OPENAI_API_KEY=sk-...
```

`.env` is **gitignored** — it never enters version control. `deploy.sh` installs it on the
tablet (first deploy only), where the systemd service loads it via `EnvironmentFile`. For
manual runs, `export $(cat .env)` first. Never put the key in `riddle.toml`; that file is
committed as documentation.

## Running

### As a permanent service

The diary can live in the tablet permanently — always listening, starting at boot,
restarting itself if it crashes:

```ini
# /etc/systemd/system/riddle.service
[Unit]
Description=riddle - Tom Riddle's diary
After=xochitl.service

[Service]
WorkingDirectory=/home/root/riddle
EnvironmentFile=-/home/root/riddle/.env
ExecStart=/home/root/riddle/riddle
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

```sh
systemctl daemon-reload
systemctl enable --now riddle    # start now and at every boot
systemctl status riddle          # is it listening?
journalctl -u riddle -f          # watch the séance log live
```

**Never run `./riddle` by hand while the service is active** — two instances inject
interleaved pen events and everything breaks in confusing ways. `systemctl stop riddle`
first, then experiment, then `systemctl start riddle`.

Firmware updates replace the system partition: the binary, config, and `.env` survive
(they live in `/home/root`), but the service file must be reinstalled afterwards.

### By hand

On the tablet, with a notebook page open and a pen tool selected:

```sh
ssh root@<tablet-ip>
cd /home/root/riddle
export $(cat .env)
./riddle
```

Write something. Lift the pen away. After the configured pause the séance begins. A finger
tap in the top-right corner sends immediately. `Ctrl-C` closes the diary.

## The séance, step by step

1. You write. Strokes are captured; the diary waits until the pen has been fully away from
   the screen (out of hover range) for `pause_secs` — or a corner tap forces it.
2. Only notebooks inside the `allowed_folder` xochitl folder are animated; ink written
   anywhere else is noted in the log and politely ignored.
3. The captured page is rendered to a PNG and the OpenAI request departs **immediately**,
   in a worker thread — the model thinks while the ink vanishes.
4. The curtain wipe erases a band reaching well past your writing on all four sides
   (`wipe_region` in `src/main.rs`), so ascenders, descenders and stray dots go with it.
5. If the answer isn't back yet, up to three thinking-dots appear, one per 1.2 s.
6. The reply is written in cursive where your words used to be, word-wrapped to the page
   margins, shifted up if it would run off the bottom.
7. It lingers `linger_base_secs` (+ `linger_per_char_secs` per character), then fades
   behind the same curtain. The exchange joins the in-memory transcript, and the diary
   listens again.

If the API call fails (no Wi-Fi, quota), the diary writes a fallback line instead and the
failed exchange is not added to the transcript.

## Configuration reference

Everything lives in [`riddle.toml`](riddle.toml) next to the binary (the tablet's copy is
authoritative at runtime and is never overwritten by deploys). All keys are optional;
defaults shown. After changing it under the service: `systemctl restart riddle`.

| Key | Default | Meaning |
|---|---|---|
| `model` | `"gpt-4o-mini"` | OpenAI vision model. `gpt-4o` reads handwriting better, costs more. |
| `engine_base_url` | `https://api.openai.com/v1` | Any OpenAI-compatible endpoint. |
| `api_key` | *(unset)* | Prefer the `OPENAI_API_KEY` env var via `.env`. |
| `prompt_path` | *(unset)* | Override the built-in persona with a file on the tablet. |
| `pause_secs` | `2.0` | Pen fully away this long → the séance begins. |
| `tap_trigger` | `true` | Corner tap also triggers, immediately. |
| `corner` | `"top-right"` | Which corner: `top-right`, `top-left`, `bottom-right`, `bottom-left`. |
| `corner_size` | `0.12` | Corner hot-zone size, fraction of the screen. |
| `reply_size_px` | `66.0` | Handwriting size (page is 1404 × 1872 px). |
| `margin_px` | `110.0` | Left/right page margins for the reply. |
| `linger_base_secs` | `5.0` | The answer stays this long after the last word… |
| `linger_per_char_secs` | `0.0` | …plus this much per character (0 = flat time). |
| `fade_reply` | `true` | `false` keeps the diary's answers in the notebook as real notes. |
| `force_pen_tool` | `true` | Tap xochitl's toolbar before writing to select the ballpoint. **Calibrate first** (see below) — wrong coordinates hit other buttons. |
| `tool_menu_tap_x/y` | `58, 170` | Screen position of the toolbar's writing-tool icon. |
| `pen_tool_tap_x/y` | `220, 170` | Screen position of the Ballpoint entry in the opened menu. |
| `tool_tap_delay_ms` | `180` | Pause between the two taps. |
| `allowed_folder` | `"Riddle"` | Only notebooks under this xochitl folder are animated. `""` disables the guard. |
| `allow_subfolders` | `true` | Subfolders of `allowed_folder` count too. |
| `metadata_dir` | *(xochitl's)* | Where notebook metadata lives; only for exotic setups. |
| `write_pace_ms` | `0` | Milliseconds between pen points while writing (0 = fastest). |
| `erase_pace_ms` | `1` | Pacing for the precise stroke-eraser (thinking dots cleanup). The curtain paces itself. |
| `pen_device` / `touch_device` | *(auto)* | e.g. `/dev/input/event1`; discovered by name if unset. |
| `touch_invert_x/y` | `false` / `true` | Touch panel axis orientation (rM2: y is bottom-up). |

## Subcommands

| Command | What it does |
|---|---|
| `./riddle` | The full experience: listen, vanish, answer, fade. Repeats forever. |
| `./riddle test-draw` | Draws an orientation test pattern (a capital **F** and *"the riddle"*). |
| `./riddle test-erase` | Scribble → pause → watch the curtain wipe it. Tests the real vanish. |
| `./riddle test-script "text"` | Writes the text in the diary's hand, lingers, fades it. |
| `./riddle dry-run` | Captures and consults the model but draws/erases nothing; saves `riddle-page.png`, prints transcription and reply. |
| `./riddle calibrate` | Prints the screen coordinates of every pen tap — for finding the toolbar button positions for `force_pen_tool`. |
| `./riddle debug-input` | Raw pen/touch event dump, for calibrating a new firmware. |
| `cargo run -- preview "text" --out p.png` | **Host-side**: renders the handwriting to a PNG. No tablet needed. |

## First-time checks

In this order — each proves one layer before the next depends on it:

1. `test-draw` — rotated or mirrored? The coordinate transform in `src/geom.rs` needs
   flipping for your firmware. This is the only file that should ever need it.
2. `test-erase` — proves eraser injection and the curtain sweep.
3. `test-script "I am the diary"` — the handwriting, written and faded.
4. `dry-run` — proves capture → render → API without touching the page.
5. If replies come out in the wrong tool (highlighter!), run `calibrate`, tap the toolbar's
   writing-tool icon and then the Ballpoint entry with the pen, put the two printed
   coordinate pairs in `riddle.toml`, and set `force_pen_tool = true`.

## The handwriting

The diary writes with a **Hershey single-stroke font** — pen *paths*, not outlines, which
is exactly what a moving pen tip produces. The active face is *Script Simplex*
(`assets/scripts.jhf`, baked into the binary); *Script Complex* (`assets/scriptc.jhf`) is
included as an alternative — swap the `include_str!` in `src/script.rs` and rebuild.

Letters whose strokes end and begin near the baseline are chained into a single pen-down
cursive run per word (`chain_cursive`), which both looks like joined-up handwriting and
writes much faster — pen lifts between letters are the slowest part of writing. Dots,
commas and other specks stay separate taps. Text is folded to ASCII first (é→e, ü→u,
"…"→"...") because the quill writes plain ASCII most beautifully.

## The persona

The diary's voice lives in [`prompts/diary.txt`](prompts/diary.txt): courteous, composed,
old-fashioned, benign. It answers **in the language you wrote** (Dutch stays Dutch), keeps
replies to ~35 words, never breaks character, and returns strict JSON
(`{"transcription", "reply"}`) — which the code parses defensively, salvaging the reply
even from truncated or fenced JSON rather than ever writing raw braces onto the page.

The prompt is baked into the binary; set `prompt_path` in `riddle.toml` to iterate on a
copy on the tablet without rebuilding.

## Troubleshooting

| Symptom | Likely cause & fix |
|---|---|
| Nothing happens when you write | Is the notebook inside the **Riddle** folder (`allowed_folder`)? The journal says `ink noticed, but…` when the guard rejects. Also: is the service actually running (`systemctl status riddle`)? |
| Nothing happens in a brand-new notebook | xochitl hasn't stamped its metadata yet. Write another line or flip the page once; reopening the notebook also fixes it. |
| Everything acts weird at once | Two riddle instances are injecting simultaneously. `pgrep riddle` should show exactly one PID. `systemctl stop riddle` + `killall riddle`, then start one. |
| Replies written with the wrong tool / in highlighter | `force_pen_tool` taps landing on the wrong buttons — run `./riddle calibrate` and fix the coordinates, or set `force_pen_tool = false` and keep the ballpoint selected yourself. |
| Erase visibly runs but ink comes back | A stray toolbar tap hit **undo**. Same fix as above. |
| Specks survive the wipe | Grow the paddings in `wipe_region` (`src/main.rs`) or tighten `COLUMN_SPACING_PX` in `src/pen.rs`. |
| Letters weld together into a scrawl | Pen lifts are being merged — raise `SETTLE_LIFT`/`SETTLE_HOVER` in `src/pen.rs` (they're at the ~1-frame floor). |
| `test-draw` comes out rotated/mirrored | Flip the transform in `src/geom.rs`. |
| Service gone after a firmware update | Reinstall `/etc/systemd/system/riddle.service` (binary/config/`.env` survive in `/home/root`). |
| "text file busy" when copying the binary | The old binary is running. Stop the service first, or copy to a temp name and `mv` over it. |

## Architecture & source map

```
src/
  main.rs      CLI, the séance loop, reply layout, wipe region, subcommands
  capture.rs   reads the digitizer; records strokes, detects the writing pause
  pen.rs       the invisible hand: injects pen/eraser events; curtain sweep lives here
  engine.rs    OpenAI chat call, transcript memory, defensive JSON reply parsing
  script.rs    Hershey font parsing, text layout, cursive letter-chaining
  render.rs    strokes → PNG for the vision model (tiny-skia, no framebuffer)
  geom.rs      screen ↔ wacom coordinate transforms, resample/densify helpers
  touch.rs     corner-tap detection on the touchscreen
  xochitl.rs   notebook-folder guard via xochitl's metadata files
  device.rs    input device discovery by name
  events.rs    Linux input_event encode/decode
assets/        Hershey JHF fonts (scripts = active, scriptc = alternative)
prompts/       the diary persona (baked in at compile time)
```

Design constraints worth knowing before hacking:

- xochitl consumes pen input at display-frame granularity (~60 Hz): tool changes,
  touch-downs and lifts need >1 frame of settle time or they merge (`SETTLE_*`).
- The eraser follows whatever xochitl considers the rubber tool; writing follows the
  **UI-selected** tool — that's why `force_pen_tool` exists at all.
- The curtain sweep paces itself from the band height so the front speed is constant
  (`FRONT_PX_PER_SEC`); don't reintroduce fixed per-point pacing there.
- Injected events echo back to our own capture reader; the `injecting` flag filters them.

## Security & privacy notes

- The API key lives in `.env` (gitignored) on your machine and `chmod 600` on the tablet.
  Don't commit it; rotate it if it ever leaks into a terminal log.
- Handwriting images and the conversation go to OpenAI — don't write secrets to the diary
  that you wouldn't put in a prompt.
- Conversations live in memory only and vanish when the diary closes; nothing persistent
  is written to the tablet besides the binary, config, and `.env`.
- Ink you erase *by hand* mid-message isn't tracked; the vanishing may miss those bits.
- The tablet must be awake, unlocked, on a notebook page, and on Wi-Fi. Consider a longer
  auto-sleep timeout for long sessions (Settings → Battery).
