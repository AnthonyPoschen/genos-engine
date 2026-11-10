#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::float_cmp, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! Outline shaping from the TrueType loader. The bitmap stays the default.

use std::path::Path;

use genos_load::{load_font, FileSource};
use genos_scene::Camera;
use genos_ui::{
    lighting_frame, paint_node_text, paint_snapshot, raster_outline, shape_ltr, Node, Pointer,
    Sizing, State, TextFace,
};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../load/fixtures/glyph.ttf")
}

#[test]
fn the_fixture_glyph_rasterizes_and_shapes_left_to_right() {
    let font = load_font(&FileSource::new(fixture())).expect("glyph.ttf");
    let glyph = font.glyph('A').expect("A");
    let mask = raster_outline(&glyph.contours, font.units_per_em(), 16.0);
    assert!(mask.advance > 0.0, "ink width {}", mask.advance);
    assert!(mask.pixels.iter().any(|pixel| *pixel > 0));

    let shaped = shape_ltr(&font, "AA", 16.0, None);
    assert_eq!(shaped.glyphs.len(), 2);
    assert_eq!(shaped.glyphs[0].x, 0.0);
    let pen = 800.0 / 1024.0 * 16.0;
    assert!((shaped.glyphs[0].advance - pen).abs() < 1.0e-3);
    assert!((shaped.glyphs[1].x - pen).abs() < 1.0e-3);
    assert!((shaped.width - pen * 2.0).abs() < 1.0e-3, "kerning changed the width");

    let wrapped = shape_ltr(&font, "AA", 16.0, Some(pen + 1.0));
    assert_eq!(wrapped.glyphs.len(), 2);
    assert!(wrapped.glyphs[1].y > shaped.glyphs[1].y);
    assert!((wrapped.width - pen).abs() < 1.0e-3);
}

#[test]
fn an_outline_label_paints_a_quad_and_the_lighting_panel_stays_on_the_bitmap() {
    let font = load_font(&FileSource::new(fixture())).expect("glyph.ttf");
    let outline = genos_ui::OutlineFont::rasterize(&font, 16.0, "A");
    let mut node = Node::new(3);
    node.face = TextFace::Outline;
    node.text = Some("A".into());
    node.width = Sizing::fixed(80.0);
    let paints = paint_node_text(&node, Some(&outline), [0.0, 0.0], [0.9, 0.9, 0.9]);
    assert!(
        paints.iter().any(|paint| paint.w > 0.0 && paint.alpha > 0.0 && paint.alpha < 1.0),
        "outline quad missing: {paints:?}"
    );

    let mut state = State::default();
    state.omarchy_current = Some(std::path::PathBuf::from("/no/such/omarchy"));
    state.outline = Some(outline);
    let camera = Camera::opening();
    let pointer = Pointer::at(0.0, 0.0, false);
    let view = [1280.0, 720.0];
    let lamp = Some([0.0, 7.0, 0.0]);
    let first = lighting_frame(&mut state, view, &camera, lamp, pointer);
    let second = lighting_frame(&mut state, view, &camera, lamp, pointer);
    assert_eq!(paint_snapshot(&first.paints), paint_snapshot(&second.paints));
    assert!(second.damage.is_empty(), "{:?}", second.damage);
    assert!(first.paints.iter().all(|paint| paint.alpha == 1.0));
}
