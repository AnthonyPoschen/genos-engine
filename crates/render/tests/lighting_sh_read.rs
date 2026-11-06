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

/// Axes the lattice read keeps. The shader skips `abs(n[ax]) < 1.0`, so a
/// slanted normal (every component below 1) contributes nothing. This is
/// that gate, not an execution of `lighting_sh_read`.
fn lattice_axis_weight(n: [f32; 3]) -> f32 {
    let mut ws = 0.0;
    for component in n {
        if component.abs() < 1.0 {
            continue;
        }
        ws += component * component;
    }
    ws
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
fn a_slanted_screen_read_is_empty_and_compose_keeps_the_probes() {
    // The axis-aligned change is the fallback inside lighting_cache_irradiance.
    // This screen return is a different function. The lattice gate rejects
    // every axis whose component is below 1, so a slanted normal gets w 0,
    // and compose's mix by that w keeps the probes. Nothing here claims the
    // irradiance fallback changed this path.
    let text = cache_shader();
    let read = function_body(&text, "vec4 lighting_sh_read(vec3 p, vec3 n, bool claim)");
    assert!(
        read.contains("if (abs(n[ax]) < 1.0)"),
        "the lattice read must skip an axis whose component is below 1"
    );
    assert!(
        read.contains("return ws > 0.0 ? vec4(sum / ws, 1.0) : vec4(0.0);"),
        "no accepted axis is an empty read, w 0"
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
    assert!(arm.contains("return lighting_sh_read(pos, n, true);"));
    assert!(arm.contains("return vec4(0.0);"));
    assert!(
        !arm.contains("lighting_sh_tilt"),
        "the screen arm does not take the owning-cell tilt"
    );
    let compose = shader("lighting_compose.comp");
    assert!(compose.contains("return mix(probes, world.rgb, clamp(world.w, 0.0, 1.0));"));

    let slanted = [0.0_f32, 0.8, 0.6];
    assert!(
        lattice_axis_weight(slanted).abs() <= f32::EPSILON,
        "a slanted normal accepts no lattice axis"
    );
    let axis = [0.0_f32, 1.0, 0.0];
    assert!((lattice_axis_weight(axis) - 1.0).abs() <= 1.0e-6);
    let probes = [0.3_f32, 0.4, 0.5];
    let empty = [9.0_f32, 9.0, 9.0];
    let shown = mix(probes, empty, 0.0_f32.clamp(0.0, 1.0));
    assert!(near(shown, probes), "w 0 shows the probes, got {shown:?}");
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
