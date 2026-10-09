//! Live debugging and verification tools for any example.
//!
//! An example creates [`Tools`], calls [`Tools::scene_dt`] for its scene time,
//! [`Tools::before_draw`] before it draws and [`Tools::after_draw`] after. The same
//! commands then arrive from the MCP port (`tools/call`, `POST /cmd/<name>`) and
//! from rhai scripts ([`script`]), which the headless runner uses for regression
//! tests. See `docs/systems/debugging.md`.

pub mod commands;
pub mod image;
pub mod knobs;
pub mod png;
pub mod reference;
pub mod script;
pub mod tools;
mod value;

pub use image::{Diff, Flicker, FlickerReport, Image, Luminance, Region};
pub use knobs::{lighting_knobs, set_lighting_knob, Host, Knob};
pub use reference::{compare, render as render_reference, Compare, RefSetup, Reference};
pub use tools::{Ctx, FrameTiming, Look, ProbeColor, Tools, DEFAULT_STEP};
