//! Level-0 rings for the sparse cascade.
#![allow(dead_code)]
//!
//! The shader in `lighting_cascade.glsl` is the copy the picture runs. These
//! functions are the same rule, so a test can check a ring without a GPU.
//! GI v1 stays removed.

/// World size over distance. A ring start keeps this projected size.
pub const PROJ: f32 = 0.05;
/// First ring starts at one metre of Chebyshev distance.
pub const RING_NEAR: f32 = 1.0;
/// Last ring index. Farther samples stay on this ring.
pub const RING_MAX: u32 = 6;
/// Outer fraction of a ring that blends into the next ring.
pub const OVERLAP: f32 = 0.1;

/// Chebyshev distance from the eye. The rings are cubes, not spheres.
pub fn chebyshev(eye: [f32; 3], pos: [f32; 3]) -> f32 {
    let x = (pos[0] - eye[0]).abs();
    let y = (pos[1] - eye[1]).abs();
    let z = (pos[2] - eye[2]).abs();
    x.max(y).max(z)
}

fn log_ring(cheb: f32) -> f32 {
    (cheb.max(RING_NEAR) / RING_NEAR).log2()
}

fn ring_max_f() -> f32 {
    let mut edge = 0.0;
    let mut left = RING_MAX;
    while left > 0 {
        edge += 1.0;
        left -= 1;
    }
    edge
}

fn scale_of(index: u32) -> f32 {
    let mut scale = 1.0;
    let mut left = index.min(RING_MAX);
    while left > 0 {
        scale *= 2.0;
        left -= 1;
    }
    scale
}

/// Ring index. Distance doubles from one ring to the next.
pub fn ring(cheb: f32) -> u32 {
    let level = log_ring(cheb).floor();
    if level <= 0.0 {
        return 0;
    }
    let mut index = 0;
    let mut edge = 0.0;
    while index < RING_MAX {
        edge += 1.0;
        if level < edge {
            break;
        }
        index += 1;
    }
    index
}

/// World size of one level-0 cell on `ring`. Doubles with the ring, so the
/// projected size at the ring start stays [`PROJ`].
pub fn cell_size(ring: u32) -> f32 {
    PROJ * RING_NEAR * scale_of(ring)
}

/// Finest bounce cell. Matches `LIGHTING_SC_HIT`. A cell this size keeps the
/// full trace.
pub const FINE: f32 = 0.25;
/// Trace start for the finest cell. Contact, not a 1 m shell.
pub const TRACE_NEAR: f32 = 1.0e-4;
/// Far end of the last shell, and of the finest cell.
pub const TRACE_FAR: f32 = 200.0;
/// Last upper-level index. Its shell runs out to [`TRACE_FAR`].
pub const INTERVAL_MAX: f32 = 6.0;

/// Ray interval for a world cell of `size` metres.
///
/// The 25 cm cell returns the full trace. A coarser cell returns
/// `[2^level, 2^(level+1)]`, and level 6 returns `[64, 200]`.
pub fn interval(size: f32) -> (f32, f32) {
    if size.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || size < FINE * 2.0 {
        return (TRACE_NEAR, TRACE_FAR);
    }
    let ratio = (size / FINE).max(1.0);
    let level = (ratio.log2() + 0.5).floor();
    if level < 1.0 {
        return (TRACE_NEAR, TRACE_FAR);
    }
    let level = level.min(INTERVAL_MAX);
    let near = 2.0_f32.powf(level);
    let far = if level >= INTERVAL_MAX {
        TRACE_FAR
    } else {
        2.0_f32.powf(level + 1.0)
    };
    (near, far)
}

/// True when a parent sample survives the thin-wall gates.
pub fn keeps_sample(facing: f32, plane: f32, half_cell: f32, stale: bool, weight: f32) -> bool {
    !stale && facing > 0.0 && plane.abs() <= half_cell && weight > 0.5
}

/// First sample whose weight is above a half. Order is the lattice, the
/// fine cell, then each coarser size. An empty entry is not black when a
/// later entry has data.
pub fn shown_sample(samples: &[[f32; 4]]) -> [f32; 4] {
    for sample in samples {
        if sample[3] > 0.5 {
            return [
                sample[0].max(0.0),
                sample[1].max(0.0),
                sample[2].max(0.0),
                1.0,
            ];
        }
    }
    [0.0, 0.0, 0.0, 0.0]
}

/// `L(a,c) = L(a,b) + beta * L(b,c)`. Beta is 0 when the nearer sample exists.
pub fn merge(near: [f32; 3], near_w: f32, far: [f32; 3], far_w: f32) -> ([f32; 3], f32) {
    let near_ok = near_w > 0.5;
    let far_ok = far_w > 0.5;
    let beta = if near_ok { 0.0 } else { 1.0 };
    let rgb = [
        if near_ok { near[0].max(0.0) } else { 0.0 }
            + if far_ok { beta * far[0].max(0.0) } else { 0.0 },
        if near_ok { near[1].max(0.0) } else { 0.0 }
            + if far_ok { beta * far[1].max(0.0) } else { 0.0 },
        if near_ok { near[2].max(0.0) } else { 0.0 }
            + if far_ok { beta * far[2].max(0.0) } else { 0.0 },
    ];
    let w = if near_ok || far_ok { 1.0 } else { 0.0 };
    (rgb, w)
}

/// Weight of the next ring. Zero until the outer tenth of this ring.
pub fn overlap_blend(cheb: f32) -> f32 {
    let level = log_ring(cheb);
    if level >= ring_max_f() {
        return 0.0;
    }
    let frac = level - level.floor();
    let start = 1.0 - OVERLAP;
    if frac <= start {
        return 0.0;
    }
    (frac - start) / OVERLAP
}

#[cfg(test)]
mod tests {
    use super::{
        cell_size, chebyshev, interval, keeps_sample, merge, overlap_blend, ring, shown_sample,
        FINE, OVERLAP, PROJ, RING_MAX, RING_NEAR, TRACE_FAR, TRACE_NEAR,
    };

    #[test]
    fn chebyshev_is_the_longest_axis() {
        let eye = [1.0, 2.0, 3.0];
        assert_eq!(chebyshev(eye, [1.0, 2.0, 3.0]), 0.0);
        assert_eq!(chebyshev(eye, [4.0, 2.5, 3.2]), 3.0);
    }

    #[test]
    fn a_ring_start_keeps_the_projected_size() {
        let mut distance = RING_NEAR;
        for index in 0..=RING_MAX {
            assert_eq!(ring(distance), index);
            let ratio = cell_size(index) / distance;
            assert!(
                (ratio - PROJ).abs() < 1.0e-5,
                "ring {index} projects {ratio}, wanted {PROJ}"
            );
            assert_eq!(overlap_blend(distance), 0.0);
            distance *= 2.0;
        }
    }

    #[test]
    fn the_outer_tenth_blends_into_the_next_ring() {
        let start = 1.0 - OVERLAP;
        let mid = start + OVERLAP * 0.5;
        let distance = RING_NEAR * 2.0_f32.powf(2.0 + mid);
        assert_eq!(ring(distance), 2);
        let blend = overlap_blend(distance);
        assert!((blend - 0.5).abs() < 1.0e-4, "overlap blend {blend}");
        assert_eq!(
            overlap_blend(RING_NEAR * 2.0_f32.powf(2.0 + start * 0.5)),
            0.0
        );
        assert_eq!(overlap_blend(RING_NEAR * 2.0_f32.powf(6.0 + 0.99)), 0.0);
        assert_eq!(RING_MAX, 6);
    }

    #[test]
    fn the_finest_cell_keeps_the_full_trace_and_a_coarser_cell_owns_one_shell() {
        assert_eq!(interval(FINE), (TRACE_NEAR, TRACE_FAR));
        assert_eq!(interval(0.0), (TRACE_NEAR, TRACE_FAR));
        assert_eq!(interval(0.5), (2.0, 4.0));
        assert_eq!(interval(1.0), (4.0, 8.0));
        assert_eq!(interval(2.0), (8.0, 16.0));
        assert_eq!(interval(4.0), (16.0, 32.0));
        assert_eq!(interval(8.0), (32.0, 64.0));
        assert_eq!(interval(16.0), (64.0, TRACE_FAR));
    }

    #[test]
    fn an_empty_voxel_shows_the_first_coarser_sample_with_data() {
        let black = shown_sample(&[[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]]);
        assert_eq!(black, [0.0, 0.0, 0.0, 0.0]);
        let coarse = shown_sample(&[
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.4, 0.1, 0.0, 1.0],
        ]);
        assert_eq!(coarse, [0.4, 0.1, 0.0, 1.0]);
        let lattice = shown_sample(&[[0.2, 0.0, 0.0, 1.0], [9.0, 0.0, 0.0, 1.0]]);
        assert_eq!(lattice, [0.2, 0.0, 0.0, 1.0]);
        let refused = shown_sample(&[[0.0, 0.0, 0.0, 0.5], [0.3, 0.0, 0.0, 0.9]]);
        assert_eq!(refused, [0.3, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_near_sample_hides_the_parent_and_a_missing_one_shows_it() {
        let (rgb, w) = merge([0.2, 0.0, 0.0], 1.0, [9.0, 0.0, 0.0], 1.0);
        assert_eq!(rgb, [0.2, 0.0, 0.0]);
        assert_eq!(w, 1.0);
        let (rgb, w) = merge([0.0, 0.0, 0.0], 0.0, [0.4, 0.1, 0.0], 1.0);
        assert_eq!(rgb, [0.4, 0.1, 0.0]);
        assert_eq!(w, 1.0);
        let (rgb, w) = merge([0.0, 0.0, 0.0], 0.0, [1.0, 0.0, 0.0], 0.0);
        assert_eq!(rgb, [0.0, 0.0, 0.0]);
        assert_eq!(w, 0.0);
    }

    #[test]
    fn a_thin_wall_refuses_the_far_side() {
        let half = 0.125;
        assert!(keeps_sample(1.0, 0.0, half, false, 1.0));
        assert!(!keeps_sample(0.0, 0.0, half, false, 1.0));
        assert!(!keeps_sample(-0.2, 0.0, half, false, 1.0));
        assert!(!keeps_sample(1.0, 0.2, half, false, 1.0));
        assert!(!keeps_sample(1.0, 0.0, half, true, 1.0));
        assert!(!keeps_sample(1.0, 0.0, half, false, 0.0));
    }

    #[test]
    fn the_shader_uses_this_ring_rule() {
        let shader = include_str!("../shaders/lighting_cascade.glsl");
        assert!(
            shader.contains("const float CASCADE_PROJ = 0.05;"),
            "{shader}"
        );
        assert!(shader.contains("const float CASCADE_RING_NEAR = 1.0;"));
        assert!(shader.contains("const uint CASCADE_RING_MAX = 6u;"));
        assert!(shader.contains("const float CASCADE_OVERLAP = 0.1;"));
        assert!(shader.contains("hash_cell3"));
        let gpu = include_str!("lighting_gpu.rs");
        assert!(gpu.contains("CASCADE_LEVEL0_CAP: u64 = 16384"));
        assert!(gpu.contains("CASCADE_LEVEL0_STRIDE: u64 = 12"));
        let direct = include_str!("../shaders/lighting_direct.comp");
        assert!(
            direct.contains("lighting_cascade_insert(pos, n)"),
            "the g-buffer pass inserts the surface sample"
        );
        assert!(
            direct.contains("&& idx < pixels;"),
            "a cache ray is not a product pixel, so distant lamps stay in the cell"
        );
        let trace = include_str!("../shaders/lighting_trace.comp");
        assert!(
            trace.contains("CASCADE_TRACE_NEAR"),
            "picture cells keep the full trace"
        );
        assert!(shader.contains("lighting_cascade_interval"));
        assert!(shader.contains("level < 1.0"));
        assert!(shader.contains("lighting_cascade_merge"));
        assert!(
            shader.contains("lighting_sh_lattice_at(pos, n)"),
            "publish reads the lattice centre"
        );
        assert!(
            !shader.contains("lighting_sh_compose_shown"),
            "the six-ring publish walk is deleted"
        );
        assert!(shader.contains("lighting_cascade_voxel"));
        assert!(shader.contains("lighting_cascade_ancestor"));
        assert!(shader.contains("lighting_cascade_children"));
        assert!(shader.contains("lighting_cascade_inward"));
        assert!(shader.contains("lighting_cascade_facing"));
        assert!(shader.contains("LIGHTING_DIAG_CASCADE_GAP = 11u"));
        assert!(shader.contains("LIGHTING_CELL_HALF * size"));
        assert!(shader.contains("beta * far_rgb"));
        let probe = include_str!("../shaders/lighting_screen_probe.glsl");
        assert!(
            probe.contains("lighting_cascade_published(pixel)"),
            "picture compose loads the published level-0 word"
        );
        assert!(
            probe.contains("lighting_sh_compose_lattice(pos, n, false)"),
            "maintenance still requests the lattice the publish pass reads"
        );
        let build = include_str!("../build.rs");
        assert!(
            build.contains("GENOS_LEVEL0_PICTURE"),
            "picture compose is the level-0 load"
        );
        assert!(gpu.contains("cascade_pub_pipe"));
        assert!(
            !probe.contains("lighting_cascade_insert"),
            "picture compose does not insert into the level-0 hash"
        );
    }
}
