//! Text material description.
//!
//! ```text
//! color <red> <green> <blue>
//! texture <file-name>
//! ```
//!
//! `color` is required. `texture` is optional. `#` starts a comment line.

use crate::error::LoadError;

/// A flat color and an optional texture file name.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    /// Linear red, green, and blue.
    pub color: [f32; 3],
    /// Texture file name from the description, when the line is present.
    pub texture: Option<String>,
}

pub(crate) fn decode_material(bytes: &[u8]) -> Result<Material, LoadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LoadError::Unrecognized)?;
    let mut color = None;
    let mut texture = None;
    for line in text.lines() {
        let line = strip_line(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let tag = parts.next().ok_or(LoadError::Unrecognized)?;
        match tag {
            "color" => {
                if color.is_some() {
                    return Err(LoadError::Unrecognized);
                }
                color = Some([
                    number(parts.next())?,
                    number(parts.next())?,
                    number(parts.next())?,
                ]);
                if parts.next().is_some() {
                    return Err(LoadError::Unrecognized);
                }
            }
            "texture" => {
                if texture.is_some() {
                    return Err(LoadError::Unrecognized);
                }
                let name = parts.next().ok_or(LoadError::Unrecognized)?;
                if parts.next().is_some() || name.is_empty() {
                    return Err(LoadError::Unrecognized);
                }
                texture = Some(name.to_string());
            }
            _ => return Err(LoadError::Unrecognized),
        }
    }
    Ok(Material {
        color: color.ok_or(LoadError::Unrecognized)?,
        texture,
    })
}

fn strip_line(line: &str) -> &str {
    line.trim().trim_end_matches('\r')
}

fn number(token: Option<&str>) -> Result<f32, LoadError> {
    let token = token.ok_or(LoadError::Unrecognized)?;
    let value: f32 = token.parse().map_err(|_| LoadError::Unrecognized)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(LoadError::Unrecognized)
    }
}
