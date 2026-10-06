//! Textures and 2D texture maps.
//!
//! A texture is a named image. A texture map is one image plus a text
//! description:
//!
//! ```text
//! frame <name> <x> <y> <width> <height>
//! clip <name> <frame-seconds> <frame-name> <frame-name> ...
//! ```
//!
//! `#` starts a comment line. `sample` returns the frame rectangle for a clip
//! at a time. The rectangle changes when the time crosses a frame boundary.

use crate::error::LoadError;
use crate::png::Image;

/// A named sampling image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub name: String,
    pub image: Image,
}

impl Texture {
    /// RGBA8 pixel at `x`, `y`, or `None` when the point is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        self.image.pixel(x, y)
    }
}

/// A rectangle in image pixels. The origin is the top-left of the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One frame rectangle in a texture map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapFrame {
    pub name: String,
    pub rect: Rect,
}

/// A named clip. `frames` are frame names in play order.
#[derive(Clone, Debug, PartialEq)]
pub struct MapClip {
    pub name: String,
    /// Seconds each frame stays on screen.
    pub frame_seconds: f32,
    pub frames: Vec<String>,
}

/// One image, its frame rectangles, and its named clips.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureMap {
    pub image: Image,
    pub frames: Vec<MapFrame>,
    pub clips: Vec<MapClip>,
}

impl TextureMap {
    /// Rectangle for `clip_name` at `time_seconds` from the start of the clip.
    pub fn sample(&self, clip_name: &str, time_seconds: f32) -> Result<Rect, LoadError> {
        if !time_seconds.is_finite() || time_seconds < 0.0 {
            return Err(LoadError::Unrecognized);
        }
        let clip = self
            .clips
            .iter()
            .find(|clip| clip.name == clip_name)
            .ok_or(LoadError::Unrecognized)?;
        if clip.frames.is_empty() || !(clip.frame_seconds > 0.0) {
            return Err(LoadError::Unrecognized);
        }
        let ticks = time_seconds / clip.frame_seconds;
        if !ticks.is_finite() || ticks > u64::MAX as f32 {
            return Err(LoadError::Unrecognized);
        }
        let index = (ticks.floor() as u64) % (clip.frames.len() as u64);
        let frame_name = &clip.frames[index as usize];
        self.frames
            .iter()
            .find(|frame| frame.name == *frame_name)
            .map(|frame| frame.rect)
            .ok_or(LoadError::Unrecognized)
    }
}

pub(crate) fn decode_map(image: Image, bytes: &[u8]) -> Result<TextureMap, LoadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LoadError::Unrecognized)?;
    let mut frames: Vec<MapFrame> = Vec::new();
    let mut clips: Vec<MapClip> = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let tag = parts.next().ok_or(LoadError::Unrecognized)?;
        match tag {
            "frame" => {
                let name = parts.next().ok_or(LoadError::Unrecognized)?.to_string();
                if name.is_empty() || frames.iter().any(|frame| frame.name == name) {
                    return Err(LoadError::Unrecognized);
                }
                let x = whole(parts.next())?;
                let y = whole(parts.next())?;
                let width = whole(parts.next())?;
                let height = whole(parts.next())?;
                if parts.next().is_some() || width == 0 || height == 0 {
                    return Err(LoadError::Unrecognized);
                }
                let x_end = x.checked_add(width).ok_or(LoadError::Unrecognized)?;
                let y_end = y.checked_add(height).ok_or(LoadError::Unrecognized)?;
                if x_end > image.width || y_end > image.height {
                    return Err(LoadError::Unrecognized);
                }
                frames.push(MapFrame {
                    name,
                    rect: Rect {
                        x,
                        y,
                        width,
                        height,
                    },
                });
            }
            "clip" => {
                let name = parts.next().ok_or(LoadError::Unrecognized)?.to_string();
                if name.is_empty() || clips.iter().any(|clip| clip.name == name) {
                    return Err(LoadError::Unrecognized);
                }
                let frame_seconds = seconds(parts.next())?;
                let frame_names: Vec<String> = parts.map(str::to_string).collect();
                if frame_names.is_empty() {
                    return Err(LoadError::Unrecognized);
                }
                for frame_name in &frame_names {
                    if !frames.iter().any(|frame| frame.name == *frame_name) {
                        return Err(LoadError::Unrecognized);
                    }
                }
                clips.push(MapClip {
                    name,
                    frame_seconds,
                    frames: frame_names,
                });
            }
            _ => return Err(LoadError::Unrecognized),
        }
    }
    if frames.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    Ok(TextureMap {
        image,
        frames,
        clips,
    })
}

fn whole(token: Option<&str>) -> Result<u32, LoadError> {
    let token = token.ok_or(LoadError::Unrecognized)?;
    token.parse().map_err(|_| LoadError::Unrecognized)
}

fn seconds(token: Option<&str>) -> Result<f32, LoadError> {
    let token = token.ok_or(LoadError::Unrecognized)?;
    let value: f32 = token.parse().map_err(|_| LoadError::Unrecognized)?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(LoadError::Unrecognized)
    }
}
