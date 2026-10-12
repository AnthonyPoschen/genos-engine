//! Which parts of a frame can be reused. The UI reports dirty rectangles; the
//! draw also diffs the overlay lists when no report arrived. See
//! `docs/frame-reuse.md`.

use crate::gpu::{Image, ScreenLine, ScreenRect};

/// One dirty rectangle in window pixels. The origin is the top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Inclusive-exclusive pixel bounds: x0, y0, x1, y1.
pub(crate) type Region = [i32; 4];

const MERGE_GAP: i32 = 16;
const MAX_REGIONS: usize = 8;

/// The cached scene picture and the CPU state that decides whether to reuse it.
pub(crate) struct FrameReuse {
    /// False when `GENOS_FRAME_REUSE=0`. The frozen lighting short list stays on
    /// only while this is false.
    pub enabled: bool,
    /// Lit scene without the UI. Device-local; the frame loop never maps it.
    pub scene: Image,
    /// Last layout of `scene`. `0` is undefined, `6` is transfer source.
    pub scene_layout: u32,
    /// Scene key of `scene`, once a capture has finished recording.
    pub cached_key: Option<u64>,
    /// Bumps on each capture and on each resize, so a slot can tell its colour
    /// image is stale.
    pub generation: u64,
    /// `generation` last copied into each flight's colour image.
    pub slot_gen: [u64; 2],
    /// UI damage since that flight's colour image was last repaired.
    pub slot_damage: [Vec<Region>; 2],
    /// Dirty UI rectangles for the frame being drawn. Empty when the overlay
    /// lists did not change.
    pub frame_damage: Vec<Region>,
    /// Hash of what the raster and the lighting read this frame.
    pub scene_key: u64,
    /// Flight that last wrote `scene`, until its fence is waited.
    pub scene_writer: Option<usize>,
    /// The last capture's fence has been waited, so another flight may read it.
    pub scene_ready: bool,
    pub stats: ReuseStats,
}

impl Default for FrameReuse {
    fn default() -> Self {
        Self {
            enabled: std::env::var("GENOS_FRAME_REUSE").map_or(true, |value| value != "0"),
            scene: Image::empty(),
            scene_layout: 0,
            cached_key: None,
            generation: 0,
            slot_gen: [0, 0],
            slot_damage: [Vec::new(), Vec::new()],
            frame_damage: Vec::new(),
            scene_key: 0,
            scene_writer: None,
            scene_ready: false,
            stats: ReuseStats::default(),
        }
    }
}

/// How many frames took each path. Printed beside `GPU_MS` as `FRAME_REUSE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReuseStats {
    /// The scene was rasterised and lit.
    pub scene_drawn: u64,
    /// The cached scene was kept and only dirty UI rectangles were redrawn.
    pub ui_repaired: u64,
    /// The cached scene was copied whole into this frame's picture.
    pub full_restore: u64,
    /// Nothing was presented. Lighting verification still ran.
    pub skipped: u64,
    /// UI rectangles redrawn over a cached scene.
    pub rects: u64,
}

/// Clip `r` to the window, drop it when empty, and merge it with any rectangle
/// it touches or comes within 16 px of. Past eight rectangles, keep their bounds.
pub(crate) fn add_region(list: &mut Vec<Region>, mut r: Region, extent: (u32, u32)) {
    let (w, h) = (extent.0 as i32, extent.1 as i32);
    r[0] = r[0].clamp(0, w);
    r[1] = r[1].clamp(0, h);
    r[2] = r[2].clamp(0, w);
    r[3] = r[3].clamp(0, h);
    if r[2] <= r[0] || r[3] <= r[1] {
        return;
    }
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < list.len() {
            if near(list[i], r) {
                r = union(list[i], r);
                list.swap_remove(i);
                merged = true;
            } else {
                i += 1;
            }
        }
        if !merged {
            break;
        }
    }
    list.push(r);
    if list.len() > MAX_REGIONS {
        let mut b = list[0];
        for x in &list[1..] {
            b = union(b, *x);
        }
        list.clear();
        list.push(b);
    }
}

/// Bounds of the rectangles, out to whole pixels plus one. `None` when none have area.
pub(crate) fn rects_region(rects: &[ScreenRect]) -> Option<Region> {
    let mut bounds = None;
    for rect in rects {
        if rect.w <= 0.0 || rect.h <= 0.0 || !rect.x.is_finite() || !rect.y.is_finite() {
            continue;
        }
        let r = [
            rect.x.floor() as i32 - 1,
            rect.y.floor() as i32 - 1,
            (rect.x + rect.w).ceil() as i32 + 1,
            (rect.y + rect.h).ceil() as i32 + 1,
        ];
        bounds = Some(match bounds {
            Some(b) => union(b, r),
            None => r,
        });
    }
    bounds
}

/// Bounds of the lines, `width / 2` around each end, out to whole pixels plus one.
pub(crate) fn lines_region(lines: &[ScreenLine]) -> Option<Region> {
    let mut bounds = None;
    for line in lines {
        let dx = line.b[0] - line.a[0];
        let dy = line.b[1] - line.a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if !(len > 1.0e-6) || !(line.width > 0.0) {
            continue;
        }
        let r = line.width * 0.5;
        let x0 = line.a[0].min(line.b[0]) - r;
        let y0 = line.a[1].min(line.b[1]) - r;
        let x1 = line.a[0].max(line.b[0]) + r;
        let y1 = line.a[1].max(line.b[1]) + r;
        let region = [
            x0.floor() as i32 - 1,
            y0.floor() as i32 - 1,
            x1.ceil() as i32 + 1,
            y1.ceil() as i32 + 1,
        ];
        bounds = Some(match bounds {
            Some(b) => union(b, region),
            None => region,
        });
    }
    bounds
}

pub(crate) fn union(a: Region, b: Region) -> Region {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn near(a: Region, b: Region) -> bool {
    a[0] <= b[2] + MERGE_GAP
        && b[0] <= a[2] + MERGE_GAP
        && a[1] <= b[3] + MERGE_GAP
        && b[1] <= a[3] + MERGE_GAP
}

/// Fast hash of `bytes`, starting from `seed`. Used for the scene key.
pub(crate) fn fast_hash(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    let mut chunks = bytes.chunks_exact(8);
    for chunk in chunks.by_ref() {
        let w = u64::from_le_bytes(chunk.try_into().unwrap());
        h = (h ^ w).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31);
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut tail = 0u64;
        for (i, b) in rest.iter().enumerate() {
            tail |= u64::from(*b) << (8 * i);
        }
        h = (h ^ tail).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

/// Mix one word into a [`fast_hash`] state.
pub(crate) fn mix_word(h: u64, w: u64) -> u64 {
    fast_hash(h, &w.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> ScreenRect {
        ScreenRect {
            x,
            y,
            w,
            h,
            color: [0.0; 3],
        }
    }

    #[test]
    fn close_rectangles_merge_and_distant_ones_stay() {
        let mut list = Vec::new();
        add_region(&mut list, [0, 0, 10, 10], (200, 200));
        add_region(&mut list, [20, 0, 30, 10], (200, 200));
        assert_eq!(list.len(), 1, "10 px apart, inside the 16 px gap");
        add_region(&mut list, [100, 100, 110, 110], (200, 200));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn a_region_outside_the_window_is_dropped_and_a_partial_one_is_clipped() {
        let mut list = Vec::new();
        add_region(&mut list, [-40, -40, -10, -10], (100, 80));
        assert!(list.is_empty());
        add_region(&mut list, [-5, 10, 15, 90], (100, 80));
        assert_eq!(list, vec![[0, 10, 15, 80]]);
    }

    #[test]
    fn more_than_eight_rectangles_collapse_to_their_bounds() {
        let mut list = Vec::new();
        for i in 0..9 {
            let x = i * 40;
            add_region(&mut list, [x, 0, x + 8, 8], (1000, 100));
        }
        assert_eq!(list.len(), 1);
        assert!(list[0][0] <= 0 && list[0][2] >= 8 * 40 + 8);
    }

    #[test]
    fn line_bounds_cover_the_width_around_the_ends() {
        let region = lines_region(&[ScreenLine {
            a: [100.0, 100.0],
            b: [200.0, 150.0],
            width: 4.0,
            color: [1.0; 3],
        }]);
        let r = region.expect("line");
        assert!(r[0] <= 97 && r[1] <= 97 && r[2] >= 203 && r[3] >= 153, "{r:?}");
    }

    #[test]
    fn rect_bounds_are_the_union_out_to_whole_pixels() {
        let region = rects_region(&[rect(10.2, 4.0, 5.0, 6.5), rect(1.0, 1.0, 0.0, 4.0)]);
        assert_eq!(region, Some([9, 3, 17, 12]));
    }

    #[test]
    fn the_hash_changes_with_the_bytes_and_mixes_a_word() {
        assert_ne!(fast_hash(1, b"scene"), fast_hash(1, b"scenx"));
        assert_eq!(fast_hash(1, b"scene"), fast_hash(1, b"scene"));
        assert_ne!(mix_word(7, 1), mix_word(7, 2));
    }
}
