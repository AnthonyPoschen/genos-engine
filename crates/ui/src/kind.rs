//! Retained control kinds. The game owns the values and copies them each frame.

/// What a [`crate::Node`] does. `Box` is the default, so an old tree stays a box.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Kind {
    #[default]
    Box,
    Label,
    Button {
        enabled: bool,
    },
    Toggle {
        on: bool,
    },
    Checkbox {
        checked: bool,
    },
    Radio {
        group: u32,
        selected: bool,
    },
    Slider {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        vertical: bool,
    },
    Progress {
        value: f32,
        radial: bool,
    },
}

/// A change a kind emits. `Action` stays the lighting-panel enum.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Press {
        id: u32,
    },
    Toggle {
        id: u32,
        on: bool,
    },
    Change {
        id: u32,
        value: f64,
    },
    Select {
        id: u32,
        index: u32,
    },
    /// The open popup or dialog closed. `id` is the layer root.
    Dismiss {
        id: u32,
    },
}
