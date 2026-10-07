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

/// Picture filter the lighting panel can select. `Off` is the unfiltered picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureMode {
    Off = 0,
    Fxaa = 1,
    Ssaa = 2,
}

impl PictureMode {
    /// Code the renderer reads. `0` is off, `1` is FXAA, and `2` is SSAA.
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// A control the lighting panel can fire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Set the first lamp on axis 0, 1, or 2 (`x`, `y`, `z`) to `position`.
    SetLamp { axis: u8, position: f32 },
    /// Multiply `Light.color`. Brightness is the mean of that color.
    ScaleIntensity(f32),
    /// Select the picture filter for the next presented frame.
    SetAntialias(PictureMode),
    /// Start or hold the red box path. The caller keeps the on/off state.
    ToggleBoxRun,
    /// A press on a game button in a [`button_panel`]. The game reads the id.
    Press(u32),
}

pub use layout::{layout, Align, Direction, Node, Pad, Place, Rect, Sizing, Space};
pub use panel::{
    apply_frame_action, apply_lamp, button_panel, lighting_frame, Frame, PanelButton, PanelRow,
    Paint, Pointer, Shown, State,
};
pub use profile::{
    inspect_region, profile_overlay, remember_frame, FrameSample, OpenFrame, PlotScale,
    ProfileGraph, ProfileLine, ProfilePoint, ProfileStream, ProfileView, RegionHit, StageSample,
    DRAW_STAGE, GRAPH_BUCKET, GRAPH_WINDOW, OVERLAY_PERIOD, STAGE_COUNT, STAGE_LABELS,
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
    /// Unfiltered picture.
    pub const AA_OFF: u32 = 12;
    /// Luminance-edge picture filter.
    pub const AA_FXAA: u32 = 13;
    /// Double-resolution picture, then a tent downsample.
    pub const AA_SSAA: u32 = 14;
    /// Starts or holds the red box path.
    pub const BOX_RUN: u32 = 15;
}
