#![cfg_attr(
    test,
    expect(
        clippy::panic,
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "a test aborts when a fixture fails"
    )
)]

//! The world-cell read must not mix cells from the two off-axis surfaces at a
//! corner. A slanted ray hit reads the dominant axis' plane only (ADR 0039 S2).

#[test]
fn slanted_sh_read_uses_only_the_dominant_axis() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join("lighting_cache.glsl");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let body = text
        .split("vec4 lighting_sh_read(vec3 p, vec3 n, bool claim)")
        .nth(1)
        .expect("the world-cell SH read exists")
        .split("vec4 lighting_sc_shown")
        .next()
        .unwrap();
    assert!(
        body.contains("if (abs(n[ax]) < 1.0)"),
        "slanted SH read loops over off-axis cells; that mixes corner surfaces"
    );
}

#[test]
fn an_axis_aligned_seed_does_not_blend_the_neighbour_cell() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join("lighting_cache.glsl");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let body = text
        .split("vec3 lighting_cache_irradiance(vec3 pos, vec3 n) {")
        .nth(1)
        .expect("the cache irradiance read exists")
        .split("void lighting_world_add")
        .next()
        .unwrap();
    assert!(
        !body.contains("lighting_sc_display || slanted"),
        "the display path still blends neighbour cells into an axis-aligned seed"
    );
    assert!(
        body.contains("if (slanted && hsize < LIGHTING_SH_HIT_SPAN * LIGHTING_SH_CELL)"),
        "a slanted hit must still be able to read the lattice"
    );
    assert!(text.contains("const float LIGHTING_SC_HANDOVER = 16.0"));
    assert!(text.contains("const float LIGHTING_SC_HANDOVER_K = 2.0"));
    assert!(text.contains("lighting_sc_set_seed(slot, hc, 0.25,"));
    assert!((genos_render::lighting_sh::TEXEL_HANDOVER_SETS - 16.0).abs() <= f32::EPSILON);
    assert!((genos_render::lighting_sh::TEXEL_HANDOVER_K - 2.0).abs() <= f32::EPSILON);
    assert!((genos_render::lighting_sh::TEXEL_SEED_CONFIDENCE - 0.25).abs() <= f32::EPSILON);
}
