use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub brightness: f32,
    pub gamma: f32,
    pub contrast: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { brightness: 0.0, gamma: 1.0, contrast: 0.0 }
    }
}

pub type Profiles = BTreeMap<String, Settings>;

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("gamma_control/profiles.json")
}

pub fn load() -> Profiles {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(profiles: &Profiles) -> anyhow::Result<()> {
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, serde_json::to_vec_pretty(profiles)?)?;
    Ok(())
}
