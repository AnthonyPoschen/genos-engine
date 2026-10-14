#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

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

/// A named camera pose a scene file can jump to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckpointPose {
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// Whether a scene load has installed its world yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadPace {
    /// The worker is still reading the scene. Keep pumping the window.
    Pending,
    /// The scene the next draw uses is in place.
    Ready,
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

    /// Scene names this host can load. Empty when the host has one scene.
    fn list_scenes(&self) -> Vec<String> {
        Vec::new()
    }

    /// Replace the scene. The tools drop their edits and clear the lighting cache.
    fn load_scene(&mut self, name: &str) -> Result<(), String> {
        Err(format!("this host has no scene {name}"))
    }

    /// Start a scene load. [`LoadPace::Pending`] means the frame keeps pumping
    /// and calls [`Host::poll_load`] until the scene is in place.
    fn begin_load(&mut self, name: &str) -> Result<LoadPace, String> {
        self.load_scene(name)?;
        Ok(LoadPace::Ready)
    }

    /// [`LoadPace::Ready`] when no load is running, or when the worker has installed
    /// the scene.
    fn poll_load(&mut self) -> Result<LoadPace, String> {
        Ok(LoadPace::Ready)
    }

    /// Checkpoint names for the scene that is loaded.
    fn list_checkpoints(&self) -> Vec<String> {
        Vec::new()
    }

    /// Remember `name` as the reset pose and return it. The tools move the camera.
    fn jump_checkpoint(&mut self, name: &str) -> Result<CheckpointPose, String> {
        Err(format!("this host has no checkpoint {name}"))
    }
}

/// A bare scene: no knobs of its own.
impl Host for genos_scene::Scene {
    fn scene(&mut self) -> &mut genos_scene::Scene {
        self
    }
}
