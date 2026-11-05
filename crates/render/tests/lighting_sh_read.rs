#![cfg_attr(test, expect(clippy::panic, clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! The world-cell read must not mix cells from the two off-axis surfaces at a
//! corner. A slanted ray hit reads the dominant axis' plane only (ADR 0039 S2).

#[test]
fn slanted_sh_read_uses_only_the_dominant_axis() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join("lighting_cache.glsl");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let body = text.split("vec4 lighting_sh_read(vec3 p, vec3 n, bool claim)").nth(1)
        .expect("the world-cell SH read exists").split("vec4 lighting_sc_shown").next().unwrap();
    assert!(
        body.contains("if (abs(n[ax]) < 1.0)"),
        "slanted SH read loops over off-axis cells; that mixes corner surfaces"
    );
}
