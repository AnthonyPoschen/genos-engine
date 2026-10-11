//! Wavefront OBJ with its MTL materials, as a scene of the glTF shape.
//!
//! `load_obj_file` reads `v`, `vt`, `vn`, `f`, `usemtl` and `mtllib` and
//! skips every other tag (groups, smoothing, curves, `vp`), so files from
//! exporters that write extra tags still load. Faces become triangle fans;
//! corners are welded by their (position, texcoord, normal) indices; one
//! primitive per material in first-use order, one mesh, one identity instance.
//! A face without normals gets its flat normal. Texture coordinates are
//! flipped to glTF's convention (v down).
//!
//! MTL: `newmtl`, `Kd`, `Ke`, `Ns`, `d`, `Tr`, `map_Kd`, `map_Ke`, `map_Bump`
//! / `bump` / `norm`. `Ns` becomes roughness sqrt(2 / (Ns + 2)); metallic is
//! 0. A texture the image decoder cannot read (not PNG or JPEG) or cannot find
//! is left off and the material keeps its factor. A missing MTL file leaves
//! the default material.

use std::collections::HashMap;

use crate::error::LoadError;
use crate::gltf::{
    AlphaMode, GltfMesh, GltfScene, MeshInstance, PbrMaterial, Primitive, Sampler, TextureSlot,
};
use crate::png::Image;

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Material and images read from MTL files, by material name.
#[derive(Default)]
struct Library {
    materials: Vec<PbrMaterial>,
    by_name: HashMap<String, usize>,
    images: Vec<Image>,
    image_by_path: HashMap<String, Option<usize>>,
}

impl Library {
    fn image(
        &mut self,
        path: &str,
        resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
    ) -> Option<usize> {
        if let Some(found) = self.image_by_path.get(path) {
            return *found;
        }
        let decoded = resolve(path).ok().and_then(|b| crate::decode_image(&b).ok());
        let index = decoded.map(|image| {
            self.images.push(image);
            self.images.len() - 1
        });
        self.image_by_path.insert(path.to_string(), index);
        index
    }
}

fn slot(image: usize, scale: f32) -> TextureSlot {
    TextureSlot {
        image,
        texcoord: 0,
        sampler: Sampler::default(),
        scale,
    }
}

/// The file name of a map statement: the last token (options such as
/// `-bm 1.0` or `-s 1 1 1` come before it).
fn map_path(rest: &[&str]) -> Option<String> {
    rest.last().map(|s| s.replace('\\', "/"))
}

fn floats(rest: &[&str], n: usize) -> Option<Vec<f32>> {
    let v: Vec<f32> = rest
        .iter()
        .take(n)
        .filter_map(|t| t.parse::<f32>().ok())
        .filter(|x| x.is_finite())
        .collect();
    (v.len() == n).then_some(v)
}

fn decode_mtl(
    text: &str,
    lib: &mut Library,
    resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
) {
    let mut current: Option<usize> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let (tag, rest) = (parts[0], &parts[1..]);
        if tag == "newmtl" {
            let name = rest.join(" ");
            let material = PbrMaterial {
                name: Some(name.clone()),
                metallic: 0.0,
                roughness: 1.0,
                base_color: [0.8, 0.8, 0.8, 1.0],
                ..PbrMaterial::default()
            };
            lib.materials.push(material);
            let index = lib.materials.len() - 1;
            lib.by_name.insert(name, index);
            current = Some(index);
            continue;
        }
        let Some(index) = current else { continue };
        match tag {
            "Kd" => {
                if let Some(v) = floats(rest, 3) {
                    let m = &mut lib.materials[index];
                    m.base_color = [v[0], v[1], v[2], m.base_color[3]];
                }
            }
            "Ke" => {
                if let Some(v) = floats(rest, 3) {
                    lib.materials[index].emissive = [v[0], v[1], v[2]];
                }
            }
            "Ns" => {
                if let Some(v) = floats(rest, 1) {
                    lib.materials[index].roughness = (2.0 / (v[0].max(0.0) + 2.0)).sqrt();
                }
            }
            "d" | "Tr" => {
                if let Some(v) = floats(rest, 1) {
                    let alpha = if tag == "d" { v[0] } else { 1.0 - v[0] };
                    let alpha = alpha.clamp(0.0, 1.0);
                    let m = &mut lib.materials[index];
                    m.base_color[3] = alpha;
                    m.alpha_mode = if alpha < 0.999 { AlphaMode::Blend } else { AlphaMode::Opaque };
                }
            }
            "map_Kd" | "map_Ke" | "map_Bump" | "map_bump" | "bump" | "norm" => {
                let Some(path) = map_path(rest) else { continue };
                let Some(image) = lib.image(&path, resolve) else { continue };
                let m = &mut lib.materials[index];
                match tag {
                    "map_Kd" => m.base_color_texture = Some(slot(image, 1.0)),
                    "map_Ke" => {
                        m.emissive_texture = Some(slot(image, 1.0));
                        if m.emissive == [0.0; 3] {
                            m.emissive = [1.0; 3];
                        }
                    }
                    _ => m.normal_texture = Some(slot(image, 1.0)),
                }
            }
            _ => {}
        }
    }
}

fn parse_floats<const N: usize>(rest: &[&str]) -> Result<[f32; N], LoadError> {
    let mut out = [0.0f32; N];
    for (i, o) in out.iter_mut().enumerate() {
        let t = rest.get(i).ok_or(LoadError::Unrecognized)?;
        let v: f32 = t.parse().map_err(|_| LoadError::Unrecognized)?;
        if !v.is_finite() {
            return Err(LoadError::Unrecognized);
        }
        *o = v;
    }
    Ok(out)
}

fn index(token: &str, len: usize) -> Result<Option<usize>, LoadError> {
    if token.is_empty() {
        return Ok(None);
    }
    let value: i64 = token.parse().map_err(|_| LoadError::Unrecognized)?;
    let resolved = if value > 0 {
        value - 1
    } else if value < 0 {
        len as i64 + value
    } else {
        return Err(LoadError::Unrecognized);
    };
    if resolved < 0 || resolved as usize >= len {
        return Err(LoadError::Unrecognized);
    }
    Ok(Some(resolved as usize))
}

#[derive(Default)]
struct Builder {
    prim: Primitive,
    weld: HashMap<(usize, usize, usize), u32>,
    /// Some primitive vertices lack texcoords; filled with zeros at the end
    /// only if any vertex has them.
    any_uv: bool,
}

/// Decode OBJ text; `resolve` reads files named by `mtllib` and the maps.
pub(crate) fn decode_obj_scene(
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
) -> Result<GltfScene, LoadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LoadError::Unrecognized)?;
    let mut lib = Library::default();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    // Material key: index into lib.materials, or usize::MAX for the default.
    let mut builders: Vec<(usize, Builder)> = Vec::new();
    let mut current = usize::MAX;
    let mut pending_name: Option<String> = None;
    let mut mtl_loaded = false;
    let mut corners: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let (tag, rest) = (parts[0], &parts[1..]);
        match tag {
            "v" => positions.push(parse_floats::<3>(rest)?),
            "vt" => {
                let uv = parse_floats::<2>(rest)?;
                texcoords.push([uv[0], 1.0 - uv[1]]);
            }
            "vn" => normals.push(parse_floats::<3>(rest)?),
            "mtllib" => {
                for name in rest {
                    if let Ok(b) = resolve(&name.replace('\\', "/")) {
                        if let Ok(t) = std::str::from_utf8(&b) {
                            decode_mtl(t, &mut lib, resolve);
                            mtl_loaded = true;
                        }
                    }
                }
                if let Some(name) = pending_name.take() {
                    current = lib.by_name.get(&name).copied().unwrap_or(usize::MAX);
                }
            }
            "usemtl" => {
                let name = rest.join(" ");
                match lib.by_name.get(&name) {
                    Some(&i) => current = i,
                    None => {
                        current = usize::MAX;
                        if !mtl_loaded {
                            pending_name = Some(name);
                        }
                    }
                }
            }
            "f" => {
                corners.clear();
                for token in rest {
                    let mut bits = token.split('/');
                    let p = index(bits.next().unwrap_or(""), positions.len())?
                        .ok_or(LoadError::Unrecognized)?;
                    let t = index(bits.next().unwrap_or(""), texcoords.len())?;
                    let n = index(bits.next().unwrap_or(""), normals.len())?;
                    corners.push((p, t, n));
                }
                if corners.len() < 3 {
                    return Err(LoadError::Unrecognized);
                }
                let at = match builders.iter().position(|(k, _)| *k == current) {
                    Some(at) => at,
                    None => {
                        builders.push((current, Builder::default()));
                        builders.len() - 1
                    }
                };
                let b = &mut builders[at].1;
                let flat = corners.iter().any(|c| c.2.is_none());
                let face_n = if flat {
                    let mut n = [0.0f32; 3];
                    // Newell's method: the face normal of a polygon.
                    for i in 0..corners.len() {
                        let p = positions[corners[i].0];
                        let q = positions[corners[(i + 1) % corners.len()].0];
                        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
                        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
                        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
                    }
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    if len > 0.0 {
                        [n[0] / len, n[1] / len, n[2] / len]
                    } else {
                        [0.0, 1.0, 0.0]
                    }
                } else {
                    [0.0; 3]
                };
                let mut ids = Vec::with_capacity(corners.len());
                for &(p, t, n) in &corners {
                    // A flat-shaded corner is never welded with a smooth one.
                    let key = (p, t.unwrap_or(usize::MAX), n.unwrap_or(usize::MAX));
                    let id = if let (false, Some(&id)) = (flat, b.weld.get(&key)) {
                        id
                    } else {
                        let id = b.prim.positions.len() as u32;
                        b.prim.positions.push(positions[p]);
                        b.prim.normals.push(n.map_or(face_n, |n| normals[n]));
                        b.prim.texcoords.push(t.map_or([0.0, 0.0], |t| texcoords[t]));
                        b.any_uv |= t.is_some();
                        if !flat {
                            b.weld.insert(key, id);
                        }
                        id
                    };
                    ids.push(id);
                }
                for i in 1..ids.len() - 1 {
                    b.prim.indices.extend_from_slice(&[ids[0], ids[i], ids[i + 1]]);
                }
            }
            _ => {}
        }
    }
    if builders.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    let primitives = builders
        .into_iter()
        .map(|(key, mut b)| {
            if !b.any_uv {
                b.prim.texcoords.clear();
            }
            b.prim.material = (key != usize::MAX).then_some(key);
            b.prim
        })
        .collect();
    Ok(GltfScene {
        meshes: vec![GltfMesh {
            name: None,
            primitives,
        }],
        materials: lib.materials,
        images: lib.images,
        instances: vec![MeshInstance {
            mesh: 0,
            node_name: None,
            transform: IDENTITY,
        }],
    })
}
