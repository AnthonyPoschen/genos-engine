#![cfg_attr(
    test,
    expect(
        clippy::expect_used,
        reason = "a test aborts when a fixture fails"
    )
)]

//! Participating-medium settings.
//!
//! The froxel reads this value. The grid, the history slot, and the column
//! target are not settings and are not replicated. History stays off: the
//! pass does not blend a previous frame. Turning that blend on is an open
//! decision. `docs/systems/fog-and-volumetric-lighting-design.md`.

use std::fmt::Write as _;

use crate::EngineSettings;

/// A box or a sphere of analytic density. Volumes add to height fog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FogShape {
    /// Axis-aligned box. `extents` are the half-sizes in metres.
    Box,
    /// Sphere. The radius is `extents[0]`.
    Sphere,
}

impl FogShape {
    fn name(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Sphere => "sphere",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "box" => Some(Self::Box),
            "sphere" => Some(Self::Sphere),
            _ => None,
        }
    }
}

/// One local volume. Outside the bound the weight is zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogVolume {
    /// Box or sphere.
    pub shape: FogShape,
    /// World centre, metres.
    pub position: [f32; 3],
    /// Box half-extents, or the sphere radius in `x`.
    pub extents: [f32; 3],
    /// Extinction added inside the bound, per metre.
    pub density: f32,
    /// Single-scatter albedo, clamped to `[0, 1]`.
    pub albedo: f32,
    /// Henyey-Greenstein `g` for this volume.
    pub phase_g: f32,
    /// Metres. Weight is `distance-to-bound / falloff`, clamped to 1.
    /// Zero or negative is a hard bound (weight 1 inside).
    pub falloff: f32,
}

/// Fog and volumetric lighting. A public [`EngineSettings`] value on the scene.
///
/// Keys: `enabled`, `density`, `height`, `thickness`, `albedo`, `phase_g`,
/// `gi`, `history`. A `volume` line appends one local volume.
#[derive(Clone, Debug, PartialEq)]
pub struct FogSettings {
    /// Off skips the volume pass.
    pub enabled: bool,
    /// Height-fog extinction at `height`, per metre. Zero skips the pass.
    pub density: f32,
    /// World height where `density` is the extinction, metres.
    pub height: f32,
    /// Scale height, metres. A non-positive thickness keeps `density` flat.
    pub thickness: f32,
    /// Single-scatter albedo of the height field, clamped to `[0, 1]`.
    pub albedo: f32,
    /// Henyey-Greenstein `g` of the height field.
    pub phase_g: f32,
    /// Read the world cache when a later lighting phase provides it.
    /// The stub contributes nothing. Presets leave this off until that phase.
    pub gi: bool,
    /// Reserved. The pass does not blend history, whatever this flag says.
    pub history: bool,
    /// Local boxes and spheres. They add to the height field.
    pub volumes: Vec<FogVolume>,
}

/// `--fog` and the matching flags. `None` means the argument was omitted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FogCli {
    /// `--fog on|off`.
    pub enabled: Option<bool>,
    /// `--fog-density`.
    pub density: Option<f32>,
    /// `--fog-height`.
    pub height: Option<f32>,
    /// `--fog-thickness`.
    pub thickness: Option<f32>,
    /// `--fog-albedo`.
    pub albedo: Option<f32>,
    /// `--fog-phase`.
    pub phase_g: Option<f32>,
    /// `--fog-gi on|off`.
    pub gi: Option<bool>,
    /// `--fog-history on|off`.
    pub history: Option<bool>,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            density: 0.0,
            height: 0.0,
            thickness: 40.0,
            albedo: 1.0,
            phase_g: 0.4,
            gi: false,
            history: false,
            volumes: Vec::new(),
        }
    }
}

impl FogSettings {
    /// The pass runs only when it is enabled and the height density is positive
    /// or a local volume is present. A density of zero with no volumes does
    /// not dispatch.
    #[must_use]
    pub fn dispatches(&self) -> bool {
        self.enabled && (self.density > 0.0 || !self.volumes.is_empty())
    }

    /// One line per key. `volume` lines follow the scalars.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "enabled={}", on_off(self.enabled));
        let _ = writeln!(out, "density={}", num(self.density));
        let _ = writeln!(out, "height={}", num(self.height));
        let _ = writeln!(out, "thickness={}", num(self.thickness));
        let _ = writeln!(out, "albedo={}", num(self.albedo));
        let _ = writeln!(out, "phase_g={}", num(self.phase_g));
        let _ = writeln!(out, "gi={}", on_off(self.gi));
        let _ = writeln!(out, "history={}", on_off(self.history));
        for volume in &self.volumes {
            let _ = writeln!(
                out,
                "volume={} {} {} {} {} {} {} {} {} {} {}",
                volume.shape.name(),
                num(volume.position[0]),
                num(volume.position[1]),
                num(volume.position[2]),
                num(volume.extents[0]),
                num(volume.extents[1]),
                num(volume.extents[2]),
                num(volume.density),
                num(volume.albedo),
                num(volume.phase_g),
                num(volume.falloff),
            );
        }
        out
    }

    /// Parse the text form. Unknown keys fail.
    ///
    /// # Errors
    ///
    /// Returns an error on a bad line, an unknown key, or a value that does
    /// not match the key.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::default();
        settings.volumes.clear();
        for raw in text.split(['\n', ';']) {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("use key=value, got {line}"));
            };
            settings.set_key(key.trim(), value.trim())?;
        }
        Ok(settings)
    }

    /// Overlay one key. `volume` appends. `clear_volumes` drops the list.
    ///
    /// # Errors
    ///
    /// Returns an error on an unknown key or a value that does not match.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        match key {
            "enabled" => self.enabled = parse_on(value)?,
            "density" => self.density = parse_f32("density", value)?,
            "height" => self.height = parse_f32("height", value)?,
            "thickness" => self.thickness = parse_f32("thickness", value)?,
            "albedo" => self.albedo = parse_f32("albedo", value)?.clamp(0.0, 1.0),
            "phase_g" => self.phase_g = parse_f32("phase_g", value)?.clamp(-0.99, 0.99),
            "gi" => self.gi = parse_on(value)?,
            "history" => self.history = parse_on(value)?,
            "clear_volumes" => {
                if parse_on(value)? {
                    self.volumes.clear();
                }
            }
            "volume" => self.volumes.push(parse_volume(value)?),
            _ => return Err(format!("unknown fog key {key}")),
        }
        Ok(())
    }

    /// Arguments win. Environment text applies only for the flags the
    /// arguments left empty. Empty or unreadable environment text is ignored.
    #[must_use]
    pub fn resolve(&self, cli: &FogCli, env: &FogEnv) -> Self {
        let mut out = self.clone();
        out.enabled = cli.enabled.or(env.enabled).unwrap_or(out.enabled);
        out.density = cli.density.or(env.density).unwrap_or(out.density);
        out.height = cli.height.or(env.height).unwrap_or(out.height);
        out.thickness = cli.thickness.or(env.thickness).unwrap_or(out.thickness);
        out.albedo = cli.albedo.or(env.albedo).unwrap_or(out.albedo);
        out.phase_g = cli.phase_g.or(env.phase_g).unwrap_or(out.phase_g);
        out.gi = cli.gi.or(env.gi).unwrap_or(out.gi);
        out.history = cli.history.or(env.history).unwrap_or(out.history);
        out
    }
}

/// Environment mirrors, already parsed by the process edge.
///
/// A `None` field means the variable was omitted. The argument wins when
/// both are set; the caller leaves the env field empty in that case.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FogEnv {
    /// `GENOS_FOG`.
    pub enabled: Option<bool>,
    /// `GENOS_FOG_DENSITY`.
    pub density: Option<f32>,
    /// `GENOS_FOG_HEIGHT`.
    pub height: Option<f32>,
    /// `GENOS_FOG_THICKNESS`.
    pub thickness: Option<f32>,
    /// `GENOS_FOG_ALBEDO`.
    pub albedo: Option<f32>,
    /// `GENOS_FOG_PHASE`.
    pub phase_g: Option<f32>,
    /// `GENOS_FOG_GI`.
    pub gi: Option<bool>,
    /// `GENOS_FOG_HISTORY`.
    pub history: Option<bool>,
}

impl EngineSettings for FogSettings {
    fn to_text(&self) -> String {
        Self::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        Self::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        Self::set_key(self, key, value)
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn num(value: f32) -> String {
    let text = format!("{value:.6}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_on(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" => Ok(true),
        "0" | "off" | "false" | "no" => Ok(false),
        _ => Err(format!("expected on or off, got {value}")),
    }
}

fn parse_f32(key: &str, value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("{key} is a number, got {value}"))
}

fn parse_volume(value: &str) -> Result<FogVolume, String> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 11 {
        return Err(format!(
            "volume is shape x y z ex ey ez density albedo phase_g falloff, got {value}"
        ));
    }
    let shape = FogShape::parse(parts[0])
        .ok_or_else(|| format!("volume shape is box or sphere, got {}", parts[0]))?;
    let mut numbers = [0.0_f32; 10];
    for (index, part) in parts.iter().skip(1).enumerate() {
        numbers[index] = parse_f32("volume", part)?;
    }
    Ok(FogVolume {
        shape,
        position: [numbers[0], numbers[1], numbers[2]],
        extents: [numbers[3], numbers[4], numbers[5]],
        density: numbers[6],
        albedo: numbers[7].clamp(0.0, 1.0),
        phase_g: numbers[8].clamp(-0.99, 0.99),
        falloff: numbers[9],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_form_round_trips() {
        let mut settings = FogSettings {
            enabled: true,
            density: 0.08,
            height: 1.5,
            thickness: 40.0,
            albedo: 0.9,
            phase_g: 0.4,
            gi: false,
            history: false,
            volumes: vec![FogVolume {
                shape: FogShape::Box,
                position: [0.0, 1.0, -5.0],
                extents: [1.0, 2.0, 1.5],
                density: 0.5,
                albedo: 1.0,
                phase_g: 0.0,
                falloff: 0.25,
            }],
        };
        let parsed = FogSettings::parse(&settings.to_text()).expect("parse");
        assert_eq!(parsed, settings);
        settings.set_key("history", "on").expect("history");
        assert!(settings.history);
        let resolved = FogSettings::default().resolve(
            &FogCli {
                density: Some(0.2),
                ..FogCli::default()
            },
            &FogEnv {
                density: Some(9.0),
                enabled: Some(true),
                ..FogEnv::default()
            },
        );
        assert!(resolved.enabled);
        assert!((resolved.density - 0.2).abs() < 1.0e-6);
    }
}
