//! Binary FBX (version 7.x, the format Blender, Maya and 3ds Max write), as a
//! scene of the glTF shape.
//!
//! Read: the node tree (32- and 64-bit record headers, zlib-compressed
//! arrays), `Geometry` meshes (control points, polygon indices, the first
//! normal, UV and material layers in any mapping and reference mode),
//! `Model` transforms (translation, rotation with pre/post rotation and any
//! Euler order, scaling, pivots and offsets, geometric transforms) and the
//! parent chain, `Material` colours (diffuse, emissive, opacity, shininess),
//! diffuse, normal and emissive `Texture`s (from embedded `Video` content or a
//! file next to the FBX), and `GlobalSettings` axes and unit scale (the scene
//! comes out Y up, in metres).
//! ASCII FBX (7.x) reads into the same tree.
//! Not read: skins, blend shapes, animation, cameras,
//! lights, NURBS and patches. Polygons become triangle fans.

use std::collections::HashMap;

use crate::error::LoadError;
use crate::gltf::{
    AlphaMode, GltfMesh, GltfScene, MeshInstance, PbrMaterial, Primitive, Sampler, TextureSlot,
};
use crate::png::Image;

const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
const ARRAY_LIMIT: usize = 1 << 30;

#[derive(Clone, Debug)]
enum Prop {
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

impl Prop {
    fn int(&self) -> Option<i64> {
        match self {
            Prop::Int(v) => Some(*v),
            Prop::Float(v) => Some(*v as i64),
            _ => None,
        }
    }
    fn float(&self) -> Option<f64> {
        match self {
            Prop::Float(v) => Some(*v),
            Prop::Int(v) => Some(*v as f64),
            _ => None,
        }
    }
    fn str(&self) -> Option<&[u8]> {
        match self {
            Prop::Str(v) => Some(v),
            _ => None,
        }
    }
    fn text(&self) -> String {
        // Binary "Name\0\x01Class": the name before the separator; ASCII
        // "Class::Name": the name after it.
        let s = self.str().unwrap_or(&[]);
        let end = s.windows(2).position(|w| w == b"\x00\x01").unwrap_or(s.len());
        let text = String::from_utf8_lossy(&s[..end]).into_owned();
        match text.split_once("::") {
            Some((_, name)) if end == s.len() => name.to_string(),
            _ => text,
        }
    }
    fn floats(&self) -> Vec<f64> {
        match self {
            Prop::Floats(v) => v.clone(),
            Prop::Ints(v) => v.iter().map(|&x| x as f64).collect(),
            _ => Vec::new(),
        }
    }
    fn ints(&self) -> Vec<i64> {
        match self {
            Prop::Ints(v) => v.clone(),
            Prop::Floats(v) => v.iter().map(|&x| x as i64).collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Node {
    name: String,
    props: Vec<Prop>,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    fn prop(&self, i: usize) -> Option<&Prop> {
        self.props.get(i)
    }
    /// A `Properties70` entry: `P: "name", "type", "label", "flags", values...`.
    fn p70(&self, name: &str) -> Option<&[Prop]> {
        let p = self.child("Properties70")?;
        p.children_named("P")
            .find(|e| e.prop(0).and_then(Prop::str) == Some(name.as_bytes()))
            .map(|e| if e.props.len() > 4 { &e.props[4..] } else { &[][..] })
    }
    fn p70_vec3(&self, name: &str) -> Option<[f64; 3]> {
        let v = self.p70(name)?;
        Some([v.first()?.float()?, v.get(1)?.float()?, v.get(2)?.float()?])
    }
    fn p70_f(&self, name: &str) -> Option<f64> {
        self.p70(name)?.first()?.float()
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    wide: bool,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        let end = self.at.checked_add(n).ok_or(LoadError::Truncated)?;
        let s = self.b.get(self.at..end).ok_or(LoadError::Truncated)?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, LoadError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, LoadError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, LoadError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn word(&mut self) -> Result<u64, LoadError> {
        if self.wide {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }

    fn array(&mut self, size: usize) -> Result<Vec<u8>, LoadError> {
        let len = self.u32()? as usize;
        let encoding = self.u32()?;
        let stored = self.u32()? as usize;
        let raw = self.take(stored)?;
        let bytes = len.checked_mul(size).ok_or(LoadError::Unrecognized)?;
        if bytes > ARRAY_LIMIT {
            return Err(LoadError::Unrecognized);
        }
        let data = match encoding {
            0 => raw.to_vec(),
            1 => crate::inflate::zlib_decompress(raw, bytes)?,
            _ => return Err(LoadError::Unrecognized),
        };
        if data.len() != bytes {
            return Err(LoadError::Truncated);
        }
        Ok(data)
    }

    fn prop(&mut self) -> Result<Prop, LoadError> {
        let code = self.u8()?;
        Ok(match code {
            b'Y' => Prop::Int(i16::from_le_bytes(self.take(2)?.try_into().unwrap()).into()),
            b'C' => Prop::Int(self.u8()?.into()),
            b'I' => Prop::Int(i32::from_le_bytes(self.take(4)?.try_into().unwrap()).into()),
            b'L' => Prop::Int(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            b'F' => Prop::Float(f32::from_le_bytes(self.take(4)?.try_into().unwrap()).into()),
            b'D' => Prop::Float(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            b'S' | b'R' => {
                let n = self.u32()? as usize;
                Prop::Str(self.take(n)?.to_vec())
            }
            b'f' => Prop::Floats(
                self.array(4)?
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()).into())
                    .collect(),
            ),
            b'd' => Prop::Floats(
                self.array(8)?
                    .chunks_exact(8)
                    .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
            ),
            b'i' => Prop::Ints(
                self.array(4)?
                    .chunks_exact(4)
                    .map(|c| i32::from_le_bytes(c.try_into().unwrap()).into())
                    .collect(),
            ),
            b'l' => Prop::Ints(
                self.array(8)?
                    .chunks_exact(8)
                    .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
            ),
            b'b' => Prop::Ints(self.array(1)?.iter().map(|&x| x.into()).collect()),
            _ => return Err(LoadError::Unrecognized),
        })
    }

    /// One node record, or `None` at the null record that ends a list.
    fn node(&mut self, depth: usize) -> Result<Option<Node>, LoadError> {
        if depth > 64 {
            return Err(LoadError::Unrecognized);
        }
        let end = self.word()? as usize;
        let count = self.word()? as usize;
        let _list_len = self.word()?;
        let name_len = self.u8()? as usize;
        if end == 0 {
            return Ok(None);
        }
        if end > self.b.len() || end <= self.at {
            return Err(LoadError::Truncated);
        }
        let name = String::from_utf8_lossy(self.take(name_len)?).into_owned();
        let mut props = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            props.push(self.prop()?);
        }
        let mut children = Vec::new();
        while self.at < end {
            match self.node(depth + 1)? {
                Some(n) => children.push(n),
                None => break,
            }
        }
        self.at = end;
        Ok(Some(Node {
            name,
            props,
            children,
        }))
    }
}

fn parse(bytes: &[u8]) -> Result<Node, LoadError> {
    if !bytes.starts_with(MAGIC) {
        return Err(LoadError::Unrecognized);
    }
    let version = u32::from_le_bytes(
        bytes
            .get(23..27)
            .ok_or(LoadError::Truncated)?
            .try_into()
            .unwrap(),
    );
    let mut r = Reader {
        b: bytes,
        at: 27,
        wide: version >= 7500,
    };
    let mut root = Node::default();
    let header = if r.wide { 25 } else { 13 };
    while r.at + header <= bytes.len() {
        match r.node(0)? {
            Some(n) => root.children.push(n),
            None => break,
        }
    }
    Ok(root)
}

// Column-major 4x4 matrices (glTF's layout), f64 while composing.
type Mat = [f64; 16];
const I4: Mat = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut o = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            o[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    o
}
fn translate(t: [f64; 3]) -> Mat {
    let mut m = I4;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}
fn scale(s: [f64; 3]) -> Mat {
    let mut m = I4;
    m[0] = s[0];
    m[5] = s[1];
    m[10] = s[2];
    m
}
fn neg(v: [f64; 3]) -> [f64; 3] {
    [-v[0], -v[1], -v[2]]
}
fn axis_rot(axis: usize, deg: f64) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    let mut m = I4;
    let (a, b) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    m[a * 4 + a] = c;
    m[a * 4 + b] = s;
    m[b * 4 + a] = -s;
    m[b * 4 + b] = c;
    m
}
/// FBX Euler rotation in degrees; `order` is eEulerXYZ (0) .. eEulerZYX (5):
/// the first named axis applies first.
fn euler(r: [f64; 3], order: i64) -> Mat {
    let seq: [usize; 3] = match order {
        1 => [0, 2, 1],
        2 => [1, 2, 0],
        3 => [1, 0, 2],
        4 => [2, 0, 1],
        5 => [2, 1, 0],
        _ => [0, 1, 2],
    };
    let mut m = I4;
    for &a in &seq {
        m = mul(&axis_rot(a, r[a]), &m);
    }
    m
}
fn inverse_affine(m: &Mat) -> Mat {
    // Inverse of the upper 3x3 (general) and the translation.
    let a = |r: usize, c: usize| m[c * 4 + r];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1))
        - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-30 {
        return I4;
    }
    let inv = 1.0 / det;
    let mut o = I4;
    let set = |o: &mut Mat, r: usize, c: usize, v: f64| o[c * 4 + r] = v * inv;
    set(&mut o, 0, 0, a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1));
    set(&mut o, 0, 1, a(0, 2) * a(2, 1) - a(0, 1) * a(2, 2));
    set(&mut o, 0, 2, a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1));
    set(&mut o, 1, 0, a(1, 2) * a(2, 0) - a(1, 0) * a(2, 2));
    set(&mut o, 1, 1, a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0));
    set(&mut o, 1, 2, a(0, 2) * a(1, 0) - a(0, 0) * a(1, 2));
    set(&mut o, 2, 0, a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    set(&mut o, 2, 1, a(0, 1) * a(2, 0) - a(0, 0) * a(2, 1));
    set(&mut o, 2, 2, a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0));
    let t = [m[12], m[13], m[14]];
    for r in 0..3 {
        o[12 + r] = -(0..3).map(|k| o[k * 4 + r] * t[k]).sum::<f64>();
    }
    o
}

/// A Model's local transform (the FBX SDK's formula):
/// T * Roff * Rp * Rpre * R * Rpost^-1 * Rp^-1 * Soff * Sp * S * Sp^-1.
fn local(model: &Node) -> Mat {
    let v = |n: &str, d: f64| model.p70_vec3(n).unwrap_or([d; 3]);
    let order = model
        .p70("RotationOrder")
        .and_then(|p| p.first()?.int())
        .unwrap_or(0);
    let rp = v("RotationPivot", 0.0);
    let sp = v("ScalingPivot", 0.0);
    let mut m = translate(v("Lcl Translation", 0.0));
    for f in [
        translate(v("RotationOffset", 0.0)),
        translate(rp),
        euler(v("PreRotation", 0.0), 0),
        euler(v("Lcl Rotation", 0.0), order),
        inverse_affine(&euler(v("PostRotation", 0.0), 0)),
        translate(neg(rp)),
        translate(v("ScalingOffset", 0.0)),
        translate(sp),
        scale(v("Lcl Scaling", 1.0)),
        translate(neg(sp)),
    ] {
        m = mul(&m, &f);
    }
    m
}
fn geometric(model: &Node) -> Mat {
    let v = |n: &str, d: f64| model.p70_vec3(n).unwrap_or([d; 3]);
    let m = mul(
        &translate(v("GeometricTranslation", 0.0)),
        &euler(v("GeometricRotation", 0.0), 0),
    );
    mul(&m, &scale(v("GeometricScaling", 1.0)))
}

/// One layer element's values for polygon-vertex `pv` of polygon `poly` at
/// control point `cp`.
struct Layer {
    values: Vec<f64>,
    index: Vec<i64>,
    by: Mapping,
    width: usize,
}
#[derive(PartialEq)]
enum Mapping {
    PolygonVertex,
    ControlPoint,
    Polygon,
    AllSame,
}
impl Layer {
    fn read(node: &Node, values: &str, index: &str, width: usize) -> Option<Layer> {
        let map = node.child("MappingInformationType")?.prop(0)?.text();
        let reference = node
            .child("ReferenceInformationType")
            .and_then(|n| n.prop(0))
            .map(Prop::text)
            .unwrap_or_default();
        let by = match map.as_str() {
            "ByPolygonVertex" => Mapping::PolygonVertex,
            "ByVertice" | "ByVertex" | "ByControlPoint" => Mapping::ControlPoint,
            "ByPolygon" => Mapping::Polygon,
            "AllSame" => Mapping::AllSame,
            _ => return None,
        };
        let values = node.child(values)?.prop(0)?.floats();
        let index = if reference == "IndexToDirect" || reference == "Index" {
            node.child(index).and_then(|n| n.prop(0)).map(Prop::ints).unwrap_or_default()
        } else {
            Vec::new()
        };
        Some(Layer {
            values,
            index,
            by,
            width,
        })
    }
    fn at(&self, pv: usize, poly: usize, cp: usize) -> Option<&[f64]> {
        let i = match self.by {
            Mapping::PolygonVertex => pv,
            Mapping::ControlPoint => cp,
            Mapping::Polygon => poly,
            Mapping::AllSame => 0,
        };
        let i = if self.index.is_empty() {
            i
        } else {
            usize::try_from(*self.index.get(i)?).ok()?
        };
        self.values.get(i * self.width..(i + 1) * self.width)
    }
}

fn texture_slot(image: usize) -> TextureSlot {
    TextureSlot {
        image,
        texcoord: 0,
        sampler: Sampler::default(),
        scale: 1.0,
    }
}

/// Decode a binary FBX; `resolve` reads texture files the FBX names (relative
/// to it) when they are not embedded.
pub(crate) fn decode_fbx(
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
) -> Result<GltfScene, LoadError> {
    let root = if bytes.starts_with(MAGIC) {
        parse(bytes)?
    } else {
        parse_ascii(bytes)?
    };
    let objects = root.child("Objects").ok_or(LoadError::Unrecognized)?;
    let mut by_id: HashMap<i64, &Node> = HashMap::new();
    for o in &objects.children {
        if let Some(id) = o.prop(0).and_then(Prop::int) {
            by_id.insert(id, o);
        }
    }
    // Connections, in file order: child -> parents, parent -> children.
    let mut parents: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    let mut children: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    if let Some(c) = root.child("Connections") {
        for e in c.children_named("C") {
            let (Some(kind), Some(child), Some(parent)) = (
                e.prop(0).map(Prop::text),
                e.prop(1).and_then(Prop::int),
                e.prop(2).and_then(Prop::int),
            ) else {
                continue;
            };
            let label = if kind == "OP" {
                e.prop(3).map(Prop::text).unwrap_or_default()
            } else {
                String::new()
            };
            parents.entry(child).or_default().push((parent, label.clone()));
            children.entry(parent).or_default().push((child, label));
        }
    }
    let kids = |id: i64, name: &str| -> Vec<(i64, String)> {
        children
            .get(&id)
            .map(|v| {
                v.iter()
                    .filter(|(c, _)| by_id.get(c).is_some_and(|n| n.name == name))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };

    // Axis system and unit: the scene comes out Y up, right-handed, metres.
    let settings = root.child("GlobalSettings");
    let gi = |name: &str, d: i64| {
        settings
            .and_then(|s| s.p70(name))
            .and_then(|p| p.first()?.int())
            .unwrap_or(d)
    };
    let unit = settings
        .and_then(|s| s.p70_f("UnitScaleFactor"))
        .unwrap_or(1.0)
        / 100.0;
    let (up, up_sign) = (gi("UpAxis", 1) as usize, gi("UpAxisSign", 1) as f64);
    let (front, front_sign) = (gi("FrontAxis", 2) as usize, gi("FrontAxisSign", 1) as f64);
    let (coord, coord_sign) = (gi("CoordAxis", 0) as usize, gi("CoordAxisSign", 1) as f64);
    let mut axes = I4;
    if up < 3 && front < 3 && coord < 3 && up != front && up != coord && front != coord {
        // Rows: output x <- file coord axis, y <- up, z <- front.
        axes = [0.0; 16];
        axes[15] = 1.0;
        axes[coord * 4] = coord_sign;
        axes[up * 4 + 1] = up_sign;
        axes[front * 4 + 2] = front_sign;
    }
    let global = mul(&scale([unit; 3]), &axes);

    // Images: embedded Video content first, then the file.
    let mut images: Vec<Image> = Vec::new();
    let mut image_of: HashMap<i64, Option<usize>> = HashMap::new();
    let mut image_for = |tex: i64, images: &mut Vec<Image>| -> Option<usize> {
        if let Some(found) = image_of.get(&tex) {
            return *found;
        }
        let node = by_id.get(&tex)?;
        let mut decoded = None;
        for (video, _) in kids(tex, "Video") {
            if let Some(content) = by_id[&video].child("Content").and_then(|n| n.prop(0)).and_then(Prop::str) {
                if !content.is_empty() {
                    decoded = crate::decode_image(content).ok();
                }
            }
        }
        if decoded.is_none() {
            for field in ["RelativeFilename", "FileName"] {
                let Some(path) = node.child(field).and_then(|n| n.prop(0)).map(Prop::text) else {
                    continue;
                };
                let path = path.replace('\\', "/");
                let base = path.rsplit('/').next().unwrap_or(&path).to_string();
                for candidate in [path.clone(), base] {
                    if let Some(img) = resolve(&candidate).ok().and_then(|b| crate::decode_image(&b).ok()) {
                        decoded = Some(img);
                        break;
                    }
                }
                if decoded.is_some() {
                    break;
                }
            }
        }
        let index = decoded.map(|img| {
            images.push(img);
            images.len() - 1
        });
        image_of.insert(tex, index);
        index
    };

    // Materials.
    let mut materials: Vec<PbrMaterial> = Vec::new();
    let mut material_index: HashMap<i64, usize> = HashMap::new();
    for o in objects.children_named("Material") {
        let Some(id) = o.prop(0).and_then(Prop::int) else { continue };
        let colour = |name: &str| o.p70_vec3(name);
        let factor = |name: &str| o.p70_f(name);
        let mut m = PbrMaterial {
            name: o.prop(1).map(Prop::text),
            metallic: 0.0,
            roughness: 1.0,
            ..PbrMaterial::default()
        };
        let d = colour("DiffuseColor")
            .or_else(|| colour("Diffuse"))
            .unwrap_or([0.8; 3]);
        let df = factor("DiffuseFactor").unwrap_or(1.0);
        m.base_color = [(d[0] * df) as f32, (d[1] * df) as f32, (d[2] * df) as f32, 1.0];
        let e = colour("EmissiveColor").unwrap_or([0.0; 3]);
        let ef = factor("EmissiveFactor").unwrap_or(1.0);
        m.emissive = [(e[0] * ef) as f32, (e[1] * ef) as f32, (e[2] * ef) as f32];
        let opacity = factor("Opacity").or_else(|| {
            factor("TransparencyFactor").map(|t| 1.0 - t)
        });
        if let Some(a) = opacity {
            let a = a.clamp(0.0, 1.0) as f32;
            if a < 0.999 && a > 0.0 {
                m.base_color[3] = a;
                m.alpha_mode = AlphaMode::Blend;
            }
        }
        if let Some(s) = factor("ShininessExponent").or_else(|| factor("Shininess")) {
            m.roughness = (2.0 / (s.max(0.0) + 2.0)).sqrt() as f32;
        }
        for (tex, label) in kids(id, "Texture") {
            let Some(image) = image_for(tex, &mut images) else { continue };
            match label.as_str() {
                "DiffuseColor" | "Maya|baseColor" | "3dsMax|Parameters|base_color_map" => {
                    m.base_color_texture = Some(texture_slot(image));
                }
                "NormalMap" | "Bump" | "Maya|normalCamera" => m.normal_texture = Some(texture_slot(image)),
                "EmissiveColor" | "Maya|emissionColor" => {
                    m.emissive_texture = Some(texture_slot(image));
                    if m.emissive == [0.0; 3] {
                        m.emissive = [1.0; 3];
                    }
                }
                _ => {}
            }
        }
        material_index.insert(id, materials.len());
        materials.push(m);
    }

    // Models with geometry: one mesh and one instance each.
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    for model in objects.children_named("Model") {
        let Some(id) = model.prop(0).and_then(Prop::int) else { continue };
        let geometries = kids(id, "Geometry");
        if geometries.is_empty() {
            continue;
        }
        // World: global * parents... * local * geometric.
        let mut world = mul(&local(model), &geometric(model));
        let mut at = id;
        for _ in 0..256 {
            let Some(parent) = parents.get(&at).and_then(|ps| {
                ps.iter()
                    .find(|(p, _)| by_id.get(p).is_some_and(|n| n.name == "Model"))
                    .map(|(p, _)| *p)
            }) else {
                break;
            };
            world = mul(&local(by_id[&parent]), &world);
            at = parent;
        }
        let world = mul(&global, &world);
        let mats: Vec<usize> = kids(id, "Material")
            .iter()
            .filter_map(|(m, _)| material_index.get(m).copied())
            .collect();
        for (geo, _) in geometries {
            let g = by_id[&geo];
            if let Some(mesh) = build_mesh(g, &mats)? {
                meshes.push(GltfMesh {
                    name: model.prop(1).map(Prop::text),
                    primitives: mesh,
                });
                instances.push(MeshInstance {
                    mesh: meshes.len() - 1,
                    node_name: model.prop(1).map(Prop::text),
                    transform: world.map(|x| x as f32),
                });
            }
        }
    }
    if meshes.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    Ok(GltfScene {
        meshes,
        materials,
        images,
        instances,
    })
}

/// Triangles of one Geometry, one primitive per material slot it uses.
fn build_mesh(g: &Node, mats: &[usize]) -> Result<Option<Vec<Primitive>>, LoadError> {
    let Some(verts) = g.child("Vertices").and_then(|n| n.prop(0)).map(Prop::floats) else {
        return Ok(None);
    };
    let Some(poly) = g.child("PolygonVertexIndex").and_then(|n| n.prop(0)).map(Prop::ints) else {
        return Ok(None);
    };
    let normals = g
        .child("LayerElementNormal")
        .and_then(|n| Layer::read(n, "Normals", "NormalsIndex", 3));
    let uvs = g
        .child("LayerElementUV")
        .and_then(|n| Layer::read(n, "UV", "UVIndex", 2));
    let material_layer = g.child("LayerElementMaterial").and_then(|n| {
        let map = n.child("MappingInformationType")?.prop(0)?.text();
        let list = n.child("Materials")?.prop(0)?.ints();
        Some((map == "AllSame", list))
    });
    let cps = verts.len() / 3;
    let mut prims: Vec<(usize, Primitive, HashMap<[u64; 6], u32>)> = Vec::new();
    let mut start = 0usize;
    let mut poly_i = 0usize;
    let mut i = 0usize;
    while i < poly.len() {
        let end = poly[i] < 0;
        i += 1;
        if !end {
            continue;
        }
        let corners = start..i;
        start = i;
        let this_poly = poly_i;
        poly_i += 1;
        if corners.len() < 3 {
            continue;
        }
        let slot = match &material_layer {
            Some((true, list)) => list.first().copied().unwrap_or(0),
            Some((false, list)) => list.get(this_poly).copied().unwrap_or(0),
            None => 0,
        };
        let slot = usize::try_from(slot).unwrap_or(0);
        let at = match prims.iter().position(|(s, _, _)| *s == slot) {
            Some(at) => at,
            None => {
                prims.push((slot, Primitive::default(), HashMap::new()));
                prims.len() - 1
            }
        };
        let (_, prim, weld) = &mut prims[at];
        let mut ids = Vec::with_capacity(corners.len());
        let cp_of = |pv: usize| {
            let v = poly[pv];
            (if v < 0 { !v } else { v }) as usize
        };
        // Flat normal for a polygon without a normal layer.
        let flat = {
            let p = |pv: usize| {
                let c = cp_of(pv);
                [verts[c * 3], verts[c * 3 + 1], verts[c * 3 + 2]]
            };
            let mut n = [0.0f64; 3];
            let list: Vec<usize> = corners.clone().collect();
            for k in 0..list.len() {
                if cp_of(list[k]) >= cps || cp_of(list[(k + 1) % list.len()]) >= cps {
                    return Err(LoadError::Unrecognized);
                }
                let a = p(list[k]);
                let b = p(list[(k + 1) % list.len()]);
                n[0] += (a[1] - b[1]) * (a[2] + b[2]);
                n[1] += (a[2] - b[2]) * (a[0] + b[0]);
                n[2] += (a[0] - b[0]) * (a[1] + b[1]);
            }
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if l > 0.0 {
                [n[0] / l, n[1] / l, n[2] / l]
            } else {
                [0.0, 1.0, 0.0]
            }
        };
        for pv in corners {
            let cp = cp_of(pv);
            let pos = [verts[cp * 3], verts[cp * 3 + 1], verts[cp * 3 + 2]];
            let n = normals
                .as_ref()
                .and_then(|l| l.at(pv, this_poly, cp))
                .map_or(flat, |v| [v[0], v[1], v[2]]);
            let uv = uvs.as_ref().and_then(|l| l.at(pv, this_poly, cp)).map(|v| [v[0], 1.0 - v[1]]);
            let key = [
                cp as u64,
                (n[0] as f32).to_bits() as u64,
                (n[1] as f32).to_bits() as u64,
                (n[2] as f32).to_bits() as u64,
                uv.map_or(u64::MAX, |u| (u[0] as f32).to_bits() as u64),
                uv.map_or(u64::MAX, |u| (u[1] as f32).to_bits() as u64),
            ];
            let id = *weld.entry(key).or_insert_with(|| {
                prim.positions.push(pos.map(|x| x as f32));
                prim.normals.push(n.map(|x| x as f32));
                if uvs.is_some() {
                    let u = uv.unwrap_or([0.0, 0.0]);
                    prim.texcoords.push([u[0] as f32, u[1] as f32]);
                }
                (prim.positions.len() - 1) as u32
            });
            ids.push(id);
        }
        for k in 1..ids.len() - 1 {
            prim.indices.extend_from_slice(&[ids[0], ids[k], ids[k + 1]]);
        }
    }
    if prims.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        prims
            .into_iter()
            .map(|(slot, mut p, _)| {
                p.material = mats.get(slot).copied();
                p
            })
            .collect(),
    ))
}

/// ASCII FBX: `Name: value, value, ... { children }`, `;` comments, arrays
/// as `*N { a: v, v, ... }`.
fn parse_ascii(bytes: &[u8]) -> Result<Node, LoadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LoadError::Unrecognized)?;
    if !text.trim_start().starts_with(';') && !text.contains("FBXHeaderExtension") {
        return Err(LoadError::Unrecognized);
    }
    let b = text.as_bytes();
    let mut at = 0usize;
    let mut root = Node::default();
    ascii_list(b, &mut at, &mut root.children, 0)?;
    if root.child("Objects").is_none() {
        return Err(LoadError::Unrecognized);
    }
    Ok(root)
}

fn skip_space(b: &[u8], at: &mut usize, newlines: bool) {
    while *at < b.len() {
        match b[*at] {
            b' ' | b'\t' | b'\r' => *at += 1,
            b'\n' if newlines => *at += 1,
            b';' => {
                while *at < b.len() && b[*at] != b'\n' {
                    *at += 1;
                }
            }
            _ => break,
        }
    }
}

fn ascii_list(b: &[u8], at: &mut usize, out: &mut Vec<Node>, depth: usize) -> Result<(), LoadError> {
    if depth > 64 {
        return Err(LoadError::Unrecognized);
    }
    loop {
        skip_space(b, at, true);
        if *at >= b.len() {
            return Ok(());
        }
        if b[*at] == b'}' {
            *at += 1;
            return Ok(());
        }
        let start = *at;
        while *at < b.len() && b[*at] != b':' && b[*at] != b'\n' {
            *at += 1;
        }
        if *at >= b.len() || b[*at] != b':' {
            return Err(LoadError::Unrecognized);
        }
        let name = String::from_utf8_lossy(&b[start..*at]).trim().to_string();
        *at += 1;
        let mut node = Node {
            name,
            ..Node::default()
        };
        let mut star = false;
        loop {
            skip_space(b, at, false);
            if *at >= b.len() || b[*at] == b'\n' {
                break;
            }
            match b[*at] {
                b'{' => {
                    *at += 1;
                    ascii_list(b, at, &mut node.children, depth + 1)?;
                    break;
                }
                b',' => {
                    *at += 1;
                    // A value list continues on the next line after a comma.
                    skip_space(b, at, true);
                }
                b'"' => {
                    *at += 1;
                    let s = *at;
                    while *at < b.len() && b[*at] != b'"' {
                        *at += 1;
                    }
                    node.props.push(Prop::Str(b[s..(*at).min(b.len())].to_vec()));
                    *at += 1;
                }
                b'*' => {
                    star = true;
                    while *at < b.len() && !matches!(b[*at], b' ' | b'\t' | b'{' | b'\n') {
                        *at += 1;
                    }
                }
                _ => {
                    let s = *at;
                    while *at < b.len() && !matches!(b[*at], b',' | b'\n' | b'\r' | b'{' | b' ' | b'\t') {
                        *at += 1;
                    }
                    let tok = std::str::from_utf8(&b[s..*at]).unwrap_or("");
                    node.props.push(if let Ok(v) = tok.parse::<i64>() {
                        Prop::Int(v)
                    } else if let Ok(v) = tok.parse::<f64>() {
                        Prop::Float(v)
                    } else {
                        Prop::Str(tok.as_bytes().to_vec())
                    });
                }
            }
        }
        if star {
            let values: Vec<f64> = node
                .child("a")
                .map(|a| a.props.iter().filter_map(Prop::float).collect())
                .unwrap_or_default();
            node.props = vec![Prop::Floats(values)];
            node.children.clear();
        }
        out.push(node);
    }
}
