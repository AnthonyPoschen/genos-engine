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
    /// Render above the presented resolution, then tent-downsample.
    Ssaa,
}

impl Antialias {
    /// Panel code. `0` is off, `1` is FXAA, and `2` is SSAA.
    pub fn from_picture(code: u8) -> Self {
        match code {
            1 => Self::Fxaa,
            2 => Self::Ssaa,
            _ => Self::Off,
        }
    }
}
