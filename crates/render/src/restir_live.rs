//! `CPU` side of the live `ReSTIR` `DI` prefix (`ADR` 0022 stage C5).
//!
//! The light tree is the checker tree (`genos-restir-ref`). The `GPU` walks it
//! with the same shader includes. This module only packs the header, the
//! point lamps, and the tree into the binding-19 prefix. Reservoirs stay
//! on the device.

use genos_math::Vec3;
use genos_restir_ref::shade::Lamp;
use genos_restir_ref::{LightTree, DEFAULT_M_CLAMP};

use crate::pack::GpuLamp;

/// Header words. The live shader reads these indices.
pub const HEADER_WORDS: usize = 64;
pub const LAMP_STRIDE: usize = 8;
/// Reservoir words per history slot (lamp, weight, M, target, depth, normal).
pub const HIST_STRIDE: u32 = 6;
pub const CANDIDATES: u32 = 8;
pub const PROPOSAL_TREE: u32 = 2;
pub const SPATIAL_NEIGHBORS: u32 = 4;
pub const SPATIAL_RADIUS: u32 = 2;
/// Worst prefix: header, 4096 lamps, and a full binary tree (two nodes per lamp).
pub const PREFIX_CAP_WORDS: u64 = 64 + 4096 * 8 + 8192 * 12;

/// Previous camera, in the same basis the scene block stores.
#[derive(Clone, Copy, Debug, Default)]
pub struct RestirCam {
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub tan: f32,
    pub aspect: f32,
    pub valid: bool,
}

/// Words uploaded before the reservoirs, and the device buffer size in words.
pub struct RestirPrefix {
    pub words: Vec<u32>,
    pub total_words: u64,
}

/// Device bytes for `pixels`, including the prefix cap. Reservoirs are not
/// in the upload; the cap keeps a lamp-list change from moving them.
#[must_use]
pub fn buffer_bytes(pixels: u64) -> u64 {
    let reservoirs = pixels.saturating_mul(u64::from(HIST_STRIDE) * 2 + 4) * 4;
    PREFIX_CAP_WORDS.saturating_mul(4).saturating_add(reservoirs)
}

fn push_f(words: &mut Vec<u32>, value: f32) {
    words.push(value.to_bits());
}

fn word_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Point lamps in scene order, then the light tree. `scene_index` is the
/// index `scene_lamp` uses, stored in the last word of each lamp.
#[must_use]
pub fn prefix(
    lamps: &[GpuLamp],
    previous: &RestirCam,
    width: u32,
    height: u32,
    frame: u32,
    reset: bool,
    prev_slot: u32,
) -> Option<RestirPrefix> {
    let mut refs = Vec::new();
    let mut scene_index = Vec::new();
    for (index, lamp) in lamps.iter().enumerate() {
        if lamp.directional {
            continue;
        }
        let Some(id) = u32::try_from(index).ok() else {
            break;
        };
        refs.push(Lamp {
            position: Vec3::new(lamp.pos[0], lamp.pos[1], lamp.pos[2]),
            intensity: Vec3::new(lamp.color[0], lamp.color[1], lamp.color[2]),
        });
        scene_index.push(id);
    }
    let tree = LightTree::build(&refs);
    let nodes = tree.pack_words();
    let mut words = vec![0u32; HEADER_WORDS];
    let lamp_base = words.len();
    for (lamp, id) in refs.iter().zip(&scene_index) {
        let source = lamps.get(usize::try_from(*id).unwrap_or(0));
        let range = source.map_or(0.0, |item| item.range);
        push_f(&mut words, lamp.position.x);
        push_f(&mut words, lamp.position.y);
        push_f(&mut words, lamp.position.z);
        push_f(&mut words, range);
        push_f(&mut words, lamp.intensity.x);
        push_f(&mut words, lamp.intensity.y);
        push_f(&mut words, lamp.intensity.z);
        words.push(*id);
    }
    let tree_base = words.len();
    if tree_base.saturating_sub(lamp_base) != refs.len().saturating_mul(LAMP_STRIDE) {
        return None;
    }
    words.extend(nodes);
    if words.len() > usize::try_from(PREFIX_CAP_WORDS).unwrap_or(usize::MAX) {
        return None;
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    let hist0 = u64::try_from(words.len()).unwrap_or(u64::MAX);
    let hist_words = pixels.saturating_mul(u64::from(HIST_STRIDE));
    let hist1 = hist0.saturating_add(hist_words);
    let scratch = hist1.saturating_add(hist_words);
    let total = scratch.saturating_add(pixels.saturating_mul(4));
    let hist0_u = u32::try_from(hist0).ok()?;
    let hist1_u = u32::try_from(hist1).ok()?;
    let scratch_u = u32::try_from(scratch).ok()?;
    let count = word_u32(refs.len());
    words[0] = count;
    words[1] = word_u32(lamp_base);
    words[2] = word_u32(tree.node_count());
    words[3] = word_u32(tree_base);
    words[4] = hist0_u;
    words[5] = hist1_u;
    words[6] = scratch_u;
    words[7] = width;
    words[8] = height;
    words[9] = CANDIDATES;
    words[10] = PROPOSAL_TREE;
    words[11] = DEFAULT_M_CLAMP;
    words[12] = 0;
    words[13] = SPATIAL_NEIGHBORS;
    words[14] = SPATIAL_RADIUS;
    words[15] = u32::from(reset);
    words[16] = frame;
    words[17] = 0xC5A1_7E11 ^ frame;
    words[18] = 0x0051_5EED;
    words[19] = HIST_STRIDE;
    words[20] = prev_slot & 1;
    words[21] = u32::from(previous.valid);
    push_cam(&mut words, previous);
    Some(RestirPrefix {
        words,
        total_words: total,
    })
}

fn push_cam(words: &mut [u32], cam: &RestirCam) {
    let mut at = 22usize;
    let triples = [cam.eye, cam.right, cam.up, cam.forward];
    // eye, right, tan, up, pad, forward, aspect. See the shader.
    for (index, triple) in triples.iter().enumerate() {
        if index == 2 {
            if let Some(slot) = words.get_mut(at) {
                *slot = cam.tan.to_bits();
            }
            at = at.saturating_add(1);
        }
        for value in triple {
            if let Some(slot) = words.get_mut(at) {
                *slot = value.to_bits();
            }
            at = at.saturating_add(1);
        }
        if index == 2 {
            if let Some(slot) = words.get_mut(at) {
                *slot = 0;
            }
            at = at.saturating_add(1);
        }
    }
    if let Some(slot) = words.get_mut(at) {
        *slot = cam.aspect.to_bits();
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::{prefix, RestirCam, HEADER_WORDS, PREFIX_CAP_WORDS};
    use crate::pack::GpuLamp;

    fn at(words: &[u32], index: u32) -> u32 {
        words.get(usize::try_from(index).unwrap_or(0)).copied().unwrap_or(0)
    }

    #[test]
    fn the_prefix_fits_the_cap_and_names_the_scene_lamp() {
        let lamps = [
            GpuLamp {
                pos: [0.0, 2.0, 0.0],
                color: [1.0, 0.0, 0.0],
                directional: true,
                range: 0.0,
            },
            GpuLamp {
                pos: [1.0, 2.0, -1.0],
                color: [0.2, 0.4, 0.6],
                directional: false,
                range: 8.0,
            },
        ];
        let packed = prefix(&lamps, &RestirCam::default(), 32, 16, 3, true, 0)
            .unwrap_or_else(|| panic!("prefix"));
        assert!(u64::try_from(packed.words.len()).unwrap_or(u64::MAX) <= PREFIX_CAP_WORDS);
        assert_eq!(at(&packed.words, 0), 1, "the sun is not a leaf");
        assert_eq!(at(&packed.words, 10), 2, "proposal is the light tree");
        assert_eq!(at(&packed.words, 11), 20, "history M clamp");
        assert_eq!(at(&packed.words, 15), 1, "reset");
        let lamp_base = usize::try_from(at(&packed.words, 1)).unwrap_or(0);
        assert!(lamp_base >= HEADER_WORDS);
        assert_eq!(
            packed.words.get(lamp_base + 7).copied().unwrap_or(0),
            1,
            "scene index skips the sun"
        );
        assert!(at(&packed.words, 2) >= 1, "a lamp is a leaf");
        assert!(packed.total_words > u64::try_from(packed.words.len()).unwrap_or(0));
    }
}
