//! UI layout and pointer behavior.
//!
//! A tree resolves to rectangles with no window and no Vulkan. A screen root
//! stays on the viewport. A world root follows a world point through the camera.
//! Children in the flow use Clay sizing. A child with a [`Place`] stays out of
//! that flow. The lighting frame draws a sun on the first lamp and moves that
//! lamp with notched sliders. On Omarchy, the lighting panel uses the current
//! theme colors.

mod layout;
pub mod omarchy;
mod panel;
mod profile;
mod text;

/// Idle, hover, and pressed colors. Pointer state picks one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub background: [f32; 3],
    pub border: [f32; 3],
}

/// A control the lighting panel can fire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Set the first lamp on axis 0, 1, or 2 (`x`, `y`, `z`) to `position`.
    SetLamp { axis: u8, position: f32 },
    /// Multiply `Light.color`. Brightness is the mean of that color.
    ScaleIntensity(f32),
}

pub use layout::{layout, Align, Direction, Node, Pad, Place, Rect, Sizing, Space};
pub use panel::{apply_lamp, lighting_frame, Frame, Paint, Pointer, Shown, State};
pub use profile::{
    profile_overlay, profiler_enabled, FrameSample, OpenFrame, ProfileLine, ProfilePoint,
    ProfileStream, ProfileView, StageSample, DRAW_STAGE, STAGE_COUNT, STAGE_LABELS,
};

/// Ids for the lighting panel the camera frame submits.
pub mod id {
    pub const PANEL: u32 = 1;
    pub const TITLE: u32 = 2;
    /// Notched slider for the lamp X position.
    pub const X: u32 = 3;
    /// Notched slider for the lamp Y position.
    pub const Y: u32 = 4;
    /// Notched slider for the lamp Z position.
    pub const Z: u32 = 5;
    pub const DIM: u32 = 9;
    pub const BRIGHT: u32 = 10;
    /// World-space sun on the first lamp. It is not a control.
    pub const SUN: u32 = 11;
}
