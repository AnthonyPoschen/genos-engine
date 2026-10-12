//! The screen regions one UI frame changed, against the frame before.
//!
//! Paints are grouped by a key the producer picks (an element id, a graph). A
//! group whose paints changed dirties its old and its new bounds; a group that
//! appears or goes away dirties its bounds. The renderer redraws everything inside
//! the reported rectangles, so an unchanged element over a dirty one is redrawn too.

use std::collections::HashMap;

use crate::layout::Rect;
use crate::panel::Paint;
use crate::Segment;

/// The most rectangles one frame reports. Past it, the pair that grows least merges.
pub const MAX_DAMAGE: usize = 8;
/// Rectangles closer than this, in pixels, merge into one.
const MERGE_GAP: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Group {
    hash: u64,
    /// Whole pixels: left, top, right, bottom. `None` paints nothing.
    bounds: Option<[f32; 4]>,
}

/// Diffs each frame's paint groups against the last frame's.
///
/// Call [`DamageTracker::begin`], then [`DamageTracker::paints`] or
/// [`DamageTracker::segments`] for each group, then [`DamageTracker::finish`].
#[derive(Clone, Debug, Default)]
pub struct DamageTracker {
    last: HashMap<u64, Group>,
    next: HashMap<u64, Group>,
    viewport: [f32; 2],
    full: bool,
}

impl DamageTracker {
    /// Start a frame. A viewport change dirties the whole screen.
    pub fn begin(&mut self, viewport: [f32; 2]) {
        self.next.clear();
        self.full = viewport != self.viewport;
        self.viewport = viewport;
    }

    /// Add rectangles to group `key`. A key given twice in a frame joins both lists.
    pub fn paints(&mut self, key: u64, paints: &[Paint]) {
        let mut hash = FNV_OFFSET;
        let mut bounds = None;
        for paint in paints {
            for v in [paint.x, paint.y, paint.w, paint.h] {
                hash = mix(hash, v.to_bits());
            }
            for v in paint.color {
                hash = mix(hash, v.to_bits());
            }
            if paint.w > 0.0 && paint.h > 0.0 {
                bounds = join(bounds, Some([paint.x, paint.y, paint.x + paint.w, paint.y + paint.h]));
            }
        }
        self.add(key, hash, bounds);
    }

    /// Add lines to group `key`. Each line covers `width / 2` around its ends.
    pub fn segments(&mut self, key: u64, segments: &[Segment]) {
        let mut hash = FNV_OFFSET ^ 0x5e9;
        let mut bounds = None;
        for s in segments {
            for v in [s.a[0], s.a[1], s.b[0], s.b[1], s.width] {
                hash = mix(hash, v.to_bits());
            }
            for v in s.color {
                hash = mix(hash, v.to_bits());
            }
            let r = s.width * 0.5;
            bounds = join(
                bounds,
                Some([
                    s.a[0].min(s.b[0]) - r,
                    s.a[1].min(s.b[1]) - r,
                    s.a[0].max(s.b[0]) + r,
                    s.a[1].max(s.b[1]) + r,
                ]),
            );
        }
        self.add(key, hash, bounds);
    }

    fn add(&mut self, key: u64, hash: u64, bounds: Option<[f32; 4]>) {
        let bounds = bounds.map(whole_pixels);
        let group = self.next.entry(key).or_insert(Group {
            hash: FNV_OFFSET,
            bounds: None,
        });
        group.hash = mix(mix(group.hash, hash as u32), (hash >> 32) as u32);
        group.bounds = join(group.bounds, bounds);
    }

    /// End the frame: the changed regions in window pixels, at most [`MAX_DAMAGE`],
    /// clipped to the viewport. Empty when nothing changed.
    pub fn finish(&mut self) -> Vec<Rect> {
        let mut dirty = Vec::new();
        for (key, group) in &self.next {
            match self.last.get(key) {
                Some(old) if old == group => {}
                Some(old) => {
                    dirty.extend(old.bounds);
                    dirty.extend(group.bounds);
                }
                None => dirty.extend(group.bounds),
            }
        }
        for (key, old) in &self.last {
            if !self.next.contains_key(key) {
                dirty.extend(old.bounds);
            }
        }
        std::mem::swap(&mut self.last, &mut self.next);
        self.next.clear();
        let [w, h] = self.viewport;
        if self.full {
            self.full = false;
            if w > 0.0 && h > 0.0 {
                return vec![Rect { x: 0.0, y: 0.0, w, h }];
            }
            return Vec::new();
        }
        merge(dirty, [w, h])
            .into_iter()
            .map(|[x0, y0, x1, y1]| Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            })
            .collect()
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn mix(hash: u64, word: u32) -> u64 {
    (hash ^ u64::from(word)).wrapping_mul(FNV_PRIME)
}

fn join(a: Option<[f32; 4]>, b: Option<[f32; 4]>) -> Option<[f32; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Out to whole pixels, plus one on each side: a rasterized edge never lands
/// outside the reported region.
fn whole_pixels(b: [f32; 4]) -> [f32; 4] {
    [b[0].floor() - 1.0, b[1].floor() - 1.0, b[2].ceil() + 1.0, b[3].ceil() + 1.0]
}

fn area(b: [f32; 4]) -> f32 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] + MERGE_GAP && b[0] <= a[2] + MERGE_GAP && a[1] <= b[3] + MERGE_GAP && b[1] <= a[3] + MERGE_GAP
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Clip to the viewport, join rectangles that touch or nearly touch, then join the
/// pairs that grow least until at most [`MAX_DAMAGE`] are left.
fn merge(rects: Vec<[f32; 4]>, viewport: [f32; 2]) -> Vec<[f32; 4]> {
    let mut out: Vec<[f32; 4]> = rects
        .into_iter()
        .map(|b| [b[0].max(0.0), b[1].max(0.0), b[2].min(viewport[0]), b[3].min(viewport[1])])
        .filter(|b| b[2] > b[0] && b[3] > b[1])
        .collect();
    'join: loop {
        for i in 0..out.len() {
            for j in i + 1..out.len() {
                if near(out[i], out[j]) {
                    out[i] = union(out[i], out[j]);
                    out.swap_remove(j);
                    continue 'join;
                }
            }
        }
        break;
    }
    while out.len() > MAX_DAMAGE {
        let mut best = (0, 1, f32::INFINITY);
        for i in 0..out.len() {
            for j in i + 1..out.len() {
                let grow = area(union(out[i], out[j])) - area(out[i]) - area(out[j]);
                if grow < best.2 {
                    best = (i, j, grow);
                }
            }
        }
        out[best.0] = union(out[best.0], out[best.1]);
        out.swap_remove(best.1);
    }
    out
}
