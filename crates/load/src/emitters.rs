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

/// Promote switch and flux fraction. The fraction is of the brightest lamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialSettings {
    pub emissive_promote: bool,
    pub emissive_promote_flux: f32,
}

impl Default for MaterialSettings {
    fn default() -> Self {
        Self {
            emissive_promote: true,
            emissive_promote_flux: 0.01,
        }
    }
}

impl MaterialSettings {
    #[must_use]
    pub fn to_text(self) -> String {
        format!(
            "emissive_promote {}\nemissive_promote_flux {}\n",
            on_off(self.emissive_promote),
            self.emissive_promote_flux
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
            "emissive_promote" => self.emissive_promote = parse_on(value)?,
            "emissive_promote_flux" => {
                self.emissive_promote_flux = parse_flux(value)?;
            }
            _ => return Err(format!("unknown material setting {key}")),
        }
        Ok(())
    }

    /// Overlay launch arguments. Each environment string applies only when its
    /// argument was omitted. Other arguments are left for their own parsers.
    pub fn overlay_args(
        &mut self,
        args: &[&str],
        promote_env: Option<&str>,
        flux_env: Option<&str>,
    ) -> Result<(), String> {
        let mut saw_promote = false;
        let mut saw_flux = false;
        let mut index = 0;
        while index < args.len() {
            match args[index] {
                "--material-emissive-promote" => {
                    let value = args
                        .get(index + 1)
                        .copied()
                        .ok_or("--material-emissive-promote needs on or off")?;
                    self.emissive_promote = parse_on(value)?;
                    saw_promote = true;
                    index += 2;
                }
                "--material-emissive-promote-flux" => {
                    let value = args
                        .get(index + 1)
                        .copied()
                        .ok_or("--material-emissive-promote-flux needs a number")?;
                    self.emissive_promote_flux = parse_flux(value)?;
                    saw_flux = true;
                    index += 2;
                }
                _ => index += 1,
            }
        }
        if !saw_promote {
            if let Some(value) = promote_env {
                self.emissive_promote = parse_on(value)?;
            }
        }
        if !saw_flux {
            if let Some(value) = flux_env {
                self.emissive_promote_flux = parse_flux(value)?;
            }
        }
        Ok(())
    }
}

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

fn parse_flux(value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("emissive promote flux wants a number, got {value}"))
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
        settings
            .overlay_args(
                &["--material-emissive-promote", "on", "--other", "x"],
                Some("off"),
                Some("0.5"),
            )
            .expect("overlay");
        assert!(settings.emissive_promote);
        assert!((settings.emissive_promote_flux - 0.5).abs() < 1.0e-6);
        settings
            .overlay_args(&[], Some("off"), None)
            .expect("env");
        assert!(!settings.emissive_promote);
        let text = settings.to_text();
        let parsed = MaterialSettings::parse(&text).expect("round trip");
        assert_eq!(parsed, settings);
    }
}
