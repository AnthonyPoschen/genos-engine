//! Never-shown surface-cache texels are pre-filled before a turn brings them on screen.
//!
//! Ignored: a normal `cargo test` does not open a window. `tools/lighting/headless.sh`
//! runs this on lavapipe under a headless Weston. The scene is the small stress hall,
//! sun frozen, lamps and boxes still. The camera settles at the bench pose, then turns
//! in place. The lighting dump (`GENOS_LIGHTING_DUMP`, two exact frames) is the only
//! read of the cache.
//!
//! `GENOS_GI_PREFILL_SETTLE` (default 96) is the still frames before the turn.
//! `GENOS_GI_PREFILL_TURN` (default 24) is the turn, `GENOS_GI_PREFILL_YAW` (default
//! 0.5 rad) how far it goes. `GENOS_GI_PREFILL_MIN` / `GENOS_GI_PREFILL_SHOWN` are the
//! two asserts (64 pre-filled texels, 8 of them shown after the turn).
//! After the turn, unfilled shaded pixels must stay under
//! `GENOS_GI_PREFILL_UNFILLED_PCT` (see `DEFAULT_UNFILLED_PCT`).
//! The camera then holds for `GENOS_GI_PREFILL_HOLD` frames (default 32).
//! Shown pre-filled texels must still have a finished set, and their luminance
//! must sit near a second run that never pre-filled and settled at the end yaw
//! (`GENOS_GI_PREFILL_REF_SETTLE`, default 192; `GENOS_GI_PREFILL_BIAS`).
//! `GENOS_LIGHTING_PREFILL=0` is that second run. `GENOS_GI_PREFILL_OUT` keeps the dumps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use genos_render::{Renderer, UnfilledControl};
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// 12:12 as a fraction of the stress day (0 is 06:00).
const DAY: f32 = (12.2 - 6.0) / 24.0;
/// Light-cache slots (lighting_cache_slots.glsl).
const SLOTS: usize = 1 << 20;
/// Fine texels are 12.5 cm. World radiance cache cells are 25 cm.
const FINE: f32 = 0.13;
const BENCH_POS: [f32; 3] = [11.59, 1.7, 23.52];
const BENCH_YAW: f32 = -1.0;
const BENCH_PITCH: f32 = -0.25;
const DEFAULT_SETTLE: u64 = 96;
const DEFAULT_TURN: u64 = 24;
const DEFAULT_YAW: f32 = 0.5;
/// Unfilled share of shaded pixels after the turn, on lavapipe at 320x180.
/// A lavapipe run (settle 96, turn 24, yaw 0.5) ended at 0.95% (499 px, max_run 36)
/// after the plain-mean handoff. `GENOS_GI_PREFILL_UNFILLED_PCT` overrides this.
const DEFAULT_UNFILLED_PCT: f32 = 2.0;
/// Still frames at the end yaw after the turn, before the luminance compare.
const DEFAULT_HOLD: u64 = 32;
/// Pre-fill-off settle at the end yaw. Longer than the turn's settle: at 96
/// frames the reference mean is still climbing, and texels with 16 sets have
/// levelled off by 192.
const DEFAULT_REF_SETTLE: u64 = 192;
/// Mean relative luminance of revealed pre-filled texels against a prefill-off
/// settle at the same pose. `GENOS_GI_PREFILL_BIAS` overrides this.
///
/// The lavapipe defaults (settle 96, turn 24, hold 32, reference settle 192)
/// measured 0.374 over 3385 keys. The cap sits above that gap. It locks a
/// regression. The brightness difference remains.
const DEFAULT_BIAS: f32 = 0.50;
/// Draws after the last dumped frame so `lighting_dump_write` runs (it waits
/// until a later begin sees `frame >= last + 3`).
const FLUSH: u64 = 5;

#[derive(Clone, Copy)]
struct Fine {
    key: u32,
    sets: f32,
    shown: u32,
    quarters: u32,
    lum: f32,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(default)
}

fn env_u64_any(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn stage() -> Stage {
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 0,
            layout: Layout::Spread,
        },
        1.0,
        DAY,
        120.0,
    );
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    stage
}

fn camera(yaw: f32) -> Camera {
    let [x, y, z] = BENCH_POS;
    let mut camera = Camera::new(x, z, yaw);
    camera.set_pose(Vec3::new(x, y, z), yaw, BENCH_PITCH);
    camera
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &genos_render::World, yaw: f32) {
    let _ = window.pump();
    renderer.draw(world, &camera(yaw), false).expect("draw");
}

fn word(cache: &[u8], float_index: usize) -> u32 {
    let b = float_index * 4;
    u32::from_le_bytes(cache[b..b + 4].try_into().expect("cache word"))
}

/// Live fine texels in one dump. `shown` is the frame stamp (0: never on screen).
/// `sets` is the committed sample count. `quarters` is the unfinished set.
fn fine_texels(dir: &Path, frame: u64) -> Vec<Fine> {
    let cache_path = dir.join(format!("f{frame}_cache.bin"));
    let keys_path = dir.join(format!("f{frame}_keys.bin"));
    let cache =
        std::fs::read(&cache_path).unwrap_or_else(|e| panic!("{}: {e}", cache_path.display()));
    let keys = std::fs::read(&keys_path).unwrap_or_else(|e| panic!("{}: {e}", keys_path.display()));
    assert!(
        cache.len() >= 7 * SLOTS * 16 && keys.len() >= SLOTS * 4,
        "dump frame {frame} is short (cache {} bytes, keys {} bytes)",
        cache.len(),
        keys.len()
    );
    let mut out = Vec::new();
    for i in 0..SLOTS {
        let key = u32::from_le_bytes(keys[i * 4..i * 4 + 4].try_into().unwrap());
        if key == 0 {
            continue;
        }
        let size = f32::from_bits(word(&cache, (3 * i + 1) * 4 + 3));
        if size > FINE {
            continue;
        }
        let base = (3 * i + 2) * 4;
        let r = f32::from_bits(word(&cache, base));
        let g = f32::from_bits(word(&cache, base + 1));
        let b = f32::from_bits(word(&cache, base + 2));
        out.push(Fine {
            key,
            sets: f32::from_bits(word(&cache, base + 3)),
            shown: word(&cache, (3 * SLOTS + i) * 4 + 2),
            quarters: word(&cache, (6 * SLOTS + i) * 4 + 3),
            lum: 0.2126 * r + 0.7152 * g + 0.0722 * b,
        });
    }
    out
}

fn by_key(texels: &[Fine]) -> HashMap<u32, Fine> {
    let mut map: HashMap<u32, Fine> = HashMap::new();
    for texel in texels {
        match map.get_mut(&texel.key) {
            Some(prev) if texel.sets > prev.sets => *prev = *texel,
            Some(_) => {}
            None => {
                map.insert(texel.key, *texel);
            }
        }
    }
    map
}

/// Mean of |a-b| / max(a, b, 0.05). Dark texels do not dominate the ratio.
fn mean_relative(pairs: &[(f32, f32)]) -> f32 {
    let n = pairs.len();
    if n == 0 {
        return 0.0;
    }
    let sum: f32 = pairs
        .iter()
        .map(|(a, b)| (a - b).abs() / a.max(*b).max(0.05))
        .sum();
    sum / n as f32
}

fn out_dir() -> PathBuf {
    let dir = std::env::var_os("GENOS_GI_PREFILL_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("genos-gi-prefill"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("pre-fill dump directory");
    dir
}

/// Clears the dump env even when the assert fails.
struct DumpEnv;

impl Drop for DumpEnv {
    fn drop(&mut self) {
        std::env::remove_var("GENOS_LIGHTING_DUMP");
        std::env::remove_var("GENOS_LIGHTING_DUMP_DIR");
        std::env::remove_var("GENOS_LIGHTING_SC_TRACE");
    }
}

#[test]
#[ignore = "needs Wayland; lavapipe via tools/lighting/headless.sh"]
fn never_shown_texels_are_prefilled_before_a_turn() {
    let _gpu = {
        static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
        GPU.lock().unwrap_or_else(|err| err.into_inner())
    };
    let settle = env_u64("GENOS_GI_PREFILL_SETTLE", DEFAULT_SETTLE);
    let turn = env_u64("GENOS_GI_PREFILL_TURN", DEFAULT_TURN);
    let yaw_delta = env_f32("GENOS_GI_PREFILL_YAW", DEFAULT_YAW);
    let min_prefilled = env_usize("GENOS_GI_PREFILL_MIN", 64);
    let min_shown = env_usize("GENOS_GI_PREFILL_SHOWN", 8);
    let hold = env_u64_any("GENOS_GI_PREFILL_HOLD", DEFAULT_HOLD);
    let ref_settle = env_u64("GENOS_GI_PREFILL_REF_SETTLE", DEFAULT_REF_SETTLE);
    let bias_cap = env_f32("GENOS_GI_PREFILL_BIAS", DEFAULT_BIAS);
    let dir = out_dir();
    let _env = DumpEnv;
    let turn_frame = settle + turn;
    let bias_frame = turn_frame + hold;
    let mut spec = format!("{settle},{turn_frame}");
    if hold > 0 {
        spec.push_str(&format!(",{bias_frame}"));
    }
    std::env::set_var("GENOS_LIGHTING_DUMP", spec);
    std::env::set_var("GENOS_LIGHTING_DUMP_DIR", &dir);
    std::env::set_var("GENOS_LIGHTING_SC_TRACE", "1");

    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    assert_eq!(
        (size.width, size.height),
        (WIDTH, HEIGHT),
        "the window keeps its size"
    );
    let mut renderer =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    renderer.set_unfilled(UnfilledControl {
        count: true,
        tint: false,
        dump_pct: None,
    });
    let stage = stage();
    eprintln!(
        "prefill: settle {settle} turn {turn} hold {hold} yaw {yaw_delta} dir {}",
        dir.display()
    );
    for i in 1..=settle {
        draw(&mut window, &mut renderer, &stage.world, BENCH_YAW);
        if i % 16 == 0 || i == settle {
            eprintln!("prefill: settle frame {i}/{settle}");
        }
    }
    for i in 1..=turn {
        let yaw = BENCH_YAW + yaw_delta * (i as f32 / turn as f32);
        draw(&mut window, &mut renderer, &stage.world, yaw);
        if i % 8 == 0 || i == turn {
            eprintln!("prefill: turn frame {i}/{turn}");
        }
    }
    let unfilled = renderer.unfilled_stats();
    eprintln!("prefill: {}", unfilled.line());
    let unfilled_cap = std::env::var("GENOS_GI_PREFILL_UNFILLED_PCT")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|n| n.is_finite() && *n >= 0.0)
        .unwrap_or(DEFAULT_UNFILLED_PCT);
    assert!(
        unfilled.shaded > 0 && unfilled.pct < unfilled_cap,
        "unfilled after the turn is {:.2}% of {} shaded pixels (want under {unfilled_cap}); {}",
        unfilled.pct,
        unfilled.shaded,
        unfilled.line()
    );
    let yaw_end = BENCH_YAW + yaw_delta;
    for i in 1..=hold {
        draw(&mut window, &mut renderer, &stage.world, yaw_end);
        if i % 16 == 0 || i == hold {
            eprintln!("prefill: hold frame {i}/{hold}");
        }
    }
    for _ in 0..FLUSH {
        draw(&mut window, &mut renderer, &stage.world, yaw_end);
    }
    drop(renderer);
    drop(window);

    let settle_path = dir.join(format!("f{settle}_cache.bin"));
    let turn_frame = settle + turn;
    let turn_path = dir.join(format!("f{turn_frame}_cache.bin"));
    assert!(
        settle_path.is_file() && turn_path.is_file(),
        "dumps were not written (settle {}, turn {}); drew {} frames after the turn",
        settle_path.display(),
        turn_path.display(),
        FLUSH
    );

    let settled = fine_texels(&dir, settle);
    let mut sets_hist = [0u32; 5];
    let mut never = 0usize;
    let mut partial = 0usize;
    let mut prefilled = Vec::new();
    for texel in &settled {
        if texel.shown != 0 {
            continue;
        }
        never += 1;
        let bin = texel.sets.floor().clamp(0.0, 4.0) as usize;
        sets_hist[bin] += 1;
        if texel.sets < 1.0 && texel.quarters > 0 {
            partial += 1;
        }
        if texel.sets >= 1.0 {
            prefilled.push(texel.key);
        }
    }
    prefilled.sort_unstable();
    prefilled.dedup();
    eprintln!(
        "settle frame {settle}: fine {} never-shown {never} prefilled {} partial-quarters {partial} sets0..4+ {sets_hist:?}",
        settled.len(),
        prefilled.len()
    );

    let arrived = fine_texels(&dir, turn_frame);
    let mut after: HashMap<u32, bool> = HashMap::new();
    for texel in &arrived {
        let shown = after.entry(texel.key).or_insert(false);
        if texel.shown != 0 {
            *shown = true;
        }
    }
    let mut on_screen = 0usize;
    let mut still_hidden = 0usize;
    let mut gone = 0usize;
    for key in &prefilled {
        match after.get(key) {
            Some(true) => on_screen += 1,
            Some(false) => still_hidden += 1,
            None => gone += 1,
        }
    }
    eprintln!(
        "turn frame {turn_frame}: of {} prefilled keys, shown {on_screen}, still hidden {still_hidden}, gone {gone}",
        prefilled.len()
    );
    assert!(
        prefilled.len() >= min_prefilled,
        "after {settle} still frames, {never} fine texels had never been shown and {} of them held a finished set (want {min_prefilled}); sets 0..4+ {sets_hist:?}, unfinished quarters {partial}",
        prefilled.len()
    );
    assert!(
        on_screen >= min_shown,
        "the {yaw_delta} rad turn brought {on_screen} pre-filled texels on screen (want {min_shown}); {still_hidden} stayed hidden, {gone} left the cache"
    );

    let revealed = by_key(&arrived);
    let mut empty_on_show = 0usize;
    for key in &prefilled {
        if let Some(texel) = revealed.get(key) {
            if texel.shown != 0 && texel.sets < 1.0 {
                empty_on_show += 1;
            }
        }
    }
    assert!(
        empty_on_show == 0,
        "{empty_on_show} pre-filled texels were on screen at frame {turn_frame} with no finished set"
    );

    let bias_src = if hold > 0 { bias_frame } else { turn_frame };
    let biased = by_key(&fine_texels(&dir, bias_src));

    let ref_dir = dir.join("reference");
    std::fs::create_dir_all(&ref_dir).expect("reference dump directory");
    std::env::set_var("GENOS_LIGHTING_DUMP", format!("{ref_settle}"));
    std::env::set_var("GENOS_LIGHTING_DUMP_DIR", &ref_dir);
    let mut ref_window = Window::open_proof(WIDTH, HEIGHT).expect("reference window");
    let ref_size = ref_window.pump();
    assert_eq!((ref_size.width, ref_size.height), (WIDTH, HEIGHT));
    let mut ref_renderer = Renderer::open(
        ref_window.display,
        ref_window.surface,
        ref_size.width,
        ref_size.height,
    )
    .expect("reference renderer");
    ref_renderer.set_prefill(false);
    assert!(!ref_renderer.prefill_on());
    eprintln!("prefill: reference settle {ref_settle} at yaw {yaw_end}");
    for i in 1..=ref_settle {
        draw(&mut ref_window, &mut ref_renderer, &stage.world, yaw_end);
        if i % 16 == 0 || i == ref_settle {
            eprintln!("prefill: reference frame {i}/{ref_settle}");
        }
    }
    for _ in 0..FLUSH {
        draw(&mut ref_window, &mut ref_renderer, &stage.world, yaw_end);
    }
    drop(ref_renderer);
    drop(ref_window);
    drop(_gpu);

    let reference = by_key(&fine_texels(&ref_dir, ref_settle));
    let mut leaked = 0usize;
    for texel in reference.values() {
        if texel.shown == 0 && texel.sets >= 1.0 {
            leaked += 1;
        }
    }
    eprintln!(
        "reference frame {ref_settle}: fine {} never-shown finished sets {leaked}",
        reference.len()
    );
    assert!(
        leaked == 0,
        "pre-fill off still left {leaked} never-shown fine texels with a finished set"
    );

    let mut pairs = Vec::new();
    let mut missing = 0usize;
    for key in &prefilled {
        let Some(shown) = revealed.get(key) else {
            continue;
        };
        if shown.shown == 0 {
            continue;
        }
        let Some(after_hold) = biased.get(key) else {
            missing += 1;
            continue;
        };
        let Some(settled_ref) = reference.get(key) else {
            missing += 1;
            continue;
        };
        if settled_ref.sets < 1.0 || settled_ref.shown == 0 {
            missing += 1;
            continue;
        }
        pairs.push((after_hold.lum, settled_ref.lum));
    }
    let mean = mean_relative(&pairs);
    eprintln!(
        "bias frame {bias_src}: {} matched keys, {missing} skipped, mean relative luminance {mean:.3} (cap {bias_cap})",
        pairs.len()
    );
    assert!(
        pairs.len() >= min_shown,
        "only {} revealed pre-filled keys had a lit reference texel (want {min_shown})",
        pairs.len()
    );
    assert!(
        mean < bias_cap,
        "revealed pre-fill luminance differs from the prefill-off settle by {mean:.3} (want under {bias_cap}) over {} keys",
        pairs.len()
    );
}
