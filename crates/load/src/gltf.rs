//! glTF 2.0: `.gltf` (JSON with external or `data:` buffers and images) and `.glb`.
//!
//! The loader returns flat arrays: meshes made of indexed triangle primitives,
//! metallic-roughness materials, decoded images (PNG or JPEG), and one instance
//! per node that carries a mesh, with the node's world matrix. Triangle strips
//! and fans become lists; points and lines are skipped. A primitive without
//! normals gets flat normals, as the specification asks.
//!
//! Supported extensions: `KHR_materials_emissive_strength`. A file that
//! requires any other extension (Draco, meshopt, KHR_texture_basisu, ...) is
//! refused rather than drawn wrong.

use genos_json::Value;

use crate::error::LoadError;
use crate::png::Image;

/// A loaded glTF scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfScene {
    pub meshes: Vec<GltfMesh>,
    pub materials: Vec<PbrMaterial>,
    pub images: Vec<Image>,
    /// Every node with a mesh in the chosen scene, in depth-first node order.
    pub instances: Vec<MeshInstance>,
}

/// One glTF mesh: a list of primitives, each with one material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfMesh {
    pub name: Option<String>,
    pub primitives: Vec<Primitive>,
}

/// Indexed triangles with one material. Every attribute array has one entry per vertex.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// `TEXCOORD_0`, or empty.
    pub texcoords: Vec<[f32; 2]>,
    /// `TANGENT` (xyz and handedness w), or empty.
    pub tangents: Vec<[f32; 4]>,
    /// Three per triangle.
    pub indices: Vec<u32>,
    /// Index into [`GltfScene::materials`]; `None` is the default material.
    pub material: Option<usize>,
}

/// A placed mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshInstance {
    pub mesh: usize,
    pub node_name: Option<String>,
    /// Column-major object-to-world matrix (glTF's layout).
    pub transform: [f32; 16],
}

/// How alpha is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlphaMode {
    Opaque,
    /// Discard below the cutoff.
    Mask(f32),
    Blend,
}

/// Texture filters and wrap modes, as glTF numbers them (0 when unset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampler {
    pub mag_filter: u32,
    pub min_filter: u32,
    pub wrap_s: u32,
    pub wrap_t: u32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            mag_filter: 0,
            min_filter: 0,
            wrap_s: 10497,
            wrap_t: 10497,
        }
    }
}

/// A material's use of an image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureSlot {
    /// Index into [`GltfScene::images`].
    pub image: usize,
    pub texcoord: u32,
    pub sampler: Sampler,
    /// Normal map scale or occlusion strength; 1 elsewhere.
    pub scale: f32,
}

/// glTF metallic-roughness material.
#[derive(Clone, Debug, PartialEq)]
pub struct PbrMaterial {
    pub name: Option<String>,
    /// Linear RGBA factor.
    pub base_color: [f32; 4],
    /// sRGB-encoded.
    pub base_color_texture: Option<TextureSlot>,
    pub metallic: f32,
    pub roughness: f32,
    /// Roughness in G, metallic in B (linear).
    pub metallic_roughness_texture: Option<TextureSlot>,
    pub normal_texture: Option<TextureSlot>,
    pub occlusion_texture: Option<TextureSlot>,
    /// Linear RGB factor, before `emissive_strength`.
    pub emissive: [f32; 3],
    pub emissive_texture: Option<TextureSlot>,
    pub emissive_strength: f32,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            name: None,
            base_color: [1.0; 4],
            base_color_texture: None,
            metallic: 1.0,
            roughness: 1.0,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive: [0.0; 3],
            emissive_texture: None,
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
        }
    }
}

impl PbrMaterial {
    /// The diffuse albedo GI uses: base colour x (1 - metallic), ignoring textures.
    pub fn diffuse_albedo(&self) -> [f32; 3] {
        let k = 1.0 - self.metallic.clamp(0.0, 1.0);
        [
            self.base_color[0] * k,
            self.base_color[1] * k,
            self.base_color[2] * k,
        ]
    }
}

impl GltfScene {
    /// Every triangle of every instance in world space, with its material.
    pub fn world_triangles(&self) -> Vec<([[f32; 3]; 3], Option<usize>)> {
        let mut out = Vec::new();
        for instance in &self.instances {
            let Some(mesh) = self.meshes.get(instance.mesh) else {
                continue;
            };
            for primitive in &mesh.primitives {
                for tri in primitive.indices.chunks_exact(3) {
                    let corner = |i: u32| {
                        transform_point(&instance.transform, primitive.positions[i as usize])
                    };
                    out.push((
                        [corner(tri[0]), corner(tri[1]), corner(tri[2])],
                        primitive.material,
                    ));
                }
            }
        }
        out
    }
}

/// Apply a column-major 4x4 matrix to a point.
pub fn transform_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
const SUPPORTED_EXTENSIONS: &[&str] = &["KHR_materials_emissive_strength"];
const GLB_MAGIC: u32 = 0x4654_6C67;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

/// Decode a `.gltf` or `.glb`. `resolve` reads a relative URI (an external
/// buffer or image next to the file); `data:` URIs are decoded here.
pub(crate) fn decode_gltf(
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
) -> Result<GltfScene, LoadError> {
    let (json, bin) = split_glb(bytes)?;
    let text = std::str::from_utf8(json).map_err(|_| LoadError::Unrecognized)?;
    let doc = genos_json::parse(text).map_err(|_| LoadError::Unrecognized)?;
    if !doc
        .get("asset")
        .and_then(|a| a.get("version"))
        .and_then(Value::as_str)
        .is_some_and(|v| v.starts_with("2."))
    {
        return Err(LoadError::Unrecognized);
    }
    for ext in list(&doc, "extensionsRequired") {
        if !ext
            .as_str()
            .is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e))
        {
            return Err(LoadError::Unrecognized);
        }
    }
    let mut buffers = Vec::new();
    for (i, buffer) in list(&doc, "buffers").iter().enumerate() {
        let length = usize_of(buffer, "byteLength")?;
        let data = match buffer.get("uri").and_then(Value::as_str) {
            Some(uri) => read_uri(uri, resolve)?,
            None if i == 0 => bin.ok_or(LoadError::Unrecognized)?.to_vec(),
            None => return Err(LoadError::Unrecognized),
        };
        if data.len() < length {
            return Err(LoadError::Truncated);
        }
        buffers.push(data);
    }
    let reader = Reader {
        doc: &doc,
        buffers: &buffers,
    };

    let mut images = Vec::new();
    for image in list(&doc, "images") {
        let data = match (
            image.get("uri").and_then(Value::as_str),
            image.get("bufferView"),
        ) {
            (Some(uri), _) => read_uri(uri, resolve)?,
            (None, Some(view)) => reader.view(index(view)?)?.to_vec(),
            _ => return Err(LoadError::Unrecognized),
        };
        images.push(crate::decode_image(&data)?);
    }
    let materials = list(&doc, "materials")
        .iter()
        .map(|m| material(&doc, m, images.len()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut meshes = Vec::new();
    for mesh in list(&doc, "meshes") {
        let mut primitives = Vec::new();
        for p in list(mesh, "primitives") {
            if let Some(primitive) = reader.primitive(p, materials.len())? {
                primitives.push(primitive);
            }
        }
        meshes.push(GltfMesh {
            name: mesh.get("name").and_then(Value::as_str).map(str::to_string),
            primitives,
        });
    }

    let nodes = list(&doc, "nodes");
    let roots: Vec<usize> =
        match doc
            .get("scene")
            .map(index)
            .transpose()?
            .or(if list(&doc, "scenes").is_empty() {
                None
            } else {
                Some(0)
            }) {
            Some(scene) => {
                let scene = list(&doc, "scenes")
                    .get(scene)
                    .ok_or(LoadError::Unrecognized)?;
                list(scene, "nodes")
                    .iter()
                    .map(index)
                    .collect::<Result<_, _>>()?
            }
            None => {
                // No scene: every node that is nobody's child is a root.
                let mut child = vec![false; nodes.len()];
                for node in nodes {
                    for c in list(node, "children") {
                        if let Some(flag) = child.get_mut(index(c)?) {
                            *flag = true;
                        }
                    }
                }
                (0..nodes.len()).filter(|&i| !child[i]).collect()
            }
        };
    let mut instances = Vec::new();
    let mut stack: Vec<(usize, [f32; 16], usize)> =
        roots.into_iter().rev().map(|r| (r, IDENTITY, 0)).collect();
    while let Some((n, parent, depth)) = stack.pop() {
        let node = nodes.get(n).ok_or(LoadError::Unrecognized)?;
        if depth > 256 {
            return Err(LoadError::Unrecognized);
        }
        let world = mul(&parent, &local_matrix(node)?);
        if let Some(mesh) = node.get("mesh") {
            let mesh = index(mesh)?;
            if mesh >= meshes.len() {
                return Err(LoadError::Unrecognized);
            }
            instances.push(MeshInstance {
                mesh,
                node_name: node.get("name").and_then(Value::as_str).map(str::to_string),
                transform: world,
            });
        }
        for c in list(node, "children").iter().rev() {
            stack.push((index(c)?, world, depth + 1));
        }
    }
    Ok(GltfScene {
        meshes,
        materials,
        images,
        instances,
    })
}

fn split_glb(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), LoadError> {
    if bytes.len() < 4 || u32_le(bytes, 0)? != GLB_MAGIC {
        return Ok((bytes, None));
    }
    if u32_le(bytes, 4)? != 2 {
        return Err(LoadError::Unrecognized);
    }
    let total = (u32_le(bytes, 8)? as usize).min(bytes.len());
    let mut at = 12;
    let (mut json, mut bin) = (None, None);
    while at + 8 <= total {
        let len = u32_le(bytes, at)? as usize;
        let kind = u32_le(bytes, at + 4)?;
        let body = bytes
            .get(at + 8..at + 8 + len)
            .ok_or(LoadError::Truncated)?;
        match kind {
            CHUNK_JSON if json.is_none() => json = Some(body),
            CHUNK_BIN if bin.is_none() => bin = Some(body),
            _ => {}
        }
        at += 8 + len;
    }
    Ok((json.ok_or(LoadError::Unrecognized)?, bin))
}

fn u32_le(b: &[u8], at: usize) -> Result<u32, LoadError> {
    let s = b.get(at..at + 4).ok_or(LoadError::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_uri(
    uri: &str,
    resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
) -> Result<Vec<u8>, LoadError> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',').ok_or(LoadError::Unrecognized)?;
        return if meta.ends_with(";base64") {
            base64(data)
        } else {
            Ok(percent_decode(data))
        };
    }
    resolve(&percent_decode_str(uri))
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn percent_decode_str(s: &str) -> String {
    String::from_utf8(percent_decode(s)).unwrap_or_else(|_| s.to_string())
}

/// Standard base64 with optional padding; whitespace ignored.
pub(crate) fn base64(text: &str) -> Result<Vec<u8>, LoadError> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return Err(LoadError::Unrecognized),
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).unwrap_or(&[])
}

fn index(v: &Value) -> Result<usize, LoadError> {
    v.as_u64()
        .map(|i| i as usize)
        .ok_or(LoadError::Unrecognized)
}

fn usize_of(v: &Value, key: &str) -> Result<usize, LoadError> {
    index(v.get(key).ok_or(LoadError::Unrecognized)?)
}

fn opt_usize(v: &Value, key: &str) -> Result<Option<usize>, LoadError> {
    v.get(key).map(index).transpose()
}

fn f32_of(v: &Value, key: &str, default: f32) -> Result<f32, LoadError> {
    match v.get(key) {
        None => Ok(default),
        Some(x) => x.as_f64().map(|x| x as f32).ok_or(LoadError::Unrecognized),
    }
}

fn floats<const N: usize>(v: &Value, key: &str, default: [f32; N]) -> Result<[f32; N], LoadError> {
    let Some(a) = v.get(key) else {
        return Ok(default);
    };
    let a = a.as_array().ok_or(LoadError::Unrecognized)?;
    if a.len() != N {
        return Err(LoadError::Unrecognized);
    }
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().ok_or(LoadError::Unrecognized)? as f32;
    }
    Ok(out)
}

fn local_matrix(node: &Value) -> Result<[f32; 16], LoadError> {
    if node.get("matrix").is_some() {
        return floats(node, "matrix", IDENTITY);
    }
    let t = floats(node, "translation", [0.0; 3])?;
    let [x, y, z, w] = floats(node, "rotation", [0.0, 0.0, 0.0, 1.0])?;
    let s = floats(node, "scale", [1.0; 3])?;
    let n = (x * x + y * y + z * z + w * w).sqrt();
    let (x, y, z, w) = if n > 0.0 {
        (x / n, y / n, z / n, w / n)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };
    // Columns of R * S, then the translation.
    Ok([
        (1.0 - 2.0 * (y * y + z * z)) * s[0],
        (2.0 * (x * y + z * w)) * s[0],
        (2.0 * (x * z - y * w)) * s[0],
        0.0,
        (2.0 * (x * y - z * w)) * s[1],
        (1.0 - 2.0 * (x * x + z * z)) * s[1],
        (2.0 * (y * z + x * w)) * s[1],
        0.0,
        (2.0 * (x * z + y * w)) * s[2],
        (2.0 * (y * z - x * w)) * s[2],
        (1.0 - 2.0 * (x * x + y * y)) * s[2],
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ])
}

fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    out
}

fn texture_slot(
    doc: &Value,
    slot: Option<&Value>,
    images: usize,
    scale_key: &str,
) -> Result<Option<TextureSlot>, LoadError> {
    let Some(slot) = slot else {
        return Ok(None);
    };
    let texture = list(doc, "textures")
        .get(usize_of(slot, "index")?)
        .ok_or(LoadError::Unrecognized)?;
    let Some(image) = opt_usize(texture, "source")? else {
        // Only extension sources (basisu, webp, ...): no image we can use.
        return Ok(None);
    };
    if image >= images {
        return Err(LoadError::Unrecognized);
    }
    let sampler = match opt_usize(texture, "sampler")? {
        None => Sampler::default(),
        Some(s) => {
            let s = list(doc, "samplers")
                .get(s)
                .ok_or(LoadError::Unrecognized)?;
            Sampler {
                mag_filter: opt_usize(s, "magFilter")?.unwrap_or(0) as u32,
                min_filter: opt_usize(s, "minFilter")?.unwrap_or(0) as u32,
                wrap_s: opt_usize(s, "wrapS")?.unwrap_or(10497) as u32,
                wrap_t: opt_usize(s, "wrapT")?.unwrap_or(10497) as u32,
            }
        }
    };
    Ok(Some(TextureSlot {
        image,
        texcoord: opt_usize(slot, "texCoord")?.unwrap_or(0) as u32,
        sampler,
        scale: if scale_key.is_empty() {
            1.0
        } else {
            f32_of(slot, scale_key, 1.0)?
        },
    }))
}

fn material(doc: &Value, m: &Value, images: usize) -> Result<PbrMaterial, LoadError> {
    let default_pbr = Value::Object(Vec::new());
    let pbr = m.get("pbrMetallicRoughness").unwrap_or(&default_pbr);
    let alpha_mode = match m
        .get("alphaMode")
        .and_then(Value::as_str)
        .unwrap_or("OPAQUE")
    {
        "OPAQUE" => AlphaMode::Opaque,
        "MASK" => AlphaMode::Mask(f32_of(m, "alphaCutoff", 0.5)?),
        "BLEND" => AlphaMode::Blend,
        _ => return Err(LoadError::Unrecognized),
    };
    let emissive_strength = match m
        .get("extensions")
        .and_then(|e| e.get("KHR_materials_emissive_strength"))
    {
        Some(ext) => f32_of(ext, "emissiveStrength", 1.0)?,
        None => 1.0,
    };
    Ok(PbrMaterial {
        name: m.get("name").and_then(Value::as_str).map(str::to_string),
        base_color: floats(pbr, "baseColorFactor", [1.0; 4])?,
        base_color_texture: texture_slot(doc, pbr.get("baseColorTexture"), images, "")?,
        metallic: f32_of(pbr, "metallicFactor", 1.0)?,
        roughness: f32_of(pbr, "roughnessFactor", 1.0)?,
        metallic_roughness_texture: texture_slot(
            doc,
            pbr.get("metallicRoughnessTexture"),
            images,
            "",
        )?,
        normal_texture: texture_slot(doc, m.get("normalTexture"), images, "scale")?,
        occlusion_texture: texture_slot(doc, m.get("occlusionTexture"), images, "strength")?,
        emissive: floats(m, "emissiveFactor", [0.0; 3])?,
        emissive_texture: texture_slot(doc, m.get("emissiveTexture"), images, "")?,
        emissive_strength,
        alpha_mode,
        double_sided: m
            .get("doubleSided")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

struct Reader<'a> {
    doc: &'a Value,
    buffers: &'a [Vec<u8>],
}

impl Reader<'_> {
    /// A whole buffer view (for images).
    fn view(&self, view: usize) -> Result<&[u8], LoadError> {
        let v = list(self.doc, "bufferViews")
            .get(view)
            .ok_or(LoadError::Unrecognized)?;
        let buffer = self
            .buffers
            .get(usize_of(v, "buffer")?)
            .ok_or(LoadError::Unrecognized)?;
        let offset = opt_usize(v, "byteOffset")?.unwrap_or(0);
        let length = usize_of(v, "byteLength")?;
        buffer
            .get(offset..offset + length)
            .ok_or(LoadError::Truncated)
    }

    /// An accessor as `count` rows of `N` floats (normalized integers become 0..1 or -1..1).
    fn floats<const N: usize>(&self, accessor: usize) -> Result<Vec<[f32; N]>, LoadError> {
        let rows = self.read(accessor, N)?;
        Ok(rows
            .chunks_exact(N)
            .map(|c| std::array::from_fn(|i| c[i] as f32))
            .collect())
    }

    fn read(&self, accessor: usize, want: usize) -> Result<Vec<f64>, LoadError> {
        let a = list(self.doc, "accessors")
            .get(accessor)
            .ok_or(LoadError::Unrecognized)?;
        let count = usize_of(a, "count")?;
        let comps = match a
            .get("type")
            .and_then(Value::as_str)
            .ok_or(LoadError::Unrecognized)?
        {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT2" => 4,
            "MAT3" => 9,
            "MAT4" => 16,
            _ => return Err(LoadError::Unrecognized),
        };
        if comps != want {
            return Err(LoadError::Unrecognized);
        }
        let ctype = usize_of(a, "componentType")?;
        let normalized = a
            .get("normalized")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let size = component_size(ctype)?;
        let mut out = vec![0.0f64; count * comps];
        if let Some(view) = opt_usize(a, "bufferView")? {
            let v = list(self.doc, "bufferViews")
                .get(view)
                .ok_or(LoadError::Unrecognized)?;
            let stride = opt_usize(v, "byteStride")?.unwrap_or(0).max(size * comps);
            let data = self.view(view)?;
            let base = opt_usize(a, "byteOffset")?.unwrap_or(0);
            for i in 0..count {
                for c in 0..comps {
                    let at = base + i * stride + c * size;
                    out[i * comps + c] = component(data, at, ctype, normalized)?;
                }
            }
        }
        if let Some(sparse) = a.get("sparse") {
            let n = usize_of(sparse, "count")?;
            let idx = sparse.get("indices").ok_or(LoadError::Unrecognized)?;
            let val = sparse.get("values").ok_or(LoadError::Unrecognized)?;
            let idata = self.view(usize_of(idx, "bufferView")?)?;
            let ioff = opt_usize(idx, "byteOffset")?.unwrap_or(0);
            let itype = usize_of(idx, "componentType")?;
            let vdata = self.view(usize_of(val, "bufferView")?)?;
            let voff = opt_usize(val, "byteOffset")?.unwrap_or(0);
            for k in 0..n {
                let row =
                    component(idata, ioff + k * component_size(itype)?, itype, false)? as usize;
                if row >= count {
                    return Err(LoadError::Unrecognized);
                }
                for c in 0..comps {
                    out[row * comps + c] =
                        component(vdata, voff + (k * comps + c) * size, ctype, normalized)?;
                }
            }
        }
        Ok(out)
    }

    fn primitive(&self, p: &Value, materials: usize) -> Result<Option<Primitive>, LoadError> {
        let mode = opt_usize(p, "mode")?.unwrap_or(4);
        if !(4..=6).contains(&mode) {
            return Ok(None);
        }
        let attrs = p.get("attributes").ok_or(LoadError::Unrecognized)?;
        let positions: Vec<[f32; 3]> = self.floats(usize_of(attrs, "POSITION")?)?;
        let n = positions.len();
        let attr = |key: &str| -> Result<Option<usize>, LoadError> { opt_usize(attrs, key) };
        let mut normals: Vec<[f32; 3]> = attr("NORMAL")?
            .map(|a| self.floats(a))
            .transpose()?
            .unwrap_or_default();
        let texcoords: Vec<[f32; 2]> = attr("TEXCOORD_0")?
            .map(|a| self.floats(a))
            .transpose()?
            .unwrap_or_default();
        let tangents: Vec<[f32; 4]> = attr("TANGENT")?
            .map(|a| self.floats(a))
            .transpose()?
            .unwrap_or_default();
        for len in [normals.len(), texcoords.len(), tangents.len()] {
            if len != 0 && len != n {
                return Err(LoadError::Unrecognized);
            }
        }
        let raw: Vec<u32> = match opt_usize(p, "indices")? {
            Some(a) => self.read(a, 1)?.into_iter().map(|i| i as u32).collect(),
            None => (0..n as u32).collect(),
        };
        if raw.iter().any(|&i| i as usize >= n) {
            return Err(LoadError::Unrecognized);
        }
        let mut indices = Vec::with_capacity(raw.len());
        match mode {
            4 => indices.extend(raw.chunks_exact(3).flatten()),
            5 => {
                for i in 2..raw.len() {
                    let (a, b) = if i % 2 == 0 {
                        (raw[i - 2], raw[i - 1])
                    } else {
                        (raw[i - 1], raw[i - 2])
                    };
                    indices.extend([a, b, raw[i]]);
                }
            }
            _ => {
                for i in 2..raw.len() {
                    indices.extend([raw[0], raw[i - 1], raw[i]]);
                }
            }
        }
        let material = opt_usize(p, "material")?;
        if material.is_some_and(|m| m >= materials) {
            return Err(LoadError::Unrecognized);
        }
        let mut primitive = Primitive {
            positions,
            normals: Vec::new(),
            texcoords,
            tangents,
            indices,
            material,
        };
        if normals.is_empty() {
            primitive = flat_shaded(primitive);
        } else {
            primitive.normals = std::mem::take(&mut normals);
        }
        Ok(Some(primitive))
    }
}

/// Unweld so every triangle has its own corners, with the face normal.
fn flat_shaded(p: Primitive) -> Primitive {
    let mut out = Primitive {
        material: p.material,
        ..Primitive::default()
    };
    for tri in p.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| p.positions[i as usize]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = if len > 0.0 {
            n.map(|x| x / len)
        } else {
            [0.0, 1.0, 0.0]
        };
        for &i in tri {
            out.indices.push(out.positions.len() as u32);
            out.positions.push(p.positions[i as usize]);
            out.normals.push(n);
            if !p.texcoords.is_empty() {
                out.texcoords.push(p.texcoords[i as usize]);
            }
            if !p.tangents.is_empty() {
                out.tangents.push(p.tangents[i as usize]);
            }
        }
    }
    out
}

fn component_size(ctype: usize) -> Result<usize, LoadError> {
    match ctype {
        5120 | 5121 => Ok(1),
        5122 | 5123 => Ok(2),
        5125 | 5126 => Ok(4),
        _ => Err(LoadError::Unrecognized),
    }
}

fn component(data: &[u8], at: usize, ctype: usize, normalized: bool) -> Result<f64, LoadError> {
    let b = data
        .get(at..at + component_size(ctype)?)
        .ok_or(LoadError::Truncated)?;
    Ok(match ctype {
        5120 => {
            let v = b[0] as i8 as f64;
            if normalized {
                (v / 127.0).max(-1.0)
            } else {
                v
            }
        }
        5121 => {
            let v = b[0] as f64;
            if normalized {
                v / 255.0
            } else {
                v
            }
        }
        5122 => {
            let v = i16::from_le_bytes([b[0], b[1]]) as f64;
            if normalized {
                (v / 32767.0).max(-1.0)
            } else {
                v
            }
        }
        5123 => {
            let v = u16::from_le_bytes([b[0], b[1]]) as f64;
            if normalized {
                v / 65535.0
            } else {
                v
            }
        }
        5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
    })
}
