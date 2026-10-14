//! On-screen frame-rate counter.
//!
//! The value is off until a game, the debug menu, MCP, or `--fps-counter` turns
//! it on. `rate_hz` is how often the readout is replaced. The default is 4 Hz,
//! one average per 250 ms window. `--fps-counter` and `--fps-hz` win.
//! `GENOS_FPS_COUNTER` and `GENOS_FPS_HZ` apply only when that argument was
//! omitted, and then a set variable wins every frame.

use crate::EngineSettings;

/// Whether the frame-rate overlay is drawn, and how often its number changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FpsCounterSettings {
    /// Off draws nothing and does not time frames.
    pub enabled: bool,
    /// Readout updates per second. The window is `1 / rate_hz` seconds.
    pub rate_hz: f32,
}

/// `--fps-counter` and `--fps-hz` from one argument list. Other arguments are ignored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FpsCli {
    pub enabled: Option<bool>,
    pub rate_hz: Option<f32>,
}

impl Default for FpsCounterSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            rate_hz: Self::DEFAULT_RATE_HZ,
        }
    }
}

impl FpsCounterSettings {
    /// Four updates a second: a 250 ms average.
    pub const DEFAULT_RATE_HZ: f32 = 4.0;
    /// Slowest accepted update. One number every four seconds.
    pub const MIN_RATE_HZ: f32 = 0.25;
    /// Fastest accepted update. At 60 fps this is still every other frame.
    pub const MAX_RATE_HZ: f32 = 30.0;

    /// Flags in `args`. A bare `--fps-counter` turns the overlay on.
    /// `--fps-counter off` turns it off. `--fps-hz` needs a number in range.
    pub fn cli_from(args: &[String]) -> Result<FpsCli, String> {
        let mut enabled = None;
        let mut rate_hz = None;
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if let Some(value) = arg.strip_prefix("--fps-counter=") {
                enabled = Some(parse_enabled(value)?);
                index += 1;
                continue;
            }
            if arg == "--fps-counter" {
                if let Some(value) = args.get(index + 1).filter(|value| !value.starts_with('-')) {
                    enabled = Some(parse_enabled(value)?);
                    index += 2;
                } else {
                    enabled = Some(true);
                    index += 1;
                }
                continue;
            }
            if let Some(value) = arg.strip_prefix("--fps-hz=") {
                rate_hz = Some(parse_rate(value)?);
                index += 1;
                continue;
            }
            if arg == "--fps-hz" {
                let value = args.get(index + 1).ok_or("--fps-hz needs a value")?;
                rate_hz = Some(parse_rate(value)?);
                index += 2;
                continue;
            }
            index += 1;
        }
        Ok(FpsCli { enabled, rate_hz })
    }

    /// The process arguments. A bad flag is an error so the binary can exit at startup.
    pub fn cli() -> Result<FpsCli, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::cli_from(&args)
    }

    /// The value the overlay uses.
    ///
    /// A CLI slot wins. When that slot is empty, a set environment variable wins.
    /// Otherwise the field is used. `GENOS_FPS_COUNTER` and `GENOS_FPS_HZ` are read
    /// here, so a set variable wins on the frame it is set.
    pub fn effective(&self) -> Self {
        let cli = Self::cli().unwrap_or(FpsCli {
            enabled: None,
            rate_hz: None,
        });
        let env_enabled = if cli.enabled.is_none() {
            std::env::var("GENOS_FPS_COUNTER").ok()
        } else {
            None
        };
        let env_rate = if cli.rate_hz.is_none() {
            std::env::var("GENOS_FPS_HZ").ok()
        } else {
            None
        };
        self.resolve(
            cli.enabled,
            cli.rate_hz,
            env_enabled.as_deref(),
            env_rate.as_deref(),
        )
    }

    /// Same rule as [`Self::effective`], with the CLI and environment supplied.
    /// An environment string that is empty or not a value is ignored.
    pub fn resolve(
        &self,
        cli_enabled: Option<bool>,
        cli_rate_hz: Option<f32>,
        env_enabled: Option<&str>,
        env_rate_hz: Option<&str>,
    ) -> Self {
        let enabled = if let Some(on) = cli_enabled {
            on
        } else if let Some(on) = env_flag(env_enabled) {
            on
        } else {
            self.enabled
        };
        let rate_hz = if let Some(hz) = cli_rate_hz {
            hz
        } else if let Some(hz) = env_rate(env_rate_hz) {
            hz
        } else {
            self.rate_hz
        };
        Self { enabled, rate_hz }
    }

    pub fn to_text(&self) -> String {
        format!(
            "enabled={}\nrate_hz={}\n",
            if self.enabled { 1 } else { 0 },
            num_text(self.rate_hz)
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
            "enabled" | "fps_counter" => {
                self.enabled = parse_enabled(value)?;
                Ok(())
            }
            "rate_hz" | "fps_hz" => {
                self.rate_hz = parse_rate(value)?;
                Ok(())
            }
            _ => Err(format!("unknown fps counter setting {key}")),
        }
    }
}

impl EngineSettings for FpsCounterSettings {
    fn to_text(&self) -> String {
        FpsCounterSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        FpsCounterSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        FpsCounterSettings::set_key(self, key, value)
    }
}

pub fn parse_enabled(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(format!("enabled is on or off, got {value}")),
    }
}

pub fn parse_rate(value: &str) -> Result<f32, String> {
    let hz: f32 = value
        .trim()
        .parse()
        .map_err(|_| format!("rate_hz needs a number, got {value}"))?;
    if !hz.is_finite()
        || !(FpsCounterSettings::MIN_RATE_HZ..=FpsCounterSettings::MAX_RATE_HZ).contains(&hz)
    {
        return Err(format!(
            "rate_hz is {} to {} Hz, got {value}",
            FpsCounterSettings::MIN_RATE_HZ,
            FpsCounterSettings::MAX_RATE_HZ
        ));
    }
    Ok(hz)
}

fn num_text(value: f32) -> String {
    format!("{value:?}")
}

fn env_flag(value: Option<&str>) -> Option<bool> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_enabled(text).ok()
}

fn env_rate(value: Option<&str>) -> Option<f32> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_rate(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off_at_four_hertz() {
        let settings = FpsCounterSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.rate_hz, 4.0);
        let text = EngineSettings::to_text(&settings);
        assert_eq!(text, "enabled=0\nrate_hz=4.0\n");
        let parsed = <FpsCounterSettings as EngineSettings>::parse(&text).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn toggle_and_rate_round_trip_through_set_key() {
        let mut settings = FpsCounterSettings::default();
        EngineSettings::set_key(&mut settings, "enabled", "on").unwrap();
        EngineSettings::set_key(&mut settings, "rate_hz", "8").unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.rate_hz, 8.0);
        let parsed = FpsCounterSettings::parse(&settings.to_text()).unwrap();
        assert_eq!(parsed, settings);
        settings.set_key("fps_counter", "0").unwrap();
        settings.set_key("fps_hz", "2.5").unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.rate_hz, 2.5);
        assert!(settings.set_key("rate_hz", "0").is_err());
        assert!(settings.set_key("rate_hz", "100").is_err());
        assert!(settings.set_key("home", "1").is_err());
        assert!(FpsCounterSettings::parse("enabled\n").is_err());
    }

    #[test]
    fn cli_wins_over_the_env_mirror_and_the_field() {
        let stored = FpsCounterSettings {
            enabled: false,
            rate_hz: 4.0,
        };
        let locked = stored.resolve(Some(true), Some(10.0), Some("off"), Some("2"));
        assert!(locked.enabled);
        assert_eq!(locked.rate_hz, 10.0);
        let from_env = stored.resolve(None, None, Some("on"), Some("2"));
        assert!(from_env.enabled);
        assert_eq!(from_env.rate_hz, 2.0);
        let ignored = stored.resolve(None, None, Some("maybe"), Some("nope"));
        assert_eq!(ignored, stored);
        let off = stored.resolve(Some(false), None, Some("on"), Some("9"));
        assert!(!off.enabled);
        assert_eq!(off.rate_hz, 9.0);
    }

    #[test]
    fn cli_flags_parse_from_an_argument_list() {
        let on = FpsCounterSettings::cli_from(&[
            "--scene".into(),
            "room".into(),
            "--fps-counter".into(),
            "--fps-hz".into(),
            "8".into(),
        ])
        .unwrap();
        assert_eq!(on.enabled, Some(true));
        assert_eq!(on.rate_hz, Some(8.0));
        let off = FpsCounterSettings::cli_from(&["--fps-counter".into(), "off".into()]).unwrap();
        assert_eq!(off.enabled, Some(false));
        assert!(off.rate_hz.is_none());
        let eq = FpsCounterSettings::cli_from(&["--fps-counter=on".into(), "--fps-hz=1.5".into()])
            .unwrap();
        assert_eq!(eq.enabled, Some(true));
        assert_eq!(eq.rate_hz, Some(1.5));
        assert!(FpsCounterSettings::cli_from(&["--fps-hz".into(), "0".into()]).is_err());
        assert!(FpsCounterSettings::cli_from(&["--fps-counter".into(), "maybe".into()]).is_err());
    }

    #[test]
    fn a_script_sets_the_counter() {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/camera/scene.rhai"
        ));
        let mut source = std::fs::read_to_string(path).unwrap();
        source.push_str("\nfps_counter(\"enabled=1; rate_hz=2\")\n");
        let scene = crate::load_str(&source).unwrap();
        assert!(scene.fps_counter.enabled);
        assert_eq!(scene.fps_counter.rate_hz, 2.0);
        assert!(!crate::load_str(&{
            let mut off = std::fs::read_to_string(path).unwrap();
            off.push_str("\nfps_counter(\"enabled=maybe\")\n");
            off
        })
        .is_ok());
    }
}
