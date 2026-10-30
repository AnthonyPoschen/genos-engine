// Which lighting dispatches a LightingSettings records.
//
// The tuned path (preset high, every feature on, this frame due) builds the
// same group list the frame recorded before settings existed. Anything else
// drops the dispatches the disabled feature owned.

use genos_scene::{
    AmbientMode, LampPickMode, LightingSettings, ShadowFilter, LIGHTING_BOUNCES_UNLIMITED,
};

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

/// One frame of GI pass time and work, after the fence for that frame.
///
/// `resolve_ms` is gather + filter + compose (the screen resolve). It is not
/// added on top of those passes. `lighting_ms` leaves out `shadows`, which is
/// the same dispatches as direct and hit light. `trace_ms` is screen-probe
/// trace plus cache rays. `direct_ms` is the direct dispatch plus hit light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiSample {
    pub seq: u64,
    pub direct_ms: f64,
    pub atrous_ms: f64,
    pub compose_ms: f64,
    pub trace_ms: f64,
    pub filter_ms: f64,
    pub gather_ms: f64,
    pub resolve_ms: f64,
    pub lighting_ms: f64,
    pub rays: u32,
    pub cache_entries_refreshed: u32,
    pub lamps_shaded: u32,
}

/// Fold bucket milliseconds and the stats words into one [`GiSample`].
///
/// `world_rays` and `world_relit` are a finished async cache round that has
/// not been counted yet. Screen-probe rays are stats word 6. Cache rays are
/// word 16. Relit entries are word 1. Lamps shaded are the fresh shadow rays
/// in the lamp-pick diag words (pixels, kept hits, fills, cache rays).
pub fn gi_sample_from(
    seq: u64,
    bucket: &[f64; BUCKETS],
    stats: &[u32],
    world_rays: u32,
    world_relit: u32,
) -> GiSample {
    let gather = bucket[B_GATHER as usize];
    let filter = bucket[B_FILTER as usize];
    let compose = bucket[B_COMPOSE as usize];
    let word = |i: usize| stats.get(i).copied().unwrap_or(0);
    let lamps = word(LAMP_PICK_DIAG_WORD)
        .saturating_add(word(LAMP_PICK_DIAG_WORD + 1))
        .saturating_add(word(LAMP_PICK_DIAG_WORD + 2))
        .saturating_add(word(LAMP_PICK_DIAG_WORD + 3));
    GiSample {
        seq,
        direct_ms: bucket[B_DIRECT as usize] + bucket[B_HIT as usize],
        atrous_ms: bucket[B_ATROUS as usize],
        compose_ms: compose,
        trace_ms: bucket[B_TRACE as usize] + bucket[B_CACHE_RAYS as usize],
        filter_ms: filter,
        gather_ms: gather,
        resolve_ms: gather + filter + compose,
        lighting_ms: bucket.iter().sum::<f64>() - bucket[B_SHADOWS as usize],
        rays: word(6).saturating_add(word(16)).saturating_add(world_rays),
        cache_entries_refreshed: word(1).saturating_add(world_relit),
        lamps_shaded: lamps,
    }
}

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
/// Screen wavelet over visible fine texels. Bit 19. Bits 19 and 21 were
/// half-res and tiles (S79); 19 is this filter now. Bit 21 stays free.
const CTRL_DENOISE_SCREEN: u32 = 1 << 19;
/// Skip later wavelet iterations on stable texels. Bit 20. Off unless the
/// experimental knob is on.
const CTRL_DENOISE_ADAPTIVE: u32 = 1 << 20;
/// Relative variance threshold in bits 22..31, in steps of 0.0001.
const CTRL_DENOISE_VAR_SHIFT: u32 = 22;

/// What a buffer rebuild depends on. Screen size is compared beside this.
pub(crate) fn lighting_alloc_key(settings: &LightingSettings) -> (u32, u32, u32, u32, u32, u32, u32) {
    let (cache, probes, rays, tile) = settings.buffer_class();
    (
        cache,
        probes,
        rays,
        tile,
        u32::from(settings.direct.refresh_frames > 1),
        u32::from(settings.denoiser.enabled && settings.denoiser.screen),
        // Bit 0: reservoirs are allocated. Bit 1: a second slot for spatial reuse.
        // Radius 0 is one slot (temporal only).
        u32::from(settings.lamp_pick.reuse()) | (u32::from(settings.lamp_pick.radius > 0) << 1),
    )
}

/// Which shade the noise gate asks for. Production is [`Product`](Self::Product).
///
/// The bits land in `pc.lamp.y`. They are not a saved setting: a game does not
/// persist a fault injection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LampShadeGate {
    /// The preset's shade. A quiet held view can take the exact sum and hold.
    #[default]
    Product,
    /// Every point lamp, inside the reuse shade. The gate's baseline.
    Exact,
    /// The random path, with the exact-sum hold off, on a still view too.
    Random,
    /// Uniform pick and shade weight 1. No pairwise weight. The gate must fail.
    Broken,
}

/// Bits 18–20 of `pc.lamp.y`. Combined with [`lamp_pick_push`].
pub(crate) fn lamp_shade_bits(gate: LampShadeGate) -> u32 {
    match gate {
        LampShadeGate::Product => 0,
        LampShadeGate::Exact => 1 << 19,
        LampShadeGate::Random => 1 << 18,
        // Hold off as well, or a still view would shade the exact sum and pass.
        LampShadeGate::Broken => (1 << 18) | (1 << 20),
    }
}

/// Point lamps at or under this count shade the exact sum.
///
/// Matches `LAMP_PICK_EXACT_LIT` in `lighting_lamp_pick.glsl`.
const DIRECT_PLAIN_LAMPS: u32 = 8;

/// Bind the pre-reservoir direct shade.
///
/// Off traces every lamp. Reuse of at most [`DIRECT_PLAIN_LAMPS`] point lamps
/// does too, then holds on a matching signature. The reservoir module's
/// register file was the direct-pass cost on that path. The noise gate stays
/// on the reservoir module so a still view can grade the random path. Hits,
/// pixels, and both stay there: those modes are the lamp-pick tests.
pub(crate) fn direct_shade_plain(mode: LampPickMode, point_lamps: u32, gate: LampShadeGate) -> bool {
    if gate != LampShadeGate::Product {
        return false;
    }
    match mode {
        LampPickMode::Off => true,
        LampPickMode::Reuse => point_lamps <= DIRECT_PLAIN_LAMPS,
        LampPickMode::Hits | LampPickMode::Pixels | LampPickMode::Both => false,
    }
}

/// Push words for `pc.lamp`. Zero keeps the legacy shade loop.
///
/// `GENOS_LIGHTING_LAMP_PICK_DIAG` sets bit 17 so the shade loop counts shadow
/// rays. The mode stays off when the setting is off, and the counters still run.
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
pub(crate) fn lamp_pick_push(settings: &LightingSettings) -> [u32; 4] {
    let diag = u32::from(genos_scene::lighting_env_os("GENOS_LIGHTING_LAMP_PICK_DIAG").is_some());
    let mode = settings.lamp_pick.mode as u32;
    if mode == 0 && diag == 0 {
        return [0; 4];
    }
    let y = (settings.lamp_pick.candidates & 0xff)
        | ((settings.lamp_pick.rays & 0xff) << 8)
        | (u32::from(settings.lamp_pick.unbiased) << 16)
        | (diag << 17);
    [
        mode,
        y,
        settings.lamp_pick.radius,
        settings.lamp_pick.history,
    ]
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
    // The cache and the work buffer stay 4096-byte stand-ins unless probes or
    // the world cache allocated them. The GI bit turns on pixel reuse, which
    // indexes the work buffer at the cache-ray offset, and bounce reads index
    // the cache. Lavapipe faults on either read, so both stay off until the
    // buffers exist. Bounce 0 records no GI dispatch. The settings struct
    // still records the request.
    let cache = settings.allocates_cache();
    if settings.gi.enabled && settings.bounce.count > 0 && cache {
        bits |= CTRL_GI;
        bits |= CTRL_BOUNCE_READS;
    }
    if settings.bounce.count == LIGHTING_BOUNCES_UNLIMITED && cache {
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
    if settings.denoiser.adaptive {
        bits |= CTRL_DENOISE_ADAPTIVE;
    }
    if settings.denoiser.enabled && settings.denoiser.screen {
        bits |= CTRL_DENOISE_SCREEN;
    }
    let variance_steps = (settings.denoiser.variance_threshold * 10000.0)
        .round()
        .clamp(0.0, 1023.0) as u32;
    bits |= variance_steps << CTRL_DENOISE_VAR_SHIFT;
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
        bounce: if cache { settings.bounce.count } else { 0 },
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

/// The group list for this frame. `probe_ray_count` is probes × rays.
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
    if frozen && full {
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
        out.push(2, (probe_ray_count + 2 * kept).div_ceil(64), 1, B_TRACE);
        out.push(3, (pixels + 2 * kept).div_ceil(64), 1, B_DIRECT);
        out.push(4, probes, 1, B_GATHER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(7, w.div_ceil(8), h.div_ceil(8), B_COMPOSE);
    } else if round
        && full
        && settings.cache_rays_this_frame(frame)
        && settings.atrous_this_frame(frame)
        && settings.denoiser.iterations == 5
    {
        out.push(0, slots, 1, B_COMPACT);
        out.push(1, probes.div_ceil(64), 1, B_PLACE);
        out.push(4, probes.div_ceil(64), 1, B_GATHER);
        out.push(0, 1, LIGHTING_ARGS, B_COMPACT);
        out.push(
            2,
            (probe_ray_count + 2 * kept).div_ceil(64),
            LIGHTING_MODE1,
            B_TRACE,
        );
        out.push(2, 0, LIGHTING_DENSE_RAYS, B_CACHE_RAYS);
        out.push(
            3,
            (pixels + 2 * kept).div_ceil(64),
            LIGHTING_MODE1,
            B_DIRECT,
        );
        out.push(3, 0, LIGHTING_DENSE_RAYS, B_HIT);
        out.push(4, probes, 1, B_GATHER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(5, probes.div_ceil(64), 1, B_FILTER);
        out.push(6, 0, LIGHTING_DENSE_ENTRIES, B_CACHE);
        push_denoise(settings, w, h, slots, &mut out);
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
            &mut out,
        );
    }
    if settings.shadows.enabled
        && (out.counts[B_DIRECT as usize] > 0 || out.counts[B_HIT as usize] > 0)
    {
        out.counts[B_SHADOWS as usize] =
            out.counts[B_DIRECT as usize].saturating_add(out.counts[B_HIT as usize]);
    }
    out
}

/// Screen wavelet: one 8×8 group per picture block. Hash wavelet: one group
/// per 64 cache slots.
///
/// Screen stages: 0 stores the visible slot, 1 picks one pixel per cell and
/// runs the hash variance, then one stage per wavelet iteration, then the
/// push. The hash stages stay 0, the iterations, and 6.
fn push_denoise(settings: &LightingSettings, w: u32, h: u32, slots: u32, out: &mut LightGroups) {
    let iters = settings.denoiser.iterations;
    let (gx, gy) = if settings.denoiser.screen {
        (w.div_ceil(8).max(1), h.div_ceil(8).max(1))
    } else {
        (slots, 1)
    };
    if settings.denoiser.screen {
        out.push(8, gx, gy, B_ATROUS);
        out.atrous_stages.push(0);
        out.push(8, gx, gy, B_ATROUS);
        out.atrous_stages.push(1);
        for i in 0..iters {
            out.push(8, gx, gy, B_ATROUS);
            out.atrous_stages.push(2 + i);
        }
        out.push(8, gx, gy, B_ATROUS);
        out.atrous_stages.push(2 + iters);
        return;
    }
    out.push(8, gx, gy, B_ATROUS);
    out.atrous_stages.push(0);
    for stage in 1..=iters {
        out.push(8, gx, gy, B_ATROUS);
        out.atrous_stages.push(stage);
    }
    out.push(8, gx, gy, B_ATROUS);
    out.atrous_stages.push(6);
}

/// GI with screen probes and no surface cache. A still scene settles onto the
/// pixel-direct idle; the surface cache (mode 3) is not this path.
fn probe_only(settings: &LightingSettings) -> bool {
    settings.gi.enabled
        && settings.bounce.count > 0
        && settings.probes.enabled
        && settings.world_mode() == 0
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
        out.push(2, (probe_ray_count + 2 * kept).div_ceil(64), gy, B_TRACE);
    }
    if round && buckets.cache_rays {
        out.push(2, 0, LIGHTING_DENSE_RAYS, B_CACHE_RAYS);
    }
    let combined = buckets.direct && buckets.hit_light && k == 1 && settings.bounce.count > 0;
    if combined {
        let gy = if round { LIGHTING_MODE1 } else { 1 };
        out.push(3, (pixels + 2 * kept).div_ceil(64), gy, B_DIRECT);
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
        push_denoise(settings, w, h, slots, out);
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
        light_groups(settings, 8, frozen, round, 320, 180, 40, 40 * 64, 0)
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
    fn gi_without_a_cache_does_not_ask_the_shader_to_read_one() {
        let mut settings = LightingSettings::preset(LightingPreset::Fallback);
        settings.gi.enabled = true;
        settings.bounce.count = u32::MAX;
        let bytes = pack_lighting_ctrl(&settings, 0, 1.0, 0, 0);
        let bits = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let bounce = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(bounce, 0, "no cache, so the shader must not look one up");
        assert_eq!(bits & CTRL_GI, 0, "no cache, so the shader must not index one");
        assert_eq!(bits & CTRL_BOUNCE_UNLIMITED, 0);
        assert_eq!(bits & CTRL_BOUNCE_READS, 0);
        let high = pack_lighting_ctrl(&LightingSettings::preset(LightingPreset::High), 0, 1.0, 0, 0);
        let high_bits = u32::from_le_bytes(high[0..4].try_into().unwrap());
        let high_bounce = u32::from_le_bytes(high[8..12].try_into().unwrap());
        assert_eq!(high_bounce, u32::MAX);
        assert_ne!(high_bits & CTRL_GI, 0);
        assert_ne!(high_bits & CTRL_BOUNCE_UNLIMITED, 0);
    }

    #[test]
    fn lower_bounce_counts_record_fewer_dispatches() {
        let counts = |bounce: u32| {
            let mut settings = LightingSettings::preset(LightingPreset::High);
            settings.bounce.count = bounce;
            light_groups(&settings, 1, false, true, 320, 180, 40, 40 * 64, 8).counts
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
    fn screen_denoise_launches_the_picture_and_hash_keeps_the_slots() {
        let slots = (LIGHTING_CACHE_SLOTS as u32).div_ceil(64);
        let high = LightingSettings::preset(LightingPreset::High);
        assert!(high.denoiser.screen, "high defaults to the screen launch");
        let screen_groups = groups(&high, false, true);
        let picture: Vec<_> = screen_groups
            .groups
            .iter()
            .filter(|(pass, _, _)| *pass == 8)
            .collect();
        assert_eq!(picture.len(), 8, "gather, variance, five iterations, push");
        assert_eq!(screen_groups.atrous_stages, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        for (pass, gx, gy) in &picture {
            assert_eq!(*pass, 8);
            assert_eq!(*gx, 320u32.div_ceil(8));
            assert_eq!(*gy, 180u32.div_ceil(8));
            assert_ne!(*gx, slots);
        }
        let mut hash_settings = high.clone();
        hash_settings
            .set_key("denoise_launch", "hash")
            .expect("denoise_launch");
        let hash = groups(&hash_settings, false, true);
        let hash_atrous: Vec<_> = hash
            .groups
            .iter()
            .filter(|(pass, _, _)| *pass == 8)
            .collect();
        assert_eq!(hash_atrous.len(), 7, "the launch key returns to hash slots");
        assert_eq!(hash.atrous_stages, vec![0, 1, 2, 3, 4, 5, 6]);
        for (_, gx, gy) in hash_atrous {
            assert_eq!(*gx, slots);
            assert_eq!(*gy, 1);
        }
        let medium = groups(
            &LightingSettings::preset(LightingPreset::Medium),
            false,
            true,
        );
        let medium_atrous: Vec<_> = medium
            .groups
            .iter()
            .filter(|(pass, _, _)| *pass == 8)
            .collect();
        assert_eq!(medium_atrous.len(), 5, "variance, three iterations, push");
        for (_, gx, gy) in medium_atrous {
            assert_eq!(*gx, slots);
            assert_eq!(*gy, 1);
        }
        let bytes = pack_lighting_ctrl(&high, 0, 1.0, 0, 0);
        let bits = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        assert_ne!(bits & CTRL_DENOISE_SCREEN, 0);
        let off = pack_lighting_ctrl(&hash_settings, 0, 1.0, 0, 0);
        let off_bits = u32::from_le_bytes(off[0..4].try_into().unwrap());
        assert_eq!(off_bits & CTRL_DENOISE_SCREEN, 0);
    }

    #[test]
    fn gi_sample_folds_passes_without_double_counting_shadows() {
        let mut bucket = [0.0; BUCKETS];
        bucket[B_DIRECT as usize] = 1.0;
        bucket[B_HIT as usize] = 0.25;
        bucket[B_SHADOWS as usize] = 1.25;
        bucket[B_ATROUS as usize] = 2.0;
        bucket[B_COMPOSE as usize] = 0.4;
        bucket[B_TRACE as usize] = 0.3;
        bucket[B_CACHE_RAYS as usize] = 0.2;
        bucket[B_FILTER as usize] = 0.1;
        bucket[B_GATHER as usize] = 0.05;
        bucket[B_CACHE as usize] = 0.07;
        let mut stats = vec![0u32; LAMP_PICK_DIAG_WORD + 4];
        stats[1] = 7;
        stats[6] = 100;
        stats[16] = 50;
        stats[LAMP_PICK_DIAG_WORD] = 3;
        stats[LAMP_PICK_DIAG_WORD + 1] = 4;
        stats[LAMP_PICK_DIAG_WORD + 2] = 5;
        stats[LAMP_PICK_DIAG_WORD + 3] = 6;
        let sample = gi_sample_from(4, &bucket, &stats, 10, 2);
        assert!((sample.direct_ms - 1.25).abs() < 1e-9);
        assert!((sample.atrous_ms - 2.0).abs() < 1e-9);
        assert!((sample.compose_ms - 0.4).abs() < 1e-9);
        assert!((sample.trace_ms - 0.5).abs() < 1e-9);
        assert!((sample.filter_ms - 0.1).abs() < 1e-9);
        assert!((sample.gather_ms - 0.05).abs() < 1e-9);
        assert!((sample.resolve_ms - 0.55).abs() < 1e-9);
        let parts = sample.direct_ms
            + sample.atrous_ms
            + sample.trace_ms
            + sample.gather_ms
            + sample.filter_ms
            + sample.compose_ms
            + bucket[B_CACHE as usize];
        assert!((sample.lighting_ms - parts).abs() < 1e-9);
        assert!((sample.lighting_ms - (parts + sample.resolve_ms)).abs() > 0.1);
        assert_eq!(sample.rays, 160);
        assert_eq!(sample.cache_entries_refreshed, 9);
        assert_eq!(sample.lamps_shaded, 18);
        assert_eq!(sample.seq, 4);
    }

    #[test]
    fn few_lamp_reuse_binds_the_pre_reservoir_shade() {
        use genos_scene::LampPickMode;
        assert!(direct_shade_plain(LampPickMode::Off, 5, LampShadeGate::Product));
        assert!(direct_shade_plain(LampPickMode::Reuse, 5, LampShadeGate::Product));
        assert!(direct_shade_plain(LampPickMode::Reuse, 8, LampShadeGate::Product));
        assert!(!direct_shade_plain(LampPickMode::Reuse, 9, LampShadeGate::Product));
        assert!(!direct_shade_plain(LampPickMode::Reuse, 16, LampShadeGate::Product));
        assert!(!direct_shade_plain(LampPickMode::Reuse, 5, LampShadeGate::Random));
        assert!(!direct_shade_plain(LampPickMode::Reuse, 5, LampShadeGate::Exact));
        assert!(!direct_shade_plain(LampPickMode::Reuse, 5, LampShadeGate::Broken));
        assert!(!direct_shade_plain(LampPickMode::Pixels, 3, LampShadeGate::Product));
        assert!(!direct_shade_plain(LampPickMode::Both, 3, LampShadeGate::Product));
        assert!(!direct_shade_plain(LampPickMode::Hits, 3, LampShadeGate::Product));
    }
}
