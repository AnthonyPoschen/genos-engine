//! GPU particle budget and the stage-1 emitter.
//!
//! The quality knobs are a preset table: Low 64K, Medium 256K, High 1M.
//! `count` is the live cap and cannot exceed `max_particles`. The compute
//! step reads this struct. It does not keep a second copy of the numbers.
//!
//! `.gfx` curve emitters are a later stage. This struct is the one emitter
//! the stage-1 compute shader simulates.

use std::fmt::Write;

use crate::EngineSettings;

/// Particles the stage-1 buffer can hold.
pub const LOW_MAX_PARTICLES: u32 = 65_536;
/// Particles the stage-1 buffer can hold.
pub const MEDIUM_MAX_PARTICLES: u32 = 262_144;
/// Particles the stage-1 buffer can hold.
pub const HIGH_MAX_PARTICLES: u32 = 1_048_576;

/// Seed the step uses when the setting is zero. A zero xorshift stays zero.
pub const DEFAULT_SEED: u32 = 0xA341_316C;

/// Live cap `--fx-spawn` and `fx.spawn` use when the emitter is at count 0
/// and neither `--fx-count` nor `GENOS_FX_COUNT` set one.
pub const SPAWN_COUNT: u32 = 64;

/// Named budget. `Custom` means a knob was set after the preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticlePreset {
    /// 64K, sort off, vertex lit, half resolution, no depth collision.
    Low,
    /// 256K, sort on, vertex lit, full resolution, depth collision.
    Medium,
    /// 1M, sort on, froxel lit, full resolution, depth collision.
    High,
    /// A knob no longer matches a named row.
    Custom,
}

impl ParticlePreset {
    /// `low`, `medium`, `high`, or `custom`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Custom => "custom",
        }
    }

    /// Parse a preset name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// How a later stage lights a particle. Stage 1 stores the choice and does not sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleLight {
    /// Low and Medium. A later stage lights from the vertex.
    Vertex,
    /// High. A later stage samples the froxel and the GI cache.
    Froxel,
}

impl ParticleLight {
    /// `vertex` or `froxel`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Froxel => "froxel",
        }
    }

    /// Parse a light mode.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "vertex" | "vertex-only" => Some(Self::Vertex),
            "froxel" | "gi" => Some(Self::Froxel),
            _ => None,
        }
    }
}

/// Stage-1 particle settings. A public [`EngineSettings`] value on the scene.
///
/// Keys: `preset`, `enabled`, `max_particles`, `count`, `sort`, `lit`,
/// `half_res`, `depth_collision`, `freeze`, `seed`, `origin`, `rate`, `life`,
/// `rise`, `spread`, `size`, `albedo`, `soft_band`.
#[allow(clippy::struct_excessive_bools, reason = "the design lists each knob")]
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleSettings {
    /// Which named row the quality knobs last came from.
    pub preset: ParticlePreset,
    /// Off skips the compute dispatch.
    pub enabled: bool,
    /// Buffer budget. Low 65536, Medium 262144, High 1048576.
    pub max_particles: u32,
    /// Live cap. Zero spawns nothing. The step uses the smaller of this and the budget.
    pub count: u32,
    /// Stored for the later sort. Stage 1 does not sort.
    pub sort: bool,
    /// Stored for the later lighting stage.
    pub lit: ParticleLight,
    /// Stored for a later half-resolution draw. Stage 1 draws in the existing pass.
    pub half_res: bool,
    /// Retire a particle that is behind the depth buffer.
    pub depth_collision: bool,
    /// Keep the positions and skip integrate and emit.
    pub freeze: bool,
    /// Xorshift seed. Zero selects [`DEFAULT_SEED`].
    pub seed: u32,
    /// Emitter origin, metres.
    pub origin: [f32; 3],
    /// Births per second.
    pub rate: f32,
    /// Lifetime, seconds.
    pub life: f32,
    /// Upward speed, metres per second.
    pub rise: f32,
    /// Spawn radius and horizontal speed, metres.
    pub spread: f32,
    /// Card width, metres.
    pub size: f32,
    /// Unlit card colour.
    pub albedo: [f32; 3],
    /// Depth-fade width in NDC. Zero disables the fade and still collides.
    pub soft_band: f32,
}

impl Default for ParticleSettings {
    fn default() -> Self {
        Self::preset(ParticlePreset::Low)
    }
}

impl ParticleSettings {
    /// Quality knobs for `preset`. The emitter starts disabled with `count` 0.
    #[must_use]
    pub fn preset(preset: ParticlePreset) -> Self {
        let (max_particles, sort, lit, half_res, depth_collision) = match preset {
            ParticlePreset::Low | ParticlePreset::Custom => {
                (LOW_MAX_PARTICLES, false, ParticleLight::Vertex, true, false)
            }
            ParticlePreset::Medium => (
                MEDIUM_MAX_PARTICLES,
                true,
                ParticleLight::Vertex,
                false,
                true,
            ),
            ParticlePreset::High => (
                HIGH_MAX_PARTICLES,
                true,
                ParticleLight::Froxel,
                false,
                true,
            ),
        };
        Self {
            preset,
            enabled: false,
            max_particles,
            count: 0,
            sort,
            lit,
            half_res,
            depth_collision,
            freeze: false,
            seed: DEFAULT_SEED,
            origin: [0.0, 1.0, 0.0],
            rate: 8.0,
            life: 1.0,
            rise: 1.0,
            spread: 0.2,
            size: 0.4,
            albedo: [1.0, 0.55, 0.12],
            soft_band: 0.08,
        }
    }

    /// The compute runs when the emitter is on, the budget is non-zero, and `count` is non-zero.
    #[must_use]
    pub fn dispatches(&self) -> bool {
        self.enabled && self.max_particles > 0 && self.count > 0
    }

    /// Slots the step will fill. Never above the budget.
    #[must_use]
    pub fn live_cap(&self) -> u32 {
        self.count.min(self.max_particles)
    }

    /// Seed the shader and the CPU mirror share. Zero becomes [`DEFAULT_SEED`].
    #[must_use]
    pub fn rng_seed(&self) -> u32 {
        if self.seed == 0 {
            DEFAULT_SEED
        } else {
            self.seed
        }
    }

    /// One line per key.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "preset={}", self.preset.name());
        let _ = writeln!(out, "enabled={}", on_off(self.enabled));
        let _ = writeln!(out, "max_particles={}", self.max_particles);
        let _ = writeln!(out, "count={}", self.count);
        let _ = writeln!(out, "sort={}", on_off(self.sort));
        let _ = writeln!(out, "lit={}", self.lit.name());
        let _ = writeln!(out, "half_res={}", on_off(self.half_res));
        let _ = writeln!(out, "depth_collision={}", on_off(self.depth_collision));
        let _ = writeln!(out, "freeze={}", on_off(self.freeze));
        let _ = writeln!(out, "seed={}", self.seed);
        let _ = writeln!(
            out,
            "origin={} {} {}",
            num(self.origin[0]),
            num(self.origin[1]),
            num(self.origin[2])
        );
        let _ = writeln!(out, "rate={}", num(self.rate));
        let _ = writeln!(out, "life={}", num(self.life));
        let _ = writeln!(out, "rise={}", num(self.rise));
        let _ = writeln!(out, "spread={}", num(self.spread));
        let _ = writeln!(out, "size={}", num(self.size));
        let _ = writeln!(
            out,
            "albedo={} {} {}",
            num(self.albedo[0]),
            num(self.albedo[1]),
            num(self.albedo[2])
        );
        let _ = writeln!(out, "soft_band={}", num(self.soft_band));
        out
    }

    /// Parse the text form. Unknown keys fail.
    ///
    /// # Errors
    ///
    /// Returns an error on a bad line, an unknown key, or a value that does not match.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::default();
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

    /// Overlay one key.
    ///
    /// # Errors
    ///
    /// Returns an error on an unknown key or a value that does not match.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        match key {
            "preset" => self.apply_preset_name(value)?,
            "enabled" => self.enabled = parse_on(value)?,
            "max_particles" | "max" => {
                self.max_particles = parse_u32("max_particles", value)?;
                self.preset = ParticlePreset::Custom;
            }
            "count" => self.count = parse_u32("count", value)?,
            "sort" => self.sort = parse_on(value)?,
            "lit" => {
                self.lit = ParticleLight::parse(value)
                    .ok_or_else(|| format!("lit is vertex or froxel, got {value}"))?;
            }
            "half_res" => self.half_res = parse_on(value)?,
            "depth_collision" => self.depth_collision = parse_on(value)?,
            "freeze" => self.freeze = parse_on(value)?,
            "seed" => self.seed = parse_u32("seed", value)?,
            "origin" => self.origin = parse_vec3("origin", value)?,
            "origin_x" => self.origin[0] = parse_f32("origin_x", value)?,
            "origin_y" => self.origin[1] = parse_f32("origin_y", value)?,
            "origin_z" => self.origin[2] = parse_f32("origin_z", value)?,
            "rate" => self.rate = parse_f32("rate", value)?.max(0.0),
            "life" => self.life = parse_f32("life", value)?.max(0.0),
            "rise" => self.rise = parse_f32("rise", value)?,
            "spread" => self.spread = parse_f32("spread", value)?.max(0.0),
            "size" => self.size = parse_f32("size", value)?.max(0.0),
            "albedo" => self.albedo = parse_vec3("albedo", value)?,
            "soft_band" => self.soft_band = parse_f32("soft_band", value)?.max(0.0),
            _ => return Err(format!("unknown particle setting {key}")),
        }
        Ok(())
    }

    /// Arguments win. An omitted argument keeps the scene value, unless the mirror is set.
    #[must_use]
    pub fn resolve(&self, cli: &ParticleCli, env: &ParticleEnv) -> Self {
        let mut out = self.clone();
        if let Some(preset) = cli.preset.or(env.preset) {
            out.apply_named(preset);
        }
        if let Some(max_particles) = cli.max_particles.or(env.max_particles) {
            out.max_particles = max_particles;
            out.preset = ParticlePreset::Custom;
        }
        if let Some(count) = cli.count.or(env.count) {
            out.count = count;
        }
        if let Some(freeze) = cli.freeze.or(env.freeze) {
            out.freeze = freeze;
        }
        if let Some(enabled) = cli.enabled {
            out.enabled = enabled;
        }
        let count_set = cli.count.is_some() || env.count.is_some();
        if let Some(origin) = cli.origin {
            out.origin = origin;
            out.enabled = true;
            if !count_set && out.count == 0 {
                out.count = SPAWN_COUNT;
            }
        }
        out
    }

    fn apply_preset_name(&mut self, value: &str) -> Result<(), String> {
        let preset = ParticlePreset::parse(value)
            .ok_or_else(|| format!("preset is low, medium, or high, got {value}"))?;
        self.apply_named(preset);
        Ok(())
    }

    fn apply_named(&mut self, preset: ParticlePreset) {
        let fresh = Self::preset(preset);
        self.preset = fresh.preset;
        self.max_particles = fresh.max_particles;
        self.sort = fresh.sort;
        self.lit = fresh.lit;
        self.half_res = fresh.half_res;
        self.depth_collision = fresh.depth_collision;
    }
}

impl EngineSettings for ParticleSettings {
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

/// `--fx-*`. `None` means the argument was omitted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleCli {
    /// `--fx-preset`.
    pub preset: Option<ParticlePreset>,
    /// `--fx-max`.
    pub max_particles: Option<u32>,
    /// `--fx-count`.
    pub count: Option<u32>,
    /// `--fx-freeze`.
    pub freeze: Option<bool>,
    /// `--fx-spawn` also turns the emitter on.
    pub enabled: Option<bool>,
    /// `--fx-spawn X Y Z`.
    pub origin: Option<[f32; 3]>,
}

/// Mirrors of the particle arguments. Set only when that argument was omitted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleEnv {
    /// `GENOS_FX_PRESET`.
    pub preset: Option<ParticlePreset>,
    /// `GENOS_FX_MAX`.
    pub max_particles: Option<u32>,
    /// `GENOS_FX_COUNT`.
    pub count: Option<u32>,
    /// `GENOS_FX_FREEZE`.
    pub freeze: Option<bool>,
}

/// Whether `--fx-*` takes a following argument.
#[must_use]
pub fn particle_flag_needs_value(arg: &str) -> Option<bool> {
    match arg.split('=').next().unwrap_or(arg) {
        "--fx-preset" | "--fx-max" | "--fx-count" | "--fx-spawn" => Some(true),
        "--fx-freeze" => Some(false),
        _ => None,
    }
}

/// Write one particle argument into `cli`.
///
/// # Errors
///
/// Returns an error when the flag is unknown or the value does not match.
pub fn apply_particle_arg(cli: &mut ParticleCli, arg: &str, next: Option<&str>) -> Result<(), String> {
    let (name, inline) = split_flag(arg);
    let value = inline.or(next);
    match name {
        "--fx-preset" => {
            let text = value.ok_or_else(|| format!("{name} needs a value"))?;
            cli.preset = Some(
                ParticlePreset::parse(text)
                    .ok_or_else(|| format!("preset is low, medium, or high, got {text}"))?,
            );
        }
        "--fx-max" => {
            let text = value.ok_or_else(|| format!("{name} needs a value"))?;
            cli.max_particles = Some(parse_u32("max_particles", text)?);
        }
        "--fx-count" => {
            let text = value.ok_or_else(|| format!("{name} needs a value"))?;
            cli.count = Some(parse_u32("count", text)?);
        }
        "--fx-freeze" => cli.freeze = Some(true),
        "--fx-spawn" => {
            let text = value.ok_or_else(|| format!("{name} needs x y z"))?;
            cli.origin = Some(parse_vec3("origin", text)?);
            cli.enabled = Some(true);
        }
        _ => return Err(format!("unknown particle argument {arg}")),
    }
    Ok(())
}

fn split_flag(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (arg, None),
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
    let mut text = format!("{value:.6}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text.is_empty() || text == "-" {
        "0".to_string()
    } else {
        text
    }
}

fn parse_on(value: &str) -> Result<bool, String> {
    match value {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        _ => Err(format!("expected on or off, got {value}")),
    }
}

fn parse_u32(key: &str, value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|_| format!("{key} is a whole number, got {value}"))
}

fn parse_f32(key: &str, value: &str) -> Result<f32, String> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| format!("{key} is a number, got {value}"))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(format!("{key} is a number, got {value}"))
    }
}

fn parse_vec3(key: &str, value: &str) -> Result<[f32; 3], String> {
    let parts: Vec<&str> = value.split([' ', ',']).filter(|part| !part.is_empty()).collect();
    if parts.len() != 3 {
        return Err(format!("{key} is three numbers, got {value}"));
    }
    Ok([
        parse_f32(key, parts[0])?,
        parse_f32(key, parts[1])?,
        parse_f32(key, parts[2])?,
    ])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::*;

    #[test]
    fn presets_match_the_budget_table() {
        let low = ParticleSettings::preset(ParticlePreset::Low);
        let medium = ParticleSettings::preset(ParticlePreset::Medium);
        let high = ParticleSettings::preset(ParticlePreset::High);
        assert_eq!(low.max_particles, 65_536);
        assert_eq!(medium.max_particles, 262_144);
        assert_eq!(high.max_particles, 1_048_576);
        assert!(!low.sort);
        assert!(medium.sort && high.sort);
        assert_eq!(low.lit, ParticleLight::Vertex);
        assert_eq!(high.lit, ParticleLight::Froxel);
        assert!(low.half_res);
        assert!(!high.half_res);
        assert!(!low.depth_collision);
        assert!(medium.depth_collision && high.depth_collision);
        assert!(!low.dispatches());
    }

    #[test]
    fn text_round_trips_and_count_is_clamped_by_the_budget() {
        let mut settings = ParticleSettings::preset(ParticlePreset::High);
        settings.set_key("enabled", "on").unwrap();
        settings.set_key("count", "1000").unwrap();
        settings.set_key("max_particles", "64").unwrap();
        let parsed = ParticleSettings::parse(&settings.to_text()).unwrap();
        assert_eq!(parsed, settings);
        assert_eq!(parsed.preset, ParticlePreset::Custom);
        assert_eq!(parsed.live_cap(), 64);
        assert!(parsed.dispatches());
    }

    #[test]
    fn the_argument_wins_over_the_mirror() {
        let scene = ParticleSettings::default();
        let cli = ParticleCli {
            max_particles: Some(32),
            count: Some(4),
            ..ParticleCli::default()
        };
        let env = ParticleEnv {
            max_particles: Some(8),
            count: Some(2),
            freeze: Some(true),
            ..ParticleEnv::default()
        };
        let resolved = scene.resolve(&cli, &env);
        assert_eq!(resolved.max_particles, 32);
        assert_eq!(resolved.count, 4);
        assert!(resolved.freeze);
        assert_eq!(resolved.preset, ParticlePreset::Custom);
    }

    #[test]
    fn spawn_without_a_count_dispatches() {
        let scene = ParticleSettings::default();
        let mut cli = ParticleCli::default();
        apply_particle_arg(&mut cli, "--fx-spawn", Some("1,2,3")).unwrap();
        let spawned = scene.resolve(&cli, &ParticleEnv::default());
        assert!(spawned.enabled);
        assert_eq!(spawned.origin, [1.0, 2.0, 3.0]);
        assert_eq!(spawned.count, SPAWN_COUNT);
        assert!(spawned.dispatches());
        let mut counted = cli.clone();
        counted.count = Some(0);
        let held = scene.resolve(&counted, &ParticleEnv::default());
        assert_eq!(held.count, 0);
        assert!(!held.dispatches());
    }
}
