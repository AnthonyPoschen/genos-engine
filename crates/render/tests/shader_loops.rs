//! The ray and shadow helpers are inlined at every call site. A loop with a large
//! constant bound in them invites the driver to unroll it into each copy: a DDA bounded
//! by 4096 sent the NVIDIA compiler past 17 GB without finishing the pipeline, while
//! lavapipe compiled it in a moment. Loops run to a bound read from the scene block.

const SHADERS: [&str; 20] = [
    "overlay.frag",
    "transmit.comp",
    "scene_rays.glsl",
    "scene_data.glsl",
    "mesh_field.glsl",
    "lighting_common.glsl",
    "lighting_pack.glsl",
    "gbuffer.frag",
    "lighting_place.comp",
    "lighting_trace.comp",
    "lighting_direct.comp",
    "lighting_gather.comp",
    "lighting_compose.comp",
    "lighting_sun_compose.comp",
    "lighting_cache.comp",
    "lighting_filter.comp",
    "lighting_compact.comp",
    "lighting_cache.glsl",
    "lighting_cache_slots.glsl",
    "lighting_lamp_pick.glsl",
];

/// Largest constant trip count a loop may have.
const MAX_CONSTANT_TRIPS: u64 = 64;

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
