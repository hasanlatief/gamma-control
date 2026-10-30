use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::icc::{self, LEVELS, Levels, Sliders};

/// The control the user shaped the curve with.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Tab {
    Sliders,
    Levels,
    Draw,
}

/// Full editor state: profiles store it whole, and a revert restores the tab and its controls too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub tab: Tab,
    pub sliders: Sliders,
    pub levels: Levels,
    /// What gets applied; the slider tabs regenerate it every frame.
    pub curve: Vec<f32>,
}

impl Default for Settings {
    fn default() -> Self {
        let sliders = Sliders::default();
        Self { tab: Tab::Sliders, sliders, levels: Levels::default(), curve: icc::sliders_curve(&sliders) }
    }
}

pub type Profiles = BTreeMap<String, Settings>;

#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Current(Settings),
    // Profiles saved before the tab and curve were stored.
    Legacy(Sliders),
    Unknown(serde::de::IgnoredAny),
}

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("gamma_control/profiles.json")
}

/// Clamps hand-edited values into ranges the UI and curve maths can handle.
fn sanitize(mut s: Settings) -> Option<Settings> {
    s.curve = if s.curve.len() == LEVELS && s.curve.iter().all(|v| v.is_finite()) {
        s.curve.iter().map(|v| v.clamp(0.0, 1.0)).collect()
    } else {
        icc::resample(&s.curve)?
    };
    let sl = &mut s.sliders;
    sl.brightness = sl.brightness.clamp(*icc::BRIGHTNESS_RANGE.start(), *icc::BRIGHTNESS_RANGE.end());
    sl.gamma = sl.gamma.clamp(*icc::GAMMA_RANGE.start(), *icc::GAMMA_RANGE.end());
    sl.contrast = sl.contrast.clamp(*icc::CONTRAST_RANGE.start(), *icc::CONTRAST_RANGE.end());
    let l = s.levels;
    if !(0.0 <= l.black && l.black + 0.01 <= l.white && l.white <= 1.0) {
        s.levels = Levels::default();
    }
    Some(s)
}

/// Parses a profiles file, returning whether anything was not already in the current format.
fn parse(bytes: &[u8]) -> (Profiles, bool) {
    let stored: BTreeMap<String, Stored> = serde_json::from_slice(bytes).unwrap_or_default();
    let mut changed = false;
    let profiles = stored
        .into_iter()
        .filter_map(|(name, s)| {
            let settings = match s {
                Stored::Current(s) => {
                    let clean = sanitize(s.clone());
                    changed |= clean.as_ref() != Some(&s);
                    clean
                }
                Stored::Legacy(sliders) => {
                    changed = true;
                    sanitize(Settings { sliders, curve: icc::sliders_curve(&sliders), ..Settings::default() })
                }
                Stored::Unknown(_) => {
                    changed = true;
                    None
                }
            };
            Some((name, settings?))
        })
        .collect();
    (profiles, changed)
}

pub fn load() -> Profiles {
    let path = path();
    let Ok(bytes) = std::fs::read(&path) else { return Profiles::new() };
    let (profiles, changed) = parse(&bytes);
    if changed {
        // Keep the original in case the conversion dropped something.
        let _ = std::fs::write(path.with_extension("json.bak"), &bytes);
        let _ = save(&profiles);
    }
    profiles
}

pub fn save(profiles: &Profiles) -> anyhow::Result<()> {
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, serde_json::to_vec_pretty(profiles)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_legacy_profiles() {
        let (p, changed) = parse(br#"{"old": {"brightness": 0, "gamma": 1.5, "contrast": 0}, "bad": "x"}"#);
        assert!(changed);
        assert_eq!(p.len(), 1);
        let sliders = Sliders { gamma: 1.5, ..Sliders::default() };
        assert_eq!(p["old"].tab, Tab::Sliders);
        assert_eq!(p["old"].sliders, sliders);
        assert_eq!(p["old"].curve, icc::sliders_curve(&sliders));
        let (p2, changed) = parse(&serde_json::to_vec(&p).unwrap());
        assert!(!changed);
        assert_eq!(p2, p);
    }
}
