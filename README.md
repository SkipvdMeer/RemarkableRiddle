# riddle

*"My name is Tom Riddle."*

A [Chamber of Secrets diary](https://harrypotter.fandom.com/wiki/Tom_Riddle%27s_diary) for the
**reMarkable 2**. Write on the page with your marker and pause: your ink sinks into the paper,
the diary considers for a moment (three pulsing dots of ink), and an answer rises up through
the page in elegant cursive — written by no visible hand. It lingers just long enough to be
read, then fades away. The page is blank once more, but the diary remembers.

Inspired by [awwaiid/ghostwriter](https://github.com/awwaiid/ghostwriter), rebuilt around the
disappearing-ink séance and made robust for recent firmware (developed against **3.27.1.0**).

## How it works

Everything happens through the Linux input layer — no framebuffer hacks, no kernel modules,
no xochitl patching, which is what usually breaks on firmware updates:

- **Capture** — your pen strokes are recorded straight from the Wacom digitizer
  (`/dev/input/event*`, discovered by device name).
- **The vanishing** — your exact stroke paths are replayed with injected *eraser-tool* events
  (`BTN_TOOL_RUBBER`, the same thing the Marker Plus eraser end sends), in the order you wrote
  them. xochitl erases them like any eraser stroke — real ink, really gone.
- **The mind** — the recorded strokes are rendered to an image in-process and sent to an
  OpenAI vision model with a diary persona. The session transcript is kept, so it remembers
  the conversation.
- **The hand** — the reply is turned into pen paths using a Hershey single-stroke script font
  and injected as paced pen events: genuinely *written*, letter by letter, as notebook ink.

## Build (macOS)

```sh
brew install rustup zig cargo-zigbuild
rustup default stable
rustup target add armv7-unknown-linux-musleabihf
./deploy.sh                    # USB (10.11.99.1), or: ./deploy.sh root@<tablet-ip>
```

The SSH password is on the tablet under **Settings → Help → Copyrights and licenses**
(bottom of the page, "GPLv3 Compliance").

## API key

Copy `.env.example` to `.env` and put your OpenAI key in it. The file is gitignored;
`deploy.sh` installs it on the tablet (first deploy only), where the systemd service
loads it via `EnvironmentFile`. Never put the key in `riddle.toml` — that file is
committed as documentation.

## Run

On the tablet, with a notebook page open and a pen tool selected:

```sh
ssh root@<tablet-ip>
cd /home/root/riddle
export $(cat .env)   # or: export OPENAI_API_KEY=sk-...
./riddle
```

Write something. Lift the pen away. After ~3 s the séance begins. A finger tap in the
top-right corner sends immediately. `Ctrl-C` closes the diary.

Tune everything in [riddle.toml](riddle.toml) — trigger delay, corner, handwriting size,
how long answers linger, whether they fade at all (`fade_reply = false` keeps them as real
notes), writing speed, model.

The persona lives in [prompts/diary.txt](prompts/diary.txt) (baked into the binary; set
`prompt_path` in riddle.toml to use an edited copy on the tablet without rebuilding).

## First-time checks (in this order)

1. `./riddle test-draw` — draws a capital **F** top-left and *"the riddle"* mid-page.
   Rotated or mirrored? The transform in `src/geom.rs` needs flipping for your firmware.
2. `./riddle test-erase` — scribble, pause, watch it get unwritten. This proves the
   eraser-injection trick on your firmware before anything depends on it.
3. `./riddle test-script "I am the diary"` — the handwriting, written and faded.
4. `./riddle dry-run` — captures + consults the model but touches nothing: saves
   `riddle-page.png` and prints the transcription and reply.
5. `./riddle debug-input` — raw event dump, for calibrating if a future firmware moves things.

There is also a host-side preview (no tablet needed):
`cargo run -- preview "What is your name?" --out preview.png`

## Notes & limitations

- The tablet must be **awake, unlocked, on a notebook page**, and on Wi-Fi (for the API).
  Consider a longer auto-sleep timeout for long sessions (Settings → Battery).
- Ink you erase *by hand* mid-message isn't tracked; the vanishing may miss those bits.
- The quill writes plain ASCII; accented letters are folded (é→e) automatically.
- Nothing persistent is written on the tablet besides the binary and config; conversations
  live in memory and vanish when the diary closes. API calls go to OpenAI — don't write
  secrets to the diary that you wouldn't put in a prompt.
