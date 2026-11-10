//! A glTF skin and the clips that move it.
//!
//! A joint is one entry in a skin. Channel sampling reads the source `f32`
//! keys. `linear` blends translation and scale with `a + (b - a) * t`, and
//! rotation with short-arc slerp. `step` holds the left key until the next
//! key time, and uses that next key when the time lands on it.

use genos_math::Quat;

use crate::LoadError;

/// Column-major identity, the inverse bind used when glTF omits the accessor.
pub const IDENTITY_BIND: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Which local property a channel writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelPath {
    Translation,
    Rotation,
    Scale,
}

impl ChannelPath {
    /// Floats stored per key.
    #[must_use]
    pub const fn stride(self) -> usize {
        match self {
            Self::Translation | Self::Scale => 3,
            Self::Rotation => 4,
        }
    }
}

/// glTF sampler interpolation this loader keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
}

/// One joint in a skin, parent before the children when the cook writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    /// Index into the same skin, or −1 when this joint is a root.
    pub parent: i32,
    /// Column-major inverse bind matrix, glTF order.
    pub inverse_bind: [f32; 16],
    pub translation: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    /// glTF node this joint came from.
    pub node: usize,
}

/// One skeleton.
#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    pub name: Option<String>,
    pub joints: Vec<Joint>,
    /// Dropped or repaired features for this skin (missing inverse bind, and so on).
    pub notes: Vec<String>,
}

/// One property track aimed at a joint.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub joint: u16,
    pub path: ChannelPath,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// Tightly packed keys. The stride is [`ChannelPath::stride`].
    pub values: Vec<f32>,
}

/// Channels that share one glTF animation and one skin.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Seconds. The latest key time, or 0 when the clip has no keys.
    pub duration: f32,
    /// Index into [`crate::GltfScene::skins`].
    pub skin: usize,
    pub channels: Vec<Channel>,
}

/// What [`pack_influences`] changed while keeping four weights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InfluenceFlags {
    /// A vertex had more than four non-zero weights.
    pub reduced: bool,
    /// A vertex's weights summed to zero, so joint 0 carries weight 1.
    pub zero_sum: bool,
}

/// Four influences per vertex, after the reduction in [`pack_influences`].
#[derive(Clone, Debug, PartialEq)]
pub struct PackedInfluences {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub flags: InfluenceFlags,
}

/// A joint index stored in a `JOINTS_*` accessor. glTF integer components
/// arrive as the exact `f32` of that integer.
///
/// # Errors
///
/// A non-finite, negative, fractional, or out-of-range value is
/// [`LoadError::Unrecognized`].
pub fn joint_index(value: f32) -> Result<u16, LoadError> {
    if !value.is_finite() || value < 0.0 || value > f32::from(u16::MAX) {
        return Err(LoadError::Unrecognized);
    }
    let target = value.round();
    if (target - value).abs() > 1.0e-3 {
        return Err(LoadError::Unrecognized);
    }
    let mut lo = 0u32;
    let mut hi = u32::from(u16::MAX);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let mid_f = u16::try_from(mid).map_or(f32::from(u16::MAX), f32::from);
        if mid_f < target {
            lo = mid.saturating_add(1);
        } else {
            hi = mid;
        }
    }
    let found = u16::try_from(lo).map_err(|_| LoadError::Unrecognized)?;
    if f32::from(found).to_bits() == target.to_bits() {
        Ok(found)
    } else {
        Err(LoadError::Unrecognized)
    }
}

/// Keep four influences per vertex.
///
/// `extra` is `JOINTS_1` / `WEIGHTS_1` when the file has them. More than four
/// non-zero weights keep the four largest, then those four are divided by
/// their sum. A zero sum writes weight 1 on joint 0.
///
/// # Errors
///
/// A length mismatch or a joint index that is not an integer is
/// [`LoadError::Unrecognized`].
pub fn pack_influences(
    primary_joints: &[[f32; 4]],
    primary_weights: &[[f32; 4]],
    extra_joints: Option<&[[f32; 4]]>,
    extra_weights: Option<&[[f32; 4]]>,
) -> Result<PackedInfluences, LoadError> {
    if primary_joints.len() != primary_weights.len() {
        return Err(LoadError::Unrecognized);
    }
    if let (Some(joints), Some(weights)) = (extra_joints, extra_weights) {
        if joints.len() != primary_joints.len() || weights.len() != primary_joints.len() {
            return Err(LoadError::Unrecognized);
        }
    } else if extra_joints.is_some() || extra_weights.is_some() {
        return Err(LoadError::Unrecognized);
    }
    let mut out_j = Vec::with_capacity(primary_joints.len());
    let mut out_w = Vec::with_capacity(primary_joints.len());
    let mut flags = InfluenceFlags::default();
    for (index, (joints, weights)) in primary_joints.iter().zip(primary_weights).enumerate() {
        let mut slots = [(0u16, 0.0f32); 8];
        let mut count = 0usize;
        push_set(&mut slots, &mut count, joints, weights)?;
        if let (Some(joints), Some(weights)) = (extra_joints, extra_weights) {
            push_set(&mut slots, &mut count, &joints[index], &weights[index])?;
        }
        let nonzero = (0..count).filter(|&i| slots[i].1 > 0.0).count();
        let mut order = [0usize; 8];
        for (slot, item) in order.iter_mut().enumerate() {
            *item = slot;
        }
        let keep = if nonzero > 4 {
            flags.reduced = true;
            order[..count]
                .sort_by(|&a, &b| slots[b].1.total_cmp(&slots[a].1).then_with(|| a.cmp(&b)));
            4
        } else {
            count.min(4)
        };
        let mut picked = [(0u16, 0.0f32); 4];
        let mut sum = 0.0f32;
        for slot in 0..keep {
            let (joint, weight) = slots[order[slot]];
            picked[slot] = (joint, weight.max(0.0));
            sum += picked[slot].1;
        }
        if sum <= 0.0 {
            flags.zero_sum = true;
            picked = [(0, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)];
        } else if nonzero > 4 {
            let inv = 1.0 / sum;
            for slot in &mut picked {
                slot.1 *= inv;
            }
        }
        out_j.push([picked[0].0, picked[1].0, picked[2].0, picked[3].0]);
        out_w.push([picked[0].1, picked[1].1, picked[2].1, picked[3].1]);
    }
    Ok(PackedInfluences {
        joints: out_j,
        weights: out_w,
        flags,
    })
}

fn push_set(
    slots: &mut [(u16, f32); 8],
    count: &mut usize,
    joints: &[f32; 4],
    weights: &[f32; 4],
) -> Result<(), LoadError> {
    for (joint, weight) in joints.iter().zip(weights) {
        if *count >= slots.len() {
            return Err(LoadError::Unrecognized);
        }
        slots[*count] = (joint_index(*joint)?, *weight);
        *count += 1;
    }
    Ok(())
}

/// Write the channel's value at `time` into `out`.
///
/// `out` must hold at least the path stride. The function does not allocate.
/// Time before the first key returns that key. Time after the last key returns
/// that key.
#[must_use]
pub fn sample_channel(channel: &Channel, time: f32, out: &mut [f32]) -> bool {
    let stride = channel.path.stride();
    if out.len() < stride || !time.is_finite() {
        return false;
    }
    let keys = channel.times.len().min(channel.values.len() / stride);
    if keys == 0 {
        return false;
    }
    let copy = |index: usize, out: &mut [f32]| {
        let base = index * stride;
        out[..stride].copy_from_slice(&channel.values[base..base + stride]);
    };
    if time <= channel.times[0] {
        copy(0, out);
        return true;
    }
    let mut index = 0usize;
    while index + 1 < keys && channel.times[index + 1] <= time {
        index += 1;
    }
    if channel.interpolation == Interpolation::Step || index + 1 >= keys {
        copy(index, out);
        return true;
    }
    let left_t = channel.times[index];
    let right_t = channel.times[index + 1];
    let span = right_t - left_t;
    if span <= 0.0 {
        copy(index, out);
        return true;
    }
    let t = (time - left_t) / span;
    let right = index + 1;
    match channel.path {
        ChannelPath::Translation | ChannelPath::Scale => {
            for (component, slot) in out.iter_mut().enumerate().take(stride) {
                let a = channel.values[index * stride + component];
                let b = channel.values[right * stride + component];
                *slot = a + (b - a) * t;
            }
        }
        ChannelPath::Rotation => {
            let a = quat_at(channel, index);
            let b = quat_at(channel, right);
            let blended = a.slerp(b, t);
            out[0] = blended.x;
            out[1] = blended.y;
            out[2] = blended.z;
            out[3] = blended.w;
        }
    }
    true
}

fn quat_at(channel: &Channel, index: usize) -> Quat {
    let base = index * 4;
    Quat::new(
        channel.values[base],
        channel.values[base + 1],
        channel.values[base + 2],
        channel.values[base + 3],
    )
}

/// Local translation, rotation, and scale sampled from `clip` at `time`.
///
/// A joint with no channel keeps `rest`. `locals` is one TRS per joint and
/// must already be that long. The function writes in place and does not allocate.
pub fn sample_clip(
    clip: &Clip,
    rest: &[[f32; 3]],
    rest_rot: &[[f32; 4]],
    rest_scale: &[[f32; 3]],
    time: f32,
    locals: &mut [[f32; 10]],
) {
    let count = locals
        .len()
        .min(rest.len())
        .min(rest_rot.len())
        .min(rest_scale.len());
    for index in 0..count {
        locals[index][0] = rest[index][0];
        locals[index][1] = rest[index][1];
        locals[index][2] = rest[index][2];
        locals[index][3] = rest_rot[index][0];
        locals[index][4] = rest_rot[index][1];
        locals[index][5] = rest_rot[index][2];
        locals[index][6] = rest_rot[index][3];
        locals[index][7] = rest_scale[index][0];
        locals[index][8] = rest_scale[index][1];
        locals[index][9] = rest_scale[index][2];
    }
    let mut value = [0.0f32; 4];
    for channel in &clip.channels {
        let joint = usize::from(channel.joint);
        if joint >= count || !sample_channel(channel, time, &mut value) {
            continue;
        }
        match channel.path {
            ChannelPath::Translation => {
                locals[joint][0] = value[0];
                locals[joint][1] = value[1];
                locals[joint][2] = value[2];
            }
            ChannelPath::Rotation => {
                locals[joint][3] = value[0];
                locals[joint][4] = value[1];
                locals[joint][5] = value[2];
                locals[joint][6] = value[3];
            }
            ChannelPath::Scale => {
                locals[joint][7] = value[0];
                locals[joint][8] = value[1];
                locals[joint][9] = value[2];
            }
        }
    }
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::*;

    #[test]
    fn linear_midpoint_is_the_exact_half() {
        let channel = Channel {
            joint: 0,
            path: ChannelPath::Translation,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: vec![0.0, 0.0, 0.0, 2.0, 4.0, 6.0],
        };
        let mut out = [0.0; 3];
        assert!(sample_channel(&channel, 0.5, &mut out));
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn step_holds_the_left_key_until_the_next_time() {
        let channel = Channel {
            joint: 1,
            path: ChannelPath::Translation,
            interpolation: Interpolation::Step,
            times: vec![0.0, 1.0],
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        };
        let mut just_before = [0.0; 3];
        let mut on_key = [0.0; 3];
        assert!(sample_channel(&channel, 0.999, &mut just_before));
        assert!(sample_channel(&channel, 1.0, &mut on_key));
        assert_eq!(just_before, [1.0, 2.0, 3.0]);
        assert_eq!(on_key, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn five_influences_keep_the_four_largest_and_renormalize() {
        let primary_j = vec![[0.0, 1.0, 0.0, 1.0]];
        let primary_w = vec![[0.5, 0.25, 0.125, 0.0625]];
        let extra_j = vec![[1.0, 0.0, 0.0, 0.0]];
        let extra_w = vec![[0.125, 0.0, 0.0, 0.0]];
        let packed =
            pack_influences(&primary_j, &primary_w, Some(&extra_j), Some(&extra_w)).expect("pack");
        assert!(packed.flags.reduced);
        assert!(!packed.flags.zero_sum);
        assert_eq!(packed.joints, vec![[0, 1, 0, 1]]);
        assert_eq!(packed.weights, vec![[0.5, 0.25, 0.125, 0.125]]);
    }

    #[test]
    fn a_zero_weight_sum_writes_joint_zero() {
        let joints = vec![[0.0, 1.0, 2.0, 3.0]];
        let weights = vec![[0.0, 0.0, 0.0, 0.0]];
        let packed = pack_influences(&joints, &weights, None, None).expect("pack");
        assert!(packed.flags.zero_sum);
        assert_eq!(packed.joints, vec![[0, 0, 0, 0]]);
        assert_eq!(packed.weights, vec![[1.0, 0.0, 0.0, 0.0]]);
    }
}
