#![expect(
    clippy::missing_errors_doc,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! Planar mirrors, portals, and water. The picture reads this value.
//!
//! `--reflections`, `--reflection-rays`, `--mirror-views`, `--water`,
//! `--caustics`, and `--reflection-tracer` win. `GENOS_REFLECTIONS`,
//! `GENOS_REFLECTION_RAYS`, `GENOS_MIRROR_VIEWS`, `GENOS_WATER`,
//! `GENOS_CAUSTICS`, and `GENOS_REFLECTION_TRACER` apply only when that
//! argument was omitted.

use genos_math::optics::{self, ReflectionPreset};

use crate::fps_counter::parse_enabled;
use crate::EngineSettings;

/// Where a caustic map is written. The map itself waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caustics {
    Off,
    Image,
    Cache,
}

/// Which tracer a reflection ray would use. Low does not ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReflectionTracer {
    Off,
    Software,
    RayQuery,
}

/// Knobs for mirrors, portals, and water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionSettings {
    pub enabled: bool,
    pub rough_cutoff: f32,
    pub mirror_cutoff: f32,
    pub mirror_views: u32,
    pub rays: u32,
    pub shore_cap: f32,
    pub caustics: Caustics,
    pub tracer: ReflectionTracer,
    pub water: bool,
}

/// Arguments present on the command line. `None` means the flag was omitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReflectionCli {
    pub enabled: Option<bool>,
    pub rays: Option<u32>,
    pub mirror_views: Option<u32>,
    pub water: Option<bool>,
    pub caustics: Option<Caustics>,
    pub tracer: Option<ReflectionTracer>,
}

/// Environment mirrors. Read only for slots the command line left empty.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReflectionEnv<'a> {
    pub enabled: Option<&'a str>,
    pub rays: Option<&'a str>,
    pub mirror_views: Option<&'a str>,
    pub water: Option<&'a str>,
    pub caustics: Option<&'a str>,
    pub tracer: Option<&'a str>,
}

impl Default for ReflectionSettings {
    fn default() -> Self {
        Self::for_preset(ReflectionPreset::High)
    }
}

impl ReflectionSettings {
    pub const ROUGH_CUTOFF: f32 = 0.6;
    pub const MIRROR_CUTOFF: f32 = optics::MIRROR_FRESNEL_CUTOFF;
    pub const SHORE_CAP: f32 = optics::SHORE_CAP;

    /// High caps, water on, caustics and the tracer off.
    #[must_use]
    pub fn for_preset(preset: ReflectionPreset) -> Self {
        Self {
            enabled: true,
            rough_cutoff: Self::ROUGH_CUTOFF,
            mirror_cutoff: Self::MIRROR_CUTOFF,
            mirror_views: optics::reflection_view_cap(preset),
            rays: optics::reflection_ray_cap(preset),
            shore_cap: Self::SHORE_CAP,
            caustics: Caustics::Off,
            tracer: ReflectionTracer::Off,
            water: true,
        }
    }

    pub fn cli_from(args: &[String]) -> Result<ReflectionCli, String> {
        let mut cli = ReflectionCli::default();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if let Some((name, value)) = inline_flag(arg) {
                apply_flag(&mut cli, name, value)?;
                index += 1;
                continue;
            }
            if flag_name(arg).is_some() {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("{arg} needs a value"))?;
                if value.starts_with('-') {
                    return Err(format!("{arg} needs a value"));
                }
                apply_flag(&mut cli, arg, value)?;
                index += 2;
                continue;
            }
            index += 1;
        }
        Ok(cli)
    }

    pub fn cli() -> Result<ReflectionCli, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::cli_from(&args)
    }

    /// Command line, then the environment mirror, then the stored field.
    #[must_use]
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): pass this settings value in from the caller. docs/LINT_RULES.md"
    )]
    pub fn effective(&self) -> Self {
        let cli = Self::cli().unwrap_or_default();
        let enabled = cli
            .enabled
            .is_none()
            .then(|| std::env::var("GENOS_REFLECTIONS").ok())
            .flatten();
        let rays = cli
            .rays
            .is_none()
            .then(|| std::env::var("GENOS_REFLECTION_RAYS").ok())
            .flatten();
        let views = cli
            .mirror_views
            .is_none()
            .then(|| std::env::var("GENOS_MIRROR_VIEWS").ok())
            .flatten();
        let water = cli
            .water
            .is_none()
            .then(|| std::env::var("GENOS_WATER").ok())
            .flatten();
        let caustics = cli
            .caustics
            .is_none()
            .then(|| std::env::var("GENOS_CAUSTICS").ok())
            .flatten();
        let tracer = cli
            .tracer
            .is_none()
            .then(|| std::env::var("GENOS_REFLECTION_TRACER").ok())
            .flatten();
        self.resolve(
            &cli,
            &ReflectionEnv {
                enabled: enabled.as_deref(),
                rays: rays.as_deref(),
                mirror_views: views.as_deref(),
                water: water.as_deref(),
                caustics: caustics.as_deref(),
                tracer: tracer.as_deref(),
            },
        )
    }

    #[must_use]
    pub fn resolve(&self, cli: &ReflectionCli, env: &ReflectionEnv<'_>) -> Self {
        Self {
            enabled: cli
                .enabled
                .or_else(|| env_flag(env.enabled))
                .unwrap_or(self.enabled),
            rough_cutoff: self.rough_cutoff,
            mirror_cutoff: self.mirror_cutoff,
            mirror_views: cli
                .mirror_views
                .or_else(|| env_views(env.mirror_views))
                .unwrap_or(self.mirror_views),
            rays: cli.rays.or_else(|| env_rays(env.rays)).unwrap_or(self.rays),
            shore_cap: self.shore_cap,
            caustics: cli
                .caustics
                .or_else(|| env_caustics(env.caustics))
                .unwrap_or(self.caustics),
            tracer: cli
                .tracer
                .or_else(|| env_tracer(env.tracer))
                .unwrap_or(self.tracer),
            water: cli
                .water
                .or_else(|| env_flag(env.water))
                .unwrap_or(self.water),
        }
    }

    pub fn to_text(&self) -> String {
        format!(
            "enabled={}\nrough_cutoff={}\nmirror_cutoff={}\nmirror_views={}\nrays={}\nshore_cap={}\ncaustics={}\ntracer={}\nwater={}\n",
            flag(self.enabled),
            num_text(self.rough_cutoff),
            num_text(self.mirror_cutoff),
            self.mirror_views,
            self.rays,
            num_text(self.shore_cap),
            self.caustics.as_str(),
            self.tracer.as_str(),
            flag(self.water),
        )
    }

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
            settings.set_key(key, value)?;
        }
        Ok(settings)
    }

    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        match key {
            "enabled" | "reflections" => self.enabled = parse_enabled(value)?,
            "rough_cutoff" => self.rough_cutoff = parse_unit(value, "rough_cutoff")?,
            "mirror_cutoff" => self.mirror_cutoff = parse_unit(value, "mirror_cutoff")?,
            "mirror_views" => self.mirror_views = parse_views(value)?,
            "rays" | "reflection_rays" => self.rays = parse_rays(value)?,
            "shore_cap" => self.shore_cap = parse_unit(value, "shore_cap")?,
            "caustics" => self.caustics = parse_caustics(value)?,
            "tracer" | "reflection_tracer" => self.tracer = parse_tracer(value)?,
            "water" => self.water = parse_enabled(value)?,
            _ => return Err(format!("unknown reflection setting {key}")),
        }
        Ok(())
    }
}

impl EngineSettings for ReflectionSettings {
    fn to_text(&self) -> String {
        ReflectionSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        ReflectionSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        ReflectionSettings::set_key(self, key, value)
    }
}

impl Caustics {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Image => "image",
            Self::Cache => "cache",
        }
    }
}

impl ReflectionTracer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Software => "software",
            Self::RayQuery => "ray_query",
        }
    }
}

/// `Some(true)` when `arg` reads the next argument. `Some(false)` when the value is inline.
pub fn reflection_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, inline) = split_flag(arg)?;
    flag_name(name)?;
    Some(inline.is_none())
}

/// Reject a bad reflection argument.
pub fn check_reflection_arg(arg: &str, next: Option<&str>) -> Result<(), String> {
    let Some((name, inline)) = split_flag(arg) else {
        return Ok(());
    };
    if flag_name(name).is_none() {
        return Ok(());
    }
    let value = inline
        .or(next)
        .ok_or_else(|| format!("{name} needs a value"))?;
    let mut cli = ReflectionCli::default();
    apply_flag(&mut cli, name, value)?;
    Ok(())
}

fn inline_flag(arg: &str) -> Option<(&str, &str)> {
    let (name, value) = arg.split_once('=')?;
    flag_name(name)?;
    Some((name, value))
}

fn apply_flag(cli: &mut ReflectionCli, name: &str, value: &str) -> Result<(), String> {
    match name {
        "--reflections" => cli.enabled = Some(parse_enabled(value)?),
        "--reflection-rays" => cli.rays = Some(parse_rays(value)?),
        "--mirror-views" => cli.mirror_views = Some(parse_views(value)?),
        "--water" => cli.water = Some(parse_enabled(value)?),
        "--caustics" => cli.caustics = Some(parse_caustics(value)?),
        "--reflection-tracer" => cli.tracer = Some(parse_tracer(value)?),
        _ => return Err(format!("unknown reflection argument {name}")),
    }
    Ok(())
}

fn split_flag(arg: &str) -> Option<(&str, Option<&str>)> {
    if !arg.starts_with("--") {
        return None;
    }
    match arg.split_once('=') {
        Some((name, value)) => Some((name, Some(value))),
        None => Some((arg, None)),
    }
}

fn flag_name(name: &str) -> Option<()> {
    match name {
        "--reflections"
        | "--reflection-rays"
        | "--mirror-views"
        | "--water"
        | "--caustics"
        | "--reflection-tracer" => Some(()),
        _ => None,
    }
}

fn parse_caustics(value: &str) -> Result<Caustics, String> {
    match value.trim() {
        "off" => Ok(Caustics::Off),
        "image" => Ok(Caustics::Image),
        "cache" => Ok(Caustics::Cache),
        _ => Err(format!("caustics is off, image, or cache, got {value}")),
    }
}

fn parse_tracer(value: &str) -> Result<ReflectionTracer, String> {
    match value.trim() {
        "off" => Ok(ReflectionTracer::Off),
        "software" => Ok(ReflectionTracer::Software),
        "ray_query" => Ok(ReflectionTracer::RayQuery),
        _ => Err(format!(
            "tracer is off, software, or ray_query, got {value}"
        )),
    }
}

fn parse_unit(value: &str, key: &str) -> Result<f32, String> {
    let number: f32 = value
        .trim()
        .parse()
        .map_err(|_| format!("{key} needs a number, got {value}"))?;
    if !number.is_finite() || !(0.0..=1.0).contains(&number) {
        return Err(format!("{key} is 0 to 1, got {value}"));
    }
    Ok(number)
}

fn parse_views(value: &str) -> Result<u32, String> {
    let number = parse_u32(value, "mirror_views")?;
    if number > 3 {
        return Err(format!("mirror_views is 0 to 3, got {value}"));
    }
    Ok(number)
}

fn parse_rays(value: &str) -> Result<u32, String> {
    let number = parse_u32(value, "rays")?;
    if number > 128_000 {
        return Err(format!("rays is 0 to 128000, got {value}"));
    }
    Ok(number)
}

fn parse_u32(value: &str, key: &str) -> Result<u32, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{key} needs a number, got {value}"))
}

fn flag(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn num_text(value: f32) -> String {
    format!("{value:?}")
}

fn env_flag(value: Option<&str>) -> Option<bool> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_enabled(text).ok()
}

fn env_rays(value: Option<&str>) -> Option<u32> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_rays(text).ok()
}

fn env_views(value: Option<&str>) -> Option<u32> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_views(text).ok()
}

fn env_caustics(value: Option<&str>) -> Option<Caustics> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_caustics(text).ok()
}

fn env_tracer(value: Option<&str>) -> Option<ReflectionTracer> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_tracer(text).ok()
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "a failed settings parse aborts the test"
)]
mod tests {
    use super::{Caustics, ReflectionCli, ReflectionEnv, ReflectionSettings, ReflectionTracer};
    use genos_math::optics::ReflectionPreset;

    #[test]
    fn low_preset_schedules_no_rays() {
        let low = ReflectionSettings::for_preset(ReflectionPreset::Low);
        assert_eq!(low.rays, 0);
        assert_eq!(low.mirror_views, 1);
        assert!(low.water);
        let high = ReflectionSettings::default();
        assert_eq!(high.rays, 128_000);
        assert_eq!(high.mirror_views, 3);
    }

    #[test]
    fn command_line_wins_over_the_mirror() {
        let stored = ReflectionSettings::default();
        let cli = ReflectionCli {
            enabled: Some(false),
            rays: Some(0),
            mirror_views: None,
            water: None,
            caustics: Some(Caustics::Image),
            tracer: None,
        };
        let env = ReflectionEnv {
            enabled: Some("on"),
            rays: Some("64"),
            mirror_views: Some("1"),
            water: Some("off"),
            caustics: Some("cache"),
            tracer: Some("software"),
        };
        let got = stored.resolve(&cli, &env);
        assert!(!got.enabled);
        assert_eq!(got.rays, 0);
        assert_eq!(got.mirror_views, 1);
        assert!(!got.water);
        assert_eq!(got.caustics, Caustics::Image);
        assert_eq!(got.tracer, ReflectionTracer::Software);
    }

    #[test]
    fn text_round_trips() {
        let mut settings = ReflectionSettings::for_preset(ReflectionPreset::Low);
        settings.tracer = ReflectionTracer::RayQuery;
        settings.caustics = Caustics::Cache;
        let parsed = ReflectionSettings::parse(&settings.to_text()).expect("parse");
        assert_eq!(parsed, settings);
    }
}
