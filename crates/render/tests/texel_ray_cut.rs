//! Lavapipe dispatch of the ray-cut hold and of production `lighting_compact.comp`.
//!
//! The hold is the function compact calls after the 25 cm lists. A second
//! pipeline is that compact shader, on the same buffers, flag off then on.
//! The change-block head is `lighting_change_head`. Software Vulkan only.
//! List entries are not a frame time and not an accuracy gate. This dispatch
//! does not present a swapchain image. It does not run the stage 3 read-only
//! compose, and it does not claim that specialization passed.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
#![expect(clippy::panic, reason = "a test aborts when a fixture fails")]

use genos_render::{lighting_change_head, GpuKind, LightingPreset, LightingSettings, Renderer};
use genos_window::Window;

const ROWS: usize = 26;
const STRIDE: usize = 8;
const HEAD: usize = 8;
const TAIL: usize = 8;
const AGREE: usize = HEAD + ROWS * STRIDE + TAIL;
const LAMP_ROWS: usize = 3;
const LAMP_AT: usize = AGREE;
const CMP_AT: usize = AGREE + HEAD + LAMP_ROWS * STRIDE;
const CMP_N: usize = 7;
const CMP_LISTS: usize = 6;
const CMP_BASE: usize = 16;
const CMP_BLOCK: usize = CMP_N * CMP_LISTS + 4;
const DETAIL_BIT: u32 = 1 << 17;
const RAY_CUT_BIT: u32 = 1 << 18;
const RAYS_PER_ENTRY: u32 = 16;

const STABLE: usize = 0;
const SEED_DIFF: usize = 1;
const OWN_DISAGREE: usize = 2;
const OWN_AGREE: usize = 3;
const FUTURE: usize = 4;
const BLACK: usize = 5;
const ABSENT: usize = 6;
const EMPTY: usize = 7;
const MIX_OK: usize = 8;
const MIX_BAD: usize = 9;
const CONTACT: usize = 10;
const BACK: usize = 11;
const PLANE: usize = 12;
const STALE_CELL: usize = 13;
const STALE_TEXEL: usize = 14;
const LAMP: usize = 15;
const MOVE: usize = 16;
const DETAIL_OFF: usize = 17;
const CUT_OFF: usize = 18;
const LOW: usize = 19;
const COARSE: usize = 20;
const MOVER: usize = 21;
const HUE: usize = 22;
const CHROMA: usize = 23;
const LOBE_FLAT: usize = 24;
const LOBE_SLANT: usize = 25;
const CMP_FULL: usize = 0;
const CMP_QUARTER: usize = 1;
const CMP_IDLE: usize = 2;
const CMP_DISAGREE: usize = 3;
const CMP_STALE: usize = 4;
const CMP_HUE: usize = 5;
const CMP_CELL: usize = 6;

struct Row {
    held: u32,
    rays: u32,
    looked: u32,
    accepted: u32,
    cell_same: u32,
    seed_same: u32,
    mean_same: u32,
    planted: u32,
}

fn shader(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn function_body<'a>(text: &'a str, sig: &str) -> &'a str {
    let at = text.find(sig).unwrap_or_else(|| panic!("missing {sig}"));
    let rest = &text[at + sig.len()..];
    let brace = rest.find('{').unwrap_or_else(|| panic!("missing body of {sig}"));
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
        held: word(words, at),
        rays: word(words, at + 1),
        looked: word(words, at + 2),
        accepted: word(words, at + 3),
        cell_same: word(words, at + 4),
        seed_same: word(words, at + 5),
        mean_same: word(words, at + 6),
        planted: word(words, at + 7),
    }
}

fn luminance_would_hold(a: [f32; 3], b: [f32; 3]) -> bool {
    let y = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let hi = y(a).max(y(b));
    let lo = y(a).min(y(b));
    !(hi > 1.0e-3 && hi > 1.3 * lo.max(0.0))
}

fn lists(compact: &[f32], pass: usize, id: usize) -> [u32; 6] {
    let at = CMP_BASE + pass * CMP_BLOCK + id * CMP_LISTS;
    let mut out = [0; 6];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = word(compact, at + i);
    }
    out
}

fn compact_tail(compact: &[f32], pass: usize) -> (u32, u32) {
    let at = CMP_BASE + pass * CMP_BLOCK + CMP_N * CMP_LISTS;
    (word(compact, at), word(compact, at + 1))
}

fn both_on() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    settings.surface_cache.sh_texel_detail = true;
    settings.surface_cache.sh_texel_ray_cut = true;
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
fn ray_cut_bit_is_packed_only_from_the_setting() {
    let on = lighting_change_head(&both_on());
    let mut cut_off = both_on();
    cut_off.surface_cache.sh_texel_ray_cut = false;
    let cut_off = lighting_change_head(&cut_off);
    let mut detail_off = both_on();
    detail_off.surface_cache.sh_texel_detail = false;
    let detail_off = lighting_change_head(&detail_off);
    let mut low = LightingSettings::preset(LightingPreset::Low);
    low.surface_cache.sh_cells = true;
    low.surface_cache.sh_texel_detail = true;
    low.surface_cache.sh_texel_ray_cut = true;
    low.sanitize();
    let low = lighting_change_head(&low);
    let mut plain = LightingSettings::preset(LightingPreset::High);
    plain.sanitize();
    let plain = lighting_change_head(&plain);
    assert_eq!(on.head_hi & RAY_CUT_BIT, RAY_CUT_BIT);
    assert_eq!(on.head_hi & DETAIL_BIT, DETAIL_BIT);
    assert_eq!(cut_off.head_hi & RAY_CUT_BIT, 0, "the bit follows the setting");
    assert_eq!(cut_off.head_hi & DETAIL_BIT, DETAIL_BIT);
    assert_eq!(
        detail_off.head_hi & RAY_CUT_BIT,
        RAY_CUT_BIT,
        "detail off does not clear the ray-cut bit; the shader ignores it"
    );
    assert_eq!(detail_off.head_hi & DETAIL_BIT, 0);
    assert_eq!(low.head_hi & RAY_CUT_BIT, RAY_CUT_BIT);
    assert_eq!(low.mode_cap / 1000, 0, "Low stays probe GI");
    assert_eq!(on.mode_cap / 1000, 3);
    assert_eq!(plain.head_hi & RAY_CUT_BIT, 0, "the preset default is off");
    assert_eq!(plain.head_hi & DETAIL_BIT, 0);
}

#[test]
fn the_hold_ignores_the_seed_and_compact_returns_before_the_lists() {
    let cache = shader("lighting_cache.glsl");
    let hold = function_body(&cache, "bool lighting_compact_hold_texel_rays(uint slot)");
    assert!(
        !hold.contains("LIGHTING_SC_SPARSE_AT"),
        "the hold must not read the stored seed"
    );
    assert!(!hold.contains("lighting_sc_seed"));
    assert!(!hold.contains("lighting_sc_set_seed"));
    assert!(!hold.contains("lighting_sh_put"));
    assert!(hold.contains(
        "lighting_sh_gather(cache.v[3u * slot].xyz, nb.xyz, false)"
    ));
    assert!(hold.contains("nb.w > LIGHTING_WRC_CELL_SIZE"));
    assert!(hold.contains("lighting_sh_lum_disagrees"));
    assert!(hold.contains("lighting_sh_channels_disagree"));
    assert!(hold.contains("gstats.diag[LIGHTING_DIAG_TEXEL_HELD]"));
    assert!(
        !hold.contains("LIGHTING_DIAG_TEXEL_CUT"),
        "there is no removed-ray counter"
    );
    assert!(
        !hold.contains("gstats.waiting"),
        "waiting is the rate scheduler, not this gate"
    );
    assert!(!hold.contains("lighting_sc_entries"));
    let sparse = function_body(&cache, "vec4 lighting_sc_sparse(uint slot, float step)");
    assert!(
        sparse.contains("lighting_sh_lum_disagrees(hi, lo)"),
        "the corner split is the existing sparse edge test"
    );
    let compact = shader("lighting_compact.comp");
    let call = compact
        .find("if (lighting_compact_hold_texel_rays(slot))")
        .expect("compact calls the hold");
    let wrc = compact
        .find("if (!(growing_wrc && lighting_sc_dark_fill()))")
        .expect("world-cell list");
    let lists = compact
        .find("bool sparse_c")
        .expect("fine lists follow the hold");
    assert!(wrc < call, "cells are listed before a texel can be held");
    assert!(call < lists, "a hold returns before the fine ray lists");
    let window = &compact[call..lists];
    assert!(window.contains("lighting_sc_expired(last, shown)"));
    assert!(window.contains("return;"));
    assert!(
        compact.contains("One invocation owns this slot"),
        "the free is this slot in this round"
    );
    let direct = shader("lighting_direct.comp");
    assert!(direct.contains("LIGHTING_DIAG_LAMP_PX"));
    assert!(direct.contains("LIGHTING_DIAG_LAMP_HIT"));
    assert!(!direct.contains("gstats.diag[38u]"));
    assert!(!direct.contains("gstats.diag[39u]"));
    let gpu = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/gpu/texel_ray_cut.rs"),
    )
    .expect("fixture source");
    assert!(
        gpu.contains("LIGHTING_COMPACT_SPV"),
        "the list proof dispatches production compact"
    );
}

#[test]
fn agreed_texel_rays_are_cut_and_disagreement_keeps_them() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let words = renderer
        .texel_ray_cut()
        .expect("production texel ray cut");
    assert_eq!(words.len(), AGREE + HEAD + LAMP_ROWS * STRIDE + CMP_BASE + 2 * CMP_BLOCK);
    let on = lighting_change_head(&both_on());
    let mut cut_off_settings = both_on();
    cut_off_settings.surface_cache.sh_texel_ray_cut = false;
    let cut_off = lighting_change_head(&cut_off_settings);
    let mut detail_off_settings = both_on();
    detail_off_settings.surface_cache.sh_texel_detail = false;
    let detail_off = lighting_change_head(&detail_off_settings);
    let mut low_settings = LightingSettings::preset(LightingPreset::Low);
    low_settings.surface_cache.sh_cells = true;
    low_settings.surface_cache.sh_texel_detail = true;
    low_settings.surface_cache.sh_texel_ray_cut = true;
    low_settings.sanitize();
    let low = lighting_change_head(&low_settings);
    assert_eq!(word(&words, 4), on.head_hi, "variant 0 is the packed head");
    assert_eq!(word(&words, 5), cut_off.head_hi);
    assert_eq!(word(&words, 6), detail_off.head_hi);
    assert_eq!(word(&words, 7), low.head_hi);
    let rows: Vec<Row> = (0..ROWS).map(|id| row(&words, id)).collect();
    check_hold_rows(&words, &rows);
    check_hue_and_lamps(&words, &rows);
    check_compact_lists(&words);
}

fn check_hold_rows(words: &[f32], rows: &[Row]) {
    for (id, row) in rows.iter().enumerate() {
        assert_eq!(row.planted, 1, "row {id} did not plant");
        assert_eq!(row.cell_same, 1, "row {id} wrote a cell");
        assert_eq!(row.seed_same, 1, "row {id} wrote the seed");
        assert_eq!(row.mean_same, 1, "row {id} wrote the texel mean");
    }
    let held_rows = [STABLE, SEED_DIFF, OWN_AGREE, BLACK, MIX_OK, LOBE_FLAT];
    for id in held_rows {
        let row = &rows[id];
        assert_eq!(row.held, 1, "row {id} should hold");
        assert_eq!(row.rays, 0, "row {id} is a visit, not a ray count");
        assert_eq!(row.looked, 8, "row {id} looked {}", row.looked);
        assert_eq!(row.accepted, 8, "row {id} accepted {}", row.accepted);
    }
    // The seed matches the cell and the own samples do not. Holding here would
    // be the cut comparing the cache with its own seed.
    let disagree = &rows[OWN_DISAGREE];
    assert_eq!(disagree.held, 0, "own samples past the ratio keep their rays");
    assert_eq!(disagree.rays, 0);
    assert_eq!(disagree.looked, 8);
    assert_eq!(disagree.accepted, 8);
    let future = &rows[FUTURE];
    assert_eq!(future.held, 0, "a later cell colour is not frozen on old samples");
    assert_eq!(future.rays, 0);
    assert_eq!(future.accepted, 8);
    for id in [MIX_BAD, CONTACT] {
        let row = &rows[id];
        assert_eq!(row.held, 0, "row {id} corners disagree");
        assert_eq!(row.rays, 0);
        assert_eq!(row.looked, 8);
        assert_eq!(row.accepted, 8, "row {id} accepted {}", row.accepted);
    }
    for id in [ABSENT, EMPTY, BACK, PLANE, STALE_CELL] {
        let row = &rows[id];
        assert_eq!(row.held, 0, "row {id} field is incomplete");
        assert_eq!(row.rays, 0);
        assert_eq!(row.looked, 8, "row {id} looked {}", row.looked);
        assert!(
            row.accepted < row.looked,
            "row {id} accepted {} of {}",
            row.accepted,
            row.looked
        );
    }
    for id in [STALE_TEXEL, MOVE, DETAIL_OFF, CUT_OFF, LOW, COARSE, MOVER, HUE, CHROMA, LOBE_SLANT]
    {
        let row = &rows[id];
        assert_eq!(row.held, 0, "row {id} must keep its rays");
        assert_eq!(row.rays, 0, "row {id} is not a ray count");
        assert_eq!(row.looked, 8, "row {id} looked {}", row.looked);
        assert_eq!(row.accepted, 8, "row {id} accepted {}", row.accepted);
    }
    let lamp = &rows[LAMP];
    assert_eq!(
        lamp.held, 0,
        "quiet bit 10 with a fresh ring is global settling, not a reaching lamp"
    );
    assert_eq!(lamp.looked, 8);
    assert_eq!(lamp.accepted, 8);
    let held: u32 = rows.iter().map(|row| row.held).sum();
    assert_eq!(word(words, 0), held);
    assert_eq!(word(words, 1), 0, "diag no longer stores a ray count");
    assert_eq!(held, 6);
    assert_eq!(word(words, 2), 0, "the gate does not list a young cell");
    assert_eq!(word(words, 3), 0, "the gate does not list a world-cell refresh");
}

fn check_hue_and_lamps(words: &[f32], rows: &[Row]) {
    let held: u32 = rows.iter().map(|row| row.held).sum();
    let tail = HEAD + ROWS * STRIDE;
    assert_eq!(word(words, tail), 3, "lamp pixel counter stays 3");
    assert_eq!(word(words, tail + 1), 5, "lamp hit counter stays 5");
    assert_eq!(word(words, tail + 3), held);
    assert_eq!(
        word(words, tail + 2),
        held + 7,
        "the held word is not the lamp counters"
    );
    let shown = [
        f32::from_bits(word(words, tail + 4)),
        f32::from_bits(word(words, tail + 5)),
        f32::from_bits(word(words, tail + 6)),
    ];
    let red = [8.0_f32, 0.0, 0.0];
    let green = [0.0_f32, 1.7008 / 0.7152, 0.0];
    assert!(
        luminance_would_hold(red, green),
        "equal luminance would keep the old hue cut"
    );
    assert_eq!(rows[HUE].held, 0, "the channel ratio resumes rays");
    assert_eq!(shown, red, "the pixel still shows the own red mean");
    assert_eq!(rows[CHROMA].held, 0, "equal-luminance corners are not one colour");
    assert_eq!(
        rows[LOBE_FLAT].held, 1,
        "this normal equals the stored axis, so the harmonic is invisible here"
    );
    assert_eq!(
        rows[LOBE_SLANT].held, 0,
        "the same harmonic at a facing stored normal fails this normal only"
    );
    let lamp_words = &words[LAMP_AT..];
    let reach = row(lamp_words, 0);
    let far = row(lamp_words, 1);
    let stale_lamp = row(lamp_words, 2);
    assert_eq!(reach.planted, 1);
    assert_eq!(far.planted, 1);
    assert_eq!(stale_lamp.planted, 1);
    assert_eq!(reach.looked, 8, "reach looked {}", reach.looked);
    assert_eq!(reach.accepted, 8, "reach accepted {}", reach.accepted);
    assert_eq!(reach.held, 0, "a lamp inside range, outside global settling, keeps rays");
    assert_eq!(far.looked, 8);
    assert_eq!(far.accepted, 8);
    assert_eq!(far.held, 1, "the same ring does not reach a far texel");
    assert_eq!(stale_lamp.looked, 8);
    assert_eq!(stale_lamp.accepted, 8);
    assert_eq!(stale_lamp.held, 1, "a ring frame older than the window is ignored");
}

fn check_compact_lists(words: &[f32]) {
    let compact = &words[CMP_AT..];
    assert_eq!(word(compact, CMP_N), 1, "compact cases planted");
    let full_off = lists(compact, 0, CMP_FULL);
    let full_on = lists(compact, 1, CMP_FULL);
    assert_eq!(full_off, [4, 0, 0, 0, 0, 0], "flag off lists the full set {full_off:?}");
    assert_eq!(full_on, [0, 0, 0, 0, 0, 0], "flag on holds the full set {full_on:?}");
    let quarter_off = lists(compact, 0, CMP_QUARTER);
    let quarter_on = lists(compact, 1, CMP_QUARTER);
    assert_eq!(
        quarter_off,
        [0, 0, 1, 0, 0, 0],
        "flag off lists one settled trickle entry {quarter_off:?}"
    );
    assert_eq!(quarter_on, [0, 0, 0, 0, 0, 0], "flag on holds that entry {quarter_on:?}");
    let idle_off = lists(compact, 0, CMP_IDLE);
    let idle_on = lists(compact, 1, CMP_IDLE);
    assert_eq!(idle_off, [0, 0, 0, 0, 0, 0], "an interval that has not elapsed lists nothing");
    assert_eq!(idle_on, idle_off, "holding that texel removes no list entry");
    for id in [CMP_DISAGREE, CMP_STALE, CMP_HUE] {
        let off = lists(compact, 0, id);
        let on = lists(compact, 1, id);
        assert_eq!(off, [4, 0, 0, 0, 0, 0], "case {id} flag off {off:?}");
        assert_eq!(on, off, "case {id} flag on still lists {on:?}");
    }
    let cell_off = lists(compact, 0, CMP_CELL);
    let cell_on = lists(compact, 1, CMP_CELL);
    assert_eq!(cell_off, cell_on, "the world cell refresh does not follow the ray-cut bit");
    assert!(
        cell_off.iter().sum::<u32>() > 0,
        "the world cell is listed {cell_off:?}"
    );
    let (wait_off, held_off) = compact_tail(compact, 0);
    let (wait_on, held_on) = compact_tail(compact, 1);
    assert_eq!(held_off, 0, "flag off holds no texel");
    assert_eq!(held_on, 3, "full, quarter, and the idle texel are visits");
    assert_eq!(
        wait_off,
        wait_on + 1,
        "waiting counts the idle scheduler skip, not the hold"
    );
    let full_entries = full_off[0] - full_on[0];
    let quarter_entries = quarter_off[2] - quarter_on[2];
    assert_eq!(full_entries * RAYS_PER_ENTRY, 64, "four mature entries are 64 potential rays");
    assert_eq!(
        quarter_entries * RAYS_PER_ENTRY,
        16,
        "one trickle entry is the settled 16-ray quarter"
    );
    assert_eq!((idle_off[2] - idle_on[2]) * RAYS_PER_ENTRY, 0);
}
