use std::process::Command;

use anyhow::{Context, bail};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct ColorState {
    /// `sRGB`, `ICC` or `EDID`.
    pub source: String,
    pub icc_path: String,
}

#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub color: ColorState,
}

pub fn list_outputs() -> anyhow::Result<Vec<Output>> {
    let out = Command::new("kscreen-doctor").arg("-j").output().context("running kscreen-doctor")?;
    let json: Value = serde_json::from_slice(&out.stdout).context("parsing kscreen-doctor -j")?;
    // `kscreen-doctor -j` doesn't report the colour profile source; KWin's saved config does.
    let kwin: Value = dirs::config_dir()
        .and_then(|d| std::fs::read(d.join("kwinoutputconfig.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let kwin_outputs = kwin
        .as_array()
        .and_then(|s| s.iter().find(|s| s["name"] == "outputs"))
        .and_then(|s| s["data"].as_array().cloned())
        .unwrap_or_default();

    Ok(json["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["enabled"] == true)
        .map(|o| {
            let name = o["name"].as_str().unwrap_or_default().to_owned();
            let icc_path = o["iccProfilePath"].as_str().unwrap_or_default().to_owned();
            let source = kwin_outputs
                .iter()
                .find(|k| k["connectorName"] == name.as_str() && k["iccProfilePath"] == icc_path.as_str())
                .and_then(|k| k["colorProfileSource"].as_str())
                .unwrap_or(if icc_path.is_empty() { "sRGB" } else { "ICC" })
                .to_owned();
            Output { name, color: ColorState { source, icc_path } }
        })
        .collect())
}

pub fn set_color(output: &str, state: &ColorState) -> anyhow::Result<()> {
    let out = Command::new("kscreen-doctor")
        .arg(format!("output.{output}.iccprofile.{}", state.icc_path))
        .arg(format!("output.{output}.colorProfileSource.{}", state.source))
        .output()
        .context("running kscreen-doctor")?;
    if !out.status.success() {
        bail!("kscreen-doctor: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
