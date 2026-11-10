//! Lavapipe dispatch of production place, trace, gather, and filter for the
//! ADR 0039 stage 5 slice.
//!
//! The command buffer plants the screen and the cells, then runs
//! `lighting_place.comp`, `lighting_trace.comp`, `lighting_gather.comp`, and
//! `lighting_filter.comp`. The live set is the probe record those passes share.
//! Software Vulkan only: the numbers are probes and rays those passes touched,
//! not a frame time and not an accuracy gate. This dispatch does not present a
//! swapchain image. It does not write diag[8], diag[38], or diag[39]. It does
//! not include parent follow-up `dfe8a29e`.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
#![expect(clippy::panic, reason = "a test aborts when a fixture fails")]

use genos_render::{GpuKind, LightingPreset, LightingSettings, Renderer, lighting_change_head};
use genos_window::Window;

const ROWS: usize = 22;
const STRIDE: usize = 16;
const HEAD: usize = 8;
const OUT_FLOATS: usize = 416;
const RAYS: u32 = 4;
const RETIRE_BIT: u32 = 1 << 19;
const DETAIL_BIT: u32 = 1 << 17;

const PARTIAL: usize = 0;
const FULL: usize = 1;
const OFF: usize = 2;
const DETAIL_OFF: usize = 3;
const LOW: usize = 4;
const STALE: usize = 5;
const EMPTY: usize = 6;
const OWN: usize = 7;
const MISSING: usize = 8;
const SPLIT: usize = 9;
const SLANT: usize = 10;
const SLANT_EMPTY: usize = 11;
const BACK: usize = 12;
const PLANE: usize = 13;
const REAPPEAR: usize = 14;
const MOVER: usize = 15;
const MOVER_MISS: usize = 16;
const SKY: usize = 17;
const KEPT_DEAD: usize = 18;
const KEPT_OFF: usize = 19;
const KEPT_HOLE: usize = 20;
const KEPT_BACK: usize = 21;
/// Kept sentinel, fill sentinel, and older stratum, both probes.
const KEPT_STILL: u32 = 0b11_1111;
/// Probe 0 still held, probe 1 opened, both strata still stored.
const KEPT_REVIVED: u32 = 0b11_0101;
/// Both kept and fill slots opened, both strata still stored.
const KEPT_OPEN: u32 = 0b11_0000;

struct Row {
    zero: u32,
    live: u32,
    opened: u32,
    gather: u32,
    filter: u32,
    left_w: u32,
    right_w: u32,
    left_held: u32,
    right_held: u32,
    cell_same: u32,
    planted: u32,
    covers: u32,
    hold: u32,
    kept: u32,
    first: u32,
}

fn shader(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn function_body<'a>(text: &'a str, sig: &str) -> &'a str {
    let at = text.find(sig).unwrap_or_else(|| panic!("missing {sig}"));
    let rest = &text[at + sig.len()..];
    let brace = rest
        .find('{')
        .unwrap_or_else(|| panic!("missing body of {sig}"));
    let mut depth = 0i32;
    for (i, c) in rest[brace..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[brace + 1..brace + i];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed {sig}");
}

fn word(words: &[f32], i: usize) -> u32 {
    words[i].to_bits()
}

fn row(words: &[f32], id: usize) -> Row {
    let at = HEAD + id * STRIDE;
    Row {
        zero: word(words, at),
        live: word(words, at + 1),
        opened: word(words, at + 2),
        gather: word(words, at + 3),
        filter: word(words, at + 4),
        left_w: word(words, at + 5),
        right_w: word(words, at + 6),
        left_held: word(words, at + 7),
        right_held: word(words, at + 8),
        cell_same: word(words, at + 9),
        planted: word(words, at + 10),
        covers: word(words, at + 11),
        hold: word(words, at + 12),
        kept: word(words, at + 14),
        first: word(words, at + 15),
    }
}

fn show(id: usize, row: &Row) -> String {
    format!(
        "row {id} zero {} live {} opened {} gather {} filter {} wl {} wr {} sl {} sr {} cell {} planted {} covers {} hold {} kept {} first {}",
        row.zero,
        row.live,
        row.opened,
        row.gather,
        row.filter,
        row.left_w,
        row.right_w,
        row.left_held,
        row.right_held,
        row.cell_same,
        row.planted,
        row.covers,
        row.hold,
        row.kept,
        row.first
    )
}

fn both_on() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    settings.surface_cache.sh_texel_detail = true;
    settings.surface_cache.sh_probe_retire = true;
    settings.sanitize();
    settings
}

fn open() -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(64, 64)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

#[test]
fn retire_bit_is_packed_only_from_the_setting() {
    let on = lighting_change_head(&both_on());
    let mut off_settings = both_on();
    off_settings.surface_cache.sh_probe_retire = false;
    let off = lighting_change_head(&off_settings);
    let mut detail_off = both_on();
    detail_off.surface_cache.sh_texel_detail = false;
    let detail_off = lighting_change_head(&detail_off);
    let mut low = LightingSettings::preset(LightingPreset::Low);
    low.surface_cache.sh_cells = true;
    low.surface_cache.sh_texel_detail = true;
    low.surface_cache.sh_probe_retire = true;
    low.sanitize();
    let low = lighting_change_head(&low);
    let mut plain = LightingSettings::preset(LightingPreset::High);
    plain.sanitize();
    let plain = lighting_change_head(&plain);
    assert_eq!(on.head_hi & RETIRE_BIT, RETIRE_BIT);
    assert_eq!(on.head_hi & DETAIL_BIT, DETAIL_BIT);
    assert_eq!(off.head_hi & RETIRE_BIT, 0, "the bit follows the setting");
    assert_eq!(off.head_hi & DETAIL_BIT, DETAIL_BIT);
    assert_eq!(
        on.head_hi & 0x1ff,
        off.head_hi & 0x1ff,
        "bit 19 stays out of sc_flags"
    );
    assert_eq!(
        detail_off.head_hi & RETIRE_BIT,
        RETIRE_BIT,
        "detail off does not clear the retire bit; the shader ignores it"
    );
    assert_eq!(detail_off.head_hi & DETAIL_BIT, 0);
    assert_eq!(low.head_hi & RETIRE_BIT, RETIRE_BIT);
    assert_eq!(low.mode_cap / 1000, 0, "Low stays probe GI");
    assert_eq!(on.mode_cap / 1000, 3);
    assert_eq!(plain.head_hi & RETIRE_BIT, 0, "the preset default is off");
}

#[test]
fn the_production_passes_return_before_the_retired_work() {
    expect_pass_order();
    expect_kept_membership();
}

fn expect_pass_order() {
    let cache = shader("shaders/lighting_cache.glsl");
    assert!(
        !cache.contains("LIGHTING_DIAG_PROBE_LIVE"),
        "this slice does not own a diagnostic word"
    );
    let skip = function_body(&cache, "bool lighting_probe_retire_skip(uint probe)");
    assert!(skip.contains("!lighting_sh_probe_retire_on()"));
    assert!(skip.contains("return false"));
    assert!(!skip.contains("atomicAdd"));
    assert!(!skip.contains("lighting_compact_hold_texel_rays"));
    assert!(!skip.contains("diag["));
    let foot = function_body(
        &cache,
        "bool lighting_probe_footprint_uncovered(uint probe)",
    );
    assert!(foot.contains("lighting_probe_pixel_samples"));
    assert!(foot.contains("lighting_sh_texel_detail_covers"));
    assert!(!foot.contains("lighting_compact_hold_texel_rays"));
    assert!(!foot.contains("atomicAdd"));
    let covers = function_body(
        &cache,
        "bool lighting_sh_texel_detail_covers(vec3 pos, vec3 n, vec3 wpos, bool obj)",
    );
    assert!(!covers.contains("lighting_compact_hold_texel_rays"));
    let trace_skip = function_body(&cache, "bool lighting_probe_trace_skip(uint ray)");
    assert!(trace_skip.contains("ray >= lighting_probe_rays()"));
    assert!(!trace_skip.contains("atomicAdd"));
    assert!(!trace_skip.contains("diag["));

    let place = shader("shaders/lighting_place.comp");
    let skip_at = place
        .find("if (lighting_probe_retire_skip(i))")
        .expect("place calls the retire gate");
    let search = place
        .find("for (uint k = 0u; k < 5u; k++)")
        .expect("place still has the 5-point search");
    assert!(
        skip_at < search,
        "a retired probe returns before the search"
    );
    assert!(place[skip_at..search].contains("return;"));
    assert!(!place.contains("lighting_probe_retire_note"));
    assert!(!place.contains("diag["));

    let trace = shader("shaders/lighting_trace.comp");
    let trace_at = trace
        .find("if (lighting_probe_trace_skip(ray))")
        .expect("trace calls the skip");
    let hit_at = trace
        .find("at = lighting_hit_at(ray)")
        .expect("trace still writes a hit slot");
    let cast = trace.find("trace_ray(").expect("trace still casts");
    assert!(
        trace_at < hit_at,
        "a retired ray returns before the hit slot"
    );
    assert!(hit_at < cast, "the hit slot is before the cast");
    assert!(trace[trace_at..hit_at].contains("return;"));
}

fn expect_kept_membership() {
    let trace = shader("shaders/lighting_trace.comp");
    let kept_skip = trace
        .find("lighting_probe_kept_skip(h)")
        .expect("kept hits check the probe record");
    let crossed = trace
        .find("lighting_kept_crossed(h, origin, dir)")
        .expect("kept hits still use the change test");
    assert!(
        kept_skip < crossed,
        "a retired probe returns before the kept re-trace"
    );
    let cache = shader("shaders/lighting_cache.glsl");
    let skip_body = function_body(&cache, "bool lighting_probe_kept_skip(");
    assert!(skip_body.contains("lighting_sh_probe_retire_on()"));
    assert!(skip_body.contains("lighting_probe_record_live"));
    assert!(!skip_body.contains("diag["));
    let clear_body = function_body(&cache, "void lighting_probe_retire_clear(");
    assert!(
        !clear_body.contains("LIGHTING_STRATA"),
        "place still clears only the current stratum"
    );
    let common = shader("shaders/lighting_common.glsl");
    let fill = function_body(&common, "bool lighting_fill_ray(");
    assert!(
        fill.contains("pp.w < 0.5"),
        "fills already require a live probe record"
    );

    let gather = shader("shaders/lighting_gather.comp");
    let bin = function_body(&gather, "void gather_bin()");
    let bin_ret = bin
        .find("if (lighting_sh_probe_retire_on() && !live)")
        .expect("gather returns for a retired probe");
    let strata = bin
        .find("for (uint s = 0u; s < LIGHTING_STRATA")
        .expect("gather still walks strata");
    assert!(
        bin_ret < strata,
        "a retired bin returns before the stratum loop"
    );
    assert!(bin[bin_ret..strata].contains("return;"));

    let filter = shader("shaders/lighting_filter.comp");
    let dead = filter
        .find("if (!lighting_probe_record_live(probe))")
        .expect("filter keeps the empty-probe path");
    let blend = filter
        .find("for (uint k = 0u; k < 9u; k++)")
        .expect("filter still has the 3x3");
    assert!(dead < blend, "a retired probe returns before the 3x3");
    assert!(filter[dead..blend].contains("return;"));

    let prod = shader("shaders/probe_retire_prod.comp");
    assert!(
        !prod.contains("trace_ray("),
        "the plant does not replace the trace pass"
    );
    assert!(!prod.contains("gstats.diag"));
    assert!(!prod.contains("atomicAdd"));
    assert!(prod.contains("lighting_compact_hold_texel_rays"));

    let harness = shader("src/gpu/probe_retire.rs");
    assert!(harness.contains("LIGHTING_PLACE_SPV"));
    assert!(harness.contains("LIGHTING_TRACE_SPV"));
    assert!(harness.contains("LIGHTING_GATHER_SPV"));
    assert!(harness.contains("LIGHTING_FILTER_SPV"));
    let passes = function_body(&harness, "fn probe_retire_passes(");
    assert!(passes.contains("place, 1, 1, 1"));
    assert!(passes.contains("trace, groups, 1, 1"));
    assert!(passes.contains("gather, 2, 1, 1"));
    assert!(passes.contains("filter, 1, 1, 1"));
    let groups = function_body(&harness, "fn trace_groups(");
    assert!(groups.contains('3'), "kept and fill use three groups");
    assert!(groups.contains('1'), "probe rays stay one group");
}

#[test]
fn covered_footprints_drop_place_trace_gather_and_filter_work() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let words = renderer.probe_retire().expect("production probe retire");
    assert_eq!(words.len(), OUT_FLOATS);
    let on = lighting_change_head(&both_on());
    let mut off_settings = both_on();
    off_settings.surface_cache.sh_probe_retire = false;
    let off = lighting_change_head(&off_settings);
    let mut detail_settings = both_on();
    detail_settings.surface_cache.sh_texel_detail = false;
    let detail = lighting_change_head(&detail_settings);
    let mut low_settings = LightingSettings::preset(LightingPreset::Low);
    low_settings.surface_cache.sh_cells = true;
    low_settings.surface_cache.sh_texel_detail = true;
    low_settings.surface_cache.sh_probe_retire = true;
    low_settings.sanitize();
    let low = lighting_change_head(&low_settings);
    assert_eq!(word(&words, 4), on.head_hi, "variant 0 is the packed head");
    assert_eq!(word(&words, 5), off.head_hi);
    assert_eq!(word(&words, 6), detail.head_hi);
    assert_eq!(word(&words, 7), low.head_hi);

    let rows: Vec<Row> = (0..ROWS).map(|id| row(&words, id)).collect();
    expect_rows(&rows);
    expect_lifecycle(&rows);
}

fn expect_rows(rows: &[Row]) {
    for (id, row) in rows.iter().enumerate() {
        let text = show(id, row);
        assert_eq!(row.planted, 1, "{text}");
        assert_eq!(row.cell_same, 1, "{text}");
        assert_eq!(row.zero, 0, "{text}");
        assert_eq!(row.opened, row.live * RAYS, "{text}");
        assert_eq!(row.gather, row.live, "{text}");
        assert_eq!(row.filter, row.live, "{text}");
    }

    let retired = [FULL, SPLIT, SLANT, MOVER, SKY];
    for id in retired {
        let row = &rows[id];
        let text = show(id, row);
        assert_eq!(row.live, 0, "{text}");
        assert_eq!(row.left_w, 0, "{text}");
        assert_eq!(row.right_w, 0, "{text}");
        assert_eq!(row.left_held, 1, "{text}");
        assert_eq!(row.right_held, 1, "{text}");
    }
    let kept = [
        OFF,
        DETAIL_OFF,
        LOW,
        STALE,
        EMPTY,
        OWN,
        MISSING,
        SLANT_EMPTY,
        BACK,
        PLANE,
        MOVER_MISS,
    ];
    for id in kept {
        let row = &rows[id];
        let text = show(id, row);
        assert_eq!(row.live, 2, "{text}");
        assert_eq!(row.left_w, 1, "{text}");
        assert_eq!(row.right_w, 1, "{text}");
        assert_eq!(row.left_held, 0, "{text}");
        assert_eq!(row.right_held, 0, "{text}");
    }
    let partial = &rows[PARTIAL];
    let partial_text = show(PARTIAL, partial);
    assert_eq!(partial.live, 1, "{partial_text}");
    assert_eq!(partial.left_w, 0, "{partial_text}");
    assert_eq!(partial.right_w, 1, "{partial_text}");
    assert_eq!(partial.left_held, 1, "{partial_text}");
    assert_eq!(partial.right_held, 0, "{partial_text}");
    assert_eq!(partial.covers, 1, "{partial_text}");
    let again = &rows[REAPPEAR];
    let again_text = show(REAPPEAR, again);
    assert_eq!(again.first, 0, "{again_text}");
    assert_eq!(again.live, 1, "{again_text}");
    assert_eq!(again.left_held, 1, "{again_text}");
    assert_eq!(again.right_held, 0, "{again_text}");
    assert_eq!(again.covers, 1, "{again_text}");

    for id in [
        PARTIAL,
        FULL,
        OFF,
        SPLIT,
        SLANT,
        REAPPEAR,
        MOVER,
        KEPT_DEAD,
        KEPT_OFF,
        KEPT_HOLE,
        KEPT_BACK,
    ] {
        assert_eq!(rows[id].covers, 1, "{}", show(id, &rows[id]));
    }
    for id in [
        DETAIL_OFF,
        LOW,
        STALE,
        EMPTY,
        OWN,
        MISSING,
        SLANT_EMPTY,
        BACK,
        PLANE,
        MOVER_MISS,
        SKY,
    ] {
        assert_eq!(rows[id].covers, 0, "{}", show(id, &rows[id]));
    }
    assert_eq!(rows[SPLIT].hold, 1, "{}", show(SPLIT, &rows[SPLIT]));
    for (id, row) in rows.iter().enumerate() {
        if id == SPLIT {
            continue;
        }
        assert_eq!(row.hold, 0, "{}", show(id, row));
    }
}

fn expect_lifecycle(rows: &[Row]) {
    let dead = &rows[KEPT_DEAD];
    let dead_text = show(KEPT_DEAD, dead);
    assert_eq!(dead.live, 0, "{dead_text}");
    assert_eq!(dead.opened, 0, "{dead_text}");
    assert_eq!(dead.left_held, 1, "{dead_text}");
    assert_eq!(dead.right_held, 1, "{dead_text}");
    assert_eq!(dead.kept, KEPT_STILL, "{dead_text}");
    assert_eq!(dead.first, 7, "{dead_text}");

    let off = &rows[KEPT_OFF];
    let off_text = show(KEPT_OFF, off);
    assert_eq!(off.live, 2, "{off_text}");
    assert_eq!(off.opened, 8, "{off_text}");
    assert_eq!(off.kept, KEPT_OPEN, "{off_text}");
    assert_eq!(off.first & 4, 0, "{off_text}");

    let hole = &rows[KEPT_HOLE];
    let hole_text = show(KEPT_HOLE, hole);
    assert_eq!(hole.live, 1, "{hole_text}");
    assert_eq!(hole.left_w, 0, "{hole_text}");
    assert_eq!(hole.right_w, 1, "{hole_text}");
    assert_eq!(hole.left_held, 1, "{hole_text}");
    assert_eq!(hole.right_held, 0, "{hole_text}");
    assert_eq!(hole.kept, KEPT_REVIVED, "{hole_text}");

    let back = &rows[KEPT_BACK];
    let back_text = show(KEPT_BACK, back);
    assert_eq!(back.first & 0xff, 0, "{back_text}");
    assert_eq!(back.first >> 8, KEPT_STILL, "{back_text}");
    assert_eq!(back.live, 1, "{back_text}");
    assert_eq!(back.left_held, 1, "{back_text}");
    assert_eq!(back.right_held, 0, "{back_text}");
    assert_eq!(back.kept, KEPT_REVIVED, "{back_text}");
}
