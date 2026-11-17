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
/// History words per pixel. Lamp and M share a word (16 bits each; the empty
/// lamp is `0xffff`, expanded to the 32-bit sentinel on load). The stored M
/// cap is at most 32 candidates times 20, so it fits. Weight and target stay
/// f32: the estimator is `w_sum / (M * target)`. The fourth word is fp16 view
/// depth and an snorm8 octahedral normal. Scratch is [`SCRATCH_STRIDE`]
/// unpacked words and is not this stride.
pub const HIST_STRIDE: u32 = 4;
/// Unpacked scratch words (lamp, weight, M, target). The shade reads these.
pub const SCRATCH_STRIDE: u32 = 4;
const _: () = assert!(32u32 * (M_CAP / CANDIDATES) <= 0xffff);
const _: () = assert!(genos_scene::MAX_LAMPS < 0xffff);
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
/// and the changed-lamp bitset.
pub const PREFIX_CAP_WORDS: u64 = 64 + 4096 * 8 + 8192 * 12 + 4096 / 32;

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

/// Device bytes for `pixels`, including the prefix cap. Reservoirs are not
/// in the upload; the cap keeps a lamp-list change from moving them.
#[must_use]
pub fn buffer_bytes(pixels: u64) -> u64 {
    let reservoirs = pixels.saturating_mul(u64::from(HIST_STRIDE) * 2 + u64::from(SCRATCH_STRIDE)) * 4;
    PREFIX_CAP_WORDS
        .saturating_mul(4)
        .saturating_add(reservoirs)
}

fn push_f(words: &mut Vec<u32>, value: f32) {
    words.push(value.to_bits());
}

fn word_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Records where the changed-lamp bitset starts and whether any bit is set,
/// so the temporal clamp can apply to every history sample alike.
fn mark_changed(words: &mut [u32], changed_base: usize) {
    words[CHANGED_AT_WORD] = word_u32(changed_base);
    let any = words
        .get(changed_base..)
        .is_some_and(|bits| bits.iter().any(|w| *w != 0));
    words[ANY_CHANGED_WORD] = u32::from(any);
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
    let (changed_base, layout_changed) = push_changes(&mut words, lamps, prev_lamps, refs.len());
    if words.len() > usize::try_from(PREFIX_CAP_WORDS).unwrap_or(usize::MAX) {
        return None;
    }
    let width = cost.width;
    let height = cost.height;
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    let hist0 = u64::try_from(words.len()).unwrap_or(u64::MAX);
    let hist_words = pixels.saturating_mul(u64::from(HIST_STRIDE));
    let hist1 = hist0.saturating_add(hist_words);
    let scratch = hist1.saturating_add(hist_words);
    let total = scratch.saturating_add(pixels.saturating_mul(u64::from(SCRATCH_STRIDE)));
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
    let candidates = cost.candidates.clamp(1, 32);
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
    words[15] = u32::from(layout_changed);
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
    push_cam(&mut words, previous);
    mark_changed(&mut words, changed_base);
    Some(RestirPrefix {
        words,
        total_words: total,
        layout_changed,
    })
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
        assert_eq!(at(&packed.words, 19), super::HIST_STRIDE, "history stride");
        let hist0 = u64::from(at(&packed.words, 4));
        let hist1 = u64::from(at(&packed.words, 5));
        let scratch = u64::from(at(&packed.words, 6));
        let cells = 32u64 * 16;
        assert_eq!(hist1 - hist0, cells * u64::from(super::HIST_STRIDE));
        assert_eq!(scratch - hist1, cells * u64::from(super::HIST_STRIDE));
        assert_eq!(
            packed.total_words - scratch,
            cells * u64::from(super::SCRATCH_STRIDE)
        );
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
        let live = include_str!("../shaders/restir_live.comp");
        let direct = include_str!("../shaders/lighting_direct.comp");
        assert!(live.contains(&format!(
            "const uint RESTIR_DIAG_HIST_ACCEPT = {}u;",
            super::HIST_ACCEPT_WORD
        )));
        assert!(live.contains(&format!(
            "const uint RESTIR_DIAG_HIST_REJECT = {}u;",
            super::HIST_REJECT_WORD
        )));
        assert!(direct.contains(&format!(
            "const uint RESTIR_DIAG_POINT_ADD = {}u;",
            super::POINT_ADD_WORD
        )));
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
        let still = prefix(&before, &before, &RestirCam::default(), 1, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        assert_eq!(
            at(&still.words, 39),
            0,
            "a still lamp leaves word 39 clear"
        );
        let fewer = [lamp(0.0), lamp(2.0)];
        let cost = super::RestirCost {
            width: 8,
            height: 8,
            ..super::RestirCost::default()
        };
        let packed = prefix(&fewer, &before, &RestirCam::default(), 1, 0, cost)
            .unwrap_or_else(|| panic!("prefix"));
        assert!(packed.layout_changed, "indices no longer match");
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

    /// `uniform_index`: n ≤ 1 returns 0 and does not draw.
    fn uniform_index(n: u32, rng: &mut dyn FnMut() -> u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        let limit = u32::MAX - (u32::MAX % n);
        loop {
            let draw = rng();
            if draw < limit {
                return draw % n;
            }
        }
    }

    fn neighbor_pool(x: u32, y: u32, width: u32, height: u32, reach: u32) -> Vec<u32> {
        let x0 = x.saturating_sub(reach);
        let y0 = y.saturating_sub(reach);
        let x1 = (x + reach).min(width - 1);
        let y1 = (y + reach).min(height - 1);
        let mut pool = Vec::new();
        for row in y0..=y1 {
            for col in x0..=x1 {
                if col == x && row == y {
                    continue;
                }
                pool.push(row * width + col);
            }
        }
        pool
    }

    fn shuffle_pool(mut pool: Vec<u32>, take: usize, rng: &mut dyn FnMut() -> u32) -> Vec<u32> {
        let take = take.min(pool.len()).min(16);
        for slot in 0..take {
            let span = u32::try_from(pool.len() - slot).unwrap_or(0);
            let pick = slot + usize::try_from(uniform_index(span, rng)).unwrap_or(0);
            pool.swap(slot, pick);
        }
        pool.truncate(take);
        pool
    }

    fn window_cell(i: u32, x0: u32, y0: u32, span_x: u32, width: u32, centre: u32) -> u32 {
        let cell = if i >= centre { i + 1 } else { i };
        (y0 + cell / span_x) * width + (x0 + cell % span_x)
    }

    /// The live swap table. `nswap` stays within the 32 entries the shader stores.
    fn shuffle_table(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        reach: u32,
        take: u32,
        rng: &mut dyn FnMut() -> u32,
    ) -> (Vec<u32>, u32) {
        let x0 = x.saturating_sub(reach);
        let y0 = y.saturating_sub(reach);
        let x1 = (x + reach).min(width - 1);
        let y1 = (y + reach).min(height - 1);
        let span_x = x1 - x0 + 1;
        let span_y = y1 - y0 + 1;
        let cells = span_x * span_y;
        if cells <= 1 {
            return (Vec::new(), 0);
        }
        let pool_n = cells - 1;
        let centre = (y - y0) * span_x + (x - x0);
        let take = take.min(pool_n).min(16);
        let mut sw_i = [0u32; 32];
        let mut sw_v = [0u32; 32];
        let mut nswap = 0u32;
        let mut picked = Vec::new();
        for slot in 0..take {
            let span = pool_n - slot;
            let pick = slot + uniform_index(span, rng);
            let mut a = 0u32;
            let mut b = 0u32;
            let mut a_hit = false;
            let mut b_hit = false;
            for s in 0..nswap {
                let si = usize::try_from(s).unwrap_or(0);
                if sw_i[si] == slot {
                    a = sw_v[si];
                    a_hit = true;
                }
                if sw_i[si] == pick {
                    b = sw_v[si];
                    b_hit = true;
                }
            }
            if !a_hit {
                a = window_cell(slot, x0, y0, span_x, width, centre);
            }
            if !b_hit {
                b = window_cell(pick, x0, y0, span_x, width, centre);
            }
            let mut wrote_slot = false;
            let mut wrote_pick = false;
            for s in 0..nswap {
                let si = usize::try_from(s).unwrap_or(0);
                if sw_i[si] == slot {
                    sw_v[si] = b;
                    wrote_slot = true;
                }
                if sw_i[si] == pick {
                    sw_v[si] = a;
                    wrote_pick = true;
                }
            }
            if !wrote_slot {
                let at = usize::try_from(nswap).unwrap_or(0);
                assert!(at < 32, "swap table overflow");
                sw_i[at] = slot;
                sw_v[at] = b;
                nswap += 1;
            }
            if !wrote_pick && pick != slot {
                let at = usize::try_from(nswap).unwrap_or(0);
                assert!(at < 32, "swap table overflow");
                sw_i[at] = pick;
                sw_v[at] = a;
                nswap += 1;
            }
            picked.push(b);
        }
        (picked, nswap)
    }

    fn lcg(seed: u32) -> impl FnMut() -> u32 {
        let mut state = seed;
        move || {
            state = state
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            state
        }
    }

    /// Lengths of arrays stored in a function, not buffer structs.
    fn spirv_private_array_lengths(bytes: &[u8]) -> Vec<u32> {
        let words = bytes.len() / 4;
        let word = |index: usize| {
            let start = index.saturating_mul(4);
            let Some(raw) = bytes.get(start..start + 4) else {
                return 0;
            };
            u32::from_le_bytes(raw.try_into().unwrap_or([0; 4]))
        };
        let mut constants = std::collections::HashMap::<u32, u32>::new();
        let mut arrays = std::collections::HashMap::<u32, u32>::new();
        let mut pointers = std::collections::HashMap::<u32, (u32, u32)>::new();
        let mut variables = Vec::new();
        let mut index = 5usize;
        while index < words {
            let header = word(index);
            let count = usize::try_from(header >> 16).unwrap_or(0);
            let op = header & 0xffff;
            if count == 0 || index.saturating_add(count) > words {
                break;
            }
            if op == 43 && count >= 4 {
                constants.insert(word(index + 2), word(index + 3));
            } else if op == 28 && count >= 4 {
                let length = constants.get(&word(index + 3)).copied().unwrap_or(u32::MAX);
                arrays.insert(word(index + 1), length);
            } else if op == 32 && count >= 4 {
                pointers.insert(word(index + 1), (word(index + 2), word(index + 3)));
            } else if op == 59 && count >= 4 {
                variables.push((word(index + 1), word(index + 3)));
            }
            index = index.saturating_add(count);
        }
        variables
            .iter()
            .filter_map(|(ty, storage)| {
                // Function storage is 7. The variable's type is a pointer.
                if *storage != 7 {
                    return None;
                }
                let (_, pointee) = pointers.get(ty)?;
                arrays.get(pointee).copied()
            })
            .collect()
    }

    /// The scalar spatial module has no 16/17/32 private arrays. The wide
    /// module still has the swap table and the 17-wide merge lists.
    #[test]
    fn the_restir_scalar_spatial_module_has_no_swap_table() {
        let scalar = include_bytes!(concat!(env!("OUT_DIR"), "/restir_spatial.comp.spv"));
        let wide = include_bytes!(concat!(env!("OUT_DIR"), "/restir_spatial_wide.comp.spv"));
        let scalar_lengths = spirv_private_array_lengths(scalar);
        assert!(
            scalar_lengths.iter().all(|len| *len < 16),
            "scalar spatial SPIR-V still has a private array: {scalar_lengths:?}"
        );
        let wide_lengths = spirv_private_array_lengths(wide);
        assert!(
            wide_lengths.contains(&32),
            "wide spatial SPIR-V lost the swap table: {wide_lengths:?}"
        );
        assert!(
            wide_lengths.contains(&17),
            "wide spatial SPIR-V lost the 17-wide lists: {wide_lengths:?}"
        );
    }

    #[test]
    fn the_swap_table_matches_neighbor_count_and_stays_in_32_entries() {
        let live = include_str!("../shaders/restir_live.comp");
        let sample = include_str!("../../restir-gpu/shaders/restir_sample.glsl");
        let direct = include_str!("../shaders/lighting_direct.comp");
        let tree_at = live.find("#define RESTIR_TREE_ONLY").unwrap_or(usize::MAX);
        let include_at = live.find("#include \"restir_sample.glsl\"").unwrap_or(0);
        assert!(tree_at < include_at, "RESTIR_TREE_ONLY must hide initial_at");
        assert!(sample.contains("#ifdef RESTIR_TREE_ONLY"));
        assert!(sample.contains("void initial_tree("));
        assert!(
            live.contains(
                "if (restir_hdr(50u) == 0u) {\n            restir_rng(pixel, 1u);\n            restir_temporal_from(pixel, lamp, w_sum, m, target);"
            ),
            "header 50 clear must pass the initial sample in registers"
        );
        assert!(live.contains("uint sw_i[32]"));
        assert!(live.contains("uint sw_v[32]"));
        assert!(
            direct.contains("hold && pc.lamp.x != 4u && ((pc.lamp.y >> 19u) & 1u) != 0u"),
            "bit 19 must not clear the mode-4 exact hold"
        );
        for reach in [1u32, 2, 4] {
            for take in [1u32, 4, 16] {
                for y in [0u32, 3, 20] {
                    for x in [0u32, 1, 15] {
                        let width = 32u32;
                        let height = 24u32;
                        if x >= width || y >= height {
                            continue;
                        }
                        let pool = neighbor_pool(x, y, width, height, reach);
                        for seed in [1u32, 7, 99, 1_000_003] {
                            let mut rng_a = lcg(seed);
                            let mut rng_b = lcg(seed);
                            let from_pool = shuffle_pool(
                                pool.clone(),
                                usize::try_from(take).unwrap_or(0),
                                &mut rng_a,
                            );
                            let (from_table, nswap) =
                                shuffle_table(x, y, width, height, reach, take, &mut rng_b);
                            assert!(nswap <= 32, "nswap {nswap}");
                            assert_eq!(
                                from_table, from_pool,
                                "reach {reach} take {take} pixel {x},{y} seed {seed}"
                            );
                        }
                    }
                }
            }
        }
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
