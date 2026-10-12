//! Named numbers a tool, a script or a slider can read and set.
//!
//! An example publishes its own (time of day, lamp count, ...) through [`Knobs`].

/// One setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    /// Key for scripts and tools, e.g. `day` or `lamps`.
    pub name: String,
    /// Label for a panel.
    pub label: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    /// Values snap to multiples of this from `min`; 0 is continuous.
    pub step: f64,
    /// Names of the values `min`, `min + step`, ... for a choice; empty for a number.
    pub choices: Vec<String>,
    /// The value as a panel shows it, e.g. `23.9 h`.
    pub text: String,
}

impl Knob {
    /// A number with `decimals` places and a unit suffix.
    pub fn number(
        name: &str,
        label: &str,
        value: f64,
        range: (f64, f64),
        step: f64,
        unit: &str,
    ) -> Self {
        let decimals = if step >= 1.0 {
            0
        } else if step >= 0.1 || step == 0.0 && range.1 - range.0 >= 10.0 {
            1
        } else if step >= 0.01 || step == 0.0 {
            2
        } else {
            3
        };
        Self {
            name: name.into(),
            label: label.into(),
            value,
            min: range.0,
            max: range.1,
            step,
            choices: Vec::new(),
            text: format!("{value:.decimals$}{unit}"),
        }
    }

    /// A choice between named values 0, 1, 2, ...
    pub fn choice(name: &str, label: &str, index: usize, choices: &[&str]) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            value: index as f64,
            min: 0.0,
            max: choices.len().saturating_sub(1) as f64,
            step: 1.0,
            choices: choices.iter().map(|c| c.to_string()).collect(),
            text: choices.get(index).copied().unwrap_or("?").to_string(),
        }
    }

    /// An on/off switch.
    pub fn switch(name: &str, label: &str, on: bool) -> Self {
        Self::choice(name, label, usize::from(on), &["off", "on"])
    }

    /// `value` clamped to the range and snapped to the step.
    pub fn snap(&self, value: f64) -> f64 {
        let v = value.clamp(self.min, self.max);
        if self.step > 0.0 {
            (self.min + ((v - self.min) / self.step).round() * self.step).clamp(self.min, self.max)
        } else {
            v
        }
    }

    /// A choice by name or a number as text.
    pub fn parse(&self, text: &str) -> Result<f64, String> {
        if let Some(i) = self.choices.iter().position(|c| c == text) {
            return Ok(i as f64);
        }
        text.parse::<f64>()
            .map_err(|_| format!("{} takes a number or one of {:?}", self.name, self.choices))
    }
}

/// What an example lets the tools reach: its scene, and settings of its own.
pub trait Host {
    /// The scene the next draw uses. Edits land after the example's own update.
    fn scene(&mut self) -> &mut genos_scene::Scene;

    /// The example's own knobs (time of day, lamp count, ...).
    fn knobs(&self) -> Vec<Knob> {
        Vec::new()
    }

    /// Set one by name. The value is already snapped to the knob's step.
    fn set_knob(&mut self, name: &str, _value: f64) -> Result<(), String> {
        Err(format!("unknown knob {name}"))
    }
}

/// A bare scene: no knobs of its own.
impl Host for genos_scene::Scene {
    fn scene(&mut self) -> &mut genos_scene::Scene {
        self
    }
}
