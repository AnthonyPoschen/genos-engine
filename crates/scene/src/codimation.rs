//! A solid moving through fixed positions on a loop.
//!
//! The sample is a pure function of the positions, the easing, and the time.
//! The frame step submits that displacement to the physics step.

use genos_math::Vec3;

/// How fast a solid moves along one segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    /// The same speed from the start of the segment to the end.
    Linear,
    /// Slow at the start of the segment, then faster.
    EaseIn,
    /// Fast at the start of the segment, then slower.
    EaseOut,
    /// Slow at both ends of the segment, faster through the middle.
    EaseInOut,
}

/// A loop through fixed positions. Each segment takes the same number of seconds.
/// After the last position, the next segment returns to the first.
#[derive(Clone, Debug)]
pub struct Codimation {
    points: Vec<Vec3>,
    easing: Easing,
    segment_seconds: f32,
    elapsed: f32,
}

impl Codimation {
    /// `points` needs two or more finite positions. `segment_seconds` is the time for one segment.
    pub fn new(points: Vec<Vec3>, easing: Easing, segment_seconds: f32) -> Option<Self> {
        if points.len() < 2 || !(segment_seconds > 0.0) || !segment_seconds.is_finite() {
            return None;
        }
        if points.iter().any(|point| !finite_point(*point)) {
            return None;
        }
        Some(Self {
            points,
            easing,
            segment_seconds,
            elapsed: 0.0,
        })
    }

    pub(crate) fn sample(&self) -> Vec3 {
        sample(
            &self.points,
            self.easing,
            self.elapsed,
            self.segment_seconds,
        )
    }

    pub(crate) fn advance(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.elapsed += dt;
        }
    }

    /// Place the loop at `elapsed` seconds. The path still wraps.
    pub fn seek(&mut self, elapsed: f32) {
        if elapsed.is_finite() && elapsed >= 0.0 {
            self.elapsed = elapsed;
        }
    }
}

fn finite_point(point: Vec3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn sample(points: &[Vec3], easing: Easing, elapsed: f32, segment_seconds: f32) -> Vec3 {
    let count = points.len();
    if count == 0 {
        return Vec3::ZERO;
    }
    if count == 1 || !(segment_seconds > 0.0) {
        return points[0];
    }
    let cycle = segment_seconds * count as f32;
    let time = elapsed.rem_euclid(cycle);
    if !time.is_finite() {
        return points[0];
    }
    let scaled = time / segment_seconds;
    let mut index = scaled.floor() as usize;
    let mut fraction = scaled - index as f32;
    if index >= count {
        index = 0;
        fraction = 0.0;
    }
    let from = points[index];
    let to = points[(index + 1) % count];
    from + (to - from) * ease(easing, fraction)
}

fn ease(easing: Easing, fraction: f32) -> f32 {
    let fraction = fraction.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => fraction,
        Easing::EaseIn => fraction * fraction,
        Easing::EaseOut => 1.0 - (1.0 - fraction) * (1.0 - fraction),
        Easing::EaseInOut => fraction * fraction * (3.0 - 2.0 * fraction),
    }
}
