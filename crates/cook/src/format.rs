//! The `.gcook` record, version 1. Little-endian.
//!
//! `GCK1`, version, asset id, settings hash, inputs (uri, SHA-256), then the
//! meshes, the instances and the unsupported notes, then a SHA-256 of every
//! byte before it. A record of another version, or one whose trailer does not
//! match, is not returned.

use genos_load::{sha256, GltfMesh, MeshInstance, Primitive};

use crate::{CookError, Cooked, CookedMesh};

pub const MAGIC: &[u8; 4] = b"GCK1";
pub const FORMAT_VERSION: u32 = 1;
const NONE: u32 = u32::MAX;

/// One file the cook read. The source itself has an empty `uri`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub uri: String,
    pub hash: [u8; 32],
}

pub(crate) struct Header {
    pub settings: [u8; 32],
    pub inputs: Vec<Input>,
}

#[must_use]
pub fn encode(c: &Cooked) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.0.extend_from_slice(MAGIC);
    w.u32(FORMAT_VERSION);
    w.0.extend_from_slice(&c.asset_id);
    w.0.extend_from_slice(&c.settings);
    w.len(c.inputs.len());
    for input in &c.inputs {
        w.str(&input.uri);
        w.0.extend_from_slice(&input.hash);
    }
    w.len(c.meshes.len());
    for m in &c.meshes {
        w.0.extend_from_slice(&m.bake_key.to_le_bytes());
        w.opt_str(m.mesh.name.as_deref());
        w.len(m.mesh.primitives.len());
        for p in &m.mesh.primitives {
            w.u32(
                p.material
                    .and_then(|i| u32::try_from(i).ok())
                    .unwrap_or(NONE),
            );
            w.floats(&p.positions);
            w.floats(&p.normals);
            w.floats(&p.texcoords);
            w.len(p.texcoord_sets.len());
            for set in &p.texcoord_sets {
                w.floats(set);
            }
            w.floats(&p.tangents);
            w.len(p.indices.len());
            p.indices.iter().for_each(|&i| w.u32(i));
        }
    }
    w.len(c.instances.len());
    for i in &c.instances {
        w.len(i.mesh);
        w.opt_str(i.node_name.as_deref());
        i.transform.iter().for_each(|&v| w.u32(v.to_bits()));
    }
    w.len(c.unsupported.len());
    c.unsupported.iter().for_each(|s| w.str(s));
    let trailer = sha256(&w.0);
    w.0.extend_from_slice(&trailer);
    w.0
}

/// Decode and check a record.
///
/// # Errors
///
/// A bad magic, version, length or trailer is `CookError::Corrupt`.
pub fn decode(bytes: &[u8]) -> Result<Cooked, CookError> {
    let body = checked_body(bytes)?;
    let mut r = R { b: body, at: 0 };
    let (asset_id, header) = header(&mut r)?;
    let mut meshes = Vec::new();
    for _ in 0..r.len()? {
        let bake_key = u64::from_le_bytes(r.array()?);
        let name = r.opt_str()?;
        let mut primitives = Vec::new();
        for _ in 0..r.len()? {
            let material = r.u32()?;
            let positions = r.floats()?;
            let normals = r.floats()?;
            let texcoords = r.floats()?;
            let mut texcoord_sets = Vec::new();
            for _ in 0..r.len()? {
                texcoord_sets.push(r.floats()?);
            }
            let tangents = r.floats()?;
            let mut indices = Vec::new();
            for _ in 0..r.len()? {
                indices.push(r.u32()?);
            }
            primitives.push(Primitive {
                positions,
                normals,
                texcoords,
                texcoord_sets,
                tangents,
                indices,
                material: (material != NONE)
                    .then(|| usize::try_from(material).ok())
                    .flatten(),
            });
        }
        meshes.push(CookedMesh {
            mesh: GltfMesh { name, primitives },
            bake_key,
        });
    }
    let mut instances = Vec::new();
    for _ in 0..r.len()? {
        let mesh = r.len()?;
        let node_name = r.opt_str()?;
        let mut transform = [0.0; 16];
        for v in &mut transform {
            *v = f32::from_bits(r.u32()?);
        }
        instances.push(MeshInstance {
            mesh,
            node_name,
            transform,
        });
    }
    let mut unsupported = Vec::new();
    for _ in 0..r.len()? {
        unsupported.push(r.str()?);
    }
    if r.at != body.len() {
        return Err(CookError::Corrupt("trailing bytes"));
    }
    Ok(Cooked {
        asset_id,
        settings: header.settings,
        inputs: header.inputs,
        meshes,
        instances,
        unsupported,
    })
}

/// The header alone, for the up-to-date check. The trailer is checked too.
pub(crate) fn decode_header(bytes: &[u8]) -> Result<Header, CookError> {
    let body = checked_body(bytes)?;
    header(&mut R { b: body, at: 0 }).map(|(_, h)| h)
}

pub(crate) fn checked_body(bytes: &[u8]) -> Result<&[u8], CookError> {
    let split = bytes
        .len()
        .checked_sub(32)
        .ok_or(CookError::Corrupt("short"))?;
    let (body, trailer) = bytes.split_at(split);
    if sha256(body) != trailer {
        return Err(CookError::Corrupt("content hash mismatch"));
    }
    Ok(body)
}

fn header(r: &mut R) -> Result<([u8; 32], Header), CookError> {
    if r.take(4)? != MAGIC {
        return Err(CookError::Corrupt("magic"));
    }
    if r.u32()? != FORMAT_VERSION {
        return Err(CookError::Corrupt("version"));
    }
    let asset_id = r.array()?;
    let settings = r.array()?;
    let mut inputs = Vec::new();
    for _ in 0..r.len()? {
        let uri = r.str()?;
        inputs.push(Input {
            uri,
            hash: r.array()?,
        });
    }
    Ok((asset_id, Header { settings, inputs }))
}

pub(crate) struct W(pub(crate) Vec<u8>);

impl W {
    pub(crate) fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn len(&mut self, n: usize) {
        self.0
            .extend_from_slice(&u64::try_from(n).unwrap_or(u64::MAX).to_le_bytes());
    }
    pub(crate) fn str(&mut self, s: &str) {
        self.len(s.len());
        self.0.extend_from_slice(s.as_bytes());
    }
    fn opt_str(&mut self, s: Option<&str>) {
        self.0.push(u8::from(s.is_some()));
        self.str(s.unwrap_or(""));
    }
    fn floats<const N: usize>(&mut self, v: &[[f32; N]]) {
        self.len(v.len());
        for x in v.iter().flatten() {
            self.u32(x.to_bits());
        }
    }
}

pub(crate) struct R<'a> {
    pub(crate) b: &'a [u8],
    pub(crate) at: usize,
}

impl R<'_> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&[u8], CookError> {
        let end = self.at.checked_add(n).ok_or(CookError::Corrupt("length"))?;
        let s = self
            .b
            .get(self.at..end)
            .ok_or(CookError::Corrupt("truncated"))?;
        self.at = end;
        Ok(s)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], CookError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CookError::Corrupt("truncated"))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, CookError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub(crate) fn len(&mut self) -> Result<usize, CookError> {
        let n = u64::from_le_bytes(self.array()?);
        let n = usize::try_from(n).map_err(|_| CookError::Corrupt("length"))?;
        if n > self.b.len() {
            return Err(CookError::Corrupt("length"));
        }
        Ok(n)
    }
    pub(crate) fn str(&mut self) -> Result<String, CookError> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| CookError::Corrupt("utf-8"))
    }
    fn opt_str(&mut self) -> Result<Option<String>, CookError> {
        let some = self.array::<1>()?[0] != 0;
        let s = self.str()?;
        Ok(some.then_some(s))
    }
    fn floats<const N: usize>(&mut self) -> Result<Vec<[f32; N]>, CookError> {
        let n = self.len()?;
        let mut out = Vec::with_capacity(n.min(self.b.len() / 4));
        for _ in 0..n {
            let mut v = [0.0; N];
            for x in &mut v {
                *x = f32::from_bits(self.u32()?);
            }
            out.push(v);
        }
        Ok(out)
    }
}
