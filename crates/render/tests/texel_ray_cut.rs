//! Lavapipe dispatch of `lighting_compact_hold_texel_rays` for ADR 0039 stage 4.
//!
//! That is the function `lighting_compact.comp` calls after the 25 cm lists.
//! The change-block head is `lighting_change_head`. The shader copies those
//! words. Software Vulkan only: the numbers are ray counts, not a frame time
//! and not an accuracy gate. This dispatch does not present a swapchain image.
//! It does not run the stage 3 read-only compose, and it does not claim that
//! specialization passed.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
#![expect(clippy::panic, reason = "a test aborts when a fixture fails")]

use genos_render::{lighting_change_head, GpuKind, LightingPreset, LightingSettings, Renderer};
use genos_window::Window;

const ROWS: usize = 22;
const STRIDE: usize = 8;
const HEAD: usize = 8;
/// One full set: four quarters of 16 rays, quarter 0 clear.
const FULL_SET: u32 = 64;
const DETAIL_BIT: u32 = 1 << 17;
const RAY_CUT_BIT: u32 = 1 << 18;

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
    assert!(hold.contains("gstats.diag[LIGHTING_DIAG_TEXEL_HELD]"));
    assert!(hold.contains("gstats.diag[LIGHTING_DIAG_TEXEL_CUT]"));
    assert!(hold.contains("lighting_sc_entries(slot, true) * LIGHTING_CACHE_RAYS"));
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
}

#[test]
fn agreed_texel_rays_are_cut_and_disagreement_keeps_them() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let words = renderer
        .texel_ray_cut()
        .expect("production texel ray cut");
    assert_eq!(words.len(), HEAD + ROWS * STRIDE);
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
    for (id, row) in rows.iter().enumerate() {
        assert_eq!(row.planted, 1, "row {id} did not plant");
        assert_eq!(row.cell_same, 1, "row {id} wrote a cell");
        assert_eq!(row.seed_same, 1, "row {id} wrote the seed");
        assert_eq!(row.mean_same, 1, "row {id} wrote the texel mean");
    }
    let held_rows = [STABLE, SEED_DIFF, OWN_AGREE, BLACK, MIX_OK];
    for id in held_rows {
        let row = &rows[id];
        assert_eq!(row.held, 1, "row {id} should hold");
        assert_eq!(row.rays, FULL_SET, "row {id} rays {}", row.rays);
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
    for id in [STALE_TEXEL, LAMP, MOVE, DETAIL_OFF, CUT_OFF, LOW, COARSE, MOVER] {
        let row = &rows[id];
        assert_eq!(row.held, 0, "row {id} must keep its rays");
        assert_eq!(row.rays, 0, "row {id} cut {}", row.rays);
        assert_eq!(row.looked, 8, "row {id} looked {}", row.looked);
        assert_eq!(row.accepted, 8, "row {id} accepted {}", row.accepted);
    }
    let held: u32 = rows.iter().map(|row| row.held).sum();
    let rays: u32 = rows.iter().map(|row| row.rays).sum();
    assert_eq!(word(&words, 0), held);
    assert_eq!(word(&words, 1), rays);
    assert_eq!(held, 5);
    assert_eq!(rays, 5 * FULL_SET);
    assert_eq!(word(&words, 2), 0, "the gate does not list a young cell");
    assert_eq!(word(&words, 3), 0, "the gate does not list a world-cell refresh");
}
