//! Find the pen and touch input devices by name instead of hardcoding
//! /dev/input/eventN — the numbering can shift between firmware versions.
//! Names are read from sysfs, so no ioctls are needed.

use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Devices {
    pub pen: PathBuf,
    pub touch: PathBuf,
    pub pen_name: String,
    pub touch_name: String,
}

pub fn discover(pen_override: Option<&str>, touch_override: Option<&str>) -> Result<Devices> {
    let mut pen: Option<(PathBuf, String)> = None;
    let mut touch: Option<(PathBuf, String)> = None;

    for entry in fs::read_dir("/sys/class/input")? {
        let entry = entry?;
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("event") {
            continue;
        }
        let name = fs::read_to_string(entry.path().join("device/name"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let lower = name.to_lowercase();
        let dev_path = PathBuf::from("/dev/input").join(&fname);

        if lower.contains("wacom") {
            pen = Some((dev_path, name));
        } else if lower.contains("cyttsp") || lower.contains("pt_mt") || lower.contains("touch") {
            touch = Some((dev_path, name));
        }
    }

    if let Some(p) = pen_override {
        pen = Some((PathBuf::from(p), format!("(override) {p}")));
    }
    if let Some(t) = touch_override {
        touch = Some((PathBuf::from(t), format!("(override) {t}")));
    }

    match (pen, touch) {
        (Some((pen, pen_name)), Some((touch, touch_name))) => Ok(Devices {
            pen,
            touch,
            pen_name,
            touch_name,
        }),
        (pen, touch) => bail!(
            "could not find input devices (pen: {:?}, touch: {:?}). \
             Run `riddle debug-input` on the tablet, check /sys/class/input/event*/device/name, \
             and set pen_device / touch_device in riddle.toml",
            pen.map(|p| p.1),
            touch.map(|t| t.1)
        ),
    }
}
