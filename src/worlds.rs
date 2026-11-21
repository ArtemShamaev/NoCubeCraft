//! Worlds on disk: each one is a folder with its description, state and terrain edits.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

const INFO_FILE: &str = "world.ron";

/// `$XDG_DATA_HOME/nocubecraft` and its equivalents; `NOCUBECRAFT_DATA` overrides it.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NOCUBECRAFT_DATA") {
        return dir.into();
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("nocubecraft");
    // Worlds from before the game was renamed come along.
    let old = base.join("barnocraft");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}

fn worlds_dir() -> PathBuf {
    data_dir().join("worlds")
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WorldInfo {
    pub name: String,
    pub seed: u32,
    pub creative: bool,
    /// Seconds since the Unix epoch.
    pub last_played: u64,
}

/// The world being played, set before entering the game.
#[derive(Resource, Clone)]
pub struct CurrentWorld {
    pub dir: PathBuf,
    pub info: WorldInfo,
}

impl CurrentWorld {
    /// Records the time and mode of play.
    pub fn touch(&mut self) {
        self.info.last_played = now();
        if let Err(e) = write_info(&self.dir, &self.info) {
            error!("could not update {}: {e}", self.dir.display());
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Saved worlds, most recently played first.
pub fn list() -> Vec<CurrentWorld> {
    let Ok(entries) = fs::read_dir(worlds_dir()) else { return Vec::new() };
    let mut worlds: Vec<CurrentWorld> = entries
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let text = fs::read_to_string(dir.join(INFO_FILE)).ok()?;
            let info = ron::from_str(&text).ok()?;
            Some(CurrentWorld { dir, info })
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.info.last_played));
    worlds
}

/// Makes a folder for a new world. An empty seed text picks one at random;
/// numbers are used as they are, anything else is hashed.
pub fn create(name: &str, seed: &str, creative: bool) -> std::io::Result<CurrentWorld> {
    let name = match name.trim() {
        "" => "Новый мир".to_string(),
        n => n.to_string(),
    };
    let seed = parse_seed(seed);
    let slug: String = name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let mut dir = worlds_dir().join(&slug);
    let mut n = 2;
    while dir.exists() {
        dir = worlds_dir().join(format!("{slug}_{n}"));
        n += 1;
    }
    fs::create_dir_all(&dir)?;
    let info = WorldInfo { name, seed, creative, last_played: now() };
    write_info(&dir, &info)?;
    Ok(CurrentWorld { dir, info })
}

pub fn delete(dir: &Path) -> std::io::Result<()> {
    // Only ever remove folders inside our own worlds directory.
    if dir.parent() != Some(worlds_dir().as_path()) {
        return Err(std::io::Error::other("not a world folder"));
    }
    fs::remove_dir_all(dir)
}

fn write_info(dir: &Path, info: &WorldInfo) -> std::io::Result<()> {
    let text = ron::ser::to_string_pretty(info, ron::ser::PrettyConfig::default()).map_err(std::io::Error::other)?;
    fs::write(dir.join(INFO_FILE), text)
}

pub fn parse_seed(text: &str) -> u32 {
    let text = text.trim();
    if text.is_empty() {
        return (now() as u32).wrapping_mul(2_654_435_761) ^ std::process::id();
    }
    text.parse::<u32>().unwrap_or_else(|_| {
        // FNV-1a, so the same word always gives the same world.
        text.bytes().fold(2_166_136_261u32, |h, b| (h ^ b as u32).wrapping_mul(16_777_619))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_from_text_are_stable() {
        assert_eq!(parse_seed("1337"), 1337);
        assert_eq!(parse_seed(" горы "), parse_seed("горы"));
        assert_ne!(parse_seed("горы"), parse_seed("море"));
    }
}
