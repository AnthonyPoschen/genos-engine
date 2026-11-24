#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]

//! HDR output settings. The renderer calls the references in `hdr_output`.
//!
//! `legacy_bytes` keeps exposure 1, bloom off, and the legacy shoulder, so a
//! picture built before this pass does not move. [`HdrSettings::for_preset`]
//! is the design default for a named lighting preset.

use crate::{EngineSettings, LightingPreset};

/// `auto` meters, `camera` uses aperture/shutter/ISO, `fixed` uses `ev100`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExposureMode {
    Auto,
    Camera,
    Fixed,
}

/// Display transform. `aces` stays empty until the Academy CTL is the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayTransform {
    Agx,
    Aces,
    Legacy,
}

/// Swapchain choice. Fallback and Low force [`OutputMode::Sdr`] on desktop and on the Deck LCD.
/// Low on the Deck OLED may take HDR10 when that pair is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Auto,
    Sdr,
    Hdr10,
    Scrgb,
}

/// One serializable HDR setup. CLI wins over the environment, which wins over the fields.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrSettings {
    pub exposure_mode: ExposureMode,
    pub ev100: f64,
    pub compensation: f64,
    pub iso: f64,
    pub aperture: f64,
    pub shutter: f64,
    pub adapt_down: f64,
    pub adapt_up: f64,
    /// How far auto exposure follows the metered average when the frame is
    /// darker than its highlight key: 1 follows it fully, 0 holds the key.
    pub adapt_strength: f64,
    /// The share of metered pixels under the highlight luminance (0.5..=1).
    pub highlight_percentile: f64,
    /// Stops above mid-grey that the highlight luminance may sit (the key).
    pub highlight_stops: f64,
    /// Histogram bins. High 256, Medium 128, Low 64, Fallback unused.
    pub histogram_bins: u32,
    pub display: DisplayTransform,
    pub bloom: bool,
    pub bloom_intensity: f64,
    pub bloom_levels: u32,
    pub output: OutputMode,
    /// Seconds a change of `ev100`, `compensation`, or the bloom intensity takes
    /// to reach the picture. 0 applies the change on the next frame.
    pub transition: f64,
    /// Exact exposure 1. The meter is not applied. Picture bytes stay on the legacy shoulder.
    pub legacy_bytes: bool,
}

/// Flags from one argument list. Empty slots were omitted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HdrCli {
    pub exposure_mode: Option<ExposureMode>,
    pub ev100: Option<f64>,
    pub compensation: Option<f64>,
    pub iso: Option<f64>,
    pub aperture: Option<f64>,
    pub shutter: Option<f64>,
    pub adapt_down: Option<f64>,
    pub adapt_up: Option<f64>,
    pub adapt_strength: Option<f64>,
    pub highlight_percentile: Option<f64>,
    pub highlight_stops: Option<f64>,
    pub display: Option<DisplayTransform>,
    pub bloom: Option<bool>,
    pub bloom_intensity: Option<f64>,
    pub bloom_levels: Option<u32>,
    pub output: Option<OutputMode>,
    pub transition: Option<f64>,
}

/// Environment mirrors of the design-table keys, then `GENOS_HDR_OUT` for `--out`.
/// The argument wins. The variable applies only when that argument was omitted.
pub const HDR_ENV_MIRRORS: [&str; 18] = [
    "GENOS_HDR_EXPOSURE_MODE",
    "GENOS_HDR_EV100",
    "GENOS_HDR_COMPENSATION",
    "GENOS_HDR_ISO",
    "GENOS_HDR_APERTURE",
    "GENOS_HDR_SHUTTER",
    "GENOS_HDR_ADAPT_DOWN",
    "GENOS_HDR_ADAPT_UP",
    "GENOS_HDR_ADAPT_STRENGTH",
    "GENOS_HDR_HIGHLIGHT_PERCENTILE",
    "GENOS_HDR_HIGHLIGHT_STOPS",
    "GENOS_HDR_DISPLAY",
    "GENOS_HDR_BLOOM",
    "GENOS_HDR_BLOOM_INTENSITY",
    "GENOS_HDR_BLOOM_LEVELS",
    "GENOS_HDR_OUTPUT",
    "GENOS_HDR_TRANSITION",
    "GENOS_HDR_OUT",
];

impl Default for HdrSettings {
    fn default() -> Self {
        Self::legacy_bytes()
    }
}

impl HdrSettings {
    /// Exposure 1, bloom off, legacy shoulder, SDR. The pre-HDR picture.
    pub fn legacy_bytes() -> Self {
        Self {
            exposure_mode: ExposureMode::Fixed,
            ev100: 0.0,
            compensation: 0.0,
            iso: 100.0,
            aperture: 4.0,
            shutter: 1.0 / 60.0,
            adapt_down: 0.40,
            adapt_up: 0.80,
            adapt_strength: 1.0,
            highlight_percentile: HIGHLIGHT_PERCENTILE,
            highlight_stops: HIGHLIGHT_STOPS,
            histogram_bins: 256,
            display: DisplayTransform::Legacy,
            bloom: false,
            bloom_intensity: 0.0,
            bloom_levels: 0,
            output: OutputMode::Sdr,
            transition: 0.0,
            legacy_bytes: true,
        }
    }

    /// Design defaults for a lighting preset. `AgX` is the display transform.
    /// Bloom is on at intensity 0.05 except Fallback. `legacy_bytes` stays on the
    /// shoulder with bloom off, so an old picture does not move.
    pub fn for_preset(preset: LightingPreset) -> Self {
        let mut settings = Self::legacy_bytes();
        settings.legacy_bytes = false;
        settings.display = DisplayTransform::Agx;
        settings.bloom = preset != LightingPreset::Fallback;
        settings.bloom_intensity = if preset == LightingPreset::Fallback {
            0.0
        } else {
            0.05
        };
        settings.adapt_strength = ADAPT_STRENGTH;
        match preset {
            LightingPreset::Fallback => {
                settings.exposure_mode = ExposureMode::Fixed;
                settings.output = OutputMode::Sdr;
                settings.bloom_levels = 0;
            }
            LightingPreset::Low => {
                settings.exposure_mode = ExposureMode::Auto;
                settings.adapt_down = 0.50;
                settings.adapt_up = 1.00;
                settings.histogram_bins = 64;
                settings.bloom_levels = 4;
                settings.output = OutputMode::Sdr;
            }
            LightingPreset::Medium | LightingPreset::DayCycle | LightingPreset::Gi1Bounce => {
                settings.exposure_mode = ExposureMode::Auto;
                settings.histogram_bins = if preset == LightingPreset::Medium {
                    128
                } else {
                    256
                };
                settings.bloom_levels = 5;
                settings.output = OutputMode::Auto;
            }
            LightingPreset::High => {
                settings.exposure_mode = ExposureMode::Auto;
                settings.histogram_bins = 256;
                settings.bloom_levels = 6;
                settings.output = OutputMode::Auto;
            }
        }
        settings
    }

    /// High plus the seventh bloom level. ADR 0027's Ultra. Not a lighting preset.
    pub fn ultra() -> Self {
        let mut settings = Self::for_preset(LightingPreset::High);
        settings.bloom_levels = 7;
        settings
    }

    pub fn cli_from(args: &[String]) -> Result<HdrCli, String> {
        let mut cli = HdrCli::default();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            let (key, inline) = if let Some((key, value)) = arg.split_once('=') {
                (key, Some(value))
            } else {
                (arg.as_str(), None)
            };
            let take = |index: &mut usize| -> Result<String, String> {
                if let Some(value) = inline {
                    return Ok(value.to_string());
                }
                let value = args
                    .get(*index + 1)
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| format!("{key} needs a value"))?;
                *index += 1;
                Ok(value.clone())
            };
            match key {
                "--exposure-mode" => cli.exposure_mode = Some(parse_exposure(&take(&mut index)?)?),
                "--ev100" => cli.ev100 = Some(parse_f64("ev100", &take(&mut index)?)?),
                "--compensation" => {
                    cli.compensation = Some(parse_f64("compensation", &take(&mut index)?)?);
                }
                "--iso" => cli.iso = Some(parse_f64("iso", &take(&mut index)?)?),
                "--aperture" => cli.aperture = Some(parse_f64("aperture", &take(&mut index)?)?),
                "--shutter" => cli.shutter = Some(parse_f64("shutter", &take(&mut index)?)?),
                "--adapt-down" => {
                    cli.adapt_down = Some(parse_f64("adapt_down", &take(&mut index)?)?);
                }
                "--adapt-up" => cli.adapt_up = Some(parse_f64("adapt_up", &take(&mut index)?)?),
                "--adapt-strength" => {
                    cli.adapt_strength = Some(parse_share("adapt_strength", &take(&mut index)?)?);
                }
                "--highlight-percentile" => {
                    cli.highlight_percentile =
                        Some(parse_percentile(&take(&mut index)?)?);
                }
                "--highlight-stops" => {
                    cli.highlight_stops = Some(parse_stops(&take(&mut index)?)?);
                }
                "--hdr-display" => cli.display = Some(parse_display(&take(&mut index)?)?),
                "--display" => {
                    let value = take(&mut index)?;
                    if let Ok(mode) = parse_display(&value) {
                        cli.display = Some(mode);
                    }
                }
                "--bloom" => cli.bloom = Some(parse_on(&take(&mut index)?)?),
                "--bloom-intensity" => {
                    cli.bloom_intensity = Some(parse_f64("bloom_intensity", &take(&mut index)?)?);
                }
                "--bloom-levels" => {
                    cli.bloom_levels = Some(parse_u32("bloom_levels", &take(&mut index)?)?);
                }
                "--output" => cli.output = Some(parse_output(&take(&mut index)?)?),
                "--hdr-transition" => {
                    cli.transition = Some(parse_seconds("transition", &take(&mut index)?)?);
                }
                _ => {}
            }
            index += 1;
        }
        Ok(cli)
    }

    /// CLI wins. A set `GENOS_HDR_*` applies only when that argument was omitted.
    #[allow(
        clippy::disallowed_methods,
        reason = "the argument wins; GENOS_HDR_* applies only when that argument was omitted"
    )]
    pub fn effective(&self) -> Result<Self, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let cli = Self::cli_from(&args)?;
        let env = |name: &str| std::env::var(name).ok();
        Ok(self.resolve(&cli, &HdrEnv {
            exposure_mode: env("GENOS_HDR_EXPOSURE_MODE"),
            ev100: env("GENOS_HDR_EV100"),
            compensation: env("GENOS_HDR_COMPENSATION"),
            iso: env("GENOS_HDR_ISO"),
            aperture: env("GENOS_HDR_APERTURE"),
            shutter: env("GENOS_HDR_SHUTTER"),
            adapt_down: env("GENOS_HDR_ADAPT_DOWN"),
            adapt_up: env("GENOS_HDR_ADAPT_UP"),
            adapt_strength: env("GENOS_HDR_ADAPT_STRENGTH"),
            highlight_percentile: env("GENOS_HDR_HIGHLIGHT_PERCENTILE"),
            highlight_stops: env("GENOS_HDR_HIGHLIGHT_STOPS"),
            display: env("GENOS_HDR_DISPLAY"),
            bloom: env("GENOS_HDR_BLOOM"),
            bloom_intensity: env("GENOS_HDR_BLOOM_INTENSITY"),
            bloom_levels: env("GENOS_HDR_BLOOM_LEVELS"),
            output: env("GENOS_HDR_OUTPUT"),
            transition: env("GENOS_HDR_TRANSITION"),
        }))
    }

    #[must_use]
    pub fn resolve(&self, cli: &HdrCli, env: &HdrEnv) -> Self {
        let mut out = self.clone();
        if let Some(mode) = cli
            .exposure_mode
            .or_else(|| env.exposure_mode.as_deref().and_then(|v| parse_exposure(v).ok()))
        {
            out.exposure_mode = mode;
            out.legacy_bytes = false;
        }
        if let Some(value) = cli.ev100.or_else(|| parse_opt_f64(env.ev100.as_deref())) {
            out.ev100 = value;
            out.legacy_bytes = false;
        }
        if let Some(value) = cli
            .compensation
            .or_else(|| parse_opt_f64(env.compensation.as_deref()))
        {
            out.compensation = value;
            out.legacy_bytes = false;
        }
        if let Some(value) = cli.iso.or_else(|| parse_opt_f64(env.iso.as_deref())) {
            out.iso = value;
            out.legacy_bytes = false;
        }
        if let Some(value) = cli
            .aperture
            .or_else(|| parse_opt_f64(env.aperture.as_deref()))
        {
            out.aperture = value;
            out.legacy_bytes = false;
        }
        if let Some(value) = cli
            .shutter
            .or_else(|| parse_opt_f64(env.shutter.as_deref()))
        {
            out.shutter = value;
            out.legacy_bytes = false;
        }
        if let Some(value) = cli
            .adapt_down
            .or_else(|| parse_opt_f64(env.adapt_down.as_deref()))
        {
            out.adapt_down = value;
        }
        if let Some(value) = cli
            .adapt_up
            .or_else(|| parse_opt_f64(env.adapt_up.as_deref()))
        {
            out.adapt_up = value;
        }
        if let Some(value) = cli.adapt_strength.or_else(|| {
            env.adapt_strength
                .as_deref()
                .and_then(|v| parse_share("adapt_strength", v).ok())
        }) {
            out.adapt_strength = value;
        }
        if let Some(value) = cli.highlight_percentile.or_else(|| {
            env.highlight_percentile
                .as_deref()
                .and_then(|v| parse_percentile(v).ok())
        }) {
            out.highlight_percentile = value;
        }
        if let Some(value) = cli
            .highlight_stops
            .or_else(|| env.highlight_stops.as_deref().and_then(|v| parse_stops(v).ok()))
        {
            out.highlight_stops = value;
        }
        if let Some(value) = cli
            .display
            .or_else(|| env.display.as_deref().and_then(|v| parse_display(v).ok()))
        {
            out.display = value;
        }
        if let Some(value) = cli
            .bloom
            .or_else(|| env.bloom.as_deref().and_then(|v| parse_on(v).ok()))
        {
            out.bloom = value;
        }
        if let Some(value) = cli
            .bloom_intensity
            .or_else(|| parse_opt_f64(env.bloom_intensity.as_deref()))
        {
            out.bloom_intensity = value;
        }
        if let Some(value) = cli
            .bloom_levels
            .or_else(|| env.bloom_levels.as_deref().and_then(|v| parse_u32("bloom_levels", v).ok()))
        {
            out.bloom_levels = value;
        }
        if let Some(value) = cli
            .output
            .or_else(|| env.output.as_deref().and_then(|v| parse_output(v).ok()))
        {
            out.output = value;
        }
        if let Some(value) = cli.transition.or_else(|| {
            env.transition
                .as_deref()
                .and_then(|v| parse_seconds("transition", v).ok())
        }) {
            out.transition = value;
        }
        out
    }

    pub fn to_text(&self) -> String {
        format!(
            "exposure_mode={}\nev100={}\ncompensation={}\niso={}\naperture={}\nshutter={}\nadapt_down={}\nadapt_up={}\nadapt_strength={}\nhighlight_percentile={}\nhighlight_stops={}\nhistogram_bins={}\ndisplay={}\nbloom={}\nbloom_intensity={}\nbloom_levels={}\noutput={}\ntransition={}\nlegacy_bytes={}\n",
            exposure_name(self.exposure_mode),
            self.ev100,
            self.compensation,
            self.iso,
            self.aperture,
            self.shutter,
            self.adapt_down,
            self.adapt_up,
            self.adapt_strength,
            self.highlight_percentile,
            self.highlight_stops,
            self.histogram_bins,
            display_name(self.display),
            if self.bloom { "on" } else { "off" },
            self.bloom_intensity,
            self.bloom_levels,
            output_name(self.output),
            self.transition,
            if self.legacy_bytes { "on" } else { "off" },
        )
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::legacy_bytes();
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
        match key.trim() {
            "exposure_mode" => {
                self.exposure_mode = parse_exposure(value)?;
                self.legacy_bytes = false;
            }
            "ev100" => {
                self.ev100 = parse_f64("ev100", value)?;
                self.legacy_bytes = false;
            }
            "compensation" => self.compensation = parse_f64("compensation", value)?,
            "iso" => self.iso = parse_f64("iso", value)?,
            "aperture" => self.aperture = parse_f64("aperture", value)?,
            "shutter" => self.shutter = parse_f64("shutter", value)?,
            "adapt_down" => self.adapt_down = parse_f64("adapt_down", value)?,
            "adapt_up" => self.adapt_up = parse_f64("adapt_up", value)?,
            "adapt_strength" => self.adapt_strength = parse_share("adapt_strength", value)?,
            "highlight_percentile" => self.highlight_percentile = parse_percentile(value)?,
            "highlight_stops" => self.highlight_stops = parse_stops(value)?,
            "histogram_bins" => self.histogram_bins = parse_u32("histogram_bins", value)?,
            "display" => self.display = parse_display(value)?,
            "bloom" => self.bloom = parse_on(value)?,
            "bloom_intensity" => self.bloom_intensity = parse_f64("bloom_intensity", value)?,
            "bloom_levels" => self.bloom_levels = parse_u32("bloom_levels", value)?,
            "output" => self.output = parse_output(value)?,
            "transition" => self.transition = parse_seconds("transition", value)?,
            "legacy_bytes" => self.legacy_bytes = parse_on(value)?,
            other => return Err(format!("unknown hdr setting {other}")),
        }
        Ok(())
    }
}

/// The keys a scene file stores as its look, in the order they are written.
/// `output` is the machine's choice. `histogram_bins` follows the preset.
/// `legacy_bytes` is internal.
pub const HDR_LOOK_KEYS: [&str; 16] = [
    "exposure_mode",
    "ev100",
    "compensation",
    "iso",
    "aperture",
    "shutter",
    "adapt_down",
    "adapt_up",
    "adapt_strength",
    "highlight_percentile",
    "highlight_stops",
    "display",
    "bloom",
    "bloom_intensity",
    "bloom_levels",
    "transition",
];

impl HdrSettings {
    /// `(key, value)` for each of [`HDR_LOOK_KEYS`]. [`HdrSettings::set_key`]
    /// reads every pair back to the same value.
    pub fn look_pairs(&self) -> Vec<(String, String)> {
        let text = self.to_text();
        HDR_LOOK_KEYS
            .iter()
            .filter_map(|key| {
                let value = text
                    .lines()
                    .find_map(|line| line.split_once('=').filter(|(name, _)| name == key))?
                    .1;
                Some(((*key).to_string(), value.to_string()))
            })
            .collect()
    }
}

/// Environment mirrors. Empty means the variable was unset.
#[derive(Clone, Debug, Default)]
pub struct HdrEnv {
    pub exposure_mode: Option<String>,
    pub ev100: Option<String>,
    pub compensation: Option<String>,
    pub iso: Option<String>,
    pub aperture: Option<String>,
    pub shutter: Option<String>,
    pub adapt_down: Option<String>,
    pub adapt_up: Option<String>,
    pub adapt_strength: Option<String>,
    pub highlight_percentile: Option<String>,
    pub highlight_stops: Option<String>,
    pub display: Option<String>,
    pub bloom: Option<String>,
    pub bloom_intensity: Option<String>,
    pub bloom_levels: Option<String>,
    pub output: Option<String>,
    pub transition: Option<String>,
}

impl EngineSettings for HdrSettings {
    fn to_text(&self) -> String {
        HdrSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        HdrSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        HdrSettings::set_key(self, key, value)
    }
}

fn parse_opt_f64(value: Option<&str>) -> Option<f64> {
    value.and_then(|value| parse_f64("number", value).ok())
}

fn parse_f64(name: &str, value: &str) -> Result<f64, String> {
    let number: f64 = value
        .trim()
        .parse()
        .map_err(|_| format!("{name} needs a number, got {value}"))?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(format!("{name} needs a finite number, got {value}"))
    }
}

/// Auto exposure's default strength in the auto presets (`adapt_strength`).
pub const ADAPT_STRENGTH: f64 = 0.3;
/// The default highlight percentile (`highlight_percentile`).
pub const HIGHLIGHT_PERCENTILE: f64 = 0.98;
/// The default highlight key in stops above mid-grey (`highlight_stops`).
pub const HIGHLIGHT_STOPS: f64 = 3.0;

/// A share from 0 to 1.
fn parse_share(name: &str, value: &str) -> Result<f64, String> {
    let share = parse_f64(name, value)?;
    if !(0.0..=1.0).contains(&share) {
        return Err(format!("{name} needs 0 to 1, got {value}"));
    }
    Ok(share)
}

fn parse_percentile(value: &str) -> Result<f64, String> {
    let share = parse_f64("highlight_percentile", value)?;
    if !(0.5..=1.0).contains(&share) {
        return Err(format!("highlight_percentile needs 0.5 to 1, got {value}"));
    }
    Ok(share)
}

fn parse_stops(value: &str) -> Result<f64, String> {
    let stops = parse_f64("highlight_stops", value)?;
    if !(0.5..=10.0).contains(&stops) {
        return Err(format!("highlight_stops needs 0.5 to 10, got {value}"));
    }
    Ok(stops)
}

/// A length of time. Negative seconds are refused.
fn parse_seconds(name: &str, value: &str) -> Result<f64, String> {
    let seconds = parse_f64(name, value)?;
    if seconds < 0.0 {
        return Err(format!("{name} needs 0 or more seconds, got {value}"));
    }
    Ok(seconds)
}

fn parse_u32(name: &str, value: &str) -> Result<u32, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{name} needs a whole number, got {value}"))
}

fn parse_on(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(format!("expected on or off, got {value}")),
    }
}

fn parse_exposure(value: &str) -> Result<ExposureMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ExposureMode::Auto),
        "camera" => Ok(ExposureMode::Camera),
        "fixed" => Ok(ExposureMode::Fixed),
        _ => Err(format!("exposure mode is auto, camera, or fixed, got {value}")),
    }
}

fn parse_display(value: &str) -> Result<DisplayTransform, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "agx" => Ok(DisplayTransform::Agx),
        "aces" => Ok(DisplayTransform::Aces),
        "legacy" => Ok(DisplayTransform::Legacy),
        _ => Err(format!("display is agx, aces, or legacy, got {value}")),
    }
}

fn parse_output(value: &str) -> Result<OutputMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(OutputMode::Auto),
        "sdr" => Ok(OutputMode::Sdr),
        "hdr10" => Ok(OutputMode::Hdr10),
        "scrgb" => Ok(OutputMode::Scrgb),
        _ => Err(format!("output is auto, sdr, hdr10, or scrgb, got {value}")),
    }
}

fn exposure_name(mode: ExposureMode) -> &'static str {
    match mode {
        ExposureMode::Auto => "auto",
        ExposureMode::Camera => "camera",
        ExposureMode::Fixed => "fixed",
    }
}

fn display_name(mode: DisplayTransform) -> &'static str {
    match mode {
        DisplayTransform::Agx => "agx",
        DisplayTransform::Aces => "aces",
        DisplayTransform::Legacy => "legacy",
    }
}

fn output_name(mode: OutputMode) -> &'static str {
    match mode {
        OutputMode::Auto => "auto",
        OutputMode::Sdr => "sdr",
        OutputMode::Hdr10 => "hdr10",
        OutputMode::Scrgb => "scrgb",
    }
}

/// `Some` when `arg` is an HDR flag. The bool is true when the value is a separate argument.
///
/// `--display` is the Wayland display name on the lighting binary. The transform
/// flag is `--hdr-display`. A `--display` value of `agx`, `aces`, or `legacy`
/// still selects the transform.
#[must_use]
pub fn hdr_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, inline) = arg
        .split_once('=')
        .map_or((arg, None), |(name, value)| (name, Some(value)));
    let known = matches!(
        name,
        "--exposure-mode"
            | "--ev100"
            | "--compensation"
            | "--iso"
            | "--aperture"
            | "--shutter"
            | "--adapt-down"
            | "--adapt-up"
            | "--adapt-strength"
            | "--highlight-percentile"
            | "--highlight-stops"
            | "--hdr-display"
            | "--bloom"
            | "--bloom-intensity"
            | "--bloom-levels"
            | "--output"
            | "--hdr-transition"
    );
    known.then_some(inline.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_meters_and_fallback_stays_sdr() {
        let high = HdrSettings::for_preset(LightingPreset::High);
        assert!(matches!(high.exposure_mode, ExposureMode::Auto));
        assert!(!high.legacy_bytes);
        assert!(matches!(high.display, DisplayTransform::Agx));
        assert!(matches!(high.output, OutputMode::Auto));
        assert!(high.bloom);
        assert_eq!(high.bloom_intensity.to_bits(), 0.05_f64.to_bits());
        assert_eq!(high.bloom_levels, 6);
        assert_eq!(HdrSettings::ultra().bloom_levels, 7);
        let low = HdrSettings::for_preset(LightingPreset::Low);
        assert!(low.bloom);
        assert_eq!(low.bloom_intensity.to_bits(), 0.05_f64.to_bits());
        assert!(matches!(low.output, OutputMode::Sdr));
        let fallback = HdrSettings::for_preset(LightingPreset::Fallback);
        assert!(matches!(fallback.exposure_mode, ExposureMode::Fixed));
        assert!(matches!(fallback.display, DisplayTransform::Agx));
        assert!(matches!(fallback.output, OutputMode::Sdr));
        assert!(!fallback.bloom);
        assert_eq!(fallback.bloom_intensity.to_bits(), 0.0_f64.to_bits());
        assert_eq!(fallback.bloom_levels, 0);
    }

    #[test]
    fn hdr_display_is_not_the_wayland_flag() {
        let args = vec![
            "--hdr-display".to_string(),
            "legacy".to_string(),
            "--exposure-mode".to_string(),
            "fixed".to_string(),
            "--display".to_string(),
            "wayland-1".to_string(),
        ];
        let cli = HdrSettings::cli_from(&args).unwrap_or_else(|err| panic!("{err}"));
        assert!(matches!(cli.display, Some(DisplayTransform::Legacy)));
        assert!(matches!(cli.exposure_mode, Some(ExposureMode::Fixed)));
        assert_eq!(
            hdr_flag_needs_value("--exposure-mode"),
            Some(true)
        );
        assert_eq!(hdr_flag_needs_value("--bloom=off"), Some(false));
        assert_eq!(hdr_flag_needs_value("--display"), None);
        assert_eq!(hdr_flag_needs_value("--adapt-down"), Some(true));
        assert_eq!(hdr_flag_needs_value("--adapt-up=0.8"), Some(false));
    }

    #[test]
    fn adapt_argument_wins_over_the_environment() {
        let stored = HdrSettings::legacy_bytes();
        let from_env = stored.resolve(
            &HdrCli::default(),
            &HdrEnv {
                adapt_down: Some("0.20".into()),
                adapt_up: Some("1.25".into()),
                ..HdrEnv::default()
            },
        );
        assert!(from_env.legacy_bytes);
        assert_eq!(from_env.adapt_down.to_bits(), 0.20_f64.to_bits());
        assert_eq!(from_env.adapt_up.to_bits(), 1.25_f64.to_bits());
        let from_cli = stored.resolve(
            &HdrCli {
                adapt_down: Some(0.90),
                ..HdrCli::default()
            },
            &HdrEnv {
                adapt_down: Some("0.20".into()),
                adapt_up: Some("1.25".into()),
                ..HdrEnv::default()
            },
        );
        assert_eq!(from_cli.adapt_down.to_bits(), 0.90_f64.to_bits());
        assert_eq!(from_cli.adapt_up.to_bits(), 1.25_f64.to_bits());
        assert_eq!(HDR_ENV_MIRRORS.len(), 15);
        assert!(HDR_ENV_MIRRORS.contains(&"GENOS_HDR_ADAPT_DOWN"));
        assert!(HDR_ENV_MIRRORS.contains(&"GENOS_HDR_ADAPT_UP"));
        assert!(HDR_ENV_MIRRORS.contains(&"GENOS_HDR_OUT"));
    }

    #[test]
    fn transition_survives_the_text_and_the_argument_wins() {
        let mut high = HdrSettings::for_preset(LightingPreset::High);
        assert_eq!(high.transition.to_bits(), 0.0_f64.to_bits());
        assert_eq!(HdrSettings::legacy_bytes().transition.to_bits(), 0.0_f64.to_bits());
        high.set_key("transition", "1.5").unwrap_or_else(|err| panic!("{err}"));
        high.compensation = -1.25;
        let back = HdrSettings::parse(&high.to_text()).unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(back, high);
        assert!(high.set_key("transition", "-1").is_err());
        assert!(high.set_key("transition", "soon").is_err());

        let args = vec!["--hdr-transition".to_string(), "2".to_string()];
        let cli = HdrSettings::cli_from(&args).unwrap_or_else(|err| panic!("{err}"));
        let env = HdrEnv {
            transition: Some("0.5".into()),
            ..HdrEnv::default()
        };
        assert_eq!(high.resolve(&cli, &env).transition.to_bits(), 2.0_f64.to_bits());
        assert_eq!(
            high.resolve(&HdrCli::default(), &env).transition.to_bits(),
            0.5_f64.to_bits()
        );
        assert_eq!(hdr_flag_needs_value("--hdr-transition"), Some(true));
        assert!(HDR_ENV_MIRRORS.contains(&"GENOS_HDR_TRANSITION"));
    }

    #[test]
    fn the_look_pairs_read_back_and_leave_the_machine_keys_out() {
        let mut look = HdrSettings::for_preset(LightingPreset::Medium);
        look.compensation = -1.0;
        look.bloom_intensity = 0.02;
        look.transition = 0.75;
        let pairs = look.look_pairs();
        let keys: Vec<&str> = pairs.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, HDR_LOOK_KEYS);
        // On another preset and another output, the pairs bring the look and
        // nothing else.
        let mut other = HdrSettings::for_preset(LightingPreset::Low);
        for (key, value) in &pairs {
            other.set_key(key, value).unwrap_or_else(|err| panic!("{err}"));
        }
        assert_eq!(other.output, OutputMode::Sdr);
        assert_eq!(other.histogram_bins, 64);
        other.output = look.output;
        other.histogram_bins = look.histogram_bins;
        assert_eq!(other, look);
    }
}
