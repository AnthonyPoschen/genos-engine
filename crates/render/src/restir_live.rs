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
pub const CANDIDATES: u32 = 4;
pub const PROPOSAL_TREE: u32 = 2;
/// Product spatial taps. The scene setting is the source; this is the prefix default.
pub const SPATIAL_NEIGHBORS: u32 = 4;
/// Chebyshev radius of that window, in reservoir pixels.
pub const SPATIAL_RADIUS: u32 = 2;
/// Screen pixels along one edge of a reservoir cell. 1 is one reservoir per pixel.
pub const RESOLUTION_SCALE: u32 = 1;
/// 1 copies the even neighbour's spatial result onto odd reservoir pixels.
pub const SPATIAL_CHECKERBOARD: u32 = 0;
/// 1 keeps initial and temporal as two dispatches so each stamp is one pass.
pub const SPLIT_PASSES: u32 = 0;
/// Live workgroup width. Matches `local_size_x` in `restir_live.comp`.
pub const GROUP_SIZE: u32 = 64;
/// Vulkan's minimum `maxComputeWorkGroupCount[0]`. One dispatch at 1440p.
pub const GROUP_LIMIT: u32 = 65535;

/// How many dispatches cover `pixels` at [`GROUP_SIZE`] threads, staying
/// inside [`GROUP_LIMIT`] groups. 1440p is one. A 65536-pixel chunk was
/// about fifty launches a pass and did not change when a knob changed.
pub const fn dispatch_count(pixels: u32) -> u32 {
    let span = GROUP_LIMIT.saturating_mul(GROUP_SIZE);
    if pixels == 0 || span == 0 {
        return 0;
    }
    pixels.div_ceil(span)
}
/// Header word: screen pixels per reservoir edge.
pub const SCALE_WORD: usize = 46;
/// Header word: checkerboard spatial copy.
pub const CHECKER_WORD: usize = 47;
/// Header words: screen width and height. The reservoir grid is words 7 and 8.
pub const SCREEN_W_WORD: usize = 48;
pub const SCREEN_H_WORD: usize = 49;
/// Header word: 1 splits initial and temporal into two dispatches.
pub const SPLIT_WORD: usize = 50;
/// Absolute history M cap: 20 frames of `CANDIDATES` initial samples. The
/// per-frame clamp (20x the current M) still applies first.
pub const M_CAP: u32 = 20 * CANDIDATES;
/// Header word: base of the previous record of each point lamp.
/// Words 22–36 are the previous camera, so this sits in the free word before
/// the changed-lamp bitset.
pub const PREV_AT_WORD: usize = 37;
/// Previous xyz, range, rgb, and one pad. Same order as [`LAMP_STRIDE`].
pub const PREV_BALL_STRIDE: usize = 8;
/// Header word: base of the changed-lamp bitset (one bit per point lamp).
pub const CHANGED_AT_WORD: usize = 38;
/// 1 when any point lamp changed since the history frame (bitset non-zero).
pub const ANY_CHANGED_WORD: usize = 39;
/// Live history counters. Three consecutive header words the host copies to
/// the stats readback tail. The cache already uses `gstats.diag` 43, 44, and
/// 45 when `LIGHTING_DIAG` is on; these header words are a different buffer.
/// Compact keeps diag 5, 6, and 13. Diag 8, 38, and 39 stay on that layout.
pub const HIST_ACCEPT_WORD: usize = 43;
/// Surfaced pixels whose temporal reprojection rejected history.
pub const HIST_REJECT_WORD: usize = 44;
/// Near point lamps the direct loop added on a `ReSTIR` pixel.
pub const POINT_ADD_WORD: usize = 45;
const _: () = assert!(HIST_ACCEPT_WORD == ANY_CHANGED_WORD + 4);
const _: () = assert!(HIST_REJECT_WORD == HIST_ACCEPT_WORD + 1);
const _: () = assert!(POINT_ADD_WORD == HIST_ACCEPT_WORD + 2);
const _: () = assert!(POINT_ADD_WORD < HEADER_WORDS);
const _: () = assert!(SPLIT_WORD < HEADER_WORDS);

/// Pass costs the prefix writes into the header. The scene settings are the
/// source. A test that omits them gets these product defaults.
#[derive(Clone, Copy, Debug)]
pub struct RestirCost {
    /// Reservoir grid. The screen size is `screen_width` and `screen_height`.
    pub width: u32,
    pub height: u32,
    pub spatial_neighbors: u32,
    pub spatial_radius: u32,
    pub resolution_scale: u32,
    pub checkerboard: u32,
    pub candidates: u32,
    pub split_passes: u32,
    pub screen_width: u32,
    pub screen_height: u32,
}

impl Default for RestirCost {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            spatial_neighbors: SPATIAL_NEIGHBORS,
            spatial_radius: SPATIAL_RADIUS,
            resolution_scale: RESOLUTION_SCALE,
            checkerboard: SPATIAL_CHECKERBOARD,
            candidates: CANDIDATES,
            split_passes: SPLIT_PASSES,
            screen_width: 0,
            screen_height: 0,
        }
    }
}

/// Reservoir grid for a screen. Scale 1 keeps the screen size. A zero screen
/// stays zero so a failed prepare does not dispatch one pixel.
pub fn reservoir_size(width: u32, height: u32, scale: u32) -> (u32, u32) {
    let scale = scale.clamp(1, 4);
    if width == 0 || height == 0 {
        return (0, 0);
    }
    ((width / scale).max(1), (height / scale).max(1))
}

/// One line of iteration counts. Lavapipe time is not this. Set
/// `GENOS_RESTIR_COST=1`. The previous full-resolution window scan (4
/// neighbours, radius 2) is printed beside the live taps.
pub fn log_cost(reservoir_w: u32, reservoir_h: u32, cost: &RestirCost) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on = *ON.get_or_init(|| {
        #[expect(clippy::disallowed_methods, reason = "diagnostic knob read once")]
        std::env::var("GENOS_RESTIR_COST").is_ok_and(|v| v == "1")
    });
    if !on {
        return;
    }
    let pixels = u64::from(reservoir_w).saturating_mul(u64::from(reservoir_h));
    let neighbors = u64::from(cost.spatial_neighbors.min(16));
    let reach = u64::from(cost.spatial_radius.min(8));
    let window = reach
        .saturating_mul(2)
        .saturating_add(1)
        .saturating_pow(2)
        .saturating_sub(1);
    let spatial = if cost.checkerboard != 0 {
        pixels.div_ceil(2).saturating_mul(neighbors)
    } else {
        pixels.saturating_mul(neighbors)
    };
    let old_window = u64::from(cost.screen_width)
        .saturating_mul(u64::from(cost.screen_height))
        .saturating_mul(24);
    eprintln!(
        "restir cost: reservoir={reservoir_w}x{reservoir_h} screen={}x{} scale={} candidates={} spatial_neighbors={} spatial_radius={} checkerboard={} split_passes={} initial_iters={} temporal_iters={pixels} spatial_phat={spatial} dispatches={} spatial_window={window} previous_full_window_scan={old_window}",
        cost.screen_width,
        cost.screen_height,
        cost.resolution_scale.clamp(1, 4),
        cost.candidates.clamp(1, 32),
        neighbors,
        reach,
        u32::from(cost.checkerboard != 0),
        u32::from(cost.split_passes != 0),
        pixels.saturating_mul(u64::from(cost.candidates.clamp(1, 32))),
        dispatch_count(u32::try_from(pixels).unwrap_or(u32::MAX)),
    );
}
/// Worst prefix: header, 4096 lamps, a full binary tree (two nodes per lamp),
/// one previous record per lamp, and the changed-lamp bitset.
pub const PREFIX_CAP_WORDS: u64 = 64 + 4096 * 8 + 8192 * 12 + 4096 * 8 + 4096 / 32;

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
    /// The point-lamp list no longer maps index for index onto last frame's
    /// (a lamp added, removed, or reordered). History lamp indices would name
    /// the wrong lamp, so the caller resets temporal reuse.
    pub layout_changed: bool,
}

fn point_lamps(lamps: &[GpuLamp]) -> impl Iterator<Item = (usize, &GpuLamp)> {
    lamps
        .iter()
        .enumerate()
        .filter(|(_, lamp)| !lamp.directional)
}

/// Same position, colour, and range: the history p-hat of this lamp is still
/// the p-hat it was stored with.
fn same_lamp(a: &GpuLamp, b: &GpuLamp) -> bool {
    let bits = |v: &[f32; 3]| v.map(f32::to_bits);
    bits(&a.pos) == bits(&b.pos)
        && bits(&a.color) == bits(&b.color)
        && a.range.to_bits() == b.range.to_bits()
}

/// Device bytes for the prefix and the cascade direct table.
///
/// The table starts at [`PREFIX_CAP_WORDS`]: two parities, 16384 slots, four
/// words (key, frame, visibility mean, count). `pixels` is unused. The
/// reservoir histories that used to follow the prefix are gone.
#[must_use]
pub fn buffer_bytes(pixels: u64) -> u64 {
    let _ = pixels;
    const CASCADE_DIRECT_CAP: u64 = 16384;
    let table = 2 * CASCADE_DIRECT_CAP * 4 * 4;
    PREFIX_CAP_WORDS.saturating_mul(4).saturating_add(table)
}

fn push_f(words: &mut Vec<u32>, value: f32) {
    words.push(value.to_bits());
}

fn word_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Records where the changed-lamp bitset starts and whether any bit is set.
/// Word 39 is the any-bit early-out. The shaders reweight history where a
/// set bit's lamp reaches the pixel.
fn mark_changed(words: &mut [u32], changed_base: usize) {
    words[CHANGED_AT_WORD] = word_u32(changed_base);
    let any = words
        .get(changed_base..)
        .is_some_and(|bits| bits.iter().any(|w| *w != 0));
    words[ANY_CHANGED_WORD] = u32::from(any);
}

/// Previous record of each current point lamp, in that same order. A count
/// mismatch stores zeros (an empty ball, not the flame's infinite one). An
/// equal-length reorder stores the old slot order. That frame sets word 15
/// and writes no changed-lamp bits, so the reach test returns on word 39
/// before those records are read. Colour is stored so a recolour has a
/// p-hat to divide by. The reach test uses xyz and range; the weight uses
/// the colour too.
fn push_ball(words: &mut Vec<u32>, pos: [f32; 3], range: f32) {
    push_f(words, pos[0]);
    push_f(words, pos[1]);
    push_f(words, pos[2]);
    push_f(words, range);
}

fn push_prev_record(words: &mut Vec<u32>, pos: [f32; 3], range: f32, color: [f32; 3]) {
    push_ball(words, pos, range);
    push_f(words, color[0]);
    push_f(words, color[1]);
    push_f(words, color[2]);
    words.push(0);
}

fn push_prev_balls(words: &mut Vec<u32>, prev_lamps: &[GpuLamp], count: usize) -> usize {
    let base = words.len();
    let prev: Vec<&GpuLamp> = point_lamps(prev_lamps).map(|(_, lamp)| lamp).collect();
    if prev.len() == count {
        for lamp in &prev {
            push_prev_record(words, lamp.pos, lamp.range, lamp.color);
        }
    } else {
        for _ in 0..count {
            push_prev_record(words, [0.0; 3], 0.0, [0.0; 3]);
        }
    }
    base
}

/// Appends the changed-lamp bitset. Returns the bitset base and whether the
/// layout changed.
fn push_changes(
    words: &mut Vec<u32>,
    lamps: &[GpuLamp],
    prev_lamps: &[GpuLamp],
    count: usize,
) -> (usize, bool) {
    let changed_base = words.len();
    let prev_points: Vec<(usize, &GpuLamp)> = point_lamps(prev_lamps).collect();
    let now_points: Vec<(usize, &GpuLamp)> = point_lamps(lamps).collect();
    let layout_changed = prev_points.len() != now_points.len()
        || prev_points
            .iter()
            .zip(&now_points)
            .any(|(old, new)| old.0 != new.0);
    words.resize(changed_base + count.div_ceil(32), 0);
    if !layout_changed {
        for (slot, (old, new)) in prev_points.iter().zip(&now_points).enumerate() {
            if !same_lamp(old.1, new.1) {
                if let Some(word) = words.get_mut(changed_base + slot / 32) {
                    *word |= 1 << (slot % 32);
                }
            }
        }
    }
    (changed_base, layout_changed)
}

/// Point lamps in scene order, then the light tree. `scene_index` is the
/// index `scene_lamp` uses, stored in the last word of each lamp.
#[must_use]
pub fn prefix(
    lamps: &[GpuLamp],
    prev_lamps: &[GpuLamp],
    previous: &RestirCam,
    frame: u32,
    prev_slot: u32,
    cost: RestirCost,
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
    let ranges: Vec<f32> = scene_index
        .iter()
        .map(|id| {
            lamps
                .get(usize::try_from(*id).unwrap_or(usize::MAX))
                .map_or(0.0, |item| item.range)
        })
        .collect();
    let tree = LightTree::build_ranged(&refs, &ranges);
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
    let prev_base = push_prev_balls(&mut words, prev_lamps, refs.len());
    if words.len().saturating_sub(prev_base) != refs.len().saturating_mul(PREV_BALL_STRIDE) {
        return None;
    }
    let (changed_base, layout_changed) = push_changes(&mut words, lamps, prev_lamps, refs.len());
    if words.len() > usize::try_from(PREFIX_CAP_WORDS).unwrap_or(usize::MAX) {
        return None;
    }
    let total = write_header(
        &mut words,
        &cost,
        previous,
        frame,
        prev_slot,
        &HeaderAt {
            count: refs.len(),
            lamp_base,
            tree_nodes: tree.node_count(),
            tree_base,
            prev_base,
            changed_base,
            layout_changed,
        },
    )?;
    Some(RestirPrefix {
        words,
        total_words: total,
        layout_changed,
    })
}

struct HeaderAt {
    count: usize,
    lamp_base: usize,
    tree_nodes: usize,
    tree_base: usize,
    prev_base: usize,
    changed_base: usize,
    layout_changed: bool,
}

fn write_header(
    words: &mut [u32],
    cost: &RestirCost,
    previous: &RestirCam,
    frame: u32,
    prev_slot: u32,
    at: &HeaderAt,
) -> Option<u64> {
    let pixels = u64::from(cost.width).saturating_mul(u64::from(cost.height));
    let hist0 = u64::try_from(words.len()).unwrap_or(u64::MAX);
    let hist_words = pixels.saturating_mul(u64::from(HIST_STRIDE));
    let hist1 = hist0.saturating_add(hist_words);
    let scratch = hist1.saturating_add(hist_words);
    let total = scratch.saturating_add(pixels.saturating_mul(4));
    let candidates = cost.candidates.clamp(1, 32);
    words[0] = word_u32(at.count);
    words[1] = word_u32(at.lamp_base);
    words[2] = word_u32(at.tree_nodes);
    words[3] = word_u32(at.tree_base);
    words[4] = u32::try_from(hist0).ok()?;
    words[5] = u32::try_from(hist1).ok()?;
    words[6] = u32::try_from(scratch).ok()?;
    words[7] = cost.width;
    words[8] = cost.height;
    words[9] = candidates;
    words[10] = PROPOSAL_TREE;
    words[11] = DEFAULT_M_CLAMP;
    words[12] = candidates.saturating_mul(M_CAP / CANDIDATES);
    words[13] = cost.spatial_neighbors.min(16);
    words[14] = cost.spatial_radius.min(8);
    words[SCALE_WORD] = cost.resolution_scale.clamp(1, 4);
    words[CHECKER_WORD] = u32::from(cost.checkerboard != 0);
    words[SCREEN_W_WORD] = cost.screen_width;
    words[SCREEN_H_WORD] = cost.screen_height;
    words[SPLIT_WORD] = u32::from(cost.split_passes != 0);
    words[15] = u32::from(at.layout_changed);
    if reuse_off() {
        // Diagnostic: no temporal history (word 15 forces the temporal pass
        // to treat history as invalid) and no spatial neighbours (word 13).
        words[13] = 0;
        words[15] = 1;
    }
    words[16] = frame;
    words[17] = 0xC5A1_7E11 ^ frame;
    words[18] = 0x0051_5EED;
    words[19] = HIST_STRIDE;
    words[20] = prev_slot & 1;
    words[21] = u32::from(previous.valid);
    words[PREV_AT_WORD] = word_u32(at.prev_base);
    push_cam(words, previous);
    mark_changed(words, at.changed_base);
    // Reorder frames set word 15 and leave the bitset clear. The previous
    // balls are not read: restir_changed_reach returns when word 39 is 0.
    debug_assert!(
        !at.layout_changed || words[ANY_CHANGED_WORD] == 0,
        "a reorder must not publish changed-lamp bits"
    );
    Some(total)
}

/// Header word 15, the history-invalid flag the shaders read. `reset` is the
/// frame's geometry/camera reset; the prefix already holds the lamp-layout and
/// `GENOS_RESTIR_REUSE=off` reasons.
pub fn set_history_reset(words: &mut [u32], reset: bool) {
    if let Some(word) = words.get_mut(15) {
        *word |= u32::from(reset);
    }
}

/// `GENOS_RESTIR_REUSE=off` turns temporal and spatial reuse off, leaving
/// initial candidates only. Diagnostic for bias hunts; logged once.
fn reuse_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        #[expect(clippy::disallowed_methods, reason = "diagnostic knob read once")]
        let off = std::env::var("GENOS_RESTIR_REUSE").is_ok_and(|v| v == "off");
        if off {
            eprintln!("restir: reuse off (temporal history invalid, spatial neighbours 0)");
        }
        off
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
        words
            .get(usize::try_from(index).unwrap_or(0))
            .copied()
            .unwrap_or(0)
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
        let cost = super::RestirCost {
            width: 32,
            height: 16,
            ..super::RestirCost::default()
        };
        let packed = prefix(&lamps, &lamps, &RestirCam::default(), 3, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        assert!(u64::try_from(packed.words.len()).unwrap_or(u64::MAX) <= PREFIX_CAP_WORDS);
        assert_eq!(at(&packed.words, 0), 1, "the sun is not a leaf");
        assert_eq!(at(&packed.words, 10), 2, "proposal is the light tree");
        assert_eq!(at(&packed.words, 11), 20, "history M clamp");
        assert_eq!(at(&packed.words, 15), 0, "same layout, no reset");
        let lamp_base = usize::try_from(at(&packed.words, 1)).unwrap_or(0);
        assert!(lamp_base >= HEADER_WORDS);
        assert_eq!(
            packed.words.get(lamp_base + 7).copied().unwrap_or(0),
            1,
            "scene index skips the sun"
        );
        assert!(at(&packed.words, 2) >= 1, "a lamp is a leaf");
        assert!(packed.total_words > u64::try_from(packed.words.len()).unwrap_or(0));
        assert_eq!(at(&packed.words, 12), super::M_CAP, "history M cap");
        assert_eq!(at(&packed.words, 13), super::SPATIAL_NEIGHBORS);
        assert_eq!(at(&packed.words, 14), super::SPATIAL_RADIUS);
        assert_eq!(at(&packed.words, 46), super::RESOLUTION_SCALE);
        assert_eq!(at(&packed.words, 50), super::SPLIT_PASSES);
        assert!(!packed.layout_changed);
        let changed = usize::try_from(at(&packed.words, 38)).unwrap_or(0);
        assert_eq!(packed.words.get(changed).copied(), Some(0), "nothing moved");
        assert_eq!(
            packed
                .words
                .get(super::HIST_ACCEPT_WORD)
                .copied()
                .unwrap_or(0),
            0
        );
        assert_eq!(
            packed
                .words
                .get(super::HIST_REJECT_WORD)
                .copied()
                .unwrap_or(0),
            0
        );
        assert_eq!(
            packed
                .words
                .get(super::POINT_ADD_WORD)
                .copied()
                .unwrap_or(0),
            0
        );
        let direct = include_str!("../shaders/lighting_direct.comp");
        assert!(direct.contains(&format!(
            "const uint RESTIR_DIAG_POINT_ADD = {}u;",
            super::POINT_ADD_WORD
        )));
        assert!(direct.contains("const uint CASCADE_DIRECT_AT = 164032u;"));
        assert_eq!(super::PREFIX_CAP_WORDS, 164032);
    }

    #[test]
    fn a_moved_lamp_is_flagged_and_a_removed_lamp_resets() {
        let lamp = |x: f32| GpuLamp {
            pos: [x, 2.0, 0.0],
            color: [1.0, 1.0, 1.0],
            directional: false,
            range: 8.0,
        };
        let before = [lamp(0.0), lamp(1.0), lamp(2.0)];
        let moved = [lamp(0.0), lamp(1.5), lamp(2.0)];
        let cost = super::RestirCost {
            width: 8,
            height: 8,
            ..super::RestirCost::default()
        };
        let packed = prefix(&moved, &before, &RestirCam::default(), 1, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        assert!(!packed.layout_changed, "same lamps, one moved");
        let changed = usize::try_from(at(&packed.words, 38)).unwrap_or(0);
        assert_eq!(
            packed.words.get(changed).copied(),
            Some(0b010),
            "only lamp 1 moved"
        );
        assert_eq!(
            at(&packed.words, 39),
            1,
            "a position-only move sets word 39"
        );
        let (prev_x, prev_range) = prev_ball(&packed.words, 1);
        assert!(
            (prev_x - 1.0).abs() < 1.0e-5 && (prev_range - 8.0).abs() < 1.0e-5,
            "lamp 1's previous ball is {prev_x} range {prev_range}"
        );
        let prev_base =
            usize::try_from(at(&packed.words, header_word(super::PREV_AT_WORD))).unwrap_or(0);
        let prev_red = f32::from_bits(
            packed
                .words
                .get(prev_base + super::PREV_BALL_STRIDE + 4)
                .copied()
                .unwrap_or(0),
        );
        assert!(
            (prev_red - 1.0).abs() < 1.0e-5,
            "lamp 1's previous colour is {prev_red}"
        );
        assert_colour_off_keeps_the_old_ball(&lamp, &moved, cost);
        assert_layout_changes_hide_the_balls(&lamp, &before, cost);
    }

    fn header_word(index: usize) -> u32 {
        u32::try_from(index).unwrap_or(0)
    }

    fn prev_ball(words: &[u32], lamp: usize) -> (f32, f32) {
        let base = usize::try_from(at(words, header_word(super::PREV_AT_WORD))).unwrap_or(0);
        let slot = base + lamp * super::PREV_BALL_STRIDE;
        let x = f32::from_bits(words.get(slot).copied().unwrap_or(0));
        let range = f32::from_bits(words.get(slot + 3).copied().unwrap_or(0));
        (x, range)
    }

    fn assert_colour_off_keeps_the_old_ball(
        lamp: &dyn Fn(f32) -> GpuLamp,
        moved: &[GpuLamp],
        cost: super::RestirCost,
    ) {
        let black = GpuLamp {
            pos: [1.5, 2.0, 0.0],
            color: [0.0, 0.0, 0.0],
            directional: false,
            range: 0.0,
        };
        let off = [lamp(0.0), black, lamp(2.0)];
        let packed = prefix(&off, moved, &RestirCam::default(), 2, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        let (_, prev_range) = prev_ball(&packed.words, 1);
        let now_range = f32::from_bits(
            packed
                .words
                .get(usize::try_from(at(&packed.words, 1)).unwrap_or(0) + super::LAMP_STRIDE + 3)
                .copied()
                .unwrap_or(1),
        );
        assert!(
            (prev_range - 8.0).abs() < 1.0e-5 && now_range == 0.0,
            "colour-to-0 keeps the previous range {prev_range} and a zero current range {now_range}"
        );
        assert_eq!(
            packed
                .words
                .get(usize::try_from(at(&packed.words, 38)).unwrap_or(0))
                .copied(),
            Some(0b010),
            "colour-to-0 flags the lamp so the previous ball is tested"
        );
    }

    fn assert_layout_changes_hide_the_balls(
        lamp: &dyn Fn(f32) -> GpuLamp,
        before: &[GpuLamp],
        cost: super::RestirCost,
    ) {
        let still = prefix(before, before, &RestirCam::default(), 1, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        assert_eq!(
            at(&still.words, 39),
            0,
            "a still lamp leaves word 39 clear"
        );
        let fewer = [lamp(0.0), lamp(2.0)];
        let packed = prefix(&fewer, before, &RestirCam::default(), 1, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        assert!(packed.layout_changed, "indices no longer match");
        let sun = GpuLamp {
            pos: [0.0, 10.0, 0.0],
            color: [1.0, 1.0, 1.0],
            directional: true,
            range: 0.0,
        };
        let shifted = [sun, lamp(0.0), lamp(1.0), lamp(2.0)];
        let packed = prefix(&shifted, before, &RestirCam::default(), 1, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        assert!(
            packed.layout_changed,
            "an equal-length point list with new scene indices is a reorder"
        );
        assert_eq!(at(&packed.words, 15), 1, "a reorder sets word 15");
        assert_eq!(
            at(&packed.words, 39),
            0,
            "a reorder writes no bits, so the previous balls are not read"
        );
    }
    #[test]
    fn history_reset_keeps_prefix_reasons() {
        // The prefix set word 15 (reuse off or a new lamp layout); a frame with
        // no geometry reset must not clear it before the shaders read it.
        let mut words = vec![0u32; 40];
        words[15] = 1;
        super::set_history_reset(&mut words, false);
        assert_eq!(
            words[15], 1,
            "shader-visible word 15 disagrees with the host"
        );
        words[15] = 0;
        super::set_history_reset(&mut words, true);
        assert_eq!(words[15], 1);
        words[15] = 0;
        super::set_history_reset(&mut words, false);
        assert_eq!(words[15], 0);
    }

    #[test]
    fn product_defaults_match_the_scene_settings_and_one_dispatch_covers_1440p() {
        assert_eq!(
            super::SPATIAL_NEIGHBORS,
            genos_scene::RESTIR_SPATIAL_NEIGHBORS
        );
        assert_eq!(super::SPATIAL_RADIUS, genos_scene::RESTIR_SPATIAL_RADIUS);
        assert_eq!(
            super::RESOLUTION_SCALE,
            genos_scene::RESTIR_RESOLUTION_SCALE
        );
        assert_eq!(super::CANDIDATES, genos_scene::RESTIR_CANDIDATES);
        assert_eq!(
            super::SPATIAL_CHECKERBOARD,
            genos_scene::RESTIR_SPATIAL_CHECKERBOARD
        );
        assert_eq!(super::SPLIT_PASSES, genos_scene::RESTIR_SPLIT_PASSES);
        assert_eq!(super::dispatch_count(2560 * 1440), 1);
        assert_eq!(super::dispatch_count(3840 * 2160), 2);
        let lamp = GpuLamp {
            pos: [1.0, 2.0, 0.0],
            color: [1.0, 1.0, 1.0],
            directional: false,
            range: 4.0,
        };
        let cost = super::RestirCost {
            width: 32,
            height: 16,
            screen_width: 32,
            screen_height: 16,
            ..super::RestirCost::default()
        };
        let from_settings = super::RestirCost {
            width: 32,
            height: 16,
            spatial_neighbors: genos_scene::RESTIR_SPATIAL_NEIGHBORS,
            spatial_radius: genos_scene::RESTIR_SPATIAL_RADIUS,
            resolution_scale: genos_scene::RESTIR_RESOLUTION_SCALE,
            checkerboard: genos_scene::RESTIR_SPATIAL_CHECKERBOARD,
            candidates: genos_scene::RESTIR_CANDIDATES,
            split_passes: genos_scene::RESTIR_SPLIT_PASSES,
            screen_width: 32,
            screen_height: 16,
        };
        let Some(a) = super::prefix(&[lamp], &[], &RestirCam::default(), 1, 0, cost) else {
            panic!("prefix");
        };
        let Some(b) = super::prefix(&[lamp], &[], &RestirCam::default(), 1, 0, from_settings)
        else {
            panic!("prefix");
        };
        for word in [9usize, 13, 14, 46, 47, 50] {
            assert_eq!(a.words[word], b.words[word], "header {word}");
        }
        assert_eq!(a.words[46], 1, "scale 1 is full resolution");
        assert_eq!(a.words[7], 32);
        assert_eq!(a.words[13], 4);
    }

    fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
        let start = src.find(name).unwrap_or(src.len());
        let rest = src.get(start..).unwrap_or("");
        let tail_len = rest.len().saturating_sub(name.len());
        let next = rest
            .get(name.len()..)
            .and_then(|tail| tail.find("\n    fn "))
            .unwrap_or(tail_len);
        rest.get(..name.len().saturating_add(next)).unwrap_or(rest)
    }

    /// Removing either cross-frame barrier, or the shade barrier, fails this.
    /// Lavapipe and a fenced readback still pass without them.
    #[test]
    fn the_restir_barriers_order_the_other_frame_and_the_shade() {
        let gpu = include_str!("lighting_gpu.rs");
        let direct = include_str!("../shaders/lighting_direct.comp");
        let prepare = function_body(gpu, "fn prepare_restir_di");
        let update = prepare
            .find("cmd_update_buffer")
            .unwrap_or(prepare.len());
        let prefix = prepare
            .find("memory_barrier(0x800 | 0x1000, 0x1000, 0x20 | 0x40 | 0x800, 0x1000)")
            .unwrap_or(prepare.len());
        assert!(
            prefix < update,
            "the prefix overwrite must wait for the other frame's compute access and transfer read"
        );
        let call = "self.dispatch_restir_di(slot, &pc, pool, first);";
        let at = gpu.find(call).unwrap_or(gpu.len());
        assert!(at < gpu.len(), "dispatch_restir_di call");
        let before = gpu.get(at.saturating_sub(700)..at).unwrap_or("");
        let last = before.rfind("memory_barrier").unwrap_or(before.len());
        assert!(
            before[last..].starts_with("memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40)"),
            "the passes must see the other frame's compute writes"
        );
        let after = gpu.get(at..at.saturating_add(800)).unwrap_or("");
        let shade_at = after.find("memory_barrier").unwrap_or(after.len());
        assert!(
            after[shade_at..].starts_with("memory_barrier(0x800, 0x800, 0x40, 0x20);"),
            "shade must wait for this frame's spatial store"
        );
        let passes = function_body(gpu, "fn lighting_passes");
        let world_at = passes
            .find("submit_lighting_world")
            .unwrap_or(passes.len());
        let bit_at = passes.find("pc[9] |= 1 << 21").unwrap_or(0);
        assert!(
            world_at < bit_at,
            "bit 21 is set only after the world submit"
        );
        assert_restir_words_under_restir_pix(direct);
    }

    /// Every `restir_words` use after the buffer declaration stays inside `restir_pix`.
    fn assert_restir_words_under_restir_pix(src: &str) {
        let body = src
            .split_once("uint restir_words[];")
            .map_or("", |(_, rest)| rest);
        assert!(!body.is_empty(), "restir_words declaration");
        let mut depth = 0i32;
        let mut guards: Vec<Option<i32>> = Vec::new();
        let mut uses = 0u32;
        let mut i = 0usize;
        while i < body.len() {
            let rest = body.get(i..).unwrap_or("");
            if rest.starts_with("if (restir_pix") {
                guards.push(None);
                i = i.saturating_add("if (restir_pix".len());
                continue;
            }
            if rest.starts_with("restir_words") {
                assert!(
                    !guards.is_empty(),
                    "restir_words load is not under restir_pix"
                );
                uses = uses.saturating_add(1);
                i = i.saturating_add("restir_words".len());
                continue;
            }
            match rest.chars().next() {
                Some('{') => {
                    depth += 1;
                    if let Some(guard) = guards.last_mut() {
                        if guard.is_none() {
                            *guard = Some(depth);
                        }
                    }
                }
                Some('}') => {
                    if guards.last().is_some_and(|guard| *guard == Some(depth)) {
                        guards.pop();
                    }
                    depth -= 1;
                }
                _ => {}
            }
            i = i.saturating_add(1);
        }
        assert!(uses > 0, "no restir_words load");
        assert!(guards.is_empty(), "unclosed restir_pix");
    }
}
