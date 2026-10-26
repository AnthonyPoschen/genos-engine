#![expect(
    clippy::missing_errors_doc,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! Longest stretch the frame thread went without pumping the window.
//!
//! The compositor sends `xdg_wm_base.ping` while a window is mapped. The ping is
//! answered inside `Window::pump`. A gap longer than about 100 ms is the stall
//! Omarchy reports as "Application not responding" once its timer fires. The
//! frame writes `longest_stall_ms`. A game, MCP, or the debug menu can read it,
//! and can write `0` to clear the high-water mark.

use crate::EngineSettings;

/// The pump-gap high-water mark, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventLoopSettings {
    /// Longest gap between seat dispatches since the last clear.
    pub longest_stall_ms: f64,
}

impl Default for EventLoopSettings {
    fn default() -> Self {
        Self {
            longest_stall_ms: 0.0,
        }
    }
}

impl EventLoopSettings {
    /// The budget the frame loop keeps. A longer gap misses a compositor ping.
    pub const BUDGET_MS: f64 = 100.0;

    /// Keep the larger of the stored mark and `stall_ms`.
    pub fn note(&mut self, stall_ms: f64) {
        if stall_ms.is_finite() && stall_ms > self.longest_stall_ms {
            self.longest_stall_ms = stall_ms;
        }
    }

    pub fn to_text(&self) -> String {
        format!("longest_stall_ms={}\n", ms_text(self.longest_stall_ms))
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
            "longest_stall_ms" | "stall_ms" => {
                let ms = value.parse::<f64>().map_err(|_| {
                    format!("longest_stall_ms is a number of milliseconds, got {value}")
                })?;
                if !ms.is_finite() || ms < 0.0 {
                    return Err(format!("longest_stall_ms is zero or more, got {value}"));
                }
                self.longest_stall_ms = ms;
                Ok(())
            }
            _ => Err(format!("unknown event-loop setting {key}")),
        }
    }
}

impl EngineSettings for EventLoopSettings {
    fn to_text(&self) -> String {
        EventLoopSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        EventLoopSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        EventLoopSettings::set_key(self, key, value)
    }
}

fn ms_text(ms: f64) -> String {
    if ms.fract() == 0.0 {
        format!("{ms:.0}")
    } else {
        format!("{ms:.3}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_round_trips_and_a_clear_writes_zero() {
        let mut settings = EventLoopSettings::default();
        settings.note(12.5);
        settings.note(4.0);
        assert_eq!(settings.longest_stall_ms, 12.5);
        let parsed = EventLoopSettings::parse(&settings.to_text()).unwrap();
        assert_eq!(parsed.longest_stall_ms, 12.5);
        settings.set_key("longest_stall_ms", "0").unwrap();
        assert_eq!(settings.longest_stall_ms, 0.0);
        assert_eq!(
            EventLoopSettings::parse("stall_ms=3")
                .unwrap()
                .longest_stall_ms,
            3.0
        );
    }

    #[test]
    fn a_negative_stall_is_refused() {
        let err = EventLoopSettings::default()
            .set_key("longest_stall_ms", "-1")
            .unwrap_err();
        assert!(err.contains("zero or more"));
    }
}
