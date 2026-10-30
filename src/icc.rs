//! Writes an sRGB display profile whose `vcgt` tag carries the adjustment.
//! KWin applies `vcgt` to the final encoded output, so the curve is used as-is.

use std::path::Path;

use lcms2::{Profile, Tag, TagSignature, ToneCurve};
use serde::{Deserialize, Serialize};

/// Number of entries in every curve the app applies and saves.
pub const LEVELS: usize = 256;

// Same ranges and ramp as nvidia-settings' colour correction page.
pub const BRIGHTNESS_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;
pub const GAMMA_RANGE: std::ops::RangeInclusive<f32> = 0.1..=3.0;
pub const CONTRAST_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sliders {
    pub brightness: f32,
    pub gamma: f32,
    pub contrast: f32,
}

impl Default for Sliders {
    fn default() -> Self {
        Self { brightness: 0.0, gamma: 1.0, contrast: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    pub black: f32,
    pub white: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self { black: 0.0, white: 1.0 }
    }
}

fn sample(f: impl Fn(f32) -> f32) -> Vec<f32> {
    (0..LEVELS).map(|i| f(i as f32 / (LEVELS - 1) as f32)).collect()
}

pub fn sliders_curve(s: &Sliders) -> Vec<f32> {
    let c = s.contrast / 3.0;
    let c = if c > 0.0 { 0.5 / (0.5 - c) } else { (0.5 + c) / 0.5 };
    sample(|x| ((x - 0.5) * c + 0.5 + s.brightness / 3.0).clamp(0.0, 1.0).powf(1.0 / s.gamma))
}

pub fn levels_curve(l: &Levels) -> Vec<f32> {
    sample(|x| ((x - l.black) / (l.white - l.black).max(1e-6)).clamp(0.0, 1.0))
}

/// Linearly interpolates a curve of evenly spaced points at `x` in `[0, 1]`.
pub fn eval(curve: &[f32], x: f32) -> f32 {
    let pos = x.clamp(0.0, 1.0) * (curve.len() - 1) as f32;
    let i = (pos as usize).min(curve.len() - 2);
    curve[i] + (curve[i + 1] - curve[i]) * (pos - i as f32)
}

/// Validates a stored curve of any length and resamples it to `LEVELS` entries.
pub fn resample(curve: &[f32]) -> Option<Vec<f32>> {
    if curve.len() < 2 || curve.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(sample(|x| eval(curve, x).clamp(0.0, 1.0)))
}

pub fn write_profile(path: &Path, curve: &[f32]) -> std::io::Result<()> {
    let table: Vec<u16> = curve.iter().map(|v| (v * 65535.0).round() as u16).collect();
    let curve = ToneCurve::new_tabulated(&table);
    let mut profile = Profile::new_srgb();
    profile.write_tag(TagSignature::VcgtTag, Tag::VcgtCurves([&curve, &curve, &curve]));
    profile.save_profile_to_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn identity() {
        let id = resample(&[0.0, 1.0]).unwrap();
        assert_eq!(id.len(), LEVELS);
        assert!(sliders_curve(&Sliders::default()).iter().zip(&id).all(|(a, b)| close(*a, *b)));
        assert!(close(eval(&id, 0.3), 0.3));
    }

    #[test]
    fn levels() {
        let c = levels_curve(&Levels { black: 0.2, white: 0.6 });
        assert_eq!(eval(&c, 0.1), 0.0);
        assert!((eval(&c, 0.4) - 0.5).abs() < 0.01);
        assert_eq!(eval(&c, 0.8), 1.0);
    }

    #[test]
    fn resample_rejects_bad_curves() {
        assert!(resample(&[]).is_none());
        assert!(resample(&[0.5]).is_none());
        assert!(resample(&[0.0, f32::NAN]).is_none());
    }
}
