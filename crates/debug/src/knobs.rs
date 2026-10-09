//! Named numbers a tool, a script or a slider can read and set.
//!
//! An example publishes its own (time of day, lamp count, ...) through [`Knobs`];
//! the lighting knobs come from the renderer and are the same for every example.

use genos_render::{Bounces, LightingConfig, TierWeights};

/// One setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    /// Key for scripts and tools, e.g. `day` or `lighting.tier_ms`.
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

const BOUNCE_CHOICES: [&str; 4] = ["0", "1", "2", "inf"];

fn bounce_index(b: Bounces) -> usize {
    match b.limit() {
        Some(n) => n as usize,
        None => 3,
    }
}

/// The renderer's lighting settings as knobs, named `lighting.*`.
pub fn lighting_knobs(c: &LightingConfig) -> Vec<Knob> {
    let w = c.weights;
    let mut out = vec![
        Knob::number(
            "lighting.tier_ms",
            "Probe budget",
            c.tier_ms,
            (0.0, 20.0),
            0.1,
            " ms",
        ),
        Knob::number(
            "lighting.near_rays",
            "Near rays",
            c.near_rays.map_or(-1.0, f64::from),
            (-1.0, 32.0),
            1.0,
            "",
        ),
        Knob::number(
            "lighting.view_ms",
            "Blend time",
            c.view_seconds as f64 * 1000.0,
            (0.0, 500.0),
            1.0,
            " ms",
        ),
        Knob::choice(
            "lighting.bounces",
            "Bounces",
            bounce_index(c.bounces),
            &BOUNCE_CHOICES,
        ),
        Knob::number(
            "lighting.notice_band",
            "Notice band",
            c.notice_band as f64,
            (0.001, 0.2),
            0.001,
            "",
        ),
        Knob::number(
            "lighting.spacing",
            "Probe spacing",
            c.spacing.unwrap_or(0.0) as f64,
            (0.0, 4.0),
            0.25,
            " m",
        ),
        Knob::number(
            "lighting.near",
            "Weight near",
            w.near as f64,
            (0.1, 32.0),
            0.1,
            " m",
        ),
        Knob::number(
            "lighting.feed",
            "Weight feed",
            w.feed as f64,
            (0.0, 2.0),
            0.05,
            "",
        ),
        Knob::number(
            "lighting.margin",
            "Weight margin",
            w.margin as f64,
            (0.0, 1.0),
            0.05,
            "",
        ),
        Knob::number(
            "lighting.edge",
            "Weight edge",
            w.edge as f64,
            (0.0, 1.0),
            0.05,
            "",
        ),
        Knob::number(
            "lighting.stale",
            "Weight stale",
            w.stale as f64,
            (0.01, 10.0),
            0.01,
            " s",
        ),
        Knob::number(
            "lighting.far",
            "Far from",
            w.far as f64,
            (1.0, 64.0),
            0.5,
            " m",
        ),
        Knob::number(
            "lighting.far_skip",
            "Far skip",
            w.far_skip as f64,
            (0.0, 0.2),
            0.005,
            "",
        ),
    ];
    if let Some(k) = out.iter_mut().find(|k| k.name == "lighting.near_rays") {
        if c.near_rays.is_none() {
            k.text = "shader".into();
        }
    }
    if let Some(k) = out.iter_mut().find(|k| k.name == "lighting.spacing") {
        if c.spacing.is_none() {
            k.text = "auto".into();
        }
    }
    out
}

/// Set one `lighting.*` knob on `c`.
pub fn set_lighting_knob(c: &mut LightingConfig, name: &str, v: f64) -> Result<(), String> {
    let w: &mut TierWeights = &mut c.weights;
    let f = v as f32;
    match name {
        "lighting.tier_ms" => c.tier_ms = v.max(0.0),
        "lighting.near_rays" => c.near_rays = (v >= 0.0).then_some(v as u32),
        "lighting.view_ms" => c.view_seconds = (f / 1000.0).max(0.0),
        "lighting.bounces" => {
            c.bounces = Bounces::from_limit(if v >= 3.0 { None } else { Some(v as u32) })
        }
        "lighting.notice_band" => c.notice_band = f,
        "lighting.spacing" => c.spacing = (f > 0.0).then_some(f),
        "lighting.near" => w.near = f,
        "lighting.feed" => w.feed = f,
        "lighting.margin" => w.margin = f,
        "lighting.edge" => w.edge = f,
        "lighting.stale" => w.stale = f,
        "lighting.far" => w.far = f,
        "lighting.far_skip" => w.far_skip = f,
        _ => return Err(format!("unknown knob {name}")),
    }
    Ok(())
}
