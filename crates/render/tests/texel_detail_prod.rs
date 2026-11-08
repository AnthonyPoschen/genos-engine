//! Lavapipe dispatch of `lighting_screen_probe_indirect` for ADR 0039.
//!
//! That is the function `lighting_compose.comp` calls. The change-block head
//! is `lighting_change_head`, the packer `lighting_passes` uploads. The
//! shader copies those words. It does not build bit 17. Software Vulkan only:
//! the numbers are not a 4070 time and not an accuracy gate. This dispatch
//! does not present a swapchain image. The production work buffer is
//! device-local, so a presented frame is not how this fixture reads the head.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use genos_render::{lighting_change_head, GpuKind, LightingPreset, LightingSettings, Renderer};
use genos_window::Window;

const STRIDE: usize = 17;
const ROWS: usize = 12;
const READ_ROWS: usize = 5;
const ABS: f32 = 1.0e-3;
const DETAIL_BIT: u32 = 1 << 17;

struct Row {
    shown: [f32; 3],
    world_w: f32,
    reads: f32,
    skipped: f32,
    seed: [f32; 3],
    conf: f32,
    cell_same: f32,
    own: f32,
    head_hi: u32,
    cell: [f32; 3],
    mark: f32,
}

fn row(words: &[f32], id: usize) -> Row {
    let at = 8 + id * STRIDE;
    let w = &words[at..at + STRIDE];
    Row {
        shown: [w[0], w[1], w[2]],
        world_w: w[3],
        reads: w[4],
        skipped: w[5],
        seed: [w[6], w[7], w[8]],
        conf: w[9],
        cell_same: w[10],
        own: w[11],
        head_hi: w[12].to_bits(),
        cell: [w[13], w[14], w[15]],
        mark: w[16],
    }
}

fn near(got: [f32; 3], want: [f32; 3]) -> bool {
    got.iter()
        .zip(want)
        .all(|(a, b)| (a - b).abs() <= ABS && a.is_finite())
}

fn settings(detail: bool, low: bool) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    settings.surface_cache.sh_texel_detail = detail;
    if low {
        settings.bounce.count = 0;
    }
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
fn the_settings_path_sets_head_bit_17_only_when_texel_detail_is_on() {
    let on = lighting_change_head(&settings(true, false));
    let off = lighting_change_head(&settings(false, false));
    let low = lighting_change_head(&settings(true, true));
    assert_ne!(on.head_hi & DETAIL_BIT, 0, "detail on packs bit 17");
    assert_eq!(
        off.head_hi & DETAIL_BIT,
        0,
        "detail off leaves bit 17 clear"
    );
    assert_ne!(
        low.head_hi & DETAIL_BIT,
        0,
        "the bit is packed even when the world mode is 0"
    );
    assert_eq!(on.mode_cap / 1000, 3, "high is the surface cache");
    assert_eq!(low.mode_cap / 1000, 0, "bounce 0 is probe GI");
    assert_ne!(on.flags & (1 << 20), 0, "the estimator bit stays on");
    assert_ne!(on.flags & (1 << 11), 0, "the default slant gate stays on");
}

#[test]
fn covered_pixels_skip_screen_probe_indirect_and_the_rest_keep_it() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let words = renderer
        .texel_detail_prod()
        .expect("production compose indirect");
    assert_eq!(words.len(), 8 + ROWS * STRIDE);
    let rows: Vec<Row> = (0..ROWS).map(|id| row(&words, id)).collect();

    let on = lighting_change_head(&settings(true, false));
    let off = lighting_change_head(&settings(false, false));
    let low = lighting_change_head(&settings(true, true));
    assert_eq!(
        words[1].to_bits(),
        on.head_hi,
        "the shader read a head it did not pack"
    );
    assert_eq!(words[6].to_bits(), (on.head_hi as f32).to_bits());
    assert_eq!(words[7].to_bits(), (off.head_hi as f32).to_bits());

    let red = [8.0, 0.0, 0.0];
    let blue = [0.0, 0.0, 7.0];
    let own = [4.0, 2.0, 0.0];
    let probe = [1.0, 2.0, 3.0];
    let green = [0.0, 6.0, 0.0];

    let covered = &rows[0];
    assert_eq!(covered.head_hi, on.head_hi);
    assert!(
        near(covered.shown, red),
        "covered pixel {:?} wanted the owning cell, not the probe {probe:?} or the green corners",
        covered.shown
    );
    assert!(
        (covered.world_w - 1.0).abs() <= ABS,
        "trust {}",
        covered.world_w
    );
    assert!(covered.reads.abs() <= ABS, "probe reads {}", covered.reads);
    assert!((covered.skipped - 1.0).abs() <= ABS);
    assert!((covered.conf - 0.25).abs() <= ABS, "conf {}", covered.conf);
    assert!(near(covered.seed, red), "stored seed {:?}", covered.seed);
    assert!(covered.own.abs() <= ABS, "own samples {}", covered.own);
    assert!(
        (covered.cell_same - 1.0).abs() <= ABS,
        "the cell was written"
    );

    let miss = &rows[1];
    assert!(
        near(miss.shown, probe),
        "an empty owner must show the probe, got {:?}",
        miss.shown
    );
    assert!(miss.reads >= 1.0, "miss still reads probes {}", miss.reads);
    assert!(miss.skipped.abs() <= ABS);
    // A miss claims an empty texel and the first-fill path stores the probe
    // color as that texel's seed, then returns it with trust 1. The mix path
    // (trust 0) is the same color. Either way the shown value is the probe,
    // not black and not a cell.
    let miss_seeded = near(miss.seed, probe) && (miss.world_w - 1.0).abs() <= ABS;
    let miss_mixed = miss.world_w.abs() <= ABS;
    assert!(
        miss_seeded || miss_mixed,
        "miss fallback was not the probe seed and not a probe mix: seed {:?} trust {}",
        miss.seed,
        miss.world_w
    );

    let detail_off = &rows[2];
    assert_eq!(detail_off.head_hi, off.head_hi);
    assert!(
        near(detail_off.shown, probe),
        "detail off with an empty cache showed {:?}, wanted the probe",
        detail_off.shown
    );
    assert!(detail_off.reads >= 1.0);
    assert!(detail_off.skipped.abs() <= ABS);

    let lamp_empty = &rows[3];
    assert!(
        near(lamp_empty.shown, probe),
        "a lamp change with an empty owner showed {:?}, wanted the probe, not the old green seed",
        lamp_empty.shown
    );
    assert!(
        lamp_empty.conf.abs() <= ABS || near(lamp_empty.seed, probe),
        "the empty owner left {:?}, conf {}",
        lamp_empty.seed,
        lamp_empty.conf
    );
    assert!(
        !near(lamp_empty.seed, green),
        "stale green seed {:?}",
        lamp_empty.seed
    );
    assert!(
        lamp_empty.reads >= 1.0,
        "the empty owner still needs probes"
    );
    assert!(lamp_empty.skipped.abs() <= ABS);
    assert!(
        near(lamp_empty.cell, [0.0; 3]),
        "the empty cell was filled {:?}",
        lamp_empty.cell
    );
    // The fallback read may stamp the cell's read time. It must not store light.

    let sampled = &rows[4];
    assert!(
        near(sampled.shown, own),
        "own samples showed {:?}, wanted the texel mean",
        sampled.shown
    );
    assert!(
        (sampled.world_w - 1.0).abs() <= ABS,
        "own trust {}",
        sampled.world_w
    );
    assert!(
        sampled.reads >= 1.0,
        "trust 1 from own samples is not coverage, reads {}",
        sampled.reads
    );
    assert!(sampled.skipped.abs() <= ABS);
    assert!((sampled.own - 1.0).abs() <= ABS);
    assert!((sampled.cell_same - 1.0).abs() <= ABS);

    let slant = &rows[5];
    assert!(
        near(slant.shown, probe),
        "a slanted pixel showed {:?}, wanted the probe. The axis SH read is empty and compose keeps the probes",
        slant.shown
    );
    assert!(slant.world_w.abs() <= ABS, "slant trust {}", slant.world_w);
    assert!(slant.reads >= 1.0);
    assert!(slant.skipped.abs() <= ABS);
    assert!(
        slant.conf.abs() <= ABS,
        "a slant stored a texel seed, conf {}",
        slant.conf
    );
    assert!((slant.cell_same - 1.0).abs() <= ABS);

    let mover = &rows[6];
    assert!(
        near(mover.shown, probe),
        "a mover showed {:?}, wanted the probe, not the cell",
        mover.shown
    );
    assert!(mover.reads >= 1.0);
    assert!(mover.skipped.abs() <= ABS);
    assert!(
        mover.conf.abs() <= ABS,
        "a mover stored the cell seed, conf {} seed {:?}",
        mover.conf,
        mover.seed
    );
    assert!(near(mover.cell, red), "mover cell {:?}", mover.cell);
    assert!((mover.cell_same - 1.0).abs() <= ABS);

    let quiet = &rows[7];
    assert!(
        near(quiet.shown, red),
        "denoise off hid the cell {:?}",
        quiet.shown
    );
    assert!((quiet.world_w - 1.0).abs() <= ABS);
    assert!(quiet.reads.abs() <= ABS, "denoise off still read probes");
    assert!((quiet.skipped - 1.0).abs() <= ABS);
    assert!(near(quiet.seed, red), "denoise off seed {:?}", quiet.seed);
    assert!((quiet.cell_same - 1.0).abs() <= ABS);

    let mode0 = &rows[8];
    assert_eq!(mode0.head_hi, low.head_hi);
    assert!(
        near(mode0.shown, probe),
        "mode 0 showed {:?}, wanted the probe. The detail bit does not retire probes without the cache",
        mode0.shown
    );
    assert!(mode0.reads >= 1.0);
    assert!(mode0.skipped.abs() <= ABS);

    let lamp_blue = &rows[9];
    assert!(
        near(lamp_blue.shown, blue),
        "a live owner after the lamp showed {:?}, wanted blue",
        lamp_blue.shown
    );
    assert!(
        near(lamp_blue.seed, blue),
        "seed after the lamp {:?}",
        lamp_blue.seed
    );
    assert!((lamp_blue.conf - 0.25).abs() <= ABS);
    assert!(lamp_blue.reads.abs() <= ABS, "the blue owner is covered");
    assert!((lamp_blue.skipped - 1.0).abs() <= ABS);
    assert!(near(lamp_blue.cell, blue));
    assert!((lamp_blue.cell_same - 1.0).abs() <= ABS);
    assert!(
        (lamp_blue.mark - 1.0).abs() <= ABS,
        "a live owner after the lamp keeps the detail mark"
    );

    let parent_rgb = [5.0, 1.0, 4.0];
    let quiet_empty = &rows[10];
    assert!(
        near(quiet_empty.shown, probe),
        "writes on, no lamp, empty owner showed {:?}, wanted the probe, not the old green seed",
        quiet_empty.shown
    );
    assert!(
        near(quiet_empty.seed, probe),
        "the miss writer stored {:?}, wanted the probe seed",
        quiet_empty.seed
    );
    assert!(
        quiet_empty.mark.abs() <= ABS,
        "the probe seed must clear the detail mark, mark {}",
        quiet_empty.mark
    );
    assert!(quiet_empty.reads >= 1.0, "the empty owner still needs probes");
    assert!(quiet_empty.skipped.abs() <= ABS);
    assert!(
        near(quiet_empty.cell, [0.0; 3]),
        "the empty cell was filled {:?}",
        quiet_empty.cell
    );

    let kept = &rows[11];
    assert!(
        near(kept.shown, parent_rgb),
        "an unmarked parent seed showed {:?}, wanted {parent_rgb:?}",
        kept.shown
    );
    assert!(
        near(kept.seed, parent_rgb),
        "the parent seed was replaced with {:?}",
        kept.seed
    );
    assert!(kept.mark.abs() <= ABS, "a parent seed is not a detail seed");
    assert!(kept.reads >= 1.0);
    assert!(kept.skipped.abs() <= ABS);
    assert!(
        (covered.mark - 1.0).abs() <= ABS,
        "the detail writer must mark its seed, mark {}",
        covered.mark
    );

    let skip = words[2].to_bits();
    let eval = words[3].to_bits();
    let reads = words[4].to_bits();
    assert_eq!(skip, 3, "covered, denoise-off, and the blue owner skip");
    assert_eq!(eval, 9, "the other rows still evaluate probe indirect");
    assert_eq!(reads, 9, "one probe irradiance read on each evaluated row");
}

#[test]
fn readonly_compose_drops_a_stale_detail_seed_and_keeps_a_parent_seed() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let words = renderer
        .texel_detail_prod_readonly()
        .expect("read-only production compose indirect");
    assert_eq!(words.len(), 8 + ROWS * STRIDE);
    let rows: Vec<Row> = (0..READ_ROWS).map(|id| row(&words, id)).collect();

    let red = [8.0, 0.0, 0.0];
    let own = [4.0, 2.0, 0.0];
    let probe = [1.0, 2.0, 3.0];
    let green = [0.0, 6.0, 0.0];
    let parent_rgb = [5.0, 1.0, 4.0];

    let stale = &rows[0];
    assert!(
        near(stale.shown, probe),
        "read-only compose with an empty owner showed {:?}, wanted the probe, not the stale green seed",
        stale.shown
    );
    assert!(
        near(stale.seed, green),
        "read-only compose must leave the stored detail seed, got {:?}",
        stale.seed
    );
    assert!(
        (stale.conf - 0.25).abs() <= ABS,
        "stored confidence {}",
        stale.conf
    );
    assert!(
        (stale.mark - 1.0).abs() <= ABS,
        "the detail mark stays, mark {}",
        stale.mark
    );
    assert!((stale.world_w - 1.0).abs() <= ABS, "trust {}", stale.world_w);
    assert!(stale.reads >= 1.0, "the empty owner still needs probes");
    assert!(stale.skipped.abs() <= ABS);
    assert!(near(stale.cell, [0.0; 3]));

    let parent = &rows[1];
    assert!(
        near(parent.shown, parent_rgb),
        "an unmarked parent seed showed {:?}, wanted {parent_rgb:?}",
        parent.shown
    );
    assert!(near(parent.seed, parent_rgb), "stored parent {:?}", parent.seed);
    assert!(parent.mark.abs() <= ABS);
    assert!(parent.reads >= 1.0);
    assert!(parent.skipped.abs() <= ABS);

    let covered = &rows[2];
    assert!(
        near(covered.shown, red),
        "a covered owner on the read-only compile showed {:?}",
        covered.shown
    );
    assert!((covered.world_w - 1.0).abs() <= ABS);
    assert!(covered.reads.abs() <= ABS, "probe reads {}", covered.reads);
    assert!((covered.skipped - 1.0).abs() <= ABS);
    assert!(
        covered.conf.abs() <= ABS,
        "read-only compose stored a seed, conf {}",
        covered.conf
    );

    let sampled = &rows[3];
    assert!(
        near(sampled.shown, own),
        "own samples showed {:?}, wanted the texel mean",
        sampled.shown
    );
    assert!((sampled.world_w - 1.0).abs() <= ABS);
    assert!(sampled.reads >= 1.0, "own samples still read probes");
    assert!(sampled.skipped.abs() <= ABS);
    assert!((sampled.own - 1.0).abs() <= ABS);

    let blend = [
        0.75 * parent_rgb[0] + 0.25 * probe[0],
        0.75 * parent_rgb[1] + 0.25 * probe[1],
        0.75 * parent_rgb[2] + 0.25 * probe[2],
    ];
    let mixed = &rows[4];
    assert!(
        near(mixed.shown, blend),
        "mixed corners showed {:?}, wanted {blend:?}. The stale green corner must not stay in the blend",
        mixed.shown
    );
    assert!(mixed.reads >= 1.0);
    assert!(mixed.skipped.abs() <= ABS);

    assert_eq!(words[2].to_bits(), 1, "only the covered owner skips");
    assert_eq!(words[3].to_bits(), 4, "the other read-only rows evaluate");
    assert_eq!(
        words[4].to_bits(),
        4,
        "one probe irradiance read on each evaluated row"
    );
}
