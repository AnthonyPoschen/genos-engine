//! Level-0 rings for the sparse cascade.
#![allow(dead_code)]
//!
//! The shader in `lighting_cascade.glsl` is the copy the picture runs. These
//! functions are the same rule, so a test can check a ring without a GPU.
//! GI v1's radiance cascades stay removed.

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
    use super::{cell_size, chebyshev, overlap_blend, ring, OVERLAP, PROJ, RING_MAX, RING_NEAR};

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
        assert!(
            (blend - 0.5).abs() < 1.0e-4,
            "overlap blend {blend}"
        );
        assert_eq!(overlap_blend(RING_NEAR * 2.0_f32.powf(2.0 + start * 0.5)), 0.0);
        assert_eq!(overlap_blend(RING_NEAR * 2.0_f32.powf(6.0 + 0.99)), 0.0);
        assert_eq!(RING_MAX, 6);
    }

    #[test]
    fn the_shader_uses_this_ring_rule() {
        let shader = include_str!("../shaders/lighting_cascade.glsl");
        assert!(shader.contains("const float CASCADE_PROJ = 0.05;"), "{shader}");
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
        let probe = include_str!("../shaders/lighting_screen_probe.glsl");
        assert!(
            !probe.contains("lighting_cascade_insert"),
            "picture compose does not walk the level-0 hash"
        );
    }
}
