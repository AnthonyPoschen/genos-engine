//! Circular marks for radial progress. Turns run clockwise from the top.

#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

use crate::Segment;

/// Polyline approximation of an arc. A quarter turn is at least eight segments.
pub fn arc_segments(
    center: [f32; 2],
    radius: f32,
    from_turns: f32,
    to_turns: f32,
    width: f32,
    color: [f32; 3],
) -> Vec<Segment> {
    let sweep = to_turns - from_turns;
    if radius <= 0.0 || width <= 0.0 || sweep.abs() < 1.0e-4 {
        return Vec::new();
    }
    let quarters = sweep.abs() * 4.0;
    let steps = (quarters.ceil().max(1.0) * 8.0).min(4096.0);
    if !steps.is_finite() {
        return Vec::new();
    }
    let count = whole_units(steps, 4096);
    let mut out = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    let mut previous = point_at(center, radius, from_turns);
    for step in 1..=count {
        let t = step as f32 / count as f32;
        let next = point_at(center, radius, from_turns + sweep * t);
        out.push(Segment {
            a: previous,
            b: next,
            width,
            color,
        });
        previous = next;
    }
    out
}

fn whole_units(value: f32, limit: u32) -> u32 {
    let mut count = 0u32;
    let mut cursor = 0.0_f32;
    while count < limit && cursor + 1.0 <= value {
        cursor += 1.0;
        count = count.saturating_add(1);
    }
    count.max(1)
}

fn point_at(center: [f32; 2], radius: f32, turns: f32) -> [f32; 2] {
    let angle = turns * std::f32::consts::TAU;
    [
        center[0] + angle.sin() * radius,
        center[1] - angle.cos() * radius,
    ]
}
