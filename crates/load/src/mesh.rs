//! Wavefront OBJ triangles.
//!
//! `v`, `vt`, `vn`, `usemtl`, and `f` are read. A face with more than three
//! vertices becomes a triangle fan. Other standard tags (`o`, `g`, `s`,
//! `mtllib`, `vp`) are ignored.

use crate::error::LoadError;

/// One triangle in file order.
#[derive(Clone, Debug, PartialEq)]
pub struct Triangle {
    pub positions: [[f32; 3]; 3],
    /// Present when every corner of the face has a texture coordinate.
    pub texcoords: Option<[[f32; 2]; 3]>,
    /// Present when every corner of the face has a normal.
    pub normals: Option<[[f32; 3]; 3]>,
    /// `usemtl` name in effect for this face.
    pub material: Option<String>,
}

/// Triangles decoded from an OBJ file.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub triangles: Vec<Triangle>,
}

pub(crate) fn decode_obj(bytes: &[u8]) -> Result<Mesh, LoadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LoadError::Unrecognized)?;
    let mut positions = Vec::new();
    let mut texcoords = Vec::new();
    let mut normals = Vec::new();
    let mut material: Option<String> = None;
    let mut triangles = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let tag = parts.next().ok_or(LoadError::Unrecognized)?;
        match tag {
            "v" => positions.push(vec3(&mut parts)?),
            "vt" => texcoords.push(vec2(&mut parts)?),
            "vn" => normals.push(vec3(&mut parts)?),
            "usemtl" => {
                let name = parts.next().ok_or(LoadError::Unrecognized)?;
                if parts.next().is_some() {
                    return Err(LoadError::Unrecognized);
                }
                material = Some(name.to_string());
            }
            "f" => {
                let corners = face(&mut parts, &positions, &texcoords, &normals)?;
                if corners.len() < 3 {
                    return Err(LoadError::Unrecognized);
                }
                for index in 1..corners.len() - 1 {
                    triangles.push(triangle(
                        &positions,
                        &texcoords,
                        &normals,
                        material.clone(),
                        [corners[0], corners[index], corners[index + 1]],
                    )?);
                }
            }
            "o" | "g" | "s" | "mtllib" | "vp" | "l" | "p" => {}
            _ => return Err(LoadError::Unrecognized),
        }
    }
    if triangles.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    Ok(Mesh { triangles })
}

#[derive(Clone, Copy)]
struct Corner {
    position: usize,
    texcoord: Option<usize>,
    normal: Option<usize>,
}

fn triangle(
    positions: &[[f32; 3]],
    texcoords: &[[f32; 2]],
    normals: &[[f32; 3]],
    material: Option<String>,
    corners: [Corner; 3],
) -> Result<Triangle, LoadError> {
    let mut position_out = [[0.0; 3]; 3];
    let mut texcoord_out = [[0.0; 2]; 3];
    let mut normal_out = [[0.0; 3]; 3];
    let mut saw_texcoord = None;
    let mut saw_normal = None;
    for (slot, corner) in corners.iter().enumerate() {
        position_out[slot] = positions[corner.position];
        match (saw_texcoord, corner.texcoord) {
            (None, value) => saw_texcoord = Some(value.is_some()),
            (Some(true), Some(_)) => {}
            (Some(false), None) => {}
            _ => return Err(LoadError::Unrecognized),
        }
        match (saw_normal, corner.normal) {
            (None, value) => saw_normal = Some(value.is_some()),
            (Some(true), Some(_)) => {}
            (Some(false), None) => {}
            _ => return Err(LoadError::Unrecognized),
        }
        if let Some(index) = corner.texcoord {
            texcoord_out[slot] = texcoords[index];
        }
        if let Some(index) = corner.normal {
            normal_out[slot] = normals[index];
        }
    }
    Ok(Triangle {
        positions: position_out,
        texcoords: if saw_texcoord == Some(true) {
            Some(texcoord_out)
        } else {
            None
        },
        normals: if saw_normal == Some(true) {
            Some(normal_out)
        } else {
            None
        },
        material,
    })
}

fn face(
    parts: &mut std::str::SplitWhitespace<'_>,
    positions: &[[f32; 3]],
    texcoords: &[[f32; 2]],
    normals: &[[f32; 3]],
) -> Result<Vec<Corner>, LoadError> {
    let mut corners = Vec::new();
    for token in parts {
        corners.push(corner(
            token,
            positions.len(),
            texcoords.len(),
            normals.len(),
        )?);
    }
    Ok(corners)
}

fn corner(
    token: &str,
    positions: usize,
    texcoords: usize,
    normals: usize,
) -> Result<Corner, LoadError> {
    let bits: Vec<&str> = token.split('/').collect();
    if bits.is_empty() || bits.len() > 3 {
        return Err(LoadError::Unrecognized);
    }
    let position = obj_index(bits[0], positions)?;
    let texcoord = if bits.len() > 1 && !bits[1].is_empty() {
        Some(obj_index(bits[1], texcoords)?)
    } else {
        None
    };
    let normal = if bits.len() > 2 && !bits[2].is_empty() {
        Some(obj_index(bits[2], normals)?)
    } else {
        None
    };
    Ok(Corner {
        position,
        texcoord,
        normal,
    })
}

fn obj_index(token: &str, len: usize) -> Result<usize, LoadError> {
    let value: i32 = token.parse().map_err(|_| LoadError::Unrecognized)?;
    let index = if value > 0 {
        value as usize - 1
    } else if value < 0 {
        let resolved = len as i32 + value;
        if resolved < 0 {
            return Err(LoadError::Unrecognized);
        }
        resolved as usize
    } else {
        return Err(LoadError::Unrecognized);
    };
    if index >= len {
        return Err(LoadError::Unrecognized);
    }
    Ok(index)
}

fn vec3(parts: &mut std::str::SplitWhitespace<'_>) -> Result<[f32; 3], LoadError> {
    let value = [
        number(parts.next())?,
        number(parts.next())?,
        number(parts.next())?,
    ];
    if let Some(extra) = parts.next() {
        let _: f32 = number(Some(extra))?;
        if parts.next().is_some() {
            return Err(LoadError::Unrecognized);
        }
    }
    Ok(value)
}

fn vec2(parts: &mut std::str::SplitWhitespace<'_>) -> Result<[f32; 2], LoadError> {
    let value = [number(parts.next())?, number(parts.next())?];
    if let Some(extra) = parts.next() {
        let _: f32 = number(Some(extra))?;
        if parts.next().is_some() {
            return Err(LoadError::Unrecognized);
        }
    }
    Ok(value)
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
