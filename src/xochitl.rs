use crate::config::Config;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(default)]
    parent: String,
    #[serde(default, rename = "type")]
    type_: String,
    #[serde(default, rename = "visibleName")]
    visible_name: String,
    #[serde(default, rename = "lastOpened")]
    last_opened: Option<String>,
    #[serde(default, rename = "lastModified")]
    last_modified: Option<String>,
}

#[derive(Debug)]
struct Entry {
    id: String,
    parent: String,
    type_: String,
    visible_name: String,
    last_opened: Option<u128>,
    last_modified: Option<u128>,
}

impl Entry {
    /// Freshly created notebooks carry `lastOpened = 0` until xochitl gets
    /// around to stamping them, while their `lastModified` is current — so
    /// recency has to consider both.
    fn recency(&self) -> u128 {
        self.last_opened
            .into_iter()
            .chain(self.last_modified)
            .max()
            .unwrap_or(0)
    }
}

pub fn active_notebook_allowed(cfg: &Config) -> Result<bool> {
    let allowed_folder = cfg.allowed_folder.trim();
    if allowed_folder.is_empty() {
        return Ok(true);
    }

    let entries = read_metadata(Path::new(&cfg.metadata_dir))?;
    let allowed_folders = resolve_folder_path(&entries, allowed_folder);
    if allowed_folders.is_empty() {
        return Ok(false);
    }

    let Some(active) = active_document(&entries) else {
        return Ok(false);
    };

    if cfg.allow_subfolders {
        Ok(has_ancestor(&entries, active, &allowed_folders))
    } else {
        Ok(allowed_folders.contains(&active.parent))
    }
}

fn read_metadata(dir: &Path) -> Result<HashMap<String, Entry>> {
    let mut entries = HashMap::new();
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("metadata") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        // A file xochitl is mid-write on (or a corrupt leftover) must not
        // take the whole guard down — skip it and judge from the rest.
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<Metadata>(&text) else {
            continue;
        };
        entries.insert(
            id.clone(),
            Entry {
                id,
                parent: meta.parent,
                type_: meta.type_,
                visible_name: meta.visible_name,
                last_opened: meta.last_opened.and_then(|v| v.parse().ok()),
                last_modified: meta.last_modified.and_then(|v| v.parse().ok()),
            },
        );
    }
    Ok(entries)
}

fn resolve_folder_path(entries: &HashMap<String, Entry>, path: &str) -> HashSet<String> {
    let mut parents = HashSet::from([String::new()]);
    for part in path.split('/').map(str::trim).filter(|p| !p.is_empty()) {
        let matches: HashSet<String> = entries
            .values()
            .filter(|entry| {
                entry.type_ == "CollectionType"
                    && entry.visible_name == part
                    && parents.contains(&entry.parent)
            })
            .map(|entry| entry.id.clone())
            .collect();
        if matches.is_empty() {
            return HashSet::new();
        }
        parents = matches;
    }
    parents
}

fn is_trashed(entries: &HashMap<String, Entry>, entry: &Entry) -> bool {
    let mut parent = entry.parent.as_str();
    while !parent.is_empty() {
        if parent == "trash" {
            return true;
        }
        let Some(e) = entries.get(parent) else {
            return false;
        };
        parent = e.parent.as_str();
    }
    false
}

fn active_document(entries: &HashMap<String, Entry>) -> Option<&Entry> {
    // Trashed documents keep their timestamps and would otherwise still
    // win: deleting a diary notebook and starting a fresh one must not
    // leave the guard pointing at the corpse in the trash.
    entries
        .values()
        .filter(|entry| entry.type_ == "DocumentType")
        .filter(|entry| !is_trashed(entries, entry))
        .filter(|entry| entry.recency() > 0)
        .max_by_key(|entry| entry.recency())
}

fn has_ancestor(
    entries: &HashMap<String, Entry>,
    document: &Entry,
    allowed_folders: &HashSet<String>,
) -> bool {
    let mut parent = document.parent.as_str();
    while !parent.is_empty() && parent != "trash" {
        if allowed_folders.contains(parent) {
            return true;
        }
        let Some(entry) = entries.get(parent) else {
            return false;
        };
        parent = entry.parent.as_str();
    }
    false
}
