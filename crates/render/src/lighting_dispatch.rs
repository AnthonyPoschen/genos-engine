// Which lighting dispatches a LightingSettings records.
//
// The tuned path (preset high, every feature on, this frame due) builds the
// same group list the frame recorded before settings existed. Anything else
// drops the dispatches the disabled feature owned.

use genos_scene::{AmbientMode, LightingSettings, ShadowFilter, LIGHTING_BOUNCES_UNLIMITED};

pub(crate) const BUCKETS: usize = 12;
pub(crate) const B_COMPACT: u8 = 0;
pub(crate) const B_PLACE: u8 = 1;
pub(crate) const B_TRACE: u8 = 2;
pub(crate) const B_DIRECT: u8 = 3;
pub(crate) const B_GATHER: u8 = 4;
pub(crate) const B_FILTER: u8 = 5;
pub(crate) const B_CACHE: u8 = 6;
pub(crate) const B_COMPOSE: u8 = 7;
pub(crate) const B_ATROUS: u8 = 8;
pub(crate) const B_CACHE_RAYS: u8 = 9;
pub(crate) const B_HIT: u8 = 10;
pub(crate) const B_SHADOWS: u8 = 11;

pub(crate) const LIGHTING_BUCKET_NAMES: [&str; BUCKETS] = [
    "compact",
    "place",
    "trace",
    "direct",
    "gather",
    "filter",
    "cache",
    "compose",
    "atrous",
    "cache_rays",
    "hit_light",
    "shadows",
];

/// First stats word of the control block appended after the a-trous counters.
pub(crate) const LIGHTING_CTRL_WORD: u64 = 124;

const CTRL_DIRECT: u32 = 1;
const CTRL_SHADOWS: u32 = 2;
const CTRL_GI: u32 = 4;
const CTRL_BOUNCE_READS: u32 = 8;
const CTRL_PROBES: u32 = 16;
const CTRL_WORLD_CACHE: u32 = 32;
const CTRL_DENOISE: u32 = 64;
const CTRL_COMPOSE_REUSE: u32 = 128;
const CTRL_SKY: u32 = 256;
const CTRL_EMISSIVE: u32 = 512;
const CTRL_PREFILL: u32 = 1024;
const CTRL_SHADOW_HARD: u32 = 2048;
const CTRL_AMBIENT: u32 = 4096;
const CTRL_HEMISPHERE: u32 = 8192;
const CTRL_BOUNCE_UNLIMITED: u32 = 16384;
const CTRL_DIRECT_FRESH: u32 = 32768;
/// Denoiser iterations in bits 16..18. 0 in that field means the tuned 5.
const CTRL_DENOISE_ITERS_SHIFT: u32 = 16;

/// What a buffer rebuild depends on. Screen size is compared beside this.
pub(crate) fn lighting_alloc_key(settings: &LightingSettings) -> (u32, u32, u32, u32, u32) {
    let (cache, probes, rays, tile) = settings.buffer_class();
    (
        cache,
        probes,
        rays,
        tile,
        u32::from(settings.direct.refresh_frames > 1),
    )
}

#[repr(C)]
struct Ctrl {
    bits: u32,
    max_lights: u32,
    bounce: u32,
    scale: u32,
    bias: u32,
    pixel_only: u32,
    ambient_mode: u32,
    /// 1 when direct light is stored for a later frame (`refresh_frames` > 1).
    history: u32,
    shadow_cell: f32,
    ambient_r: f32,
    ambient_g: f32,
    ambient_b: f32,
    ambient_i: f32,
    ground_r: f32,
    ground_g: f32,
    ground_b: f32,
}

/// 64 bytes at [`LIGHTING_CTRL_WORD`], little endian, matching the shader block.
pub(crate) fn pack_lighting_ctrl(
    settings: &LightingSettings,
    frame: u32,
    sun_height: f32,
    bias: u32,
    pixel_only: u32,
) -> [u8; 64] {
    let (color, ground, intensity, mode) = settings.ambient_at(sun_height);
    let mut bits = 0u32;
    if settings.direct.enabled {
        bits |= CTRL_DIRECT;
    }
    if settings.direct_this_frame(frame) {
        bits |= CTRL_DIRECT_FRESH;
    }
    if settings.shadows.enabled {
        bits |= CTRL_SHADOWS;
    }
    if settings.shadows.filter == ShadowFilter::Hard {
        bits |= CTRL_SHADOW_HARD;
    }
    if settings.gi.enabled && settings.bounce.count > 0 {
        bits |= CTRL_GI;
        bits |= CTRL_BOUNCE_READS;
    }
    if settings.bounce.count == LIGHTING_BOUNCES_UNLIMITED {
        bits |= CTRL_BOUNCE_UNLIMITED;
    }
    if settings.probes.enabled {
        bits |= CTRL_PROBES;
    }
    if settings.world_cache.enabled {
        bits |= CTRL_WORLD_CACHE;
    }
    if settings.denoiser.enabled {
        bits |= CTRL_DENOISE;
    }
    if settings.compose_reuse {
        bits |= CTRL_COMPOSE_REUSE;
    }
    if settings.sky {
        bits |= CTRL_SKY;
    }
    if settings.emissive {
        bits |= CTRL_EMISSIVE;
    }
    if settings.prefill_this_frame() {
        bits |= CTRL_PREFILL;
    }
    bits |= (settings.denoiser.iterations & 7) << CTRL_DENOISE_ITERS_SHIFT;
    // Bounce 0 is direct light plus this fill. GI on at one bounce or more
    // does not add it.
    if mode != AmbientMode::Off && (settings.bounce.count == 0 || !settings.gi.enabled) {
        bits |= CTRL_AMBIENT;
    }
    if mode == AmbientMode::Hemisphere {
        bits |= CTRL_HEMISPHERE;
    }
    let ambient_mode = match mode {
        AmbientMode::Off => 0,
        AmbientMode::Constant | AmbientMode::Day | AmbientMode::Curve => 1,
        AmbientMode::Hemisphere => 2,
    };
    let ctrl = Ctrl {
        bits,
        max_lights: settings.direct.max_lights,
        bounce: settings.bounce.count,
        scale: settings.direct.resolution_scale.max(1),
        bias,
        pixel_only,
        ambient_mode,
        history: u32::from(settings.direct.refresh_frames > 1),
        // 0 selects LIGHTING_SC_TEXEL in the shader, the division the tuned
        // path used before this block existed. 0.125 is that constant.
        shadow_cell: if settings.shadows.cell_metres == 0.125 {
            0.0
        } else {
            settings.shadows.cell_metres
        },
        ambient_r: color[0],
        ambient_g: color[1],
        ambient_b: color[2],
        ambient_i: intensity,
        ground_r: ground[0],
        ground_g: ground[1],
        ground_b: ground[2],
    };
    debug_assert_eq!(std::mem::size_of::<Ctrl>(), 64);
    let mut bytes = [0u8; 64];
    unsafe {
        std::ptr::copy_nonoverlapping(&ctrl as *const Ctrl as *const u8, bytes.as_mut_ptr(), 64);
    }
    bytes
}

pub(crate) struct LightGroups {
    pub groups: Vec<(usize, u32, u32)>,
    pub tags: Vec<u8>,
    pub atrous_stages: Vec<u32>,
    pub counts: [u32; BUCKETS],
    pub pixel_only: u32,
    pub bias: u32,
}

impl LightGroups {
    fn push(&mut self, pass: usize, gx: u32, gy: u32, tag: u8) {
        self.groups.push((pass, gx, gy));
        self.tags.push(tag);
        self.counts[tag as usize] = self.counts[tag as usize].saturating_add(1);
    }
}

/// Threads in the verification stride. Matches the place pass:
/// `period = max(probes / budget, 1)`, probe `% period == frame % period`.
pub(crate) fn wake_verify_threads(probes: u32, budget: u32) -> u32 {
    if budget == 0 || probes == 0 {
        return 0;
    }
    let period = (probes / budget).max(1);
    probes.div_ceil(period)
}

/// The group list for this frame. `probe_ray_count` is probes × rays.
/// `span_kept` is false when no geometry changed, so the trace launch drops the
/// kept-hit and fill tail. `wake_idle` is a rings frame whose strata are already
/// filled and whose queue is empty: the picture trace and the kept-hit direct
/// become the verification stride.
pub(crate) fn light_groups(
    settings: &LightingSettings,
    frame: u32,
    frozen: bool,
    round: bool,
    w: u32,
    h: u32,
    probes: u32,
    probe_ray_count: u32,
    kept: u32,
    wake_rings: bool,
    span_kept: bool,
    wake_idle: bool,
) -> LightGroups {
    let mut out = LightGroups {
        groups: Vec::new(),
        tags: Vec::new(),
        atrous_stages: Vec::new(),
        counts: [0; BUCKETS],
        pixel_only: 0,
        bias: 0,
    };
    let pixels = w.saturating_mul(h);
    let slots = (LIGHTING_CACHE_SLOTS as u32).div_ceil(64);
    // Preset high, this frame, not frozen and not a held frame: the list the
    // pipeline recorded before the settings struct existed.
    let full = settings.direct_this_frame(frame)
        && settings.direct.resolution_scale == 1
        && settings.probes_this_frame(frame)
        && settings.probe_rays() == LIGHTING_RAYS
        && settings.world_mode() == 3;
    // Settled rings matches the legacy idle (pixel direct and compose) and adds
    // the verification stride. The full probe launch was the still/corner cost:
    // the trace counter had already dropped, but the dispatch had not.
    let budget = settings.probes.wake_budget;
    if frozen && wake_rings && settings.gi.enabled && settings.probes.enabled && probes > 0 {
        let n = wake_verify_threads(probes, budget);
        if n > 0 {
            let gx = n.div_ceil(64).max(1);
            out.push(2, gx, LIGHTING_WAKE_VERIFY, B_TRACE);
            out.push(3, gx, LIGHTING_WAKE_VERIFY, B_DIRECT);
        }
        out.push(3, pixels.div_ceil(64), 1, B_DIRECT);
        out.push(7, w.div_ceil(8), h.div_ceil(8), B_COMPOSE);
    } else if frozen && full {
        out.push(3, pixels.div_ceil(64), 1, B_DIRECT);
        out.push(7, w.div_ceil(8), h.div_ceil(8), B_COMPOSE);
    } else if frozen && probe_only(settings) {
        // Still probe-only holds the probes and the cache rays. Pixel direct and
        // compose remain, the same idle a settled surface-cache frame records.
        // High is `full` and took the branch above.
        if settings.direct.enabled {
            let k = settings.direct.resolution_scale.max(1);
            let lit = w.div_ceil(k).saturating_mul(h.div_ceil(k));
            out.push(3, lit.div_ceil(64).max(1), 1, B_DIRECT);
            out.pixel_only = 1;
        }
        out.push(7, w.div_ceil(8).max(1), h.div_ceil(8).max(1), B_COMPOSE);
    } else if !round && full && !settings.cache_round_this_frame(frame) {
        out.push(1, probes.div_ceil(64), 1, B_PLACE);
        out.push(4, probes.div_ceil(64), 1, B_GATHER);
        push_picture_trace(&mut out, 1, probes, probe_ray_count, kept, budget, wake_rings, span_kept, wake_idle);
        push_picture_direct(&mut out, 1, pixels, probes, kept, budget, wake_rings, wake_idle);
        out.push(4, probes, 1, B_GATHER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(7, w.div_ceil(8), h.div_ceil(8), B_COMPOSE);
    } else if round && full && settings.cache_rays_this_frame(frame) && settings.atrous_this_frame(frame) && settings.denoiser.iterations == 5
    {
        out.push(0, slots, 1, B_COMPACT);
        out.push(1, probes.div_ceil(64), 1, B_PLACE);
        out.push(4, probes.div_ceil(64), 1, B_GATHER);
        out.push(0, 1, LIGHTING_ARGS, B_COMPACT);
        push_picture_trace(
            &mut out,
            LIGHTING_MODE1,
            probes,
            probe_ray_count,
            kept,
            budget,
            wake_rings,
            span_kept,
            wake_idle,
        );
        out.push(2, 0, LIGHTING_DENSE_RAYS, B_CACHE_RAYS);
        push_picture_direct(
            &mut out,
            LIGHTING_MODE1,
            pixels,
            probes,
            kept,
            budget,
            wake_rings,
            wake_idle,
        );
        out.push(3, 0, LIGHTING_DENSE_RAYS, B_HIT);
        out.push(4, probes, 1, B_GATHER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(6, 0, LIGHTING_DENSE_ENTRIES, B_CACHE);
        for stage in 0..=6 {
            out.push(8, slots, 1, B_ATROUS);
            out.atrous_stages.push(stage);
        }
        out.push(7, w.div_ceil(8), h.div_ceil(8), B_COMPOSE);
    } else {
        gated(
            settings,
            frame,
            round,
            w,
            h,
            probes,
            probe_ray_count,
            kept,
            wake_rings,
            span_kept,
            wake_idle,
            &mut out,
        );
    }
    if settings.shadows.enabled && (out.counts[B_DIRECT as usize] > 0 || out.counts[B_HIT as usize] > 0) {
        out.counts[B_SHADOWS as usize] = out.counts[B_DIRECT as usize].saturating_add(out.counts[B_HIT as usize]);
    }
    out
}

/// GI with screen probes and no surface cache. A still scene settles onto the
/// pixel-direct idle; the surface cache (mode 3) is not this path.
fn probe_only(settings: &LightingSettings) -> bool {
    settings.gi.enabled
        && settings.bounce.count > 0
        && settings.probes.enabled
        && settings.world_mode() == 0
}

fn push_picture_trace(
    out: &mut LightGroups,
    gy: u32,
    probes: u32,
    probe_ray_count: u32,
    kept: u32,
    budget: u32,
    wake_rings: bool,
    span_kept: bool,
    wake_idle: bool,
) {
    if probes == 0 {
        return;
    }
    if wake_rings && wake_idle {
        let n = wake_verify_threads(probes, budget);
        if n > 0 {
            out.push(2, n.div_ceil(64).max(1), LIGHTING_WAKE_VERIFY, B_TRACE);
        }
        return;
    }
    let threads = if wake_rings && !span_kept {
        probe_ray_count
    } else {
        probe_ray_count.saturating_add(kept.saturating_mul(2))
    };
    out.push(2, threads.div_ceil(64).max(1), gy, B_TRACE);
}

fn push_picture_direct(
    out: &mut LightGroups,
    gy: u32,
    pixels: u32,
    probes: u32,
    kept: u32,
    budget: u32,
    wake_rings: bool,
    wake_idle: bool,
) {
    if wake_rings && wake_idle {
        out.push(3, pixels.div_ceil(64).max(1), gy, B_DIRECT);
        let n = wake_verify_threads(probes, budget);
        if n > 0 {
            out.push(3, n.div_ceil(64).max(1), LIGHTING_WAKE_VERIFY, B_DIRECT);
        }
        return;
    }
    out.push(
        3,
        pixels.saturating_add(kept.saturating_mul(2)).div_ceil(64).max(1),
        gy,
        B_DIRECT,
    );
}

fn gated(
    settings: &LightingSettings,
    frame: u32,
    round: bool,
    w: u32,
    h: u32,
    probes: u32,
    probe_ray_count: u32,
    kept: u32,
    wake_rings: bool,
    span_kept: bool,
    wake_idle: bool,
    out: &mut LightGroups,
) {
    let buckets = settings.gpu_buckets(frame);
    let pixels = w.saturating_mul(h);
    let k = settings.direct.resolution_scale.max(1);
    let lit = w.div_ceil(k).saturating_mul(h.div_ceil(k));
    let slots = (LIGHTING_CACHE_SLOTS as u32).div_ceil(64);
    if !settings.gi.enabled || settings.bounce.count == 0 || k > 1 {
        out.pixel_only = 1;
    }
    if !settings.direct.enabled && settings.gi.enabled {
        out.bias = pixels;
        out.pixel_only = 0;
    }
    if round && buckets.compact {
        out.push(0, slots, 1, B_COMPACT);
    }
    if buckets.place && probes > 0 {
        out.push(1, probes.div_ceil(64), 1, B_PLACE);
    }
    if buckets.gather && probes > 0 {
        out.push(4, probes.div_ceil(64), 1, B_GATHER);
    }
    if round && (buckets.cache_rays || buckets.cache) {
        out.push(0, 1, LIGHTING_ARGS, B_COMPACT);
    }
    if buckets.probe_trace && probes > 0 {
        let gy = if round { LIGHTING_MODE1 } else { 1 };
        push_picture_trace(
            out,
            gy,
            probes,
            probe_ray_count,
            kept,
            settings.probes.wake_budget,
            wake_rings,
            span_kept,
            wake_idle,
        );
    }
    if round && buckets.cache_rays {
        out.push(2, 0, LIGHTING_DENSE_RAYS, B_CACHE_RAYS);
    }
    let combined = buckets.direct && buckets.hit_light && k == 1 && settings.bounce.count > 0;
    if combined {
        let gy = if round { LIGHTING_MODE1 } else { 1 };
        push_picture_direct(
            out,
            gy,
            pixels,
            probes,
            kept,
            settings.probes.wake_budget,
            wake_rings,
            wake_idle,
        );
    } else if buckets.direct {
        out.push(3, lit.div_ceil(64).max(1), 1, B_DIRECT);
        out.pixel_only = 1;
        out.bias = 0;
    }
    // Direct off still lights cache rays and kept hits. The dense dispatch
    // ignores the pixel bias; the kept-hit dispatch uses it. Cache-ray lighting
    // is bounce 2 and up: bounce 1 shades kept hits inside the direct dispatch
    // and does not record this one.
    if round && buckets.cache_rays {
        out.push(3, 0, LIGHTING_DENSE_RAYS, B_HIT);
    }
    if buckets.hit_light && !buckets.direct && kept > 0 {
        out.push(3, (2 * kept).div_ceil(64).max(1), LIGHTING_MODE1, B_HIT);
    }
    if buckets.gather && probes > 0 {
        out.push(4, probes, 1, B_GATHER);
    }
    if buckets.filter && probes > 0 {
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
    }
    if round && buckets.cache {
        out.push(6, 0, LIGHTING_DENSE_ENTRIES, B_CACHE);
    }
    if buckets.atrous {
        let iters = settings.denoiser.iterations;
        out.push(8, slots, 1, B_ATROUS);
        out.atrous_stages.push(0);
        for stage in 1..=iters {
            out.push(8, slots, 1, B_ATROUS);
            out.atrous_stages.push(stage);
        }
        out.push(8, slots, 1, B_ATROUS);
        out.atrous_stages.push(6);
    }
    out.push(7, w.div_ceil(8).max(1), h.div_ceil(8).max(1), B_COMPOSE);
}

/// Sun height 0..1 from a directional lamp. Rays travel along `direction`;
/// the sun is up when that direction points down.
pub(crate) fn sun_height(scene: &genos_scene::Scene) -> f32 {
    let mut best = 0.0f32;
    for light in &scene.lights {
        let d = light.direction;
        let len = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
        if len > 1.0e-4 {
            best = best.max((-d.y / len).max(0.0));
        }
    }
    best
}

#[cfg(test)]
mod probe_still_tests {
    use super::*;
    use genos_scene::LightingPreset;

    fn groups(settings: &LightingSettings, frozen: bool, round: bool) -> LightGroups {
        light_groups(settings, 8, frozen, round, 320, 180, 40, 40 * 64, 0, false, true, false)
    }

    #[test]
    fn a_settled_probe_only_frame_keeps_pixel_direct_and_compose() {
        let mut off = LightingSettings::preset(LightingPreset::High);
        off.world_cache.enabled = false;
        let busy = groups(&off, false, true);
        assert!(busy.counts[B_CACHE_RAYS as usize] > 0, "cache rays");
        assert!(busy.counts[B_HIT as usize] > 0, "hit light");
        let quiet = groups(&off, true, true);
        assert_eq!(quiet.counts[B_CACHE_RAYS as usize], 0);
        assert_eq!(quiet.counts[B_HIT as usize], 0);
        assert_eq!(quiet.counts[B_TRACE as usize], 0);
        assert_eq!(quiet.counts[B_ATROUS as usize], 0);
        assert_eq!(quiet.counts[B_DIRECT as usize], 1);
        assert_eq!(quiet.counts[B_COMPOSE as usize], 1);
        assert_eq!(quiet.pixel_only, 1);
    }

    #[test]
    fn lower_bounce_counts_record_fewer_dispatches() {
        let counts = |bounce: u32| {
            let mut settings = LightingSettings::preset(LightingPreset::High);
            settings.bounce.count = bounce;
            light_groups(&settings, 1, false, true, 320, 180, 40, 40 * 64, 8, false, true, false).counts
        };
        let zero = counts(0);
        let one = counts(1);
        let two = counts(2);
        let sum = |counts: [u32; BUCKETS]| counts.iter().copied().sum::<u32>();
        assert!(
            sum(zero) < sum(one),
            "bounce 0 dispatched {} and bounce 1 dispatched {}",
            sum(zero),
            sum(one)
        );
        assert!(
            sum(one) < sum(two),
            "bounce 1 dispatched {} and bounce 2 dispatched {}",
            sum(one),
            sum(two)
        );
        assert_eq!(zero[B_PLACE as usize], 0);
        assert_eq!(zero[B_TRACE as usize], 0);
        assert_eq!(zero[B_CACHE_RAYS as usize], 0);
        assert_eq!(zero[B_ATROUS as usize], 0);
        assert_eq!(zero[B_HIT as usize], 0);
        assert!(zero[B_DIRECT as usize] > 0);
        assert!(zero[B_COMPOSE as usize] > 0);
        assert_eq!(one[B_CACHE_RAYS as usize], 0);
        assert_eq!(one[B_ATROUS as usize], 0);
        assert_eq!(one[B_HIT as usize], 0);
        assert!(one[B_TRACE as usize] > 0);
        assert!(two[B_CACHE_RAYS as usize] > 0);
        assert!(two[B_ATROUS as usize] > 0);
        assert!(two[B_HIT as usize] > 0);
    }

    #[test]
    fn a_settled_high_frame_stays_on_the_tuned_idle_list() {
        let high = LightingSettings::preset(LightingPreset::High);
        let busy = groups(&high, false, true);
        assert!(busy.counts[B_CACHE_RAYS as usize] > 0);
        assert!(busy.counts[B_ATROUS as usize] > 0);
        let quiet = groups(&high, true, false);
        assert_eq!(quiet.pixel_only, 0);
        assert_eq!(quiet.counts[B_DIRECT as usize], 1);
        assert_eq!(quiet.counts[B_COMPOSE as usize], 1);
        assert_eq!(quiet.counts[B_CACHE_RAYS as usize], 0);
        assert_eq!(quiet.counts[B_ATROUS as usize], 0);
        assert_eq!(quiet.groups.len(), 2);
    }

    #[test]
    fn settled_rings_frame_matches_legacy_idle_plus_the_budget() {
        let high = LightingSettings::preset(LightingPreset::High);
        let probes = 40u32;
        let quiet = light_groups(
            &high, 8, true, false, 320, 180, probes, probes * 64, probes * 64 * 8, true, false, false,
        );
        let legacy = light_groups(
            &high, 8, true, false, 320, 180, probes, probes * 64, 0, false, true, false,
        );
        assert_eq!(legacy.groups.len(), 2);
        assert_eq!(quiet.counts[B_PLACE as usize], 0, "place");
        assert_eq!(quiet.counts[B_GATHER as usize], 0, "gather");
        assert_eq!(quiet.counts[B_FILTER as usize], 0, "filter");
        assert_eq!(quiet.counts[B_CACHE_RAYS as usize], 0, "the cache stays idle");
        assert_eq!(quiet.counts[B_ATROUS as usize], 0, "atrous");
        assert_eq!(quiet.counts[B_COMPOSE as usize], 1);
        assert_eq!(quiet.counts[B_TRACE as usize], 1, "one verification trace");
        assert_eq!(quiet.counts[B_DIRECT as usize], 2, "budget direct plus pixel direct");
        let verify = wake_verify_threads(probes, high.probes.wake_budget);
        assert!(verify <= probes / 2, "stride {verify} of {probes}");
        let trace = quiet.groups.iter().find(|(pass, _, _)| *pass == 2).unwrap();
        assert_eq!(trace.1, verify.div_ceil(64).max(1));
        assert_eq!(trace.2, super::LIGHTING_WAKE_VERIFY);
        // The pixel direct matches legacy's only direct dispatch.
        assert_eq!(quiet.groups.last().unwrap().0, 7);
        assert!(quiet
            .groups
            .iter()
            .any(|(pass, gx, gy)| *pass == 3 && *gy == 1 && *gx == legacy.groups[0].1));
    }

    #[test]
    fn rings_idle_frame_drops_the_kept_span() {
        let high = LightingSettings::preset(LightingPreset::High);
        let probes = 120u32 * 68;
        let rays = probes * 64;
        let kept = rays * 8;
        let full = light_groups(
            &high, 8, false, true, 1920, 1080, probes, rays, kept, true, true, false,
        );
        let slim = light_groups(
            &high, 8, false, true, 1920, 1080, probes, rays, kept, true, false, false,
        );
        let idle = light_groups(
            &high, 8, false, true, 1920, 1080, probes, rays, kept, true, false, true,
        );
        let picture_trace = |g: &LightGroups| -> u32 {
            g.groups
                .iter()
                .find(|(pass, _, gy)| *pass == 2 && *gy == super::LIGHTING_MODE1)
                .map(|(_, gx, _)| *gx)
                .unwrap_or(0)
        };
        assert_eq!(picture_trace(&full), (rays + 2 * kept).div_ceil(64));
        assert_eq!(picture_trace(&slim), rays.div_ceil(64));
        assert!(picture_trace(&slim) * 8 < picture_trace(&full));
        assert_eq!(picture_trace(&idle), 0, "idle trace is the verification kind");
        let idle_trace = idle.groups.iter().find(|(pass, _, gy)| *pass == 2 && *gy == super::LIGHTING_WAKE_VERIFY).unwrap();
        let n = wake_verify_threads(probes, high.probes.wake_budget);
        assert_eq!(idle_trace.1, n.div_ceil(64).max(1));
        assert!(n < probes / 8, "stride {n} of {probes}");
        assert!(idle.counts[B_ATROUS as usize] > 0, "a cache round still filters");
        assert_eq!(idle.counts[B_CACHE_RAYS as usize], full.counts[B_CACHE_RAYS as usize]);
        assert!(idle.counts[B_DIRECT as usize] > full.counts[B_DIRECT as usize]);
        let kept_direct = (1920u32 * 1080 + 2 * kept).div_ceil(64);
        assert!(idle.groups.iter().all(|(pass, gx, _)| *pass != 3 || *gx < kept_direct));
    }
}
