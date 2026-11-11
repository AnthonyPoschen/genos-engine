//! Canonical paint text for headless checks.
//!
//! One line per rectangle, `x y w h r g b`, two decimals, sorted by `y`, then
//! `x`, then color. A zero-area rectangle is omitted. Alpha is omitted at 1.
//! An alpha of 0 omits the rectangle. A lower alpha is a seventh number.

use std::fmt::Write;

use crate::Paint;

/// Sort `paints` into the stable text a still panel compares.
pub fn paint_snapshot(paints: &[Paint]) -> String {
    let mut rows: Vec<&Paint> = paints
        .iter()
        .filter(|paint| paint.w > 0.0 && paint.h > 0.0 && paint.alpha > 0.0)
        .collect();
    rows.sort_by(|left, right| {
        left.y
            .total_cmp(&right.y)
            .then(left.x.total_cmp(&right.x))
            .then(left.color[0].total_cmp(&right.color[0]))
            .then(left.color[1].total_cmp(&right.color[1]))
            .then(left.color[2].total_cmp(&right.color[2]))
            .then(left.alpha.total_cmp(&right.alpha))
    });
    let mut out = String::new();
    for paint in rows {
        let _ = write!(
            out,
            "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
            paint.x, paint.y, paint.w, paint.h, paint.color[0], paint.color[1], paint.color[2]
        );
        if paint.alpha < 0.999 {
            let _ = write!(out, " {:.2}", paint.alpha);
        }
        out.push('\n');
    }
    out
}
