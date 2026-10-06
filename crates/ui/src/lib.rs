//! UI layout and pointer behavior.
//!
//! A tree resolves to rectangles with no window and no Vulkan. A screen root
//! stays on the viewport. A world root follows a world point through the camera.
//! Children in the flow use Clay sizing. A child with a [`Place`] stays out of
//! that flow.

mod layout;
mod panel;
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
    /// Add `delta` to the lamp on axis 0, 1, or 2 (`x`, `y`, `z`).
    MoveLamp { axis: u8, delta: f32 },
    /// Multiply `Light.color`. Brightness is the mean of that color.
    ScaleIntensity(f32),
}

pub use layout::{layout, Align, Direction, Node, Pad, Place, Rect, Sizing, Space};
pub use panel::{apply_lamp, lighting_frame, Frame, Paint, Pointer, Shown, State};

/// Ids for the lighting panel the camera frame submits.
pub mod id {
    pub const PANEL: u32 = 1;
    pub const TITLE: u32 = 2;
    pub const X_NEG: u32 = 3;
    pub const X_POS: u32 = 4;
    pub const Y_NEG: u32 = 5;
    pub const Y_POS: u32 = 6;
    pub const Z_NEG: u32 = 7;
    pub const Z_POS: u32 = 8;
    pub const DIM: u32 = 9;
    pub const BRIGHT: u32 = 10;
}
