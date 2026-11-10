#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]

//! A zero mean is not evidence that a world-cache cell is populated.
//!
//! Eight all-zero sets used to pass `se < 0.05 * mean + 1e-4` and the
//! perceptual stop, so the texel took no further rays. A cell is resolved
//! only at `LIGHTING_SC_FULL_SETS`. These checks mirror that predicate and
//! pin the shader text. They do not run the GPU.

const FULL_SETS: f32 = 64.0;
const ASVGF_EPS: f32 = 0.05;
const ASVGF_SE: f32 = 0.05;

fn resolved(sets: f32, mean: f32, se: f32, lam: f32) -> bool {
    if lam >= ASVGF_EPS {
        return false;
    }
    if sets < FULL_SETS - 0.5 {
        return false;
    }
    if mean <= 0.0 {
        return true;
    }
    se < ASVGF_SE * mean
}

fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src.find(name).unwrap_or_else(|| panic!("missing {name}"));
    let rest = &src[start..];
    let open = rest.find('{').unwrap_or_else(|| panic!("no body {name}"));
    let mut depth = 0i32;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..=open + i];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed {name}");
}

#[test]
fn eight_zero_sets_are_not_resolved() {
    assert!(
        !resolved(8.0, 0.0, 3.5e-5, 0.0),
        "eight zero sets used to look resolved"
    );
    assert!(resolved(64.0, 0.0, 0.0, 0.0), "a full count of a real zero is dark");
    assert!(!resolved(64.0, 1.0, 0.1, 0.0), "a noisy full cell is not resolved");
    assert!(!resolved(64.0, 0.0, 0.0, 0.05), "a changing cell stays due");

    let cache = include_str!("../shaders/lighting_cache.glsl");
    let body = function_body(cache, "bool lighting_sc_resolved(uint slot, float t, float lam)");
    assert!(body.contains("t < LIGHTING_SC_FULL_SETS - 0.5"));
    assert!(body.contains("if (!(mean > 0.0))"));
    assert!(!body.contains("1.0e-4"), "the zero-mean epsilon is gone");
    assert!(!body.contains("LIGHTING_DN_MIN_SETS"));

    let noise = function_body(cache, "vec2 lighting_sc_noise(uint slot)");
    assert!(noise.contains("if (!(lum > 0.0))"));
    assert!(!noise.contains("max(lum, 1.0e-3)"));

    let settled = function_body(cache, "bool lighting_sc_settled(uint slot, float t)");
    assert!(settled.contains("floor(t) < reach - 0.5"));

    let compact = include_str!("../shaders/lighting_compact.comp");
    assert!(compact.contains(
        "t >= LIGHTING_SC_FULL_SETS - 0.5 && chg < LIGHTING_SC_TRICKLE_THR"
    ));
    assert!(compact.contains(
        "t >= LIGHTING_SC_FULL_SETS - 0.5 && !lighting_sc_local_change(slot, mover_ic)"
    ));
    assert!(compact.contains(
        "t >= LIGHTING_SC_FULL_SETS - 0.5 && !lighting_sc_promoted(slot)"
    ));
    assert!(cache.contains("const uint LIGHTING_DIAG_UNDER_TRUSTED = 57u;"));
    assert!(cache.contains("atomicAdd(gstats.diag[LIGHTING_DIAG_UNDER_TRUSTED], 1u);"));
    assert!(cache.contains("clamp(sets / LIGHTING_SC_FULL_SETS, 0.0, 1.0)"));
    assert!(
        cache.contains("y_own > 1.0e-3"),
        "a short lit texel is trust 1 from its own samples, not the shown field"
    );
    assert!(
        cache.contains("under_full"),
        "a short lit count stays in the under-full probe"
    );
    assert!(
        !cache.contains("&& trust < 1.0"),
        "display trust must not decide the under-full count"
    );
    assert!(
        cache.contains("!(y_target > 1.0e-3)"),
        "a bright filter target must not take the black-field prior"
    );

    let hold = function_body(cache, "bool lighting_compact_hold_texel_rays(uint slot)");
    assert!(hold.contains(
        "floor(max(cache.v[3u * slot + 2u].w, 0.0)) < LIGHTING_SC_FULL_SETS - 0.5"
    ));
    assert!(hold.contains("lighting_sh_blend_trust < 1.0"));

    let seed = function_body(
        cache,
        "void lighting_sc_set_seed(uint slot, vec3 rgb, float conf, float sunfrac)",
    );
    assert!(
        !seed.contains("3u * slot + 2u] ="),
        "a seed must not write the own-sample count"
    );
    assert!(!hold.contains("lighting_sc_has_seed"));
    assert!(!hold.contains("lighting_sc_seed("));
    let call = compact
        .find("if (lighting_compact_hold_texel_rays(slot))")
        .expect("compact calls the hold");
    let lists = compact.find("bool sparse_c").expect("fine lists follow the hold");
    let window = &compact[call..lists];
    assert!(
        !window.contains("lighting_sc_seed") && !window.contains("lighting_sc_has_seed"),
        "a seed must not drop the texel before the ray lists"
    );
    assert!(
        cache.contains("own_sets < 0.5"),
        "the empty-corner fallback keys off the own-sample count"
    );

    let probe = include_str!("../shaders/lighting_screen_probe.glsl");
    let indirect = function_body(
        probe,
        "vec3 lighting_screen_probe_indirect(uvec2 p, vec3 pos, vec3 n)",
    );
    assert!(indirect.contains("lighting_sh_compose_lattice(pos, n, false)"));
    assert!(indirect.contains("if (!(coarse.w > LIGHTING_CACHE_HAS_W))"));

    let pop = function_body(cache, "vec4 lighting_sc_debug_population(vec3 pos, vec3 n)");
    assert!(pop.contains("lighting_sc_seed(s)"), "the record keeps the seed apart from the mean");
    assert!(pop.contains("3u * s + 2u"), "the record reads the own-sample count");
    assert!(pop.contains("scene_inside(tp + tn * 0.02)"), "the record flags a ray origin inside geometry");
    let compose = include_str!("../shaders/lighting_compose.comp");
    let sun = include_str!("../shaders/lighting_sun_compose.comp");
    for src in [compose, sun] {
        assert!(src.contains("lighting_sc_debug_population(pos, n)"));
        assert!(src.contains("size >= 0.2 ? 128u : 0u"));
        assert!(src.contains("own_b |= 128u"));
    }
}
