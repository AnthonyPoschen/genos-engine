//! Where a debug tool writes its files.
//!
//! The default is a directory under the process working directory. `--out`
//! overrides it. `GENOS_UNFILLED_DIR` is the environment mirror of the same
//! field. The rule is `AGENTS.md` (Output) and `docs/engine/settings-principles.md`.

use crate::EngineSettings;

/// Unfilled events and the other debug files that share this directory.
///
/// `out` is the unfilled root. Each event is `<out>/<timestamp>/`. A relative
/// path is under the working directory. The process does not choose `$HOME`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugSettings {
    pub out: String,
}

impl Default for DebugSettings {
    fn default() -> Self {
        Self {
            out: Self::DEFAULT_OUT.into(),
        }
    }
}

impl DebugSettings {
    /// `./genos-debug/unfilled/<timestamp>/` when the process starts in this directory.
    pub const DEFAULT_OUT: &'static str = "genos-debug/unfilled";

    /// `--out` wins, then `GENOS_UNFILLED_DIR`, then the stored field.
    /// Empty strings are skipped. The result is never empty.
    pub fn resolve(setting: &str, env: Option<&str>, cli: Option<&str>) -> String {
        for candidate in [cli, env, Some(setting)] {
            let Some(dir) = candidate.map(str::trim).filter(|dir| !dir.is_empty()) else {
                continue;
            };
            return dir.to_string();
        }
        Self::DEFAULT_OUT.to_string()
    }

    /// The directory this process will write. `--out` on the command line wins
    /// over the environment mirror, which wins over `self.out`.
    pub fn effective(&self, cli: Option<&str>) -> Self {
        let env = std::env::var("GENOS_UNFILLED_DIR").ok();
        Self {
            out: Self::resolve(&self.out, env.as_deref(), cli),
        }
    }

    /// The first `--out <dir>` or `--out=<dir>` in the process arguments.
    pub fn cli_out() -> Option<String> {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if let Some(dir) = arg.strip_prefix("--out=") {
                let dir = dir.trim();
                if !dir.is_empty() {
                    return Some(dir.to_string());
                }
                continue;
            }
            if arg == "--out" {
                let Some(dir) = args.next() else {
                    return None;
                };
                let dir = dir.trim();
                if !dir.is_empty() {
                    return Some(dir.to_string());
                }
            }
        }
        None
    }

    pub fn to_text(&self) -> String {
        format!("out={}\n", self.out)
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
            "out" => {
                if value.is_empty() || value.contains(['\n', '\r']) {
                    return Err("out needs a directory".into());
                }
                self.out = value.to_string();
                Ok(())
            }
            _ => Err(format!("unknown debug setting {key}")),
        }
    }
}

impl EngineSettings for DebugSettings {
    fn to_text(&self) -> String {
        DebugSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        DebugSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        DebugSettings::set_key(self, key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_out_is_under_the_working_directory() {
        assert_eq!(DebugSettings::default().out, "genos-debug/unfilled");
        assert_eq!(
            DebugSettings::resolve("genos-debug/unfilled", None, None),
            "genos-debug/unfilled"
        );
        assert!(
            !DebugSettings::DEFAULT_OUT.contains("HOME")
                && !DebugSettings::default().out.starts_with('/'),
            "the default is a relative directory, not a home path"
        );
    }

    #[test]
    fn cli_overrides_the_env_mirror_and_the_setting() {
        assert_eq!(
            DebugSettings::resolve(
                "genos-debug/unfilled",
                Some("/home/a/genos-perf/unfilled"),
                Some("./captures")
            ),
            "./captures"
        );
        assert_eq!(
            DebugSettings::resolve("genos-debug/unfilled", Some("/tmp/events"), None),
            "/tmp/events"
        );
        assert_eq!(
            DebugSettings::resolve("custom/dir", Some("  "), Some("")),
            "custom/dir"
        );
        assert_eq!(DebugSettings::resolve("", None, None), "genos-debug/unfilled");
    }

    #[test]
    fn text_form_round_trips() {
        let mut settings = DebugSettings::default();
        settings.set_key("out", "genos-debug/unfilled").unwrap();
        let parsed = DebugSettings::parse(&settings.to_text()).unwrap();
        assert_eq!(parsed, settings);
        settings.set_key("out", "/tmp/a=b").unwrap();
        assert_eq!(
            DebugSettings::parse("out=/tmp/a=b\n").unwrap().out,
            "/tmp/a=b"
        );
        assert!(DebugSettings::default().set_key("out", "").is_err());
        assert!(DebugSettings::default().set_key("home", "/tmp").is_err());
    }
}
