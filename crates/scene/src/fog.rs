#![cfg_attr(
    test,
    expect(
        clippy::expect_used,
        reason = "a test aborts when a fixture fails"
    )
)]

//! Participating-medium settings.
//!
//! The froxel reads this value. Grid width, height, slices, and the sample
//! count are settings. Zero on each one keeps the lighting preset's full grid.
//! The history slot and the column target are not settings. History stays off:
//! the pass does not blend a previous frame. Turning that blend on is an open
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

/// Which froxel field the lighting debug page shows.
///
/// The name is the menu key. The pass does not read it, and the text form
/// does not store it, so it is not replicated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FogInspect {
    /// Extinction in the cell.
    #[default]
    Extinction,
    /// In-scattering in the cell.
    Inscatter,
    /// Transmittance along the column.
    Transmittance,
    /// History accept weight. Zero while history is off.
    History,
    /// Frames since the last history accept. Zero while history is off.
    Age,
    /// Samples, slices, and bytes the preset keeps.
    Budget,
    /// Local volume bounds.
    Volumes,
}

impl FogInspect {
    /// Menu keys, in slider order.
    pub const NAMES: [&str; 7] = [
        "extinction",
        "inscatter",
        "transmittance",
        "history",
        "age",
        "budget",
        "volumes",
    ];

    /// The menu key.
    #[must_use]
    pub fn name(self) -> &'static str {
        Self::NAMES
            .get(self.index())
            .copied()
            .unwrap_or("extinction")
    }

    /// Slider index of `name`.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Extinction => 0,
            Self::Inscatter => 1,
            Self::Transmittance => 2,
            Self::History => 3,
            Self::Age => 4,
            Self::Budget => 5,
            Self::Volumes => 6,
        }
    }

    /// Parse a menu key.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "extinction" => Some(Self::Extinction),
            "inscatter" => Some(Self::Inscatter),
            "transmittance" => Some(Self::Transmittance),
            "history" => Some(Self::History),
            "age" => Some(Self::Age),
            "budget" => Some(Self::Budget),
            "volumes" => Some(Self::Volumes),
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
/// `gi`, `history`, `grid_width`, `grid_height`, `grid_slices`, `samples`.
/// A `volume` line appends one local volume. Zero grid axes and a zero sample
/// count keep the preset grid, dispatched in full.
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
    /// Froxel columns. Zero keeps the lighting preset.
    pub grid_width: u32,
    /// Froxel rows. Zero keeps the lighting preset.
    pub grid_height: u32,
    /// Froxel slices. Zero keeps the lighting preset.
    pub grid_slices: u32,
    /// In-scatter sample budget. Zero dispatches the whole grid. A later
    /// auto-tune writes this; a positive value limits the dispatched cells.
    pub samples: u32,
    /// Debug page. Not part of the replicated text.
    pub inspect: FogInspect,
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
    /// `--fog-grid WxHxS`. Zero on an axis is not used here; omit the flag.
    pub grid: Option<(u32, u32, u32)>,
    /// `--fog-samples`.
    pub samples: Option<u32>,
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
            grid_width: 0,
            grid_height: 0,
            grid_slices: 0,
            samples: 0,
            inspect: FogInspect::Extinction,
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
        let _ = writeln!(out, "grid_width={}", self.grid_width);
        let _ = writeln!(out, "grid_height={}", self.grid_height);
        let _ = writeln!(out, "grid_slices={}", self.grid_slices);
        let _ = writeln!(out, "samples={}", self.samples);
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
            "grid" => {
                let (width, height, slices) = parse_grid(value)?;
                self.grid_width = width;
                self.grid_height = height;
                self.grid_slices = slices;
            }
            "grid_width" => self.grid_width = parse_u32("grid_width", value)?,
            "grid_height" => self.grid_height = parse_u32("grid_height", value)?,
            "grid_slices" => self.grid_slices = parse_u32("grid_slices", value)?,
            "samples" => self.samples = parse_u32("samples", value)?,
            "view" => {
                self.inspect = FogInspect::parse(value)
                    .ok_or_else(|| format!("view is one of the fog page keys, got {value}"))?;
            }
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
        out.albedo = cli
            .albedo
            .or(env.albedo)
            .unwrap_or(out.albedo)
            .clamp(0.0, 1.0);
        out.phase_g = cli
            .phase_g
            .or(env.phase_g)
            .unwrap_or(out.phase_g)
            .clamp(-0.99, 0.99);
        out.gi = cli.gi.or(env.gi).unwrap_or(out.gi);
        out.history = cli.history.or(env.history).unwrap_or(out.history);
        if let Some((width, height, slices)) = cli.grid.or(env.grid) {
            out.grid_width = width;
            out.grid_height = height;
            out.grid_slices = slices;
        }
        out.samples = cli.samples.or(env.samples).unwrap_or(out.samples);
        out
    }

    /// One line for the open debug view. Numbers and keys only.
    #[must_use]
    pub fn inspect_line(&self) -> String {
        match self.inspect {
            FogInspect::Extinction => format!(
                "density={} height={} thickness={}",
                num(self.density),
                num(self.height),
                num(self.thickness)
            ),
            FogInspect::Inscatter => {
                format!("albedo={} phase_g={}", num(self.albedo), num(self.phase_g))
            }
            FogInspect::Transmittance => {
                if self.dispatches() {
                    "dispatches=1".to_string()
                } else {
                    "dispatches=0".to_string()
                }
            }
            FogInspect::History => "weight=0".to_string(),
            FogInspect::Age => "age=0".to_string(),
            FogInspect::Budget => "samples capped=0 history=0".to_string(),
            FogInspect::Volumes => format!("volumes={}", self.volumes.len()),
        }
    }
}

/// `Some(true)` when `arg` is a fog flag that reads the next argument.
/// `Some(false)` when the value is after `=`. `None` for any other flag.
#[must_use]
pub fn fog_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, inline) = fog_dash(arg)?;
    fog_key(name)?;
    Some(inline.is_none())
}

/// Write one `--fog*` flag into `cli`.
///
/// # Errors
///
/// Returns an error when `arg` is a fog flag with a missing or bad value,
/// or when `arg` is not a fog flag.
pub fn apply_fog_arg(cli: &mut FogCli, arg: &str, next: Option<&str>) -> Result<(), String> {
    let (name, inline) = fog_dash(arg).ok_or_else(|| format!("unknown argument {arg}"))?;
    let key = fog_key(name).ok_or_else(|| format!("unknown argument {arg}"))?;
    let value = match inline {
        Some(value) => value,
        None => next.ok_or_else(|| format!("--{name} needs a value"))?,
    };
    let bad = |err: String| format!("bad --{name}: {err}");
    match key {
        "enabled" => cli.enabled = Some(parse_on(value).map_err(bad)?),
        "density" => cli.density = Some(parse_f32(key, value).map_err(bad)?),
        "height" => cli.height = Some(parse_f32(key, value).map_err(bad)?),
        "thickness" => cli.thickness = Some(parse_f32(key, value).map_err(bad)?),
        "albedo" => cli.albedo = Some(parse_f32(key, value).map_err(bad)?),
        "phase_g" => cli.phase_g = Some(parse_f32(key, value).map_err(bad)?),
        "gi" => cli.gi = Some(parse_on(value).map_err(bad)?),
        "history" => cli.history = Some(parse_on(value).map_err(bad)?),
        "grid" => cli.grid = Some(parse_grid(value).map_err(bad)?),
        "samples" => cli.samples = Some(parse_u32(key, value).map_err(bad)?),
        _ => return Err(format!("unknown argument {arg}")),
    }
    Ok(())
}

/// Reject a bad fog argument. `next` is the following argument when
/// [`fog_flag_needs_value`] is `Some(true)`.
///
/// # Errors
///
/// Returns an error on an unknown flag or a value that does not match.
pub fn check_fog_arg(arg: &str, next: Option<&str>) -> Result<(), String> {
    apply_fog_arg(&mut FogCli::default(), arg, next)
}

/// `on` / `off` as the process edge parsed it.
///
/// # Errors
///
/// Returns an error when `value` is not a switch.
pub fn parse_fog_switch(value: &str) -> Result<bool, String> {
    parse_on(value)
}

/// A fog number as the process edge parsed it.
///
/// # Errors
///
/// Returns an error when `value` is not a number.
pub fn parse_fog_number(key: &str, value: &str) -> Result<f32, String> {
    parse_f32(key, value)
}

/// `--fog-grid` as three cell counts.
///
/// # Errors
///
/// Returns an error when `value` is not `WxHxS`.
pub fn parse_fog_grid(value: &str) -> Result<(u32, u32, u32), String> {
    parse_grid(value)
}

/// A fog count as the process edge parsed it.
///
/// # Errors
///
/// Returns an error when `value` is not a whole number.
pub fn parse_fog_count(key: &str, value: &str) -> Result<u32, String> {
    parse_u32(key, value)
}

fn fog_dash(arg: &str) -> Option<(&str, Option<&str>)> {
    let rest = arg.strip_prefix("--")?;
    if let Some((name, value)) = rest.split_once('=') {
        Some((name, Some(value)))
    } else {
        Some((rest, None))
    }
}

fn fog_key(name: &str) -> Option<&'static str> {
    match name {
        "fog" => Some("enabled"),
        "fog-density" => Some("density"),
        "fog-height" => Some("height"),
        "fog-thickness" => Some("thickness"),
        "fog-albedo" => Some("albedo"),
        "fog-phase" => Some("phase_g"),
        "fog-gi" => Some("gi"),
        "fog-history" => Some("history"),
        "fog-grid" => Some("grid"),
        "fog-samples" => Some("samples"),
        _ => None,
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
    /// `GENOS_FOG_GRID`.
    pub grid: Option<(u32, u32, u32)>,
    /// `GENOS_FOG_SAMPLES`.
    pub samples: Option<u32>,
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

fn parse_u32(key: &str, value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|_| format!("{key} is a whole number, got {value}"))
}

fn parse_grid(value: &str) -> Result<(u32, u32, u32), String> {
    let parts: Vec<&str> = value
        .split(['x', 'X', ','])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 3 {
        return Err(format!("grid is WxHxS, got {value}"));
    }
    Ok((
        parse_u32("grid_width", parts[0])?,
        parse_u32("grid_height", parts[1])?,
        parse_u32("grid_slices", parts[2])?,
    ))
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
            grid_width: 0,
            grid_height: 0,
            grid_slices: 0,
            samples: 0,
            inspect: FogInspect::Extinction,
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
        settings.set_key("grid", "8x4x2").expect("grid");
        settings.set_key("samples", "1000").expect("samples");
        assert_eq!(settings.grid_width, 8);
        assert_eq!(settings.grid_height, 4);
        assert_eq!(settings.grid_slices, 2);
        assert_eq!(settings.samples, 1000);
    }

    #[test]
    fn the_argument_wins_and_a_bad_flag_fails() {
        let mut cli = FogCli::default();
        apply_fog_arg(&mut cli, "--fog", Some("on")).expect("fog");
        apply_fog_arg(&mut cli, "--fog-density=0.2", None).expect("density");
        apply_fog_arg(&mut cli, "--fog-grid", Some("16x8x4")).expect("grid");
        apply_fog_arg(&mut cli, "--fog-samples", Some("32")).expect("samples");
        let resolved = FogSettings::default().resolve(
            &cli,
            &FogEnv {
                density: Some(9.0),
                enabled: Some(false),
                grid: Some((1, 1, 1)),
                samples: Some(1),
                ..FogEnv::default()
            },
        );
        assert!(resolved.enabled);
        assert!((resolved.density - 0.2).abs() < 1.0e-6);
        assert_eq!(
            (resolved.grid_width, resolved.grid_height, resolved.grid_slices),
            (16, 8, 4)
        );
        assert_eq!(resolved.samples, 32);
        assert!(check_fog_arg("--fog", Some("maybe")).is_err());
        assert!(check_fog_arg("--not-fog", None).is_err());
        let mut page = FogSettings::default();
        page.set_key("view", "history").expect("view");
        assert_eq!(page.inspect_line(), "weight=0");
        assert!(!page.to_text().contains("view="));
        page.history = true;
        page.enabled = true;
        page.density = 0.08;
        assert_eq!(page.inspect, FogInspect::History);
        page.set_key("view", "transmittance").expect("view");
        assert_eq!(page.inspect_line(), "dispatches=1");
    }
}
