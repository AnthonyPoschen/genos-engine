//! `JobSettings`: workers, serial or parallel, the Must budget, and Slack.
//!
//! The text form is the same contract as `EngineSettings` (`to_text`, `parse`,
//! `set_key`). The struct lives here, not on `Scene`, until the scene migration
//! adds the field. Adding it earlier would rewrite every `Scene` literal.

use crate::Order;

/// Scheduler settings. Low is the 1060-class ceiling: 3 workers, 6 ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JobSettings {
    /// Worker threads. The frame thread is not included. `0` forces serial.
    pub workers: u32,
    /// Serial is the server and the replay. Parallel must hash-equal serial.
    pub order: Order,
    /// Must-scope ceiling in milliseconds. Must work still runs past it.
    pub frame_budget_ms: f32,
    /// `false` runs no Slack.
    pub slack: bool,
}

impl JobSettings {
    /// 3 workers, parallel, 6 ms, Slack on.
    #[must_use]
    pub const fn low() -> Self {
        Self {
            workers: 3,
            order: Order::Parallel,
            frame_budget_ms: 6.0,
            slack: true,
        }
    }

    /// Serial, no worker threads. The headless server tick.
    #[must_use]
    pub const fn server() -> Self {
        Self {
            workers: 0,
            order: Order::Serial,
            frame_budget_ms: 6.0,
            slack: false,
        }
    }

    /// `workers`, `order`, `frame_budget_ms`, and `slack`, one key per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        let order = match self.order {
            Order::Serial => "serial",
            Order::Parallel => "parallel",
        };
        let slack = if self.slack { "on" } else { "off" };
        format!(
            "workers={}\norder={order}\nframe_budget_ms={}\nslack={slack}\n",
            self.workers, self.frame_budget_ms
        )
    }

    /// Inverse of [`Self::to_text`].
    ///
    /// # Errors
    ///
    /// A missing key, an unknown key, or a value that does not parse.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::server();
        let mut seen = [false; 4];
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("expected key=value, got {line}"));
            };
            let index = key_index(key)?;
            if seen[index] {
                return Err(format!("duplicate key {key}"));
            }
            seen[index] = true;
            settings.set_key(key, value)?;
        }
        if seen.iter().any(|flag| !flag) {
            return Err("workers, order, frame_budget_ms, and slack are required".into());
        }
        Ok(settings)
    }

    /// Overlay one key. Unknown keys fail.
    ///
    /// # Errors
    ///
    /// An unknown key, or a value that does not parse.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "workers" => {
                self.workers = value
                    .parse()
                    .map_err(|_| format!("workers needs an integer, got {value}"))?;
            }
            "order" => {
                self.order = match value {
                    "serial" => Order::Serial,
                    "parallel" => Order::Parallel,
                    _ => return Err(format!("order is serial or parallel, got {value}")),
                };
            }
            "frame_budget_ms" => {
                let budget: f32 = value
                    .parse()
                    .map_err(|_| format!("frame_budget_ms needs a number, got {value}"))?;
                if !budget.is_finite() || budget < 0.0 {
                    return Err("frame_budget_ms must be finite and >= 0".into());
                }
                self.frame_budget_ms = budget;
            }
            "slack" => {
                self.slack = match value {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("slack is on or off, got {value}")),
                };
            }
            _ => return Err(format!("unknown job setting {key}")),
        }
        Ok(())
    }
}

fn key_index(key: &str) -> Result<usize, String> {
    match key {
        "workers" => Ok(0),
        "order" => Ok(1),
        "frame_budget_ms" => Ok(2),
        "slack" => Ok(3),
        _ => Err(format!("unknown job setting {key}")),
    }
}
