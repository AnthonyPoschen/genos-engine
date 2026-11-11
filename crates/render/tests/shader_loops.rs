#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::single_element_loop, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! The ray and shadow helpers are inlined at every call site. A loop with a large
//! constant bound in them invites the driver to unroll it into each copy: a DDA bounded
//! by 4096 sent the NVIDIA compiler past 17 GB without finishing the pipeline, while
//! lavapipe compiled it in a moment. Loops run to a bound read from the scene block.

const SHADERS: [&str; 24] = [
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
    "lighting_fill.glsl",
    "lighting_cache_slots.glsl",
    "lighting_lamp_pick.glsl",
    "media.glsl",
    "media_check.comp",
    "froxel_common.glsl",
    "froxel_inject.comp",
    "froxel_integrate.comp",
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
fn the_fill_resolve_folds_one_ray_with_the_warmup_weight() {
    let fill = source("lighting_fill.glsl");
    assert!(
        fill.contains("n / (rays0 + n)"),
        "the resolve must fold with an n-ray weight"
    );
    assert!(
        fill.contains("lighting_fill_hit_at"),
        "off-screen resolve must read the stashed hit, not the radiance slot"
    );
    assert!(
        fill.contains("barrier()"),
        "the claimer must finish the slot before another ray folds it"
    );
    assert!(
        fill.contains("lighting_fill_f(7)"),
        "fill_far_share must be read"
    );
    assert!(
        !fill.contains("if (prev == mark)"),
        "a second ray into a slot must be summed, not dropped"
    );
    assert!(fill.contains("lighting_ray_fill_on"), "fill must be wired");
    assert!(
        fill.contains("cache.v[LIGHTING_SC_GRAD_AT + slot] = vec4(0.0)"),
        "freeing a slot must drop the previous occupant's check state"
    );
    assert!(
        fill.contains("lighting_fill_seed"),
        "a reused slot must not keep a seed from another key"
    );
    assert!(
        fill.contains("prior >= max_picks"),
        "fill_max_picks is per cell, including a cell that wins several tiles"
    );
    assert!(
        fill.contains("lighting_round() % tiles"),
        "screen check rays must rotate with the round, not rescan the same prefix"
    );
    assert!(
        fill.contains("lighting_hash(block ^ lighting_hash(lighting_round()))"),
        "off-screen checks are one hash per block, not a scan of the slot table"
    );
    let plan = function_body(&fill, "void lighting_fill_dispatch()");
    assert!(
        !plan.contains("LIGHTING_CACHE_SLOTS"),
        "the one scheduler thread must not walk the slot table"
    );
    assert!(
        !plan.contains("LIGHTING_FILL_TILE_CAP"),
        "the one scheduler thread must not walk the tile table"
    );
    assert!(
        plan.contains("gstats.pad2"),
        "the plan records how many serial iterations it ran"
    );
    assert!(
        fill.contains("lighting_fill_real"),
        "an infinite check luma must not be stored or raise lambda"
    );
    let pick = source("lighting_lamp_pick.glsl");
    assert!(
        pick.contains("lighting_fill_check_u"),
        "a check ray's lamp pick must use the stored seed"
    );
    let direct = source("lighting_direct.comp");
    assert!(
        direct.contains("lighting_fill_check_u"),
        "a texel-direct check must not mix the frame into a fill check ray"
    );
    let compact = source("lighting_compact.comp");
    assert!(
        compact.contains("lighting_fill_visit"),
        "world mode 3 must admit through the fill visit"
    );
    assert!(
        !compact.contains("LIGHTING_WRC_SCOUT"),
        "the old refresh walk must not be listed"
    );
}

/// The share pass writes a seed. These strings are the gates that a behavioural
/// CPU test cannot see: the stamp source, the confidence scale, and the clamp.
#[test]
fn the_fill_share_writes_a_seed_under_the_five_gates() {
    let fill = source("lighting_fill.glsl");
    assert!(
        fill.contains("lighting_fill_share("),
        "an in-tile sample must reach the share pass"
    );
    let cache = source("lighting_cache.glsl");
    assert!(
        fill.contains("lighting_sc_set_fill_seed"),
        "a shared sample must enter as a fill seed"
    );
    assert!(
        cache.contains("lighting_sc_set_seed(slot, rgb, conf, sunfrac)"),
        "the fill mark is written on the seed channel"
    );
    assert!(
        !fill.contains("shared_count"),
        "F4 must not add a shared_count field"
    );
    assert!(
        fill.contains("lighting_fill_u(22)"),
        "fill_share must be read"
    );
    assert!(
        fill.contains("lighting_fill_f(23)"),
        "a near hit must read fill_share_min_hit"
    );
    let accept = function_body(&fill, "float lighting_fill_share_accept");
    assert!(
        accept.contains("lighting_sc_updated(recv)"),
        "the stamp reads the commit frame, not the visit word"
    );
    assert!(
        !accept.contains("cache.v[3u * recv].w"),
        "a bounce read updates the visit word and is not the mean"
    );
    let rgb = function_body(&fill, "vec3 lighting_fill_share_rgb");
    assert!(
        rgb.contains("b.x > 0.0"),
        "a zeroed estimator is not a firefly bound"
    );
    assert!(
        fill.contains("lighting_fill_f(29) * scale"),
        "own-count expiry scales the seed confidence"
    );
    assert!(
        fill.contains("lighting_fill_share_claim(found_w[s] * lighting_fill_share_scale(found[s]), r)"),
        "a fresher cell's scale stays in the claim"
    );
    assert!(
        fill.contains("lighting_fill_seed_spent"),
        "the fill seed's weight reaches 0"
    );
    assert!(
        fill.contains("lighting_fill_u(31)"),
        "fill_share_handover is the drop, and 0 uses the warmup count"
    );
    assert!(
        cache.contains("const float LIGHTING_SC_HANDOVER = 16.0;"),
        "the 16-set handover stays for seeds this pass did not write"
    );
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
