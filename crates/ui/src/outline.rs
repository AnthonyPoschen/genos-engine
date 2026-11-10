//! Left-to-right outline text from a TrueType glyph.
//!
//! Advances come from `Glyph.advance`. There is no kerning and no bidi.
//! Off-curve points are quadratic controls. Two off-curve points in a row
//! imply an on-curve point at their midpoint. The fill is even-odd on a
//! grid of `px` by `px` covering the em square. Y in the atlas grows down.

#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

use std::collections::BTreeMap;

use genos_load::{Font, OutlinePoint};

use crate::layout::Node;
use crate::text::{self, TextFace};
use crate::Paint;

/// One rasterized glyph. `advance` is the ink width in pixels. Shaping uses
/// the font advance instead, so a side bearing does not change the pen.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasGlyph {
    pub advance: f32,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

/// A glyph placed on a line. `x` and `y` are the top-left of its em box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub ch: char,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
}

/// A left-to-right run. `width` is the widest line. `height` is `lines * px`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedText {
    pub glyphs: Vec<PlacedGlyph>,
    pub width: f32,
    pub height: f32,
}

/// Masks and pen advances for the characters a label will draw.
///
/// `advances` is `Glyph.advance / units_per_em * px`. The mask's own
/// `advance` is the ink width and is not the pen.
#[derive(Clone, Debug, Default)]
pub struct OutlineFont {
    pub px: f32,
    pub units_per_em: u16,
    pub advances: BTreeMap<char, f32>,
    pub masks: BTreeMap<char, AtlasGlyph>,
}

impl OutlineFont {
    /// Rasterize each distinct character in `text` that the font can draw.
    pub fn rasterize(font: &Font, px: f32, text: &str) -> Self {
        let units = f32::from(font.units_per_em().max(1));
        let scale = px / units;
        let mut masks = BTreeMap::new();
        let mut advances = BTreeMap::new();
        for ch in text.chars() {
            if masks.contains_key(&ch) {
                continue;
            }
            let Ok(glyph) = font.glyph(ch) else {
                continue;
            };
            advances.insert(ch, f32::from(glyph.advance) * scale);
            masks.insert(
                ch,
                raster_outline(&glyph.contours, font.units_per_em(), px),
            );
        }
        Self {
            px,
            units_per_em: font.units_per_em(),
            advances,
            masks,
        }
    }
}

/// Even-odd fill of `contours` into an em-square mask.
pub fn raster_outline(
    contours: &[Vec<OutlinePoint>],
    units_per_em: u16,
    px: f32,
) -> AtlasGlyph {
    let side = grid_side(px);
    let scale = if units_per_em == 0 {
        0.0
    } else {
        side as f32 / f32::from(units_per_em)
    };
    let mut pixels = vec![0_u8; side as usize * side as usize];
    let mut min_x = f32::MAX;
    let mut max_x = 0.0_f32;
    let mut polys = Vec::new();
    for contour in contours {
        let poly = flatten(contour, scale, side as f32);
        for point in &poly {
            min_x = min_x.min(point[0]);
            max_x = max_x.max(point[0]);
        }
        if poly.len() >= 3 {
            polys.push(poly);
        }
    }
    fill_even_odd(&mut pixels, side, &polys);
    let advance = if min_x > max_x { 0.0 } else { max_x - min_x };
    AtlasGlyph {
        advance,
        w: side,
        h: side,
        pixels,
    }
}

/// Place `text` left to right. A line breaks on whitespace, then on a glyph
/// that does not fit `max_width`. A missing glyph, including a composite the
/// loader rejects, advances half an em and draws nothing.
pub fn shape_ltr(font: &Font, text: &str, px: f32, max_width: Option<f32>) -> ShapedText {
    let units = f32::from(font.units_per_em().max(1));
    let scale = px / units;
    shape_advances(text, px, max_width, |ch| match font.glyph(ch) {
        Ok(glyph) => f32::from(glyph.advance) * scale,
        Err(_) => px * 0.5,
    })
}

/// Shape with the pen advances stored on an [`OutlineFont`].
pub fn shape_outline(font: &OutlineFont, text: &str, max_width: Option<f32>) -> ShapedText {
    shape_advances(text, font.px, max_width, |ch| {
        font.advances.get(&ch).copied().unwrap_or(font.px * 0.5)
    })
}

/// Paint `node`'s text. Bitmap stays the lighting panel's cells. Outline
/// draws one quad per glyph whose mask is on `font`.
pub fn paint_node_text(
    node: &Node,
    font: Option<&OutlineFont>,
    origin: [f32; 2],
    color: [f32; 3],
) -> Vec<Paint> {
    let Some(text) = node.text.as_deref() else {
        return Vec::new();
    };
    match node.face {
        TextFace::Bitmap => bitmap_paints(text, origin, node_width(node), color),
        TextFace::Outline => {
            let Some(font) = font else {
                return Vec::new();
            };
            let shaped = shape_outline(font, text, node_width(node));
            let mut paints = Vec::new();
            outline_quads(&mut paints, &shaped, font, origin, color);
            paints
        }
    }
}

fn node_width(node: &Node) -> Option<f32> {
    match node.width {
        crate::layout::Sizing::Fixed(value) => {
            Some((value - node.pad.left - node.pad.right).max(0.0))
        }
        crate::layout::Sizing::Fit { .. }
        | crate::layout::Sizing::Grow { .. }
        | crate::layout::Sizing::Percent(_) => None,
    }
}

fn bitmap_paints(text: &str, origin: [f32; 2], max_width: Option<f32>, color: [f32; 3]) -> Vec<Paint> {
    let limit = max_width.unwrap_or(f32::MAX);
    text::blots(text, origin[0], origin[1], limit)
        .into_iter()
        .map(|blot| Paint {
            x: blot.x,
            y: blot.y,
            w: blot.w,
            h: blot.h,
            color,
            alpha: 1.0,
        })
        .collect()
}

pub(crate) fn paint_outline_into(
    out: &mut Vec<Paint>,
    font: &OutlineFont,
    text: &str,
    origin: [f32; 2],
    max_width: f32,
    color: [f32; 3],
) {
    let shaped = shape_outline(font, text, Some(max_width));
    outline_quads(out, &shaped, font, origin, color);
}

fn outline_quads(
    out: &mut Vec<Paint>,
    shaped: &ShapedText,
    font: &OutlineFont,
    origin: [f32; 2],
    color: [f32; 3],
) {
    for glyph in &shaped.glyphs {
        let Some(mask) = font.masks.get(&glyph.ch) else {
            continue;
        };
        let coverage = coverage(mask);
        if coverage <= 0.0 || mask.w == 0 || mask.h == 0 {
            continue;
        }
        out.push(Paint {
            x: origin[0] + glyph.x,
            y: origin[1] + glyph.y,
            w: mask.w as f32,
            h: mask.h as f32,
            color,
            alpha: coverage,
        });
    }
}

fn coverage(mask: &AtlasGlyph) -> f32 {
    if mask.pixels.is_empty() {
        return 0.0;
    }
    let set = mask.pixels.iter().filter(|pixel| **pixel > 0).count();
    set as f32 / mask.pixels.len() as f32
}

fn shape_advances(text: &str, px: f32, max_width: Option<f32>, advance_of: impl Fn(char) -> f32) -> ShapedText {
    let limit = max_width.unwrap_or(f32::INFINITY);
    let line_h = px.max(1.0);
    let mut glyphs = Vec::new();
    let mut cursor = 0.0_f32;
    let mut line = 0_u32;
    let mut widest = 0.0_f32;
    let mut line_empty = true;
    for word in split_words(text) {
        if word.space && !line_empty {
            let gap = advance_of(' ');
            if cursor + gap > limit && cursor > 0.0 {
                widest = widest.max(cursor);
                cursor = 0.0;
                line = line.saturating_add(1);
                line_empty = true;
            } else {
                cursor += gap;
            }
        }
        for ch in word.chars {
            let advance = advance_of(ch);
            if !line_empty && cursor + advance > limit {
                widest = widest.max(cursor);
                cursor = 0.0;
                line = line.saturating_add(1);
            }
            glyphs.push(PlacedGlyph {
                ch,
                x: cursor,
                y: line as f32 * line_h,
                advance,
            });
            cursor += advance;
            line_empty = false;
        }
    }
    if glyphs.is_empty() {
        return ShapedText {
            glyphs,
            width: 0.0,
            height: 0.0,
        };
    }
    widest = widest.max(cursor);
    ShapedText {
        glyphs,
        width: widest,
        height: (line + 1) as f32 * line_h,
    }
}

struct Word {
    chars: Vec<char>,
    space: bool,
}

fn split_words(text: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut current = Vec::new();
    let mut spaced = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if current.is_empty() {
                spaced = true;
            } else {
                words.push(Word {
                    chars: std::mem::take(&mut current),
                    space: spaced,
                });
                spaced = true;
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(Word {
            chars: current,
            space: spaced,
        });
    }
    words
}

fn grid_side(px: f32) -> u32 {
    let side = px.round();
    if !side.is_finite() || side < 1.0 {
        return 1;
    }
    if side > 256.0 {
        return 256;
    }
    side as u32
}

#[derive(Clone, Copy)]
struct Pt {
    x: f32,
    y: f32,
    on: bool,
}

fn flatten(contour: &[OutlinePoint], scale: f32, px: f32) -> Vec<[f32; 2]> {
    if contour.is_empty() {
        return Vec::new();
    }
    let mut src: Vec<Pt> = contour
        .iter()
        .map(|point| Pt {
            x: point.x as f32 * scale,
            y: px - point.y as f32 * scale,
            on: point.on_curve,
        })
        .collect();
    src = lead_on_curve(src);
    let Some(first) = src.first().copied() else {
        return Vec::new();
    };
    let mut poly = vec![[first.x, first.y]];
    let mut index = 1;
    while index < src.len() {
        if src[index].on {
            poly.push([src[index].x, src[index].y]);
            index += 1;
            continue;
        }
        let ctrl = src[index];
        let next = src.get(index + 1).copied();
        let (end, ate_next) = match next {
            Some(point) if point.on => (point, true),
            Some(point) => (midpoint(ctrl, point), false),
            None => (first, false),
        };
        let start = poly.last().copied().unwrap_or([first.x, first.y]);
        push_quad(&mut poly, start, [ctrl.x, ctrl.y], [end.x, end.y]);
        index += if ate_next { 2 } else { 1 };
    }
    poly
}

fn lead_on_curve(mut pts: Vec<Pt>) -> Vec<Pt> {
    let Some(first) = pts.first().copied() else {
        return pts;
    };
    if first.on {
        return pts;
    }
    let Some(last) = pts.last().copied() else {
        return pts;
    };
    if last.on {
        pts.pop();
        pts.insert(0, last);
        return pts;
    }
    pts.insert(0, midpoint(last, first));
    pts
}

fn midpoint(a: Pt, b: Pt) -> Pt {
    Pt {
        x: f32::midpoint(a.x, b.x),
        y: f32::midpoint(a.y, b.y),
        on: true,
    }
}

fn push_quad(poly: &mut Vec<[f32; 2]>, start: [f32; 2], ctrl: [f32; 2], end: [f32; 2]) {
    const STEPS: u32 = 4;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let u = 1.0 - t;
        poly.push([
            u * u * start[0] + 2.0 * u * t * ctrl[0] + t * t * end[0],
            u * u * start[1] + 2.0 * u * t * ctrl[1] + t * t * end[1],
        ]);
    }
}

fn fill_even_odd(pixels: &mut [u8], side: u32, polys: &[Vec<[f32; 2]>]) {
    if polys.is_empty() {
        return;
    }
    let side_usize = side as usize;
    for row in 0..side_usize {
        for col in 0..side_usize {
            let x = col as f32 + 0.5;
            let y = row as f32 + 0.5;
            let mut inside = false;
            for poly in polys {
                if even_odd(x, y, poly) {
                    inside = !inside;
                }
            }
            if inside {
                pixels[row * side_usize + col] = 255;
            }
        }
    }
}

fn even_odd(x: f32, y: f32, poly: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = poly.len() - 1;
    for (index, point) in poly.iter().enumerate() {
        let (xi, yi) = (point[0], point[1]);
        let (xj, yj) = (poly[previous][0], poly[previous][1]);
        let crosses = (yi > y) != (yj > y);
        if crosses {
            let denom = yj - yi;
            if denom.abs() > f32::EPSILON {
                let at = (xj - xi) * (y - yi) / denom + xi;
                if x < at {
                    inside = !inside;
                }
            }
        }
        previous = index;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_contour_fills_its_interior() {
        let contour = vec![
            OutlinePoint { x: 256, y: 256, on_curve: true },
            OutlinePoint { x: 768, y: 256, on_curve: true },
            OutlinePoint { x: 768, y: 768, on_curve: true },
            OutlinePoint { x: 256, y: 768, on_curve: true },
        ];
        let mask = raster_outline(&[contour], 1024, 16.0);
        assert!(mask.advance > 1.0, "advance {}", mask.advance);
        let center = 8 * 16 + 8;
        assert_eq!(mask.pixels.get(center).copied(), Some(255));
        assert_eq!(mask.pixels.first().copied(), Some(0));
    }

    #[test]
    fn a_hole_in_the_fixture_glyph_stays_clear() {
        let outer = vec![
            OutlinePoint { x: 128, y: 128, on_curve: true },
            OutlinePoint { x: 896, y: 128, on_curve: true },
            OutlinePoint { x: 896, y: 896, on_curve: true },
            OutlinePoint { x: 128, y: 896, on_curve: true },
        ];
        let hole = vec![
            OutlinePoint { x: 384, y: 384, on_curve: true },
            OutlinePoint { x: 640, y: 384, on_curve: true },
            OutlinePoint { x: 640, y: 640, on_curve: true },
            OutlinePoint { x: 384, y: 640, on_curve: true },
        ];
        let mask = raster_outline(&[outer, hole], 1024, 16.0);
        let at = |col: usize, row: usize| mask.pixels[row * 16 + col];
        assert_eq!(at(8, 8), 0, "the hole filled");
        assert_eq!(at(3, 8), 255, "the ring stayed empty");
        assert_eq!(at(0, 0), 0);
    }
}
