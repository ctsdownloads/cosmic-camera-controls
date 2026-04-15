use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level config: map of camera profile keys to their saved settings
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub cameras: HashMap<String, CameraProfile>,
}

/// Saved settings for a single camera
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraProfile {
    pub name: String,
    /// V4L2 control values keyed by control ID (as string for TOML)
    #[serde(default)]
    pub controls: HashMap<String, i64>,
    /// Saved format selection
    pub format: Option<SavedFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedFormat {
    pub fourcc: String,
    pub width: u32,
    pub height: u32,
    pub framerate_num: u32,
    pub framerate_den: u32,
}

impl Config {
    /// Load config from disk, returning default if missing or corrupt
    pub fn load() -> Self {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
                log::warn!("Failed to parse config at {}: {}", path.display(), e);
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Save config to disk
    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        let contents =
            toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize config: {}", e))?;
        fs::write(&path, contents).map_err(|e| format!("Failed to write config: {}", e))
    }

    /// Get saved profile for a camera by its identity key
    pub fn get_profile(&self, key: &str) -> Option<&CameraProfile> {
        self.cameras.get(key)
    }

    /// Save/update profile for a camera
    pub fn set_profile(&mut self, key: String, profile: CameraProfile) {
        self.cameras.insert(key, profile);
    }

}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("cosmic-camera-controls")
        .join("config.toml")
}
