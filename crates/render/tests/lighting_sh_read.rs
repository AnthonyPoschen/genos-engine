#![cfg_attr(
    test,
    expect(
        clippy::panic,
        clippy::expect_used,
        reason = "a test aborts when a fixture fails"
    )
)]

//! Source guards for the SH-cell read and for what a texel then shows.
//!
//! The numeric helpers transcribe expressions from `lighting_cache.glsl`.
//! They do not run the shader. Each test checks that the expression it
//! transcribes is still the one in the source, then checks a value that
//! expression produces when the own-ray mean differs from the seed.

use genos_render::LightingSettings;

const HANDOVER: f32 = 16.0;
const HANDOVER_K: f32 = 2.0;
const SEED_CONFIDENCE: f32 = 0.25;
/// `lighting_sc_flags` bit that turns sparse fill off.
const SPARSE_OFF: u32 = 1_u32 << 24;
/// Bit that restores the old seeding, where the seed counts as an own sample.
const SEED_OLD: u32 = 1_u32 << 27;
/// Bit that turns layer 2 off.
const LAYER2_OFF: u32 = 1_u32 << 29;
/// Bit that turns SVGF off. Clear means SVGF is on, and sparse fill is then off.
const SVGF_OFF: u32 = 1_u32 << 31;

fn shader(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn cache_shader() -> String {
    shader("lighting_cache.glsl")
}

/// Body of the first `{ ... }` that follows `sig`, skipping a forward
/// declaration that ends in `;`.
fn function_body<'a>(text: &'a str, sig: &str) -> &'a str {
    let mut from = 0;
    loop {
        let Some(at) = text[from..].find(sig) else {
            panic!("missing {sig}");
        };
        let rest = &text[from + at + sig.len()..];
        let trimmed = rest.trim_start();
        if trimmed.starts_with('{') {
            let brace = rest.find('{').expect("opening brace");
            return block_body(&rest[brace + 1..]);
        }
        from += at + sig.len();
    }
}

fn block_body(src: &str) -> &str {
    let mut depth = 1i32;
    let mut chars = src.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '/' {
            if chars.peek().is_some_and(|(_, n)| *n == '/') {
                while chars.next().is_some_and(|(_, n)| n != '\n') {}
                continue;
            }
            if chars.peek().is_some_and(|(_, n)| *n == '*') {
                chars.next();
                loop {
                    match chars.next() {
                        Some((_, '*')) if chars.peek().is_some_and(|(_, n)| *n == '/') => {
                            chars.next();
                            break;
                        }
                        Some(_) => {}
                        None => panic!("unclosed comment"),
                    }
                }
                continue;
            }
        }
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[..i];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed block");
}

fn precedes(hay: &str, earlier: &str, later: &str) {
    let ia = hay
        .find(earlier)
        .unwrap_or_else(|| panic!("missing {earlier}"));
    let ib = hay.find(later).unwrap_or_else(|| panic!("missing {later}"));
    assert!(ia < ib, "{earlier} should precede {later}");
}

fn near(got: [f32; 3], want: [f32; 3]) -> bool {
    got.iter().zip(want).all(|(a, b)| (a - b).abs() <= 1.0e-5)
}


/// `lighting_sc_shown` while the default preset's SVGF arm is showing a fine
/// texel and the stale-field world-cell read missed. Sun gain is 1. A fresh
/// field wins. Otherwise one own sample shows that sample, and none shows
/// the seed. The sparse mix later in the function is not this arm.
fn default_fine_shown(
    field_fresh: bool,
    field: [f32; 3],
    own: [f32; 3],
    own_w: f32,
    seed: [f32; 3],
    seed_w: f32,
) -> [f32; 3] {
    if field_fresh && field.iter().any(|c| *c > 0.0) {
        return field;
    }
    if own_w > 0.5 {
        return own;
    }
    if seed_w > 0.0 {
        return seed;
    }
    [0.0; 3]
}

/// Coarse-key decode inside `lighting_sc_shown` when sparse is on and the
/// texel level is below 2. `None` is the seed-only case: no lit coarse key.
/// `near` is `lighting_sc_sparse(slot, 2)`, `far` is step 4. rgb in xyz, the
/// shader's weight in w.
fn coarse_key(level: u32, near: [f32; 4], far: [f32; 4]) -> Option<([f32; 3], f32)> {
    if level >= 2 {
        return None;
    }
    let mut sp = [0.0_f32; 4];
    let mut ck = 0.0_f32;
    if level == 0 {
        sp = near;
        if sp[3] < -1.5 {
            ck = 0.5 * (-sp[3] - 2.0);
        } else if sp[3] > 0.5 {
            ck = sp[3] - 1.0;
        }
    }
    if sp[3] > -1.5 && sp[3] < 0.5 {
        sp = far;
        ck = if sp[3] < -1.5 {
            0.25 * (-sp[3] - 2.0)
        } else {
            0.5 * (sp[3] - 1.0)
        };
    }
    if sp[3].abs() > 0.5 {
        return Some(([sp[0], sp[1], sp[2]], ck.max(1.0e-3)));
    }
    None
}

/// Confidence-weighted prior: coarse key, then the seed, then divide by the
/// summed confidence. `None` when both are absent.
fn prior_of(
    coarse: Option<([f32; 3], f32)>,
    seed: [f32; 3],
    seed_w: f32,
) -> Option<([f32; 3], f32)> {
    let mut prior = [0.0_f32; 3];
    let mut pc = 0.0_f32;
    if let Some((rgb, ck)) = coarse {
        for (out, sample) in prior.iter_mut().zip(rgb) {
            *out = sample * ck;
        }
        pc = ck;
    }
    if seed_w > 0.0 {
        for (out, sample) in prior.iter_mut().zip(seed) {
            *out += sample * seed_w;
        }
        pc += seed_w;
    }
    if pc > 0.0 {
        let scale = pc;
        return Some((prior.map(|v| v / scale), pc));
    }
    None
}

fn mix(prior: [f32; 3], own: [f32; 3], weight: f32) -> [f32; 3] {
    std::array::from_fn(|ch| prior[ch] + (own[ch] - prior[ch]) * weight)
}

#[test]
fn an_axis_aligned_hit_falls_through_to_the_owning_cells_tilt() {
    let text = cache_shader();
    let body = function_body(&text, "vec3 lighting_cache_irradiance(vec3 pos, vec3 n)");
    assert!(
        !body.contains("lighting_sc_display || slanted"),
        "the display flag still selects the lattice blend for an axis-aligned hit"
    );
    let gate = "if (slanted && hsize < LIGHTING_SH_HIT_SPAN * LIGHTING_SH_CELL)";
    let at = body.find(gate).expect("the slanted lattice gate");
    let after = &body[at + gate.len()..];
    let open = after.find('{').expect("lattice gate brace");
    let lattice = block_body(&after[open + 1..]);
    assert!(
        lattice.contains("vec4 b = lighting_sh_read(pos, n, false);"),
        "the slanted branch must still attempt the lattice read"
    );
    assert!(
        lattice.contains("return b.rgb * keep;"),
        "a lit lattice read returns that read"
    );
    assert!(
        !lattice.contains("lighting_sh_tilt"),
        "the lattice branch must not be the owning-cell fallback"
    );
    let after_gate = after[open + 1 + lattice.len() + 1..].trim_start();
    let fallback = "return lighting_sh_tilt(slot, e.rgb, cache.v[3u * slot + 1u].xyz, n) * lighting_sc_sun_gain(slot) * keep;";
    assert!(
        after_gate.starts_with(fallback),
        "an axis-aligned hit, and a slanted lattice miss, tilt the owning slot's mean from that slot's axis. got {after_gate}"
    );
}

#[test]
fn freeze_cache_holds_the_sh_coefficients_and_a_change_marks_readers() {
    // The mark is the existing cover rect. It does not make a slanted screen
    // pixel a reader: that pixel's lattice read never calls find_sized.
    // A hit-shade tilt runs with display false, so it records no cover either.
    let text = cache_shader();
    let body = function_body(
        &text,
        "void lighting_sh_put(uint slot, vec3 c0, vec3 cx, vec3 cy, vec3 cz)",
    );
    let freeze = body
        .find("LIGHTING_TOGGLE_FREEZE_CACHE")
        .expect("freeze_cache");
    let store = body.find("LIGHTING_SH_AT").expect("the coefficient store");
    let mark = body
        .find("lighting_compose_mark_slot(slot)")
        .expect("reader mark");
    assert!(
        freeze < store,
        "freeze_cache returns before a coefficient write"
    );
    assert!(
        store < mark,
        "the mark compares the value after the store"
    );
    assert!(
        body.contains("lighting_sh_get(slot, after0, afterx, aftery, afterz)"),
        "the mark reads the stored coefficients, not the pre-quantize blend"
    );
    assert!(
        body.contains("LIGHTING_HIT_PATCH"),
        "a texel store does not take the world-cell mark"
    );
    let screen = function_body(
        &text,
        "vec4 lighting_world_irradiance_in(vec3 pos, vec3 n, vec3 wpos, bool obj)",
    );
    let gate = "if (lighting_sc_slanted(n))";
    let at = screen.find(gate).expect("the slanted screen gate");
    let after = &screen[at + gate.len()..];
    let open = after.find('{').expect("screen gate brace");
    let arm = block_body(&after[open + 1..]);
    assert!(
        !arm.contains("lighting_compose_cover") && !arm.contains("lighting_compose_mark"),
        "the slanted screen arm does not register a compose reader"
    );
}

#[test]
fn a_slanted_screen_read_uses_the_neighbour_blend_and_an_empty_read_keeps_the_probes() {
    // The axis-aligned hit fallback inside lighting_cache_irradiance stays the
    // owning cell. The screen return is this neighbour read. It accepts a
    // slant: the lookup axis is the dominant component, and the tilt uses the
    // true normal. An empty read is w 0, and compose's mix by that w keeps
    // the probes. The call does not claim.
    let text = cache_shader();
    let read = function_body(
        &text,
        "LightingShField lighting_sh_gather(vec3 p, vec3 n, bool claim)",
    );
    assert!(
        !read.contains("if (abs(n[ax]) < 1.0)"),
        "the neighbour read must not skip a slanted normal"
    );
    assert!(
        read.contains("lighting_cache_find_sized(centre, an, false, cs, p)"),
        "the read looks up corners and does not allocate"
    );
    assert!(
        !read.contains(", true, cs, p)"),
        "the neighbour read must not claim a slot"
    );
    assert!(read.contains("if (facing <= 0.0)"));
    assert!(read.contains("if (plane > LIGHTING_CELL_HALF * cs)"));
    assert!(read.contains("if (lighting_sc_stale(s))"));
    assert!(
        read.contains("ws > 0.0 ? vec4(sum / ws, 1.0) : vec4(0.0)"),
        "no accepted corner is an empty read, w 0"
    );
    let wrapper = function_body(&text, "vec4 lighting_sh_read(vec3 p, vec3 n, bool claim)");
    assert!(
        wrapper.contains("return lighting_sh_gather(p, n, claim).value;"),
        "the screen read is the gather's value, including an empty read"
    );
    precedes(read, "if (plane > LIGHTING_CELL_HALF * cs)", "if (claim && LIGHTING_SC_WRITES)");
    let screen = function_body(
        &text,
        "vec4 lighting_world_irradiance_in(vec3 pos, vec3 n, vec3 wpos, bool obj)",
    );
    let gate = "if (lighting_sc_slanted(n))";
    let at = screen.find(gate).expect("the slanted screen gate");
    let after = &screen[at + gate.len()..];
    let open = after.find('{').expect("screen gate brace");
    let arm = block_body(&after[open + 1..]);
    assert!(arm.contains("return lighting_sh_read(pos, n, false);"));
    assert!(
        !arm.contains("return lighting_sh_read(pos, n, true);"),
        "the screen arm must not claim"
    );
    assert!(arm.contains("return vec4(0.0);"));
    assert!(
        !arm.contains("lighting_sh_tilt"),
        "the screen arm does not take a separate owning-cell tilt"
    );
    let compose = shader("lighting_compose.comp");
    let sun = shader("lighting_sun_compose.comp");
    assert!(compose.contains("return lighting_screen_probe_indirect(p, pos, n);"));
    assert!(sun.contains("return lighting_screen_probe_indirect(p, pos, n);"));
    assert!(!compose.contains("lighting_probe_irradiance"));
    assert!(!sun.contains("lighting_probe_irradiance"));
    let probe = shader("lighting_screen_probe.glsl");
    let indirect = function_body(
        &probe,
        "vec3 lighting_screen_probe_indirect(uvec2 p, vec3 pos, vec3 n)",
    );
    assert!(indirect.contains("lighting_world_irradiance(pos, n)"));
    assert!(indirect.contains("return max(world.rgb, vec3(0.0));"));
    assert!(indirect.contains("lighting_coarse_world(pos, n)"));
    assert!(indirect.contains("if (!(coarse.w > LIGHTING_CACHE_HAS_W))"));
    assert!(indirect.contains("mix(coarse.rgb, max(world.rgb, vec3(0.0)), trust)"));
    assert!(
        !indirect.contains("lighting_sh_read"),
        "compose does not add a second neighbour gather"
    );
    assert!(!indirect.contains("lighting_probe_irradiance"));
    let coarse = function_body(&probe, "vec4 lighting_coarse_world(vec3 pos, vec3 n)");
    assert_eq!(
        coarse.matches("lighting_coarse_bilerp").count(),
        2,
        "two size steps, the parent and the coarser cell"
    );
    let slot = function_body(
        &probe,
        "void lighting_coarse_add(vec3 q, vec3 n, float size, float bil, inout vec3 sum, inout float wsum)",
    );
    assert!(slot.contains("lighting_cache_find_sized"));
    assert!(slot.contains("lighting_sc_stale"));
    assert!(slot.contains("lighting_sh_tilt"));
    assert!(slot.contains("LIGHTING_COARSE_PLANE"));
    assert!(!slot.contains("lighting_sh_read"));
    assert!(!slot.contains("lighting_probe_irradiance"));
    assert!(!coarse.contains("lighting_sh_read"));
    assert!(!coarse.contains("lighting_probe_irradiance"));
}

#[test]
fn a_default_fine_texel_shows_the_seed_then_its_own_mean() {
    // High leaves SVGF on (its off bit clear) and the sparse-off bit clear.
    // lighting_sc_sparse_on is false while SVGF is on, so the coarse-key mix
    // is not what a 12.5 cm texel shows. With the denoiser on, no fresh
    // field, and the stale world-cell read missed: no own sample shows the
    // seed, and one own sample shows that sample.
    let settings = LightingSettings::default();
    let flags = settings.experiments.sc_flags;
    assert_eq!(flags & SPARSE_OFF, 0, "the sparse-off switch stays clear");
    assert_eq!(flags & SEED_OLD, 0, "seed split stays on");
    assert_eq!(flags & LAYER2_OFF, 0, "layer 2 stays on");
    assert_eq!(
        flags & SVGF_OFF,
        0,
        "SVGF stays on, which turns the sparse mix off"
    );
    assert!(settings.denoiser.enabled);
    let dispatch = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("lighting_dispatch.rs"),
    )
    .expect("lighting_dispatch.rs");
    assert!(dispatch.contains("const CTRL_DENOISE: u32 = 64;"));

    let text = cache_shader();
    let sparse = function_body(&text, "bool lighting_sc_sparse_on()");
    assert!(sparse.contains("!lighting_sc_svgf()"));
    let shown = function_body(&text, "vec4 lighting_sc_shown(uint slot)");
    precedes(
        shown,
        "if ((gstats.ctrl_bits & 64u) == 0u && lighting_world_mode() == 3u)",
        "if (lighting_sc_svgf())",
    );
    precedes(
        shown,
        "if (own.w > 0.5) {\n                return vec4(own.rgb * ls, 1.0);",
        "if (sd.w > 0.0) {\n            return vec4(sd.rgb, 1.0);",
    );
    precedes(
        shown,
        "if (sd.w > 0.0) {\n            return vec4(sd.rgb, 1.0);",
        "if (lighting_sc_sparse_on() && lvl < 2u)",
    );

    let seed = [2.0_f32, 1.0, 0.4];
    let own = [8.0_f32, 0.25, 3.0];
    let no_field = default_fine_shown(false, [0.0; 3], own, 0.0, seed, SEED_CONFIDENCE);
    assert!(
        near(no_field, seed),
        "no own sample shows the seed, got {no_field:?}"
    );
    let with_own = default_fine_shown(false, [0.0; 3], own, 1.0, seed, SEED_CONFIDENCE);
    assert!(
        near(with_own, own),
        "one own sample shows that sample, got {with_own:?}"
    );
    let fade = (1.0 - 1.0 / HANDOVER).clamp(0.0, 1.0);
    let mixed = mix(seed, own, 1.0 / (1.0 + HANDOVER_K * SEED_CONFIDENCE * fade));
    assert!(
        (mixed[0] - with_own[0]).abs() > 1.0,
        "the sparse seed-only mix is a different colour from the own mean the default arm shows"
    );
    let field = [0.1_f32, 0.2, 0.3];
    assert!(near(
        default_fine_shown(true, field, own, 1.0, seed, SEED_CONFIDENCE),
        field
    ));
}

#[test]
fn the_sparse_handover_mixes_a_coarse_key_with_the_seed() {
    // The fade block in lighting_sc_shown. A default fine texel returns
    // before it (the test above). It runs when that early return does not,
    // and the coarse key joins only while sparse is on, which requires SVGF
    // off. Seed-only is the same block with no lit coarse key: the weight is
    // n / (n + 2 * 0.25 * fade). The own-ray mean is a different colour, so
    // the weight changes the result.
    let text = cache_shader();
    assert!(text.contains("const float LIGHTING_SC_HANDOVER = 16.0;"));
    assert!(text.contains("const float LIGHTING_SC_HANDOVER_K = 2.0;"));
    assert!(text.contains("lighting_sc_set_seed(slot, hc, 0.25,"));
    let shown = function_body(&text, "vec4 lighting_sc_shown(uint slot)");
    precedes(shown, "prior = sp.rgb * ck;", "prior += sd.rgb * sd.w;");
    precedes(shown, "pc = ck;", "pc += sd.w;");
    precedes(
        shown,
        "prior /= pc;",
        "float k = LIGHTING_SC_HANDOVER_K * min(pc, 1.0) * fade;",
    );
    precedes(
        shown,
        "float a = mine.w > 0.5 ? n / max(n + k, 1.0e-4) : 0.0;",
        "return vec4(mix(prior, mine.rgb, a), 1.0);",
    );
    let seed_store = function_body(
        &text,
        "void lighting_sc_set_seed(uint slot, vec3 rgb, float conf, float sunfrac)",
    );
    assert!(seed_store.contains("clamp(conf, 0.05, 1.0)"));

    let n = 1.0_f32;
    let fade = (1.0 - n / HANDOVER).clamp(0.0, 1.0);
    let seed = [2.0_f32, 1.0, 0.4];
    let own = [8.0_f32, 0.25, 3.0];
    let dark = [0.2_f32, 0.2, 0.2];

    // Seed-only. Level 2 skips the coarse block. Missed keys do too.
    assert!(coarse_key(2, [dark[0], dark[1], dark[2], 2.0], [0.0; 4]).is_none());
    assert!(coarse_key(0, [0.0; 4], [0.0; 4]).is_none());
    let (prior, pc) = prior_of(None, seed, SEED_CONFIDENCE).expect("the seed is a prior");
    assert!((pc - SEED_CONFIDENCE).abs() <= 1.0e-6);
    assert!(near(prior, seed));
    let seed_only = n / (n + HANDOVER_K * SEED_CONFIDENCE * fade);
    let shown = mix(prior, own, seed_only);
    assert!(
        (shown[0] - seed[0]).abs() > 1.0 && (shown[0] - own[0]).abs() > 1.0,
        "seed-only at one own set is between the seed and the own mean, got {shown:?}"
    );
    let at_zero_sets = mix(prior, own, 0.0);
    assert!(near(at_zero_sets, seed));

    // Default sparse shape when SVGF is off: a full-confidence 2-apart key
    // (weight 1 + confidence) plus the seed. pc exceeds 1, so the handover
    // caps it.
    let step2 = [dark[0], dark[1], dark[2], 2.0];
    let (key_rgb, ck) = coarse_key(0, step2, [9.0; 4]).expect("the 2-apart key");
    assert!(near(key_rgb, dark));
    assert!((ck - 1.0).abs() <= 1.0e-6);
    let (prior, pc) = prior_of(Some((key_rgb, ck)), seed, SEED_CONFIDENCE).expect("joined prior");
    let want_pc = 1.0 + SEED_CONFIDENCE;
    assert!((pc - want_pc).abs() <= 1.0e-6);
    let want_prior =
        std::array::from_fn(|ch| (dark[ch] * 1.0 + seed[ch] * SEED_CONFIDENCE) / want_pc);
    assert!(
        near(prior, want_prior),
        "prior {prior:?} want {want_prior:?}"
    );
    let capped = HANDOVER_K * 1.0 * fade;
    let uncapped = HANDOVER_K * want_pc * fade;
    assert!(uncapped - capped > 0.4, "min(pc, 1) must bind");
    let weight = n / (n + capped);
    let shown = mix(prior, own, weight);
    assert!((shown[0] - prior[0]).abs() > 0.5);
    assert!((shown[0] - own[0]).abs() > 0.5);
    assert!(
        (shown[0] - mix(seed, own, seed_only)[0]).abs() > 0.5,
        "the coarse key moves the prior off the seed-only mix"
    );

    // An edge on the 2-apart keys stores weight -2 - confidence, and the
    // decode keeps half of that confidence.
    let edge = [dark[0], dark[1], dark[2], -3.0];
    let (key_rgb, ck) = coarse_key(0, edge, [9.0; 4]).expect("edge key");
    assert!(near(key_rgb, dark));
    assert!((ck - 0.5).abs() <= 1.0e-6, "half of confidence 1, got {ck}");

    // A miss at step 2 falls through to step 4, at half the key confidence.
    let far = [4.0_f32, 0.0, 1.0, 2.0];
    let (key_rgb, ck) = coarse_key(0, [0.0; 4], far).expect("4-apart key");
    assert!(near(key_rgb, [4.0, 0.0, 1.0]));
    assert!(
        (ck - 0.5).abs() <= 1.0e-6,
        "4-apart confidence is halved, got {ck}"
    );

    // A key whose decoded confidence is below 1e-3 is kept at that floor.
    let (_rgb, ck) = coarse_key(0, [1.0, 0.0, 0.0, 1.0001], [0.0; 4]).expect("tiny key");
    assert!((ck - 1.0e-3).abs() <= 1.0e-8, "confidence floor, got {ck}");
}


/// `growing_wrc` in `lighting_compact.comp`: a cell under the cap stays on the
/// relight list when something has read it since its last update, or within
/// `LIGHTING_SC_READ_SLACK` frames.
fn growing_wrc(samples: f32, last: i32, updated: i32, frame: i32) -> bool {
    samples < 128.0 - 0.5 && (last - updated > 0 || frame - last <= 2)
}

/// Dark-fill blend (`lighting_cache.comp`): while `t` is under half the cap,
/// sample k's share is `1 - (t / (t + add))^3`. The first sample is the mean.
/// Later samples of a rising trace pull the mean up.
fn dark_fill(samples: &[f32]) -> f32 {
    let mut mean = samples[0];
    let mut t = 1.0_f32;
    for &sample in &samples[1..] {
        let add = 1.0_f32;
        let wa = if t < 64.0 {
            1.0 - (t / (t + add)).powi(3)
        } else {
            add / (t + add)
        };
        mean += (sample - mean) * wa.min(1.0);
        t = (t + add).min(128.0);
    }
    mean
}

#[test]
fn the_axis_seed_does_not_reach_a_slanted_ray_and_an_empty_read_does_not_claim() {
    // r14, depth-classified, both repeats: floor +0.0035 and box_slant +0.0015
    // of the +0.0051 gate rise (0.2532 to 0.2583). Mixed blocks are +0.0001.
    // The axis display seed cannot be that turned-face share. A ray hit calls
    // lighting_cache_irradiance_at, which loads the cell mean (the own-sample
    // slot) and never the seed. The seed lives in LIGHTING_SC_SPARSE_AT.
    // A flat face stores its own normal, so the tilt is the mean and matches
    // the per-axis read. What a slanted screen pixel does change is the claim
    // inside lighting_sh_gather, which lighting_sh_read returns: the lattice then skips every axis and
    // returns w 0, but the claim has already allocated the cell and stamped
    // it read. growing_wrc keeps a recently read cell on the relight list,
    // and dark fill makes a longer run of a rising trace brighter. Compose
    // still shows the probes, which read those cells.
    let text = cache_shader();
    let read = function_body(
        &text,
        "LightingShField lighting_sh_gather(vec3 p, vec3 n, bool claim)",
    );
    assert!(
        !read.contains("lighting_cache_find_sized(p, n, true, cs, p)"),
        "an empty neighbour read must not allocate the cell under p"
    );
    precedes(
        read,
        "if (facing <= 0.0)",
        "if (claim && LIGHTING_SC_WRITES)",
    );
    let at = function_body(
        &text,
        "vec3 lighting_cache_irradiance_at(vec3 pos, vec3 n, float t)",
    );
    assert!(
        !at.contains("lighting_sc_seed"),
        "a ray hit must not load the display seed"
    );
    assert!(
        !at.contains("LIGHTING_SC_SPARSE_AT"),
        "a ray hit must not load the seed store"
    );
    assert!(at.contains("!lighting_sc_slanted(n)"));
    assert!(at.contains("return lighting_cache_irradiance(pos, n);"));
    let seed_store = function_body(
        &text,
        "void lighting_sc_set_seed(uint slot, vec3 rgb, float conf, float sunfrac)",
    );
    assert!(seed_store.contains("LIGHTING_SC_SPARSE_AT"));
    assert!(
        seed_store.contains("cache.v[3u * slot + 2u].w < 0.5"),
        "the seed may look at the own-sample count"
    );
    assert!(
        !seed_store.contains("cache.v[3u * slot + 2u] ="),
        "the seed must not write the own-mean slot a ray reads"
    );
    let direct = shader("lighting_direct.comp");
    assert!(direct.contains("lighting_cache_irradiance_at(pos, n, hit_t)"));
    assert!(
        !direct.contains("lighting_sc_seed"),
        "probe and cache rays reach the cache through the ray-hit read"
    );
    let compact = shader("lighting_compact.comp");
    assert!(compact.contains(
        "bool growing_wrc = t < LIGHTING_WRC_CAP - 0.5 && (int(last - updated) > 0 || int(lighting_frame() - last) <= int(LIGHTING_SC_READ_SLACK));"
    ));
    let relight = shader("lighting_cache.comp");
    assert!(relight.contains("wa = 1.0 - pow(t / (t + add), 3.0);"));

    let yaw = 0.6_f32;
    let face = [yaw.cos(), 0.0, yaw.sin()];
    assert!(
        face.iter().all(|c| c.abs() < 1.0),
        "the yaw-0.6 face is not axis aligned, and the read still must not allocate"
    );
    let mean = [1.2_f32, 0.4, 0.3];
    let sh = genos_render::lighting_sh::ShL1::default();
    let hit = sh.tilt(mean, face, face);
    assert_eq!(hit, mean, "a flat face tilts to its own mean");
    let seed_owner = [0.2_f32, 0.2, 0.2];
    let seed_bilinear = [3.5_f32, 1.0, 0.0];
    assert_ne!(seed_owner, seed_bilinear);
    assert_ne!(hit, seed_owner);
    assert_ne!(hit, seed_bilinear);

    let frame = 300;
    assert!(
        growing_wrc(20.0, frame, 280, frame),
        "a stamp this frame keeps the cell on the relight list"
    );
    assert!(
        !growing_wrc(20.0, 280, 280, frame),
        "a cell not read since its relight, and quiet for more than the slack, leaves that list"
    );
    let rising = [0.2_f32, 0.4, 0.6, 0.8, 1.0];
    let early = dark_fill(&rising[..2]);
    let kept = dark_fill(&rising);
    assert!(
        kept - early > 0.3,
        "more relights of a rising trace are brighter: early {early} kept {kept}"
    );
}

/// Source-text model of one weight for the mean and the harmonics.
///
/// A lamp change sets that weight to 1 before either store, and a sun rescale
/// multiplies the held harmonics by the same gain as the held mean. Matching
/// this text is not the ADR 0039 picture gate.
#[test]
fn shader_text_model_settling_stores_mean_and_harmonics_with_one_weight() {
    let relight = shader("lighting_cache.comp");
    let settle = relight
        .find("if (lighting_lamp_settling())")
        .expect("lamp settle");
    let put = relight[settle..]
        .find("lighting_sh_put")
        .expect("harmonics store");
    let window = &relight[settle..settle + put];
    assert!(
        window.contains("wa = 1.0;"),
        "settling must replace the sample"
    );
    assert!(
        window.contains("lighting_tex_put"),
        "the mean is stored in that arm"
    );
    assert!(
        window.contains("o0 * sh_ls"),
        "held harmonics take the mean's sun gain"
    );
    let gain = relight.find("float sh_ls = 1.0;").expect("sun gain");
    let mean_scale = relight[gain..].find("old.rgb *= ls").expect("mean rescale");
    let sh_scale = relight[gain..]
        .find("o0 * sh_ls")
        .expect("harmonic rescale");
    assert!(
        mean_scale < sh_scale,
        "the mean is rescaled before the harmonics blend"
    );
}

#[test]
fn the_neighbour_seed_does_not_write_the_cell_or_keep_the_other_seeds() {
    let text = cache_shader();
    let detail = function_body(
        &text,
        "LightingShField lighting_sh_gather(vec3 p, vec3 n, bool claim)",
    );
    assert!(detail.contains("lighting_cache_find_sized(centre, an, false, cs, p)"));
    assert!(!detail.contains("lighting_sh_put"));
    assert!(!detail.contains("lighting_sc_set_seed"));
    let screen = function_body(
        &text,
        "vec4 lighting_world_irradiance_in(vec3 pos, vec3 n, vec3 wpos, bool obj)",
    );
    precedes(
        screen,
        "lighting_sh_read(q, n, false)",
        "lighting_sc_resample",
    );
    assert!(
        screen.contains("!detail_candidate"),
        "resample must not copy a cell the neighbour read rejected"
    );
    assert!(
        screen.contains("-abs(cache.v[LIGHTING_SC_SPARSE_AT + slot].z)"),
        "the detail mark is the sign of the confidence"
    );
    assert!(
        !screen.contains("LIGHTING_SC_SUN_AT + slot].z = 1.0"),
        "sun word z is the quarter scratch, not the detail mark"
    );
    let rejected = function_body(&text, "bool lighting_sh_detail_seed_rejected(uint slot)");
    assert!(rejected.contains("LIGHTING_SC_SPARSE_AT + slot].z < 0.0"));
    assert!(
        !rejected.contains("LIGHTING_SC_SUN_AT"),
        "the rejection must not read the quarter scratch"
    );
    assert!(screen.contains(
        "if (!detail_kept && v.w < 0.5 && mode == 3u && (lighting_sc_flags() & 33554432u) == 0u)"
    ));
    assert!(
        screen.contains("detail_candidate ? vec3(0.0) : lighting_cache_irradiance(q, n)"),
        "the hit fill stays for SH cells when texel detail is off"
    );
    assert!(
        !screen.contains("lighting_sh_on() ? vec3(0.0)"),
        "SH cells on must not zero the hit fill by itself"
    );
    assert!(
        !text.contains("LIGHTING_SH_AXIS_MIN"),
        "the unused axis-skip constant is gone"
    );
    assert!(screen.contains("if (detail_kept)"));
    assert!(
        !screen.contains("lighting_sc_parent_bilerp") && !screen.contains("lighting_sc_parent("),
        "the compose parent walk ran only with texel detail off"
    );
    let shown = function_body(&text, "vec4 lighting_sc_shown(uint slot)");
    assert!(shown.contains("lighting_sh_texel_detail_on()"));
    assert!(
        !shown.contains("lighting_sc_wrc_now"),
        "the stale-field per-axis read ran only with texel detail off"
    );
}
