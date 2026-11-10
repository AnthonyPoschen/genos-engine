//! `GSK1`, the cooked skin record (ADR 0038 Addition A).
//!
//! Little-endian. Magic `GSK1`, version 1, the same asset id and settings hash
//! as the `.gcook`, then joints (parent before child), influences, clips,
//! an empty retarget map, and notes. A SHA-256 trailer covers every byte
//! before it. `GCK1` does not grow.

use std::path::{Path, PathBuf};

use crate::{hex, sha256, Channel, ChannelPath, Clip, CookedError, Interpolation, Joint, Skin};

pub const GSKIN_MAGIC: &[u8; 4] = b"GSK1";
pub const GSKIN_VERSION: u32 = 1;

/// One primitive's four influences, in `.gcook` mesh and primitive order.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinPrimitive {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

/// What the cook writes. Joint order may still be the glTF order; [`encode_skin`]
/// puts parents first and remaps influences and channels.
#[derive(Clone, Debug)]
pub struct SkinCook {
    pub asset_id: [u8; 32],
    pub settings: [u8; 32],
    pub inputs: Vec<(String, [u8; 32])>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    pub instance_skins: Vec<Option<usize>>,
    /// One entry per mesh. `mesh_skin[i]` is the skin used by mesh `i`, when it has one.
    pub mesh_skin: Vec<Option<usize>>,
    pub primitives: Vec<SkinPrimitive>,
    /// Which mesh each primitive belongs to, parallel to [`Self::primitives`].
    pub primitive_mesh: Vec<usize>,
    pub notes: Vec<String>,
}

/// Bytes plus the clip-section size, which is `anim.clip_mb` before the unit conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedSkin {
    pub bytes: Vec<u8>,
    pub clip_bytes: u64,
}

/// A checked `.gskin`. `clip_bytes` is the size of the clip section.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedSkin {
    pub asset_id: [u8; 32],
    pub settings: [u8; 32],
    pub inputs: Vec<(String, [u8; 32])>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    /// Skin index per `.gcook` instance, or −1 when that instance is rigid.
    pub instance_skins: Vec<i32>,
    pub primitives: Vec<SkinPrimitive>,
    pub notes: Vec<String>,
    pub clip_bytes: u64,
}

/// `DIR/cooked/<asset>.gskin`.
#[must_use]
pub fn skin_path(dir: &Path, asset_id: &[u8; 32]) -> PathBuf {
    dir.join("cooked").join(format!("{}.gskin", hex(asset_id)))
}

/// # Errors
///
/// A joint index that does not fit in `u16`, or a channel aimed at a missing skin.
pub fn encode_skin(cook: &SkinCook) -> Result<EncodedSkin, CookedError> {
    let mut notes = cook.notes.clone();
    let (skins, remaps) = reorder(&cook.skins, &mut notes)?;
    let clips = remap_clips(&cook.clips, &remaps)?;
    let primitives = remap_primitives(cook, &remaps)?;
    let mut w = W(Vec::new());
    w.bytes(GSKIN_MAGIC);
    w.u32(GSKIN_VERSION);
    w.bytes(&cook.asset_id);
    w.bytes(&cook.settings);
    w.len(cook.inputs.len());
    for (uri, hash) in &cook.inputs {
        w.str(uri);
        w.bytes(hash);
    }
    w.len(skins.len());
    for skin in &skins {
        w.opt_str(skin.name.as_deref());
        w.len(skin.joints.len());
        for joint in &skin.joints {
            w.i32(joint.parent);
            w.str(&joint.name);
            for value in joint.inverse_bind {
                w.f32(value);
            }
            for value in joint.translation {
                w.f32(value);
            }
            for value in joint.rotation {
                w.f32(value);
            }
            for value in joint.scale {
                w.f32(value);
            }
        }
        w.len(skin.notes.len());
        for note in &skin.notes {
            w.str(note);
        }
    }
    w.len(cook.instance_skins.len());
    for skin in &cook.instance_skins {
        let index = match skin {
            Some(index) => i32::try_from(*index).map_err(|_| CookedError::Corrupt("gskin skin"))?,
            None => -1,
        };
        w.i32(index);
    }
    w.len(primitives.len());
    for primitive in &primitives {
        w.len(primitive.joints.len());
        for joints in &primitive.joints {
            for joint in joints {
                w.u16(*joint);
            }
        }
        for weights in &primitive.weights {
            for weight in weights {
                w.f32(*weight);
            }
        }
    }
    let clip_at = w.0.len();
    w.len(clips.len());
    for clip in &clips {
        w.str(&clip.name);
        w.f32(clip.duration);
        w.u32(u32::try_from(clip.skin).map_err(|_| CookedError::Corrupt("gskin clip"))?);
        w.len(clip.channels.len());
        for channel in &clip.channels {
            w.u16(channel.joint);
            w.u8(path_code(channel.path));
            w.u8(interp_code(channel.interpolation));
            w.len(channel.times.len());
            for time in &channel.times {
                w.f32(*time);
            }
            w.len(channel.values.len());
            for value in &channel.values {
                w.f32(*value);
            }
        }
    }
    let clip_bytes = u64::try_from(w.0.len() - clip_at).unwrap_or(u64::MAX);
    w.len(0);
    w.len(notes.len());
    for note in &notes {
        w.str(note);
    }
    let trailer = sha256(&w.0);
    w.bytes(&trailer);
    Ok(EncodedSkin {
        bytes: w.0,
        clip_bytes,
    })
}

/// # Errors
///
/// A short file, a bad magic, a bad version, or a trailer that does not match.
pub fn decode_skin(bytes: &[u8]) -> Result<DecodedSkin, CookedError> {
    let body = checked(bytes)?;
    let mut r = R { b: body, at: 0 };
    if r.take(4)? != GSKIN_MAGIC {
        return Err(CookedError::Corrupt("gskin magic"));
    }
    if r.u32()? != GSKIN_VERSION {
        return Err(CookedError::Corrupt("gskin version"));
    }
    let asset_id = r.array()?;
    let settings = r.array()?;
    let mut inputs = Vec::new();
    for _ in 0..r.len()? {
        let uri = r.str()?;
        inputs.push((uri, r.array()?));
    }
    let skins = read_skins(&mut r)?;
    let mut instance_skins = Vec::new();
    for _ in 0..r.len()? {
        instance_skins.push(r.i32()?);
    }
    let primitives = read_primitives(&mut r)?;
    let clip_at = r.at;
    let clips = read_clips(&mut r)?;
    let clip_bytes = u64::try_from(r.at - clip_at).unwrap_or(u64::MAX);
    if r.len()? != 0 {
        return Err(CookedError::Corrupt("gskin retarget"));
    }
    let mut notes = Vec::new();
    for _ in 0..r.len()? {
        notes.push(r.str()?);
    }
    if r.at != body.len() {
        return Err(CookedError::Corrupt("gskin trailing"));
    }
    Ok(DecodedSkin {
        asset_id,
        settings,
        inputs,
        skins,
        clips,
        instance_skins,
        primitives,
        notes,
        clip_bytes,
    })
}

fn read_skins(r: &mut R<'_>) -> Result<Vec<Skin>, CookedError> {
    let mut skins = Vec::new();
    for _ in 0..r.len()? {
        let name = r.opt_str()?;
        let mut joints = Vec::new();
        for _ in 0..r.len()? {
            joints.push(Joint {
                parent: r.i32()?,
                name: r.str()?,
                inverse_bind: r.f32s()?,
                translation: r.f32s()?,
                rotation: r.f32s()?,
                scale: r.f32s()?,
                node: 0,
            });
        }
        let mut notes = Vec::new();
        for _ in 0..r.len()? {
            notes.push(r.str()?);
        }
        skins.push(Skin {
            name,
            joints,
            notes,
        });
    }
    Ok(skins)
}

fn read_primitives(r: &mut R<'_>) -> Result<Vec<SkinPrimitive>, CookedError> {
    let mut primitives = Vec::new();
    for _ in 0..r.len()? {
        let count = r.len()?;
        let mut joints = Vec::with_capacity(count);
        for _ in 0..count {
            joints.push([r.u16()?, r.u16()?, r.u16()?, r.u16()?]);
        }
        let mut weights = Vec::with_capacity(count);
        for _ in 0..count {
            weights.push([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
        }
        primitives.push(SkinPrimitive { joints, weights });
    }
    Ok(primitives)
}

fn read_clips(r: &mut R<'_>) -> Result<Vec<Clip>, CookedError> {
    let mut clips = Vec::new();
    for _ in 0..r.len()? {
        let name = r.str()?;
        let duration = r.f32()?;
        let skin = usize::try_from(r.u32()?).map_err(|_| CookedError::Corrupt("gskin clip"))?;
        let mut channels = Vec::new();
        for _ in 0..r.len()? {
            channels.push(Channel {
                joint: r.u16()?,
                path: path_of(r.u8()?)?,
                interpolation: interp_of(r.u8()?)?,
                times: r.f32_vec()?,
                values: r.f32_vec()?,
            });
        }
        clips.push(Clip {
            name,
            duration,
            skin,
            channels,
        });
    }
    Ok(clips)
}

fn reorder(
    skins: &[Skin],
    notes: &mut Vec<String>,
) -> Result<(Vec<Skin>, Vec<Vec<u16>>), CookedError> {
    let mut out = Vec::with_capacity(skins.len());
    let mut remaps = Vec::with_capacity(skins.len());
    for (index, skin) in skins.iter().enumerate() {
        let (ordered, remap, moved) = parent_first(skin)?;
        if moved {
            notes.push(format!(
                "skins[{index}] joints reordered parent before child"
            ));
        }
        out.push(ordered);
        remaps.push(remap);
    }
    Ok((out, remaps))
}

fn parent_first(skin: &Skin) -> Result<(Skin, Vec<u16>, bool), CookedError> {
    let n = skin.joints.len();
    let mut children = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (index, joint) in skin.joints.iter().enumerate() {
        if let Ok(parent) = usize::try_from(joint.parent) {
            if parent < n && parent != index {
                children[parent].push(index);
                continue;
            }
        }
        roots.push(index);
    }
    let mut order = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    let mut queue = roots;
    let mut cursor = 0;
    while let Some(&index) = queue.get(cursor) {
        cursor += 1;
        if seen[index] {
            continue;
        }
        seen[index] = true;
        order.push(index);
        queue.extend_from_slice(&children[index]);
    }
    for (index, visited) in seen.iter().enumerate() {
        if !visited {
            order.push(index);
        }
    }
    let mut remap = vec![0u16; n];
    for (new_index, &old) in order.iter().enumerate() {
        remap[old] = u16::try_from(new_index).map_err(|_| CookedError::Corrupt("gskin joint"))?;
    }
    let moved = order.iter().enumerate().any(|(index, &old)| index != old);
    let joints = order
        .into_iter()
        .map(|old| {
            let joint = &skin.joints[old];
            let parent = usize::try_from(joint.parent)
                .ok()
                .filter(|parent| *parent < n)
                .map_or(-1, |parent| i32::from(remap[parent]));
            Joint {
                name: joint.name.clone(),
                parent,
                inverse_bind: joint.inverse_bind,
                translation: joint.translation,
                rotation: joint.rotation,
                scale: joint.scale,
                node: joint.node,
            }
        })
        .collect();
    Ok((
        Skin {
            name: skin.name.clone(),
            joints,
            notes: skin.notes.clone(),
        },
        remap,
        moved,
    ))
}

fn remap_clips(clips: &[Clip], remaps: &[Vec<u16>]) -> Result<Vec<Clip>, CookedError> {
    let mut out = Vec::with_capacity(clips.len());
    for clip in clips {
        let remap = remaps
            .get(clip.skin)
            .ok_or(CookedError::Corrupt("gskin clip"))?;
        let mut channels = Vec::with_capacity(clip.channels.len());
        for channel in &clip.channels {
            let joint = remap
                .get(usize::from(channel.joint))
                .copied()
                .ok_or(CookedError::Corrupt("gskin channel"))?;
            channels.push(Channel {
                joint,
                path: channel.path,
                interpolation: channel.interpolation,
                times: channel.times.clone(),
                values: channel.values.clone(),
            });
        }
        out.push(Clip {
            name: clip.name.clone(),
            duration: clip.duration,
            skin: clip.skin,
            channels,
        });
    }
    Ok(out)
}

fn remap_primitives(
    cook: &SkinCook,
    remaps: &[Vec<u16>],
) -> Result<Vec<SkinPrimitive>, CookedError> {
    if cook.primitives.len() != cook.primitive_mesh.len() {
        return Err(CookedError::Corrupt("gskin primitive"));
    }
    let mut out = Vec::with_capacity(cook.primitives.len());
    for (primitive, &mesh) in cook.primitives.iter().zip(&cook.primitive_mesh) {
        if primitive.joints.len() != primitive.weights.len() {
            return Err(CookedError::Corrupt("gskin influences"));
        }
        let skin = cook.mesh_skin.get(mesh).copied().flatten();
        let Some(skin) = skin else {
            out.push(primitive.clone());
            continue;
        };
        let remap = remaps.get(skin).ok_or(CookedError::Corrupt("gskin skin"))?;
        let mut joints = Vec::with_capacity(primitive.joints.len());
        for row in &primitive.joints {
            let mut mapped = [0u16; 4];
            for (slot, joint) in row.iter().enumerate() {
                mapped[slot] = remap
                    .get(usize::from(*joint))
                    .copied()
                    .ok_or(CookedError::Corrupt("gskin influence"))?;
            }
            joints.push(mapped);
        }
        out.push(SkinPrimitive {
            joints,
            weights: primitive.weights.clone(),
        });
    }
    Ok(out)
}

const fn path_code(path: ChannelPath) -> u8 {
    match path {
        ChannelPath::Translation => 0,
        ChannelPath::Rotation => 1,
        ChannelPath::Scale => 2,
    }
}

const fn interp_code(interpolation: Interpolation) -> u8 {
    match interpolation {
        Interpolation::Linear => 0,
        Interpolation::Step => 1,
    }
}

fn path_of(code: u8) -> Result<ChannelPath, CookedError> {
    match code {
        0 => Ok(ChannelPath::Translation),
        1 => Ok(ChannelPath::Rotation),
        2 => Ok(ChannelPath::Scale),
        _ => Err(CookedError::Corrupt("gskin path")),
    }
}

fn interp_of(code: u8) -> Result<Interpolation, CookedError> {
    match code {
        0 => Ok(Interpolation::Linear),
        1 => Ok(Interpolation::Step),
        _ => Err(CookedError::Corrupt("gskin interpolation")),
    }
}

fn checked(bytes: &[u8]) -> Result<&[u8], CookedError> {
    let split = bytes
        .len()
        .checked_sub(32)
        .ok_or(CookedError::Corrupt("gskin short"))?;
    let (body, trailer) = bytes.split_at(split);
    if sha256(body) != trailer {
        return Err(CookedError::Corrupt("gskin trailer"));
    }
    Ok(body)
}

struct W(Vec<u8>);

impl W {
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn len(&mut self, n: usize) {
        self.bytes(&u64::try_from(n).unwrap_or(u64::MAX).to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.bytes(s.as_bytes());
    }
    fn opt_str(&mut self, s: Option<&str>) {
        self.u8(u8::from(s.is_some()));
        self.str(s.unwrap_or(""));
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CookedError> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(CookedError::Corrupt("gskin short"))?;
        let s = self
            .b
            .get(self.at..end)
            .ok_or(CookedError::Corrupt("gskin short"))?;
        self.at = end;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CookedError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CookedError::Corrupt("gskin short"))
    }
    fn u32(&mut self) -> Result<u32, CookedError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u16(&mut self) -> Result<u16, CookedError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u8(&mut self) -> Result<u8, CookedError> {
        Ok(self.take(1)?[0])
    }
    fn i32(&mut self) -> Result<i32, CookedError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, CookedError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn len(&mut self) -> Result<usize, CookedError> {
        let n = u64::from_le_bytes(self.array()?);
        let n = usize::try_from(n).map_err(|_| CookedError::Corrupt("gskin length"))?;
        if n > self.b.len() {
            return Err(CookedError::Corrupt("gskin length"));
        }
        Ok(n)
    }
    fn str(&mut self) -> Result<String, CookedError> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| CookedError::Corrupt("gskin utf8"))
    }
    fn opt_str(&mut self) -> Result<Option<String>, CookedError> {
        let some = self.u8()? != 0;
        let s = self.str()?;
        Ok(some.then_some(s))
    }
    fn f32s<const N: usize>(&mut self) -> Result<[f32; N], CookedError> {
        let mut out = [0.0; N];
        for value in &mut out {
            *value = self.f32()?;
        }
        Ok(out)
    }
    fn f32_vec(&mut self) -> Result<Vec<f32>, CookedError> {
        let n = self.len()?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.f32()?);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::{decode_skin, encode_skin, SkinCook, SkinPrimitive};
    use crate::{Channel, ChannelPath, Clip, Interpolation, Joint, Skin, IDENTITY_BIND};

    fn joint(name: &str, parent: i32) -> Joint {
        Joint {
            name: name.into(),
            parent,
            inverse_bind: IDENTITY_BIND,
            translation: [0.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            node: 0,
        }
    }

    #[test]
    fn a_child_listed_first_is_written_after_its_parent() {
        let cook = SkinCook {
            asset_id: [7; 32],
            settings: [9; 32],
            inputs: vec![(String::new(), [7; 32])],
            skins: vec![Skin {
                name: Some("body".into()),
                joints: vec![joint("knee", 1), joint("hips", -1)],
                notes: vec!["skins[0] inverse bind missing".into()],
            }],
            clips: vec![Clip {
                name: "move".into(),
                duration: 1.0,
                skin: 0,
                channels: vec![Channel {
                    joint: 0,
                    path: ChannelPath::Translation,
                    interpolation: Interpolation::Linear,
                    times: vec![0.0, 1.0],
                    values: vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0],
                }],
            }],
            instance_skins: vec![Some(0)],
            mesh_skin: vec![Some(0)],
            primitives: vec![SkinPrimitive {
                joints: vec![[0, 1, 0, 0]],
                weights: vec![[1.0, 0.0, 0.0, 0.0]],
            }],
            primitive_mesh: vec![0],
            notes: vec!["meshes[0].primitives[0] omitted path weights".into()],
        };
        let encoded = encode_skin(&cook).expect("encode");
        let decoded = decode_skin(&encoded.bytes).expect("decode");
        assert_eq!(decoded.skins[0].joints[0].name, "hips");
        assert_eq!(decoded.skins[0].joints[0].parent, -1);
        assert_eq!(decoded.skins[0].joints[1].name, "knee");
        assert_eq!(decoded.skins[0].joints[1].parent, 0);
        assert_eq!(decoded.clips[0].channels[0].joint, 1);
        assert_eq!(decoded.primitives[0].joints[0], [1, 0, 1, 1]);
        assert!(decoded
            .notes
            .iter()
            .any(|note| note.contains("parent before child")));
        let mut broken = encoded.bytes.clone();
        let last = broken.len() - 1;
        broken[last] ^= 0xff;
        assert!(decode_skin(&broken).is_err());
    }
}
