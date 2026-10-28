//! Writes an sRGB display profile whose `vcgt` tag carries the adjustment.
//! KWin applies `vcgt` to the final encoded output, so the curve is used as-is.

use std::path::Path;

use lcms2::{Profile, Tag, TagSignature, ToneCurve};

use crate::profiles::Settings;

// Same ranges and ramp as nvidia-settings' colour correction page.
pub const BRIGHTNESS_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;
pub const GAMMA_RANGE: std::ops::RangeInclusive<f32> = 0.1..=3.0;
pub const CONTRAST_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// Maps an encoded channel value in `[0, 1]` through the adjustment.
fn adjust(x: f32, s: &Settings) -> f32 {
    let c = s.contrast / 3.0;
    let c = if c > 0.0 { 0.5 / (0.5 - c) } else { (0.5 + c) / 0.5 };
    ((x - 0.5) * c + 0.5 + s.brightness / 3.0).clamp(0.0, 1.0).powf(1.0 / s.gamma)
}

pub fn write_profile(path: &Path, s: &Settings) -> std::io::Result<()> {
    let table: Vec<u16> = (0..1024)
        .map(|i| (adjust(i as f32 / 1023.0, s) * 65535.0).round() as u16)
        .collect();
    let curve = ToneCurve::new_tabulated(&table);
    let mut profile = Profile::new_srgb();
    profile.write_tag(TagSignature::VcgtTag, Tag::VcgtCurves([&curve, &curve, &curve]));
    profile.save_profile_to_file(path)
}
