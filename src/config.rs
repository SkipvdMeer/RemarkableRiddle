use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// OpenAI model with vision support.
    pub model: String,
    pub engine_base_url: String,
    /// Falls back to the OPENAI_API_KEY environment variable.
    pub api_key: Option<String>,
    /// Override the built-in Tom Riddle persona with your own prompt file.
    pub prompt_path: Option<String>,

    /// Seconds the pen must stay away from the page before the ink sinks in.
    pub pause_secs: f32,
    /// Also allow a finger tap in a corner to trigger immediately.
    pub tap_trigger: bool,
    pub corner: String,
    /// Corner hot-zone size, as a fraction of the screen (0.12 = 12%).
    pub corner_size: f32,

    /// Handwriting size of the reply, in pixels (page is 1404 x 1872).
    pub reply_size_px: f32,
    /// Left/right page margin for the reply, in pixels.
    pub margin_px: f32,
    /// The reply lingers for base + per_char * len seconds, then fades.
    pub linger_base_secs: f32,
    pub linger_per_char_secs: f32,
    /// If false, the diary's answers stay on the page as real notes.
    pub fade_reply: bool,

    /// Tap xochitl's writing-tool menu before diary writing so replies use
    /// a normal pen even if the user had the highlighter selected.
    pub force_pen_tool: bool,
    pub tool_menu_tap_x: f32,
    pub tool_menu_tap_y: f32,
    pub pen_tool_tap_x: f32,
    pub pen_tool_tap_y: f32,
    pub tool_tap_delay_ms: u64,

    /// Restrict full diary mode to notebooks under this xochitl folder.
    /// Empty string disables the folder guard.
    pub allowed_folder: String,
    /// If true, notebooks in subfolders of allowed_folder also activate.
    pub allow_subfolders: bool,
    /// xochitl metadata directory used to identify the active notebook.
    pub metadata_dir: String,

    /// Milliseconds between injected points while writing (bigger = slower,
    /// more deliberate handwriting).
    pub write_pace_ms: u64,
    /// Milliseconds between injected points while erasing.
    pub erase_pace_ms: u64,

    /// Device overrides, e.g. "/dev/input/event1". Discovered by name if unset.
    pub pen_device: Option<String>,
    pub touch_device: Option<String>,
    pub touch_invert_x: bool,
    pub touch_invert_y: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            model: "gpt-4o-mini".into(),
            engine_base_url: "https://api.openai.com/v1".into(),
            api_key: None,
            prompt_path: None,
            pause_secs: 2.0,
            tap_trigger: true,
            corner: "top-right".into(),
            corner_size: 0.12,
            reply_size_px: 66.0,
            margin_px: 110.0,
            linger_base_secs: 5.0,
            linger_per_char_secs: 0.07,
            fade_reply: true,
            force_pen_tool: true,
            tool_menu_tap_x: 58.0,
            tool_menu_tap_y: 170.0,
            pen_tool_tap_x: 220.0,
            pen_tool_tap_y: 170.0,
            tool_tap_delay_ms: 180,
            allowed_folder: "Riddle".into(),
            allow_subfolders: true,
            metadata_dir: "/home/root/.local/share/remarkable/xochitl".into(),
            write_pace_ms: 0,
            erase_pace_ms: 1,
            pen_device: None,
            touch_device: None,
            touch_invert_x: false,
            touch_invert_y: true,
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let candidate = path.map(|p| p.to_path_buf()).or_else(|| {
            let default = Path::new("riddle.toml");
            default.exists().then(|| default.to_path_buf())
        });
        match candidate {
            Some(p) => {
                let text = std::fs::read_to_string(&p)
                    .with_context(|| format!("reading config {}", p.display()))?;
                toml::from_str(&text).with_context(|| format!("parsing config {}", p.display()))
            }
            None => Ok(Config::default()),
        }
    }

    pub fn api_key(&self) -> Result<String> {
        if let Some(k) = &self.api_key {
            return Ok(k.clone());
        }
        std::env::var("OPENAI_API_KEY")
            .context("no API key: set OPENAI_API_KEY or api_key in riddle.toml")
    }

    pub fn system_prompt(&self) -> Result<String> {
        match &self.prompt_path {
            Some(p) => std::fs::read_to_string(p).with_context(|| format!("reading prompt {p}")),
            None => Ok(include_str!("../prompts/diary.txt").to_string()),
        }
    }
}
