#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! Presented-picture antialiasing. The draw reads one mode. `Off` is the
//! single-sample color image.

/// Filter for the color image that is presented and read back.
///
/// `Off` keeps the single-sample raster. The other modes are different methods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Antialias {
    #[default]
    Off,
    /// Luminance-edge post-process.
    Fxaa,
}

impl Antialias {
    /// Panel code. `0` is off; `1` is FXAA, and so is `2` (SSAA went with the old
    /// lighting: the lighting passes draw at the presented size).
    pub fn from_picture(code: u8) -> Self {
        match code {
            1 | 2 => Self::Fxaa,
            _ => Self::Off,
        }
    }
}
