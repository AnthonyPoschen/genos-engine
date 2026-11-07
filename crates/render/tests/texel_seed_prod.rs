#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! Production `lighting_world_irradiance_in` on lavapipe.
//!
//! The dispatch is `texel_seed_prod.comp`, which includes the production
//! cache shader and calls that function. Default surface-cache flags, the
//! high preset's denoise bit, SH cells, and the stage 3 seed bit. It is not
//! the `texel_seed_check` helper.

use genos_render::{GpuKind, Renderer};
use genos_window::Window;

const HEAD: usize = 4;
const STRIDE: usize = 24;
const EPS: f32 = 1.0e-5;
const NAMES: [&str; 10] = [
    "compete",
    "miss",
    "off",
    "lamp",
    "own",
    "mover",
    "slant",
    "own_empty",
    "denoise_off",
    "denoise_off_setting",
];

fn open() -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(64, 64)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

fn bits(words: &[f32], index: usize) -> u32 {
    words[index].to_bits()
}

struct Record {
    shown: [f32; 3],
    weight: f32,
    before: [f32; 4],
    after: [f32; 4],
    stamp_before: u32,
    stamp_after: u32,
    hash_before: u32,
    hash_after: u32,
    confidence: f32,
    own: f32,
    seed_on: f32,
    flags: u32,
    seed: [f32; 3],
    denoise: f32,
}

fn record(words: &[f32], id: usize) -> Record {
    let at = HEAD + id * STRIDE;
    Record {
        shown: [words[at], words[at + 1], words[at + 2]],
        weight: words[at + 3],
        before: [words[at + 4], words[at + 5], words[at + 6], words[at + 7]],
        after: [words[at + 8], words[at + 9], words[at + 10], words[at + 11]],
        stamp_before: bits(words, at + 12),
        stamp_after: bits(words, at + 13),
        hash_before: bits(words, at + 14),
        hash_after: bits(words, at + 15),
        confidence: words[at + 16],
        own: words[at + 17],
        seed_on: words[at + 18],
        flags: bits(words, at + 19),
        seed: [words[at + 20], words[at + 21], words[at + 22]],
        denoise: words[at + 23],
    }
}

fn near(got: f32, expect: f32, what: &str) {
    assert!(
        (got - expect).abs() <= EPS,
        "{what}: got {got} expected {expect}"
    );
}

fn near3(got: [f32; 3], expect: [f32; 3], what: &str) {
    for (index, (got, expect)) in got.iter().zip(expect).enumerate() {
        assert!(
            (got - expect).abs() <= EPS,
            "{what}[{index}]: got {got:?} expected {expect:?}"
        );
    }
}

fn cell_held(row: &Record) {
    assert_eq!(
        row.stamp_before, row.stamp_after,
        "owning cell stamp changed"
    );
    assert_eq!(row.stamp_after, 3, "owning cell stamp was rewritten");
    for (before, after) in row.before.iter().zip(row.after) {
        assert!(
            (before - after).abs() < 1.0e-5,
            "owning cell mean or count changed: {:?} -> {:?}",
            row.before,
            row.after
        );
    }
    assert_eq!(
        row.hash_before, row.hash_after,
        "owning cell harmonics changed"
    );
}

fn expect_header(flags: u32, head_w: u32, ctrl: u32, mode: u32) {
    assert_ne!(
        flags & (1 << 19),
        0,
        "default flags include the resample bit"
    );
    assert_eq!(
        flags & (1 << 29),
        0,
        "layer 2 is on when its off bit is clear"
    );
    assert_eq!(flags & (1 << 31), 0, "SVGF is on when its off bit is clear");
    assert_ne!(head_w & (1 << 17), 0, "the stage 3 seed bit is set");
    assert_ne!(head_w & 1, 0, "SH cells are on");
    assert_ne!(ctrl & 64, 0, "the high preset denoise bit is set");
    assert_eq!(mode, 3, "world mode is the surface cache");
}

fn dump(words: &[f32]) -> Vec<Record> {
    let flags = bits(words, 0);
    let head_w = bits(words, 1);
    let ctrl = bits(words, 2);
    let mode = bits(words, 3);
    println!("header flags={flags:#x} head_w={head_w:#x} ctrl={ctrl:#x} mode={mode}");
    expect_header(flags, head_w, ctrl, mode);
    let rows: Vec<Record> = (0..NAMES.len()).map(|id| record(words, id)).collect();
    for (name, row) in NAMES.iter().zip(&rows) {
        println!(
            "{name} shown={:?} w={} seed={:?} denoise={} before={:?} after={:?} stamp={}→{} hash {:#x} {:#x} conf={} own={} seed_on={} flags={:#x}",
            row.shown,
            row.weight,
            row.seed,
            row.denoise,
            row.before,
            row.after,
            row.stamp_before,
            row.stamp_after,
            row.hash_before,
            row.hash_after,
            row.confidence,
            row.own,
            row.seed_on,
            row.flags
        );
    }
    rows
}

#[test]
fn the_owning_cell_seed_wins_a_competing_resample_on_the_production_path() {
    let mut life = open().expect("proof window");
    let renderer = &mut *life.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let rows = dump(
        &renderer
            .texel_seed_prod()
            .expect("production seed dispatch"),
    );

    expect_defaults(&rows);
    expect_denoise_off(&rows[8], &rows[9]);
}

fn expect_defaults(rows: &[Record]) {
    let compete = &rows[0];
    near(compete.denoise, 1.0, "default denoise");
    near(compete.seed_on, 1.0, "compete seed bit");
    near3(compete.shown, [8.0, 0.0, 0.0], "compete shown");
    near3(compete.seed, [8.0, 0.0, 0.0], "compete seed");
    near(compete.confidence, 0.25, "compete confidence");
    near(compete.weight, 1.0, "compete trust");
    near(compete.own, 0.0, "compete own count");
    cell_held(compete);

    let miss = &rows[1];
    near3(miss.shown, [0.0, 6.0, 0.0], "miss shown");
    near(miss.confidence, 0.25, "miss confidence");
    near(miss.weight, 1.0, "miss trust");
    assert_eq!(miss.stamp_before, 3);
    assert_eq!(miss.stamp_after, 10, "a miss still lets the fallback stamp");
    assert!(miss.before[3].abs() <= EPS && miss.after[3].abs() <= EPS);
    assert_eq!(miss.hash_before, miss.hash_after);

    let off = &rows[2];
    near(off.seed_on, 0.0, "off seed bit");
    near3(off.shown, [4.5, 2.625, 0.0], "off shown");
    near(off.confidence, 0.5, "off confidence");
    near(off.weight, 1.0, "off trust");

    let lamp = &rows[3];
    near3(lamp.shown, [0.0, 0.0, 7.0], "lamp shown");
    near3(lamp.seed, [0.0, 0.0, 7.0], "lamp seed");
    near(lamp.confidence, 0.25, "lamp confidence");
    cell_held(lamp);

    // Default SVGF with no fresh field shows lighting_sc_wrc_now before the
    // own mean. The sample still owns the slot: count stays 1 and no seed is
    // stored over it. The empty-neighbour case is the own-mean arm.
    let own = &rows[4];
    near(own.own, 1.0, "own count");
    near(own.confidence, 0.0, "own confidence");
    near3(own.seed, [0.0, 0.0, 0.0], "own seed");
    near3(own.shown, [4.5, 2.625, 0.0], "own shown");
    cell_held(own);

    let mover = &rows[5];
    near3(mover.shown, off.shown, "mover shown");
    near(mover.weight, 0.0, "mover trust");
    near(mover.confidence, 0.5, "mover confidence");

    let slant = &rows[6];
    near3(slant.shown, [0.0, 0.0, 0.0], "slant shown");
    near(slant.weight, 0.0, "slant trust");
    near(slant.confidence, 0.0, "slant confidence");
    cell_held(slant);

    let own_empty = &rows[7];
    near(own_empty.own, 1.0, "empty-neighbour count");
    near(own_empty.confidence, 0.0, "empty-neighbour confidence");
    near3(own_empty.shown, [4.0, 2.0, 0.0], "empty-neighbour shown");
}

fn expect_denoise_off(seed_on: &Record, setting_off: &Record) {
    near(seed_on.denoise, 0.0, "denoise-off ctrl");
    near(seed_on.seed_on, 1.0, "denoise-off seed bit");
    near3(seed_on.seed, [8.0, 0.0, 0.0], "denoise-off stored seed");
    near(seed_on.confidence, 0.25, "denoise-off confidence");
    near(seed_on.own, 0.0, "denoise-off own count");
    // The denoise-off pixel walk still shows the parent mix and stamps the
    // cell it reads. The stored seed is the owner, not that mix.
    near3(seed_on.shown, [4.5, 2.625, 0.0], "denoise-off shown");
    near(seed_on.weight, 1.0, "denoise-off trust");
    assert_eq!(seed_on.stamp_before, 3);
    assert_eq!(seed_on.stamp_after, 10);
    for (before, after) in seed_on.before.iter().zip(seed_on.after) {
        near(*before, after, "denoise-off cell");
    }
    assert_eq!(seed_on.hash_before, seed_on.hash_after);

    near(setting_off.denoise, 0.0, "denoise-off setting ctrl");
    near(setting_off.seed_on, 0.0, "denoise-off setting bit");
    near3(
        setting_off.shown,
        [4.5, 2.625, 0.0],
        "denoise-off setting display",
    );
    near3(
        setting_off.seed,
        [4.5, 2.625, 0.0],
        "denoise-off setting seed",
    );
    near(
        setting_off.confidence,
        0.25,
        "denoise-off setting confidence",
    );
}
