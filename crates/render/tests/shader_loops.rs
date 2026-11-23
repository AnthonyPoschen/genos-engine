#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::single_element_loop, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! The ray and shadow helpers are inlined at every call site. A loop with a large
//! constant bound in them invites the driver to unroll it into each copy: a DDA bounded
//! by 4096 sent the NVIDIA compiler past 17 GB without finishing the pipeline, while
//! lavapipe compiled it in a moment. Loops run to a bound read from the scene block.

const SHADERS: [&str; 31] = [
    "overlay.frag",
    "transmit.comp",
    "scene_rays.glsl",
    "scene_data.glsl",
    "mesh_field.glsl",
    "lighting_common.glsl",
    "lighting_screen_probe.glsl",
    "lighting_pack.glsl",
    "gbuffer.frag",
    "lighting_trace.comp",
    "lighting_direct.comp",
    "lighting_compose.comp",
    "lighting_sun_compose.comp",
    "lighting_cache.comp",
    "lighting_compact.comp",
    "lighting_cache.glsl",
    "lighting_cache_slots.glsl",
    "lighting_lamp_pick.glsl",
    "media.glsl",
    "media_check.comp",
    "froxel_common.glsl",
    "froxel_inject.comp",
    "froxel_integrate.comp",
    "lighting_cascade.glsl",
    "lighting_cascade_l0_shade.comp",
    "lighting_cascade_l0_trace.comp",
    "lighting_cascade_pub.comp",
    "lighting_cascade_upper_claim.comp",
    "lighting_cascade_upper_pick.comp",
    "lighting_cascade_upper_shade.comp",
    "lighting_cascade_upper_trace.comp",
];

/// Largest constant trip count a loop may have.
const MAX_CONSTANT_TRIPS: u64 = 64;

fn function_body<'a>(src: &'a str, signature: &str) -> &'a str {
    let at = src
        .find(signature)
        .unwrap_or_else(|| panic!("missing {signature}"));
    let rest = &src[at + signature.len()..];
    let open = rest
        .find('{')
        .unwrap_or_else(|| panic!("{signature} has no body"));
    let mut depth = 0i32;
    for (i, ch) in rest[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[open + 1..open + i];
                }
            }
            _ => {}
        }
    }
    panic!("{signature} body does not close")
}

fn source(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

#[test]
fn no_shader_loop_has_a_large_constant_bound() {
    for name in SHADERS {
        for (line_no, line) in source(name).lines().enumerate() {
            let Some(at) = line.find("for (") else {
                continue;
            };
            let header = &line[at..];
            let Some(cond) = header.split(';').nth(1) else {
                continue;
            };
            let Some((_, bound)) = cond.split_once('<') else {
                continue;
            };
            let digits: String = bound
                .trim_start_matches('=')
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(trips) = digits.parse::<u64>() {
                assert!(
                    trips <= MAX_CONSTANT_TRIPS,
                    "{name}:{}: loop bound {trips} is a large constant; bound it by scene data: {}",
                    line_no + 1,
                    line.trim()
                );
            }
        }
    }
}

#[test]
fn the_scene_ray_loops_are_kept_rolled() {
    let rays = source("scene_rays.glsl");
    // A loop over list entries or cells carries [[dont_unroll]]; a loop over the two or
    // three axes of a box may unroll.
    for line in rays.lines().filter(|l| l.trim_start().starts_with("for (")) {
        let small = ["< 2;", "< 3;"].iter().any(|b| line.contains(b));
        assert!(
            small,
            "scene_rays.glsl loop without [[dont_unroll]]: {}",
            line.trim()
        );
    }
    for name in ["transmit.comp"] {
        assert!(
            source(name).contains("#extension GL_EXT_control_flow_attributes : require"),
            "{name} includes scene_rays.glsl, which needs GL_EXT_control_flow_attributes"
        );
    }
}

/// The picture compose used to compile `scene_candidates_skip` (the occluder
/// grid, `dont_unroll`) into the pass that shades every pixel. The NVIDIA
/// compiler did not drop that call. Compose then took tens of milliseconds and
/// the driver sat in shader compile. The sun compose is a separate pipeline.
#[test]
fn picture_compose_does_not_walk_occluders() {
    for name in [
        "lighting_compose.comp",
        "lighting_sun_compose.comp",
        "lighting_common.glsl",
        "lighting_screen_probe.glsl",
    ] {
        let text = source(name);
        assert!(
            !text.contains("lighting_work.glsl"),
            "{name} includes the work tube"
        );
        let calls = text
            .lines()
            .filter(|line| {
                let code = line.split("//").next().unwrap_or(line);
                code.contains("scene_candidates")
            })
            .count();
        assert_eq!(calls, 0, "{name} calls scene_candidates");
    }
}

/// A refine child starts from the parent's radiance at one set. An empty mean
/// made the first ray throw that prior away.
#[test]
fn a_refine_child_inherits_the_parent_mean_at_one_set() {
    let cache = source("lighting_cache.glsl");
    let spawn = function_body(&cache, "void lighting_fine_spawn(");
    assert!(
        spawn.contains("cache.v[3u * born + 2u] = vec4(pre, LIGHTING_FINE_INHERIT_SETS)"),
        "the child mean is the parent radiance at the inherit count"
    );
    assert!(
        spawn.contains("LIGHTING_FINE_INHERIT_VALUE"),
        "the child value starts high"
    );
    assert!(
        !spawn.contains("1.0 - area"),
        "the area step must not scale the inherited radiance"
    );
}
