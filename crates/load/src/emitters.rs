#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Promoted emitters and the closed-room furnace.
//!
//! A cache hit adds emission to the irradiance it reads. A region smaller than
//! one L0 cell, or brighter than the flux threshold, is promoted: the light
//! tree owns that flux, and the cache's direct term skips it. An indirect hit
//! reads the leftover. A camera on the wall reads the wall's full outgoing.
//! Potato promotes nothing.
//!
//! The GPU cache does not skip promoted flux yet. That skip needs
//! `crates/render/src/lighting_gpu.rs`, which the lighting branch owns.
//! Toggling promotion does not rebuild a mesh.
//!
//! `--material-emissive-promote` and `--material-emissive-promote-flux` win.
//! `GENOS_MATERIAL_EMISSIVE_PROMOTE` and `GENOS_MATERIAL_EMISSIVE_PROMOTE_FLUX`
//! apply only when that argument was omitted. The debug menu has no description
//! line. MCP and the menu wait on the lighting branch's command catalogue.

/// Edge of one L0 cache cell, in metres. Area under this cell can be promoted.
pub const CELL_EDGE_M: f32 = 0.25;

const STEPS: u32 = 48;

/// Which preset caps promotion. Potato promotes none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromotePreset {
    Potato,
    Low,
    Medium,
    High,
}

/// How many regions a mesh may promote, and the flux that promotes a large one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PromoteBudget {
    pub enabled: bool,
    pub max_regions: u32,
    /// Absolute flux. A region above this is promoted even when it is large.
    pub flux_threshold: f32,
}

impl PromoteBudget {
    /// `flux_fraction` is the share of the brightest lamp. The design default is 0.01.
    #[must_use]
    pub fn preset(preset: PromotePreset, brightest_lamp: f32, flux_fraction: f32) -> Self {
        let (enabled, max_regions) = match preset {
            PromotePreset::Potato => (false, 0),
            PromotePreset::Low => (true, 4),
            PromotePreset::Medium => (true, 8),
            PromotePreset::High => (true, 16),
        };
        Self {
            enabled,
            max_regions,
            flux_threshold: flux_fraction * brightest_lamp,
        }
    }
}

/// Promote when the budget allows and the region is smaller than one L0 cell
/// or brighter than the flux threshold.
#[must_use]
pub fn should_promote(area: f32, flux: f32, budget: PromoteBudget, already: u32) -> bool {
    if !budget.enabled || already >= budget.max_regions {
        return false;
    }
    let cell = CELL_EDGE_M * CELL_EDGE_M;
    area < cell || flux > budget.flux_threshold
}

/// One closed room. `strip_fraction` is the strip's share of the enclosure.
///
/// `cache_adds_promoted` is the double-count bug: the cache keeps the strip
/// emission after the light tree has taken it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enclosure {
    pub albedo: f32,
    pub emission: f32,
    pub strip_emission: f32,
    pub strip_fraction: f32,
    pub promoted: bool,
    pub cache_adds_promoted: bool,
}

/// Wall outgoing after a fixed radiosity iteration.
///
/// Nothing is promoted in the furnace: set `strip_fraction` to 0 and put `E`
/// in `emission`. The limit is `E / (1 - albedo)`.
#[must_use]
pub fn settled_wall(room: Enclosure) -> f32 {
    let rest = 1.0 - room.strip_fraction;
    let mut wall = room.emission;
    let mut reflected = 0.0_f32;
    for _ in 0..STEPS {
        let cache_emission = if room.promoted && !room.cache_adds_promoted {
            0.0
        } else {
            room.strip_emission
        };
        let direct = if room.promoted {
            room.strip_fraction * room.strip_emission
        } else {
            0.0
        };
        let irradiance = rest * wall + room.strip_fraction * (reflected + cache_emission) + direct;
        wall = room.emission + room.albedo * irradiance;
        reflected = room.albedo * irradiance;
    }
    wall
}

/// Closed room, albedo `albedo`, emission `emission`, occlusion off, nothing promoted.
#[must_use]
pub fn furnace_wall(albedo: f32, emission: f32) -> f32 {
    settled_wall(Enclosure {
        albedo,
        emission,
        strip_emission: 0.0,
        strip_fraction: 0.0,
        promoted: false,
        cache_adds_promoted: false,
    })
}

/// Relative change of the wall when the strip toggles between promoted and cache-only.
#[must_use]
pub fn double_count(albedo: f32, strip_emission: f32, strip_fraction: f32, cache_adds_promoted: bool) -> f32 {
    let cache_only = settled_wall(Enclosure {
        albedo,
        emission: 0.0,
        strip_emission,
        strip_fraction,
        promoted: false,
        cache_adds_promoted: false,
    });
    let promoted = settled_wall(Enclosure {
        albedo,
        emission: 0.0,
        strip_emission,
        strip_fraction,
        promoted: true,
        cache_adds_promoted,
    });
    let scale = cache_only.abs().max(1.0e-6);
    (promoted - cache_only).abs() / scale
}

/// Sharpest mip the player allows. Ultra is the full mip. This does not follow VRAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureQuality {
    Ultra,
    High,
    Medium,
    Low,
}

impl TextureQuality {
    /// Mip index of the finest page this quality will stream. 0 is the full mip.
    #[must_use]
    pub fn mip_floor(self) -> u32 {
        match self {
            Self::Ultra => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Ultra => "ultra",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// Material knobs. Menu labels are these names and carry no description line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialSettings {
    pub texture_quality: TextureQuality,
    pub mip_bias: f32,
    pub anisotropy: u32,
    pub occlusion: bool,
    pub emissive_promote: bool,
    /// Fraction of the brightest lamp.
    pub emissive_promote_flux: f32,
    pub headroom_percent: u32,
    pub headroom_floor_mb: u32,
    pub dwell_frames: u32,
}

/// Environment mirrors. Each applies only when its argument was omitted.
#[derive(Clone, Copy, Debug, Default)]
pub struct SettingEnv<'a> {
    pub texture_quality: Option<&'a str>,
    pub mip_bias: Option<&'a str>,
    pub anisotropy: Option<&'a str>,
    pub occlusion: Option<&'a str>,
    pub promote: Option<&'a str>,
    pub flux: Option<&'a str>,
    pub headroom_percent: Option<&'a str>,
    pub headroom_floor_mb: Option<&'a str>,
    pub dwell_frames: Option<&'a str>,
    pub watch: Option<&'a str>,
    /// Deprecated. Logged once and ignored.
    pub residency_mb: Option<&'a str>,
}

/// `Some(true)` when `flag` is a material argument that takes the next token.
#[must_use]
pub fn material_flag_needs_value(flag: &str) -> Option<bool> {
    match flag {
        "--material-texture-quality"
        | "--material-mip-bias"
        | "--material-anisotropy"
        | "--material-occlusion"
        | "--material-emissive-promote"
        | "--material-emissive-promote-flux"
        | "--material-headroom-percent"
        | "--material-headroom-floor-mb"
        | "--material-dwell-frames"
        | "--material-residency-mb"
        | "--watch" => Some(true),
        _ => None,
    }
}

impl Default for MaterialSettings {
    fn default() -> Self {
        Self::preset(PromotePreset::High)
    }
}

impl MaterialSettings {
    #[must_use]
    pub fn preset(preset: PromotePreset) -> Self {
        let (mip_bias, anisotropy, occlusion, emissive_promote) = match preset {
            PromotePreset::Potato => (2.0, 1, false, false),
            PromotePreset::Low => (1.0, 2, true, true),
            PromotePreset::Medium => (0.0, 4, true, true),
            PromotePreset::High => (0.0, 8, true, true),
        };
        Self {
            texture_quality: TextureQuality::Ultra,
            mip_bias,
            anisotropy,
            occlusion,
            emissive_promote,
            emissive_promote_flux: 0.01,
            headroom_percent: crate::vram::DEFAULT_HEADROOM_PERCENT,
            headroom_floor_mb: crate::vram::DEFAULT_HEADROOM_FLOOR_MB,
            dwell_frames: crate::stream::DEFAULT_DWELL,
        }
    }

    /// Finest mip index `texture_quality` allows.
    #[must_use]
    pub fn quality_floor(self) -> u32 {
        self.texture_quality.mip_floor()
    }

    #[must_use]
    pub fn to_text(self) -> String {
        format!(
            "texture_quality {}\nmip_bias {}\nanisotropy {}\nocclusion {}\nemissive_promote {}\nemissive_promote_flux {}\nheadroom_percent {}\nheadroom_floor_mb {}\ndwell_frames {}\n",
            self.texture_quality.name(),
            self.mip_bias,
            self.anisotropy,
            on_off(self.occlusion),
            on_off(self.emissive_promote),
            self.emissive_promote_flux,
            self.headroom_percent,
            self.headroom_floor_mb,
            self.dwell_frames
        )
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let key = parts.next().ok_or("empty material setting")?;
            let value = parts.next().ok_or_else(|| format!("{key} needs a value"))?;
            if parts.next().is_some() {
                return Err(format!("{key} has an extra value"));
            }
            settings.set_key(key, value)?;
        }
        Ok(settings)
    }

    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "texture_quality" => self.texture_quality = parse_quality(value)?,
            "mip_bias" => self.mip_bias = parse_f32(value, "mip_bias")?,
            "anisotropy" => self.anisotropy = parse_u32(value, "anisotropy")?,
            "occlusion" => self.occlusion = parse_on(value)?,
            "emissive_promote" => self.emissive_promote = parse_on(value)?,
            "emissive_promote_flux" => {
                self.emissive_promote_flux = parse_f32(value, "emissive_promote_flux")?;
            }
            "headroom_percent" => {
                self.headroom_percent = parse_u32(value, "headroom_percent")?;
            }
            "headroom_floor_mb" => {
                self.headroom_floor_mb = parse_u32(value, "headroom_floor_mb")?;
            }
            "dwell_frames" => self.dwell_frames = parse_u32(value, "dwell_frames")?,
            "residency_mb" => warn_residency(),
            _ => return Err(format!("unknown material setting {key}")),
        }
        Ok(())
    }

    /// Overlay launch arguments. Each environment string applies only when its
    /// argument was omitted. Returns `--watch` when it was set.
    /// Other arguments are left for their own parsers.
    pub fn overlay_args(
        &mut self,
        args: &[&str],
        env: SettingEnv<'_>,
    ) -> Result<Option<String>, String> {
        let mut seen = 0u16;
        let mut watch = None;
        let mut index = 0;
        while index < args.len() {
            let flag = args[index];
            let next = args.get(index + 1).copied();
            match flag {
                "--material-texture-quality" => {
                    self.texture_quality = parse_quality(required(flag, next)?)?;
                    seen |= SAW_QUALITY;
                    index += 2;
                }
                "--material-residency-mb" => {
                    let _ignored = required(flag, next)?;
                    warn_residency();
                    seen |= SAW_RESIDENCY;
                    index += 2;
                }
                "--material-mip-bias" => {
                    self.mip_bias = parse_f32(required(flag, next)?, "mip_bias")?;
                    seen |= SAW_BIAS;
                    index += 2;
                }
                "--material-anisotropy" => {
                    self.anisotropy = parse_u32(required(flag, next)?, "anisotropy")?;
                    seen |= SAW_ANISOTROPY;
                    index += 2;
                }
                "--material-occlusion" => {
                    self.occlusion = parse_on(required(flag, next)?)?;
                    seen |= SAW_OCCLUSION;
                    index += 2;
                }
                "--material-emissive-promote" => {
                    self.emissive_promote = parse_on(required(flag, next)?)?;
                    seen |= SAW_PROMOTE;
                    index += 2;
                }
                "--material-emissive-promote-flux" => {
                    self.emissive_promote_flux = parse_f32(required(flag, next)?, "emissive_promote_flux")?;
                    seen |= SAW_FLUX;
                    index += 2;
                }
                "--material-headroom-percent" => {
                    self.headroom_percent = parse_u32(required(flag, next)?, "headroom_percent")?;
                    seen |= SAW_HEADROOM;
                    index += 2;
                }
                "--material-headroom-floor-mb" => {
                    self.headroom_floor_mb = parse_u32(required(flag, next)?, "headroom_floor_mb")?;
                    seen |= SAW_FLOOR;
                    index += 2;
                }
                "--material-dwell-frames" => {
                    self.dwell_frames = parse_u32(required(flag, next)?, "dwell_frames")?;
                    seen |= SAW_DWELL;
                    index += 2;
                }
                "--watch" => {
                    watch = Some(required(flag, next)?.to_string());
                    index += 2;
                }
                _ => index += 1,
            }
        }
        self.apply_omitted(env, seen, watch)
    }

    fn apply_omitted(
        &mut self,
        env: SettingEnv<'_>,
        seen: u16,
        mut watch: Option<String>,
    ) -> Result<Option<String>, String> {
        if seen & SAW_QUALITY == 0 {
            if let Some(value) = env.texture_quality {
                self.texture_quality = parse_quality(value)?;
            }
        }
        if seen & SAW_RESIDENCY == 0 && env.residency_mb.is_some() {
            warn_residency();
        }
        if seen & SAW_BIAS == 0 {
            if let Some(value) = env.mip_bias {
                self.mip_bias = parse_f32(value, "mip_bias")?;
            }
        }
        if seen & SAW_ANISOTROPY == 0 {
            if let Some(value) = env.anisotropy {
                self.anisotropy = parse_u32(value, "anisotropy")?;
            }
        }
        if seen & SAW_OCCLUSION == 0 {
            if let Some(value) = env.occlusion {
                self.occlusion = parse_on(value)?;
            }
        }
        if seen & SAW_PROMOTE == 0 {
            if let Some(value) = env.promote {
                self.emissive_promote = parse_on(value)?;
            }
        }
        if seen & SAW_FLUX == 0 {
            if let Some(value) = env.flux {
                self.emissive_promote_flux = parse_f32(value, "emissive_promote_flux")?;
            }
        }
        if seen & SAW_HEADROOM == 0 {
            if let Some(value) = env.headroom_percent {
                self.headroom_percent = parse_u32(value, "headroom_percent")?;
            }
        }
        if seen & SAW_FLOOR == 0 {
            if let Some(value) = env.headroom_floor_mb {
                self.headroom_floor_mb = parse_u32(value, "headroom_floor_mb")?;
            }
        }
        if seen & SAW_DWELL == 0 {
            if let Some(value) = env.dwell_frames {
                self.dwell_frames = parse_u32(value, "dwell_frames")?;
            }
        }
        if watch.is_none() {
            watch = env.watch.map(str::to_string);
        }
        Ok(watch)
    }
}

const SAW_QUALITY: u16 = 1;
const SAW_BIAS: u16 = 1 << 1;
const SAW_ANISOTROPY: u16 = 1 << 2;
const SAW_OCCLUSION: u16 = 1 << 3;
const SAW_PROMOTE: u16 = 1 << 4;
const SAW_FLUX: u16 = 1 << 5;
const SAW_HEADROOM: u16 = 1 << 6;
const SAW_FLOOR: u16 = 1 << 7;
const SAW_DWELL: u16 = 1 << 8;
const SAW_RESIDENCY: u16 = 1 << 9;

fn on_off(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}

fn parse_on(value: &str) -> Result<bool, String> {
    match value {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        _ => Err(format!("emissive promote wants on or off, got {value}")),
    }
}

fn parse_f32(value: &str, key: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("{key} wants a number, got {value}"))
}

fn parse_u32(value: &str, key: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|_| format!("{key} wants a whole number, got {value}"))
}

fn required<'a>(flag: &str, next: Option<&'a str>) -> Result<&'a str, String> {
    next.ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_quality(value: &str) -> Result<TextureQuality, String> {
    match value {
        "ultra" => Ok(TextureQuality::Ultra),
        "high" => Ok(TextureQuality::High),
        "medium" => Ok(TextureQuality::Medium),
        "low" => Ok(TextureQuality::Low),
        _ => Err(format!(
            "texture_quality wants ultra, high, medium, or low, got {value}"
        )),
    }
}

fn warn_residency() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "residency_mb is deprecated and does not set a pool; texture_quality and the VRAM budget decide which mips stay"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative(got: f32, want: f32) -> f32 {
        (got - want).abs() / want.abs().max(1.0e-6)
    }

    #[test]
    fn the_furnace_settles_at_five_times_the_emission() {
        let got = furnace_wall(0.8, 1.0);
        assert!(relative(got, 5.0) < 0.02, "{got}");
    }

    #[test]
    fn promoting_a_strip_does_not_change_the_wall() {
        let change = double_count(0.8, 1.0, 0.05, false);
        assert!(change < 1.0e-4, "{change}");
    }

    #[test]
    fn adding_the_strip_twice_moves_the_wall() {
        let change = double_count(0.8, 1.0, 0.05, true);
        assert!(change > 0.5, "{change}");
    }

    #[test]
    fn a_small_strip_is_promoted_and_potato_promotes_none() {
        let low = PromoteBudget::preset(PromotePreset::Low, 100.0, 0.01);
        assert!(should_promote(0.01, 0.0, low, 0));
        assert!(!should_promote(2.0, 0.0, low, 0));
        assert!(should_promote(2.0, 2.0, low, 0));
        assert!(!should_promote(0.01, 0.0, low, 4));
        let potato = PromoteBudget::preset(PromotePreset::Potato, 100.0, 0.01);
        assert!(!should_promote(0.01, 10.0, potato, 0));
        let high = PromoteBudget::preset(PromotePreset::High, 1.0, 0.01);
        assert_eq!(high.max_regions, 16);
    }

    #[test]
    fn the_argument_wins_over_the_environment() {
        let mut settings = MaterialSettings::default();
        let watch = settings
            .overlay_args(
                &[
                    "--material-emissive-promote",
                    "on",
                    "--material-texture-quality",
                    "high",
                    "--material-residency-mb",
                    "64",
                    "--watch",
                    "from-arg.png",
                    "--other",
                    "x",
                ],
                SettingEnv {
                    promote: Some("off"),
                    flux: Some("0.5"),
                    texture_quality: Some("low"),
                    residency_mb: Some("32"),
                    headroom_percent: Some("9"),
                    dwell_frames: Some("3"),
                    watch: Some("from-env.png"),
                    ..SettingEnv::default()
                },
            )
            .expect("overlay");
        assert!(settings.emissive_promote);
        assert_eq!(settings.texture_quality, TextureQuality::High);
        assert_eq!(settings.quality_floor(), 1);
        assert_eq!(settings.headroom_percent, 9);
        assert_eq!(settings.dwell_frames, 3);
        assert_eq!(watch.as_deref(), Some("from-arg.png"));
        assert!((settings.emissive_promote_flux - 0.5).abs() < 1.0e-6);
        let watch = settings
            .overlay_args(&[], SettingEnv { promote: Some("off"), ..SettingEnv::default() })
            .expect("env");
        assert!(!settings.emissive_promote);
        assert!(watch.is_none());
        let text = settings.to_text();
        let parsed = MaterialSettings::parse(&text).expect("round trip");
        assert_eq!(parsed, settings);
        let potato = MaterialSettings::preset(PromotePreset::Potato);
        assert_eq!(potato.texture_quality, TextureQuality::Ultra);
        assert!((potato.mip_bias - 2.0).abs() < 1.0e-6);
        assert!(!potato.occlusion);
        assert!(!potato.emissive_promote);
        assert!(material_flag_needs_value("--material-dwell-frames").unwrap_or(false));
        assert!(material_flag_needs_value("--preset").is_none());
    }
}
