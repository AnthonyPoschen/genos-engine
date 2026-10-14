//! Lighting settings: the reference [`crate::EngineSettings`] value.
//!
//! A game, a scene file, a script, MCP, and the debug menu edit this one struct.
//!
//! The renderer reads this and skips the GPU passes and allocations a disabled
//! feature owns. A quality knob changes how much work a feature does when it is
//! on. Presets fill the struct. The text form is what scene files, launch
//! arguments, and `GENOS_LIGHTING_*` environment variables share with the debug
//! menu. `--preset`, `--bounces`, `--ambient`, and `--time-of-day` win. Each
//! matching variable applies only when that argument was omitted.
//!
//! Defaults match the tuned GI path (preset `high`): every feature on, unlimited
//! bounces, 64 rays per probe, the surface cache, five a-trous iterations.

use std::fmt::Write as _;

/// Bounce count meaning the cache may feed itself without a cap (today's GI).
pub const LIGHTING_BOUNCES_UNLIMITED: u32 = u32::MAX;

/// Named quality tiers. Each one is a full [`LightingSettings`] value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightingPreset {
    /// Direct light, traced shadows, and a flat ambient. No GI buffers.
    Potato,
    /// Direct light, shadows, and the time-of-day ambient curve. No GI.
    DayCycle,
    /// One bounce of screen-probe GI. No surface cache, denoiser, or prefill.
    Gi1Bounce,
    /// Probe GI, one bounce, coarse tile, refresh every other frame.
    Low,
    /// Surface cache, two bounces, half the ray budget shift.
    Medium,
    /// The tuned default: unlimited bounces, 64 rays, five denoiser iterations.
    High,
}

impl LightingPreset {
    pub fn name(self) -> &'static str {
        match self {
            Self::Potato => "potato",
            Self::DayCycle => "day-cycle",
            Self::Gi1Bounce => "gi-1bounce",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "potato" => Some(Self::Potato),
            "day-cycle" | "day_cycle" => Some(Self::DayCycle),
            "gi-1bounce" | "gi_1bounce" => Some(Self::Gi1Bounce),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub fn all() -> &'static [LightingPreset] {
        &[
            Self::Potato,
            Self::DayCycle,
            Self::Gi1Bounce,
            Self::Low,
            Self::Medium,
            Self::High,
        ]
    }
}

/// How the flat fill is shaped when GI is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbientMode {
    /// No flat fill. With GI off the picture is direct light only.
    Off,
    /// One colour times intensity.
    Constant,
    /// Sky colour toward the normal, ground colour away from it.
    Hemisphere,
    /// Constant colour and intensity keyed by the sun's height.
    Day,
    /// Flat colour and intensity keyed by the time of day: night, dawn, noon, dusk.
    Curve,
}

impl AmbientMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Constant => "constant",
            Self::Hemisphere => "hemisphere",
            Self::Day => "day",
            Self::Curve => "curve",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "0" => Some(Self::Off),
            "constant" | "flat" => Some(Self::Constant),
            "hemisphere" | "hemi" => Some(Self::Hemisphere),
            "day" | "day_night" => Some(Self::Day),
            "curve" => Some(Self::Curve),
            _ => None,
        }
    }
}

/// Shadow rays inside the direct pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowFilter {
    /// Trace a shadow ray every time. No shadow-cell cache.
    Hard,
    /// Reuse visibility inside a world cell (the 12.5 cm page).
    Cell,
}

impl ShadowFilter {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hard => "hard",
            Self::Cell => "cell",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "hard" | "0" | "traced" => Some(Self::Hard),
            "cell" | "1" | "cached" => Some(Self::Cell),
            _ => None,
        }
    }
}

/// Which points sample point lamps instead of tracing every one.
///
/// The sun is never a sample. It is traced on its own. GI presets use
/// [`Hits`](Self::Hits). Potato and day-cycle stay [`Off`](Self::Off).
/// [`Reuse`](Self::Reuse) is experimental and is not a preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum LampPickMode {
    /// Every lamp. Potato and day-cycle.
    Off = 0,
    /// Bounce hits, probe fills, and cache-ray hits. Pixels stay exact.
    Hits = 1,
    /// Primary pixels only. No previous frame. Diagnostic.
    Pixels = 2,
    /// Hits and pixels, no reservoir.
    Both = 3,
    /// Experimental. Hits, plus pixels with a temporal reservoir.
    /// Spatial only if the radius is above 0. Not used by a preset.
    Reuse = 4,
}

impl LampPickMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Hits => "hits",
            Self::Pixels => "pixels",
            Self::Both => "both",
            Self::Reuse => "reuse (experimental)",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "0" => Some(Self::Off),
            "hits" | "1" => Some(Self::Hits),
            "pixels" | "2" => Some(Self::Pixels),
            "both" | "3" => Some(Self::Both),
            "reuse" | "4" | "reuse (experimental)" => Some(Self::Reuse),
            _ => None,
        }
    }
}

/// Weighted lamp picking (Bitterli et al. 2020, Talbot et al. 2005).
///
/// `rays` is the shadow-ray count (samples per shaded point). `candidates` is
/// the unshadowed reservoir fill. They are not the same knob: a candidate is
/// not a ray. `direct.max_lights` stays a hard cap and does not read these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LampPickSettings {
    pub mode: LampPickMode,
    /// Unshadowed candidates feeding a pixel reservoir. 1..=32. Default 8.
    pub candidates: u32,
    /// Shadow rays per shaded point. 1..=4. Default 1. Also `lamp_pick_samples`.
    pub rays: u32,
    /// Spatial neighbour radius in pixels. 0..=64. Default 0 (no spatial merge).
    pub radius: u32,
    /// Cap on reservoir `M` for the biased merge. 1..=64. Default 20.
    /// The unbiased path does not apply it.
    pub history: u32,
    /// Algorithm 6 when true. The section-3 merge and the `M` clamp when false.
    pub unbiased: bool,
}

impl Default for LampPickSettings {
    fn default() -> Self {
        Self {
            mode: LampPickMode::Off,
            candidates: 8,
            rays: 1,
            radius: 0,
            history: 20,
            unbiased: true,
        }
    }
}

impl LampPickSettings {
    pub fn reuse(self) -> bool {
        self.mode == LampPickMode::Reuse
    }
}

/// Direct lighting of pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectSettings {
    pub enabled: bool,
    /// Lamps shaded per pixel. 0 means every lamp that reaches the point.
    pub max_lights: u32,
    /// Recompute every Nth frame. 1 is every frame. Older frames reuse the stored direct.
    pub refresh_frames: u32,
    /// Pixel step. 1 lights every pixel. 2 lights every other pixel; compose copies it across.
    pub resolution_scale: u32,
}

/// Traced shadows. They share the direct dispatch; turning them off skips the
/// shadow-ray trace site and the shadow-cell cache, and records no shadow GPU time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSettings {
    pub enabled: bool,
    pub filter: ShadowFilter,
    /// World size of one shadow cell, metres. The cell filter reads this.
    pub cell_metres: f32,
    /// Re-trace a cached cell every Nth frame. 1 traces whenever the cell misses.
    pub refresh_frames: u32,
}

/// How a surface-cache entry blends a finished set into its published mean.
///
/// `Msme` is the multi-scale mean estimator in Barré-Brisebois et al.,
/// Ray Tracing Gems ch. 25, `MultiscaleMeanEstimator.hlsl` (short mean, short
/// variance, long mean). `Hold` is the older path: a short window while the
/// light changes, then a snap back to the new set once it is quiet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceCacheBlend {
    Msme,
    Hold,
}

impl SurfaceCacheBlend {
    pub fn name(self) -> &'static str {
        match self {
            Self::Msme => "msme",
            Self::Hold => "hold",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "msme" => Some(Self::Msme),
            "hold" => Some(Self::Hold),
            _ => None,
        }
    }
}

/// Global illumination as a whole. Off skips every GI pass and the GI buffers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiSettings {
    pub enabled: bool,
    /// Run a cache round every Nth frame. 1 is every frame.
    pub refresh_frames: u32,
    /// Divide per-round ray budgets by 2^shift (0..7). The existing cost knob.
    pub budget_shift: u32,
    /// `Some` forces the world mode (0 probes, 3 surface cache). `None` follows
    /// [`WorldCacheSettings::enabled`].
    pub world_mode: Option<u32>,
}

/// Indirect bounces. The count is `0`, `1`, `2`, or [`LIGHTING_BOUNCES_UNLIMITED`].
///
/// `0` is direct light plus the flat ambient fill. No GI dispatch is recorded.
/// `1` is one bounce: screen probes trace and their hits are shaded, and the
/// surface-cache, cache-ray, and a-trous dispatches are not recorded. `2` allows
/// cache-to-cache and records those dispatches. Unlimited is the tuned path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BounceSettings {
    pub count: u32,
}

/// Screen probes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeSettings {
    pub enabled: bool,
    /// Rays traced per probe per frame. Clamped to 4..=64. 0 keeps the default 64.
    pub rays: u32,
    /// Probe tile in pixels. 0 keeps the picture-height rule (16 px from 720p up).
    pub tile: u32,
    /// Place, trace, gather and filter every Nth frame.
    pub refresh_frames: u32,
    /// Frames a kept hit may stand before it is treated as stale.
    pub staleness_frames: u32,
}

/// Surface cache and world radiance cache (world mode 3).
/// Off selects the probe path (world mode 0) and skips the a-trous filter,
/// which only filters the surface cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldCacheSettings {
    pub enabled: bool,
    /// World cell of the radiance cache, metres.
    pub cell_metres: f32,
    /// Refresh a stale cell every Nth frame.
    pub refresh_frames: u32,
    /// Frames before a cell is stale.
    pub staleness_frames: u32,
}

/// SVGF a-trous filter of the surface cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiserSettings {
    pub enabled: bool,
    /// A-trous iterations. 1, 3, or 5. The push reads the last iteration.
    pub iterations: u32,
    /// Dirty-list fraction before the dense sweep. 0 is always dense, 1 keeps lists.
    pub dirty_fraction: f32,
    /// Drop later wavelet iterations where the texel's history is stable.
    /// Off unless the experimental knob is on. See S79.
    pub adaptive: bool,
    /// Relative luminance deviation `sqrt(variance) / mean` under which a texel
    /// counts as stable. 0 leaves the decision to sample age. Ignored unless
    /// adaptive is on.
    pub variance_threshold: f32,
}

impl DenoiserSettings {
    /// Full-resolution scalar wavelet, adaptive off.
    ///
    /// That is the gi-v2 kernel. It has its own pipeline so the adaptive branch
    /// is not in the shader the 4070 runs for high.
    pub fn plain(self) -> bool {
        !self.adaptive
    }
}

/// Flat or hemisphere fill used when GI is off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientSettings {
    pub mode: AmbientMode,
    pub color: [f32; 3],
    pub intensity: f32,
    /// Hemisphere ground colour. Zenith uses [`Self::color`].
    pub ground: [f32; 3],
    /// Night end of the day curve (sun below the horizon).
    pub night_color: [f32; 3],
    pub night_intensity: f32,
    /// Day end of the sun-height lerp (sun overhead).
    pub day_color: [f32; 3],
    pub day_intensity: f32,
    /// Dawn key of the time-of-day curve (06:00).
    pub dawn_color: [f32; 3],
    pub dawn_intensity: f32,
    /// Noon key (12:00). White at intensity 1 leaves the sky colour unchanged.
    pub noon_color: [f32; 3],
    pub noon_intensity: f32,
    /// Dusk key (18:00).
    pub dusk_color: [f32; 3],
    pub dusk_intensity: f32,
}

/// Surface-cache knobs. These are not the world-cache knobs.
///
/// A cache entry is one hashed surface patch. `slot` is only its storage index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceCacheSettings {
    /// Cache entries on a moving object are stored in that object's frame and
    /// transformed each frame. Static geometry stays on the world lattice.
    /// Off keeps those entries world-anchored.
    pub attach_movers: bool,
    /// Published blend. Only a finished long-term mean is shown.
    pub blend: SurfaceCacheBlend,
    /// Firefly clamp in standard deviations of the short-term mean.
    ///
    /// The listing uses 8. The clamp is biased: a sample above
    /// `0.1 + short mean + sigma * deviation` is pulled down before the means
    /// update. `0` leaves that `0.1` floor and still clamps. The hold path
    /// does not read this.
    pub firefly_sigma: f32,
    /// Cap on rays dispatched in one cache round. `0` keeps the existing lists.
    pub ray_budget: u32,
    /// Fresh rays a dormant cache entry keeps per frame, in steps of 16.
    /// `0` keeps the existing one-sweep-in-eight schedule.
    pub dormant_rays: u32,
}

/// Everything the lighting frame reads. PartialEq so a live edit reallocates
/// only when a size knob changes.
#[derive(Clone, Debug, PartialEq)]
pub struct LightingSettings {
    pub preset: LightingPreset,
    pub direct: DirectSettings,
    pub shadows: ShadowSettings,
    pub gi: GiSettings,
    pub surface_cache: SurfaceCacheSettings,
    pub bounce: BounceSettings,
    pub probes: ProbeSettings,
    pub world_cache: WorldCacheSettings,
    pub denoiser: DenoiserSettings,
    pub compose_reuse: bool,
    pub sky: bool,
    pub emissive: bool,
    pub prefill: bool,
    pub ambient: AmbientSettings,
    /// Weighted lamp picking. GI presets sample hits, and pixels stay on the full
    /// sum. `reuse (experimental)` is not a preset.
    pub lamp_pick: LampPickSettings,
    /// Clock hours in `[0, 24)`. Drives [`AmbientMode::Curve`] and the sky scale.
    /// `0` is midnight, `6` dawn, `12` noon, `18` dusk.
    pub time_of_day: f32,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self::preset(LightingPreset::High)
    }
}

impl LightingSettings {
    pub fn preset(preset: LightingPreset) -> Self {
        let mut s = Self::high_defaults();
        s.preset = preset;
        match preset {
            LightingPreset::High => {
                s.lamp_pick.mode = LampPickMode::Hits;
            }
            LightingPreset::DayCycle => {
                s.gi.enabled = false;
                s.bounce.count = 0;
                s.probes.enabled = false;
                s.world_cache.enabled = false;
                s.denoiser.enabled = false;
                s.compose_reuse = false;
                s.prefill = false;
                s.sky = true;
                s.ambient.mode = AmbientMode::Curve;
                s.ambient.night_color = [0.04, 0.05, 0.10];
                s.ambient.night_intensity = 0.08;
                s.ambient.dawn_color = [0.95, 0.48, 0.28];
                s.ambient.dawn_intensity = 0.28;
                s.ambient.noon_color = [0.62, 0.68, 0.78];
                s.ambient.noon_intensity = 0.42;
                s.ambient.dusk_color = [0.90, 0.38, 0.18];
                s.ambient.dusk_intensity = 0.26;
            }
            LightingPreset::Gi1Bounce => {
                s.bounce.count = 1;
                s.probes.rays = 64;
                s.world_cache.enabled = false;
                s.denoiser.enabled = false;
                s.prefill = false;
                s.lamp_pick.mode = LampPickMode::Hits;
            }
            LightingPreset::Potato => {
                s.direct.max_lights = 8;
                s.shadows.filter = ShadowFilter::Hard;
                s.gi.enabled = false;
                s.bounce.count = 0;
                s.probes.enabled = false;
                s.world_cache.enabled = false;
                s.denoiser.enabled = false;
                s.compose_reuse = false;
                s.sky = false;
                s.prefill = false;
                s.ambient.mode = AmbientMode::Constant;
                s.ambient.color = [0.42, 0.44, 0.48];
                s.ambient.intensity = 0.65;
            }
            LightingPreset::Low => {
                s.direct.max_lights = 4;
                s.shadows.cell_metres = 0.25;
                s.bounce.count = 1;
                s.probes.rays = 16;
                s.probes.tile = 32;
                s.probes.refresh_frames = 2;
                s.world_cache.enabled = false;
                s.denoiser.iterations = 1;
                // The filter does not run (no surface cache). Adaptive failed
                // the 4070 picture bar (S79), so it stays off.
                s.denoiser.adaptive = false;
                s.prefill = false;
                s.gi.refresh_frames = 2;
                s.gi.budget_shift = 2;
                s.ambient.mode = AmbientMode::Day;
                s.ambient.intensity = 0.35;
                s.lamp_pick.mode = LampPickMode::Hits;
            }
            LightingPreset::Medium => {
                s.direct.max_lights = 8;
                s.bounce.count = 2;
                s.probes.rays = 32;
                s.denoiser.iterations = 3;
                // Full-resolution scalar wavelet. Adaptive failed the 4070
                // picture bar (S79), so it stays off.
                s.denoiser.adaptive = false;
                s.gi.budget_shift = 1;
                s.ambient.mode = AmbientMode::Day;
                s.lamp_pick.mode = LampPickMode::Hits;
            }
        }
        s.sanitize();
        s
    }

    fn high_defaults() -> Self {
        Self {
            preset: LightingPreset::High,
            direct: DirectSettings {
                enabled: true,
                max_lights: 0,
                refresh_frames: 1,
                resolution_scale: 1,
            },
            shadows: ShadowSettings {
                enabled: true,
                filter: ShadowFilter::Cell,
                cell_metres: 0.125,
                refresh_frames: 1,
            },
            gi: GiSettings {
                enabled: true,
                refresh_frames: 1,
                budget_shift: 0,
                world_mode: None,
            },
            surface_cache: SurfaceCacheSettings {
                attach_movers: true,
                blend: SurfaceCacheBlend::Msme,
                firefly_sigma: 8.0,
                ray_budget: 0,
                dormant_rays: 0,
            },
            bounce: BounceSettings {
                count: LIGHTING_BOUNCES_UNLIMITED,
            },
            probes: ProbeSettings {
                enabled: true,
                rays: 64,
                tile: 0,
                refresh_frames: 1,
                staleness_frames: 8,
            },
            world_cache: WorldCacheSettings {
                enabled: true,
                cell_metres: 0.25,
                refresh_frames: 1,
                staleness_frames: 900,
            },
            denoiser: DenoiserSettings {
                enabled: true,
                iterations: 5,
                dirty_fraction: 0.25,
                adaptive: false,
                variance_threshold: 0.02,
            },
            compose_reuse: true,
            sky: true,
            emissive: true,
            prefill: true,
            ambient: AmbientSettings {
                mode: AmbientMode::Day,
                color: [0.42, 0.44, 0.48],
                intensity: 0.55,
                ground: [0.22, 0.20, 0.18],
                night_color: [0.05, 0.06, 0.10],
                night_intensity: 0.12,
                day_color: [0.55, 0.58, 0.62],
                day_intensity: 0.85,
                dawn_color: [1.0, 0.55, 0.32],
                dawn_intensity: 0.45,
                noon_color: [1.0, 1.0, 1.0],
                noon_intensity: 1.0,
                dusk_color: [1.0, 0.42, 0.22],
                dusk_intensity: 0.40,
            },
            lamp_pick: LampPickSettings::default(),
            time_of_day: 12.0,
        }
    }

    /// Clamp knobs into the range the GPU layout accepts.
    pub fn sanitize(&mut self) {
        self.direct.refresh_frames = self.direct.refresh_frames.max(1);
        self.direct.resolution_scale = self.direct.resolution_scale.clamp(1, 8);
        self.shadows.refresh_frames = self.shadows.refresh_frames.max(1);
        self.shadows.cell_metres = self.shadows.cell_metres.clamp(0.05, 2.0);
        self.gi.refresh_frames = self.gi.refresh_frames.max(1);
        self.gi.budget_shift = self.gi.budget_shift.min(7);
        self.surface_cache.firefly_sigma = if self.surface_cache.firefly_sigma.is_finite() {
            self.surface_cache.firefly_sigma.max(0.0)
        } else {
            8.0
        };
        if let Some(mode) = self.gi.world_mode {
            self.gi.world_mode = Some(mode.min(3));
        }
        if self.probes.rays != 0 {
            self.probes.rays = self.probes.rays.clamp(4, 64);
        }
        if self.probes.tile != 0 {
            self.probes.tile = self.probes.tile.clamp(4, 64);
        }
        self.probes.refresh_frames = self.probes.refresh_frames.max(1);
        self.probes.staleness_frames = self.probes.staleness_frames.max(1);
        self.world_cache.cell_metres = self.world_cache.cell_metres.clamp(0.05, 4.0);
        self.world_cache.refresh_frames = self.world_cache.refresh_frames.max(1);
        self.world_cache.staleness_frames = self.world_cache.staleness_frames.max(1);
        self.denoiser.iterations = match self.denoiser.iterations {
            0 | 1 => 1,
            2 | 3 => 3,
            _ => 5,
        };
        self.denoiser.dirty_fraction = self.denoiser.dirty_fraction.clamp(0.0, 1.0);
        // Ten bits in the control word: 0.0001 steps, up to 0.1023.
        self.denoiser.variance_threshold = self.denoiser.variance_threshold.clamp(0.0, 0.1);
        self.ambient.intensity = self.ambient.intensity.max(0.0);
        self.ambient.night_intensity = self.ambient.night_intensity.max(0.0);
        self.ambient.day_intensity = self.ambient.day_intensity.max(0.0);
        self.ambient.dawn_intensity = self.ambient.dawn_intensity.max(0.0);
        self.ambient.noon_intensity = self.ambient.noon_intensity.max(0.0);
        self.ambient.dusk_intensity = self.ambient.dusk_intensity.max(0.0);
        self.time_of_day = if self.time_of_day.is_finite() {
            self.time_of_day.rem_euclid(24.0)
        } else {
            12.0
        };
        if self.bounce.count != LIGHTING_BOUNCES_UNLIMITED {
            self.bounce.count = self.bounce.count.min(2);
        }
        self.lamp_pick.candidates = self.lamp_pick.candidates.clamp(1, 32);
        self.lamp_pick.rays = self.lamp_pick.rays.clamp(1, 4);
        self.lamp_pick.radius = self.lamp_pick.radius.min(64);
        self.lamp_pick.history = self.lamp_pick.history.clamp(1, 64);
    }

    /// World mode the shaders see. A forced mode wins once cache-to-cache is
    /// allowed. Bounce 0 and 1 stay on the probe path (mode 0): the surface
    /// cache is the extra bounce, so it is not live until bounce 2.
    pub fn world_mode(&self) -> u32 {
        if !self.gi.enabled || self.bounce.count < 2 {
            return 0;
        }
        if let Some(mode) = self.gi.world_mode {
            return mode;
        }
        if self.world_cache.enabled {
            3
        } else {
            0
        }
    }

    /// GI dispatches. Bounce 0 is direct light and flat ambient, so this is false.
    pub fn gi_passes(&self) -> bool {
        self.gi.enabled
            && self.bounce.count > 0
            && (self.probes.enabled || self.world_cache.enabled)
    }

    /// The million-slot cache and its key table. Freed when GI is off or the
    /// bounce count does not use it.
    pub fn allocates_cache(&self) -> bool {
        self.gi.enabled
            && self.bounce.count > 0
            && (self.probes.enabled || self.world_cache.enabled)
    }

    /// Probe and cache-ray work buffer. Freed when bounce 0 skips GI.
    pub fn allocates_probe_work(&self) -> bool {
        self.gi.enabled && self.bounce.count > 0 && self.probes.enabled
    }

    pub fn direct_this_frame(&self, frame: u32) -> bool {
        self.direct.enabled && frame_due(frame, self.direct.refresh_frames)
    }

    pub fn probes_this_frame(&self, frame: u32) -> bool {
        self.gi.enabled
            && self.bounce.count > 0
            && self.probes.enabled
            && frame_due(frame, self.probes.refresh_frames)
    }

    /// Surface-cache round. Recorded from bounce 2 up, where cache-to-cache runs.
    pub fn cache_round_this_frame(&self, frame: u32) -> bool {
        self.gi.enabled
            && self.bounce.count >= 2
            && (self.world_cache.enabled || self.probes.enabled)
            && frame_due(frame, self.gi.refresh_frames)
    }

    /// Cache-ray dispatches (cache-to-cache). Not recorded at bounce 0 or 1.
    pub fn cache_rays_this_frame(&self, frame: u32) -> bool {
        self.cache_round_this_frame(frame)
    }

    pub fn atrous_this_frame(&self, frame: u32) -> bool {
        self.denoiser.enabled && self.world_mode() == 3 && self.cache_round_this_frame(frame)
    }

    pub fn prefill_this_frame(&self) -> bool {
        self.prefill && self.gi.enabled && self.world_cache.enabled && self.bounce.count >= 2
    }

    /// The knobs a reference capture spends. Maximum rays, a dense filter,
    /// every pixel rewritten. The text form stores those knobs, not this
    /// action. `GENOS_LIGHTING_FULL_EFFORT` applies it after the other
    /// variables, so it wins.
    pub fn apply_full_effort(&mut self) {
        self.direct.enabled = true;
        self.direct.max_lights = 0;
        self.direct.refresh_frames = 1;
        self.direct.resolution_scale = 1;
        self.shadows.enabled = true;
        self.shadows.filter = ShadowFilter::Cell;
        self.shadows.cell_metres = 0.125;
        self.shadows.refresh_frames = 1;
        self.gi.enabled = true;
        self.gi.refresh_frames = 1;
        self.gi.budget_shift = 0;
        self.bounce.count = LIGHTING_BOUNCES_UNLIMITED;
        self.probes.enabled = true;
        self.probes.rays = 64;
        self.probes.tile = 0;
        self.probes.refresh_frames = 1;
        self.world_cache.enabled = true;
        self.world_cache.refresh_frames = 1;
        self.denoiser.enabled = true;
        self.denoiser.iterations = 5;
        self.denoiser.dirty_fraction = 0.0;
        self.compose_reuse = false;
        self.sky = true;
        self.emissive = true;
        self.prefill = true;
        self.sanitize();
        self.sync_preset_field();
    }

    /// True when applying full effort again would not change a knob.
    pub fn is_full_effort(&self) -> bool {
        let mut applied = self.clone();
        applied.apply_full_effort();
        self.same_knobs(&applied)
    }

    /// Rays the push constant and the work buffer agree on.
    pub fn probe_rays(&self) -> u32 {
        if self.probes.rays == 0 {
            64
        } else {
            self.probes.rays
        }
    }

    /// What forces a buffer rebuild. Screen size is compared separately.
    pub fn buffer_class(&self) -> (u32, u32, u32, u32) {
        (
            u32::from(self.allocates_cache()),
            u32::from(self.allocates_probe_work()),
            self.probe_rays(),
            self.probes.tile,
        )
    }

    /// Colour, ground colour, and intensity for this sun height (0 night, 1 overhead).
    pub fn ambient_at(&self, sun_height: f32) -> ([f32; 3], [f32; 3], f32, AmbientMode) {
        let t = sun_height.clamp(0.0, 1.0);
        match self.ambient.mode {
            AmbientMode::Off => ([0.0; 3], [0.0; 3], 0.0, AmbientMode::Off),
            AmbientMode::Constant => (
                self.ambient.color,
                self.ambient.ground,
                self.ambient.intensity,
                AmbientMode::Constant,
            ),
            AmbientMode::Hemisphere => (
                self.ambient.color,
                self.ambient.ground,
                self.ambient.intensity,
                AmbientMode::Hemisphere,
            ),
            AmbientMode::Day => {
                let color = lerp3(self.ambient.night_color, self.ambient.day_color, t);
                let intensity = self.ambient.night_intensity
                    + (self.ambient.day_intensity - self.ambient.night_intensity) * t;
                (color, self.ambient.ground, intensity, AmbientMode::Constant)
            }
            AmbientMode::Curve => {
                let (color, intensity) = self.curve_sample(self.time_of_day);
                (color, self.ambient.ground, intensity, AmbientMode::Constant)
            }
        }
    }

    /// Time-of-day keys: night at 0 and 24, dawn at 6, noon at 12, dusk at 18.
    /// The sample is piecewise linear and wraps at midnight.
    pub fn curve_sample(&self, hours: f32) -> ([f32; 3], f32) {
        let h = if hours.is_finite() {
            hours.rem_euclid(24.0)
        } else {
            0.0
        };
        let keys = [
            (0.0, self.ambient.night_color, self.ambient.night_intensity),
            (6.0, self.ambient.dawn_color, self.ambient.dawn_intensity),
            (12.0, self.ambient.noon_color, self.ambient.noon_intensity),
            (18.0, self.ambient.dusk_color, self.ambient.dusk_intensity),
            (24.0, self.ambient.night_color, self.ambient.night_intensity),
        ];
        for pair in keys.windows(2) {
            let (t0, c0, i0) = pair[0];
            let (t1, c1, i1) = pair[1];
            if h <= t1 {
                let u = if t1 > t0 { (h - t0) / (t1 - t0) } else { 0.0 };
                return (lerp3(c0, c1, u), i0 + (i1 - i0) * u);
            }
        }
        (self.ambient.night_color, self.ambient.night_intensity)
    }

    /// Sky colour scale for [`AmbientMode::Curve`], relative to the noon key.
    /// Noon white at intensity 1 is identity. Other modes leave the sky alone.
    pub fn sky_ambient_scale(&self) -> [f32; 3] {
        if self.ambient.mode != AmbientMode::Curve {
            return [1.0, 1.0, 1.0];
        }
        let (color, intensity) = self.curve_sample(self.time_of_day);
        let noon_i = self.ambient.noon_intensity.max(0.0);
        let mut scale = [1.0; 3];
        for i in 0..3 {
            let sample = color[i].max(0.0) * intensity.max(0.0);
            let noon = self.ambient.noon_color[i].max(0.0) * noon_i;
            scale[i] = if noon > 1.0e-4 { sample / noon } else { 1.0 };
        }
        scale
    }

    /// A named preset, or `custom` when the knobs are not
    /// exactly one of those presets. A manual edit clears the named label.
    pub fn preset_label(&self) -> &'static str {
        for preset in LightingPreset::all() {
            if self.same_knobs(&Self::preset(*preset)) {
                return preset.name();
            }
        }
        "custom"
    }

    fn same_knobs(&self, other: &Self) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        a.preset = LightingPreset::High;
        b.preset = LightingPreset::High;
        // The clock is not a preset knob. A moving time of day stays on the named preset.
        a.time_of_day = 0.0;
        b.time_of_day = 0.0;
        a == b
    }

    /// If the knobs are exactly a named preset, store that name. A manual edit
    /// that no longer matches leaves the last name in the field; [`Self::preset_label`]
    /// is what the text form prints.
    fn sync_preset_field(&mut self) {
        if let Some(preset) = LightingPreset::all()
            .iter()
            .copied()
            .find(|preset| self.same_knobs(&Self::preset(*preset)))
        {
            self.preset = preset;
        }
    }

    /// One line per knob. A scene file and a settings file store this text.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "preset={}", self.preset_label());
        let _ = writeln!(out, "direct={}", bit(self.direct.enabled));
        let _ = writeln!(out, "direct_lights={}", self.direct.max_lights);
        let _ = writeln!(out, "direct_refresh={}", self.direct.refresh_frames);
        let _ = writeln!(out, "direct_scale={}", self.direct.resolution_scale);
        let _ = writeln!(out, "shadows={}", bit(self.shadows.enabled));
        let _ = writeln!(out, "shadow_filter={}", self.shadows.filter.name());
        let _ = writeln!(out, "shadow_cell={}", num_text(self.shadows.cell_metres));
        let _ = writeln!(out, "shadow_refresh={}", self.shadows.refresh_frames);
        let _ = writeln!(out, "gi={}", bit(self.gi.enabled));
        let _ = writeln!(out, "gi_refresh={}", self.gi.refresh_frames);
        let _ = writeln!(out, "budget_shift={}", self.gi.budget_shift);
        let _ = writeln!(
            out,
            "surface_cache.attach_movers={}",
            bit(self.surface_cache.attach_movers)
        );
        let _ = writeln!(
            out,
            "surface_cache.blend={}",
            self.surface_cache.blend.name()
        );
        let _ = writeln!(
            out,
            "surface_cache.firefly_sigma={}",
            num_text(self.surface_cache.firefly_sigma)
        );
        let _ = writeln!(
            out,
            "surface_cache.ray_budget={}",
            self.surface_cache.ray_budget
        );
        let _ = writeln!(
            out,
            "surface_cache.dormant_rays={}",
            self.surface_cache.dormant_rays
        );
        if let Some(mode) = self.gi.world_mode {
            let _ = writeln!(out, "world_mode={mode}");
        }
        let _ = writeln!(out, "bounces={}", bounces_text(self.bounce.count));
        let _ = writeln!(out, "probes={}", bit(self.probes.enabled));
        let _ = writeln!(out, "probe_rays={}", self.probes.rays);
        let _ = writeln!(out, "probe_tile={}", self.probes.tile);
        let _ = writeln!(out, "probe_refresh={}", self.probes.refresh_frames);
        let _ = writeln!(out, "probe_stale={}", self.probes.staleness_frames);
        let _ = writeln!(out, "world_cache={}", bit(self.world_cache.enabled));
        let _ = writeln!(out, "world_cell={}", num_text(self.world_cache.cell_metres));
        let _ = writeln!(out, "world_refresh={}", self.world_cache.refresh_frames);
        let _ = writeln!(out, "world_stale={}", self.world_cache.staleness_frames);
        let _ = writeln!(out, "denoise={}", bit(self.denoiser.enabled));
        let _ = writeln!(out, "denoise_iters={}", self.denoiser.iterations);
        let _ = writeln!(
            out,
            "denoise_dirty={}",
            num_text(self.denoiser.dirty_fraction)
        );
        let _ = writeln!(out, "denoise_adaptive={}", bit(self.denoiser.adaptive));
        let _ = writeln!(
            out,
            "denoise_variance={}",
            num_text(self.denoiser.variance_threshold)
        );
        let _ = writeln!(out, "compose_reuse={}", bit(self.compose_reuse));
        let _ = writeln!(out, "sky={}", bit(self.sky));
        let _ = writeln!(out, "emissive={}", bit(self.emissive));
        let _ = writeln!(out, "prefill={}", bit(self.prefill));
        let _ = writeln!(out, "ambient={}", self.ambient.mode.name());
        let _ = writeln!(out, "ambient_color={}", rgb(self.ambient.color));
        let _ = writeln!(
            out,
            "ambient_intensity={}",
            num_text(self.ambient.intensity)
        );
        let _ = writeln!(out, "ambient_ground={}", rgb(self.ambient.ground));
        let _ = writeln!(out, "night_color={}", rgb(self.ambient.night_color));
        let _ = writeln!(
            out,
            "night_intensity={}",
            num_text(self.ambient.night_intensity)
        );
        let _ = writeln!(out, "day_color={}", rgb(self.ambient.day_color));
        let _ = writeln!(
            out,
            "day_intensity={}",
            num_text(self.ambient.day_intensity)
        );
        let _ = writeln!(out, "dawn_color={}", rgb(self.ambient.dawn_color));
        let _ = writeln!(
            out,
            "dawn_intensity={}",
            num_text(self.ambient.dawn_intensity)
        );
        let _ = writeln!(out, "noon_color={}", rgb(self.ambient.noon_color));
        let _ = writeln!(
            out,
            "noon_intensity={}",
            num_text(self.ambient.noon_intensity)
        );
        let _ = writeln!(out, "dusk_color={}", rgb(self.ambient.dusk_color));
        let _ = writeln!(
            out,
            "dusk_intensity={}",
            num_text(self.ambient.dusk_intensity)
        );
        let _ = writeln!(out, "lamp_pick={}", self.lamp_pick.mode.name());
        let _ = writeln!(out, "lamp_pick_candidates={}", self.lamp_pick.candidates);
        let _ = writeln!(out, "lamp_pick_rays={}", self.lamp_pick.rays);
        let _ = writeln!(out, "lamp_pick_samples={}", self.lamp_pick.rays);
        let _ = writeln!(out, "lamp_pick_radius={}", self.lamp_pick.radius);
        let _ = writeln!(out, "lamp_pick_history={}", self.lamp_pick.history);
        let _ = writeln!(out, "lamp_pick_unbiased={}", bit(self.lamp_pick.unbiased));
        let _ = writeln!(out, "lamp_pick_reuse={}", bit(self.lamp_pick.reuse()));
        let _ = writeln!(out, "time_of_day={}", num_text(self.time_of_day));
        out
    }

    /// Parse the text form. `preset=` is applied first, then each other line
    /// overlays it. Unknown keys fail.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut preset = None;
        let mut lines = Vec::new();
        for raw in text.split(['\n', ';']) {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("use key=value, got {line}"));
            };
            let key = key.trim();
            let value = value.trim();
            if key == "preset" {
                // `custom` does not replace the struct. Later lines are the value.
                if !value.eq_ignore_ascii_case("custom") {
                    preset = Some(
                        LightingPreset::parse(value)
                            .ok_or_else(|| format!("unknown preset {value}"))?,
                    );
                }
            } else {
                lines.push((key.to_string(), value.to_string()));
            }
        }
        let mut settings = preset.map(Self::preset).unwrap_or_default();
        for (key, value) in lines {
            settings.set_key(&key, &value)?;
        }
        settings.sanitize();
        settings.sync_preset_field();
        Ok(settings)
    }

    /// Overlay one knob. Used by the text form, the debug menu, and MCP.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        match key {
            "preset" => {
                if value.eq_ignore_ascii_case("custom") {
                    // Not a tier. The label is custom only when the knobs match none.
                } else {
                    let preset = LightingPreset::parse(value)
                        .ok_or_else(|| format!("unknown preset {value}"))?;
                    *self = Self::preset(preset);
                }
            }
            "direct" => self.direct.enabled = on(value)?,
            "direct_lights" => self.direct.max_lights = num_u32(value)?,
            "direct_refresh" => self.direct.refresh_frames = num_u32(value)?.max(1),
            "direct_scale" => self.direct.resolution_scale = num_u32(value)?.clamp(1, 8),
            "shadows" => self.shadows.enabled = on(value)?,
            "shadow_filter" => {
                self.shadows.filter = ShadowFilter::parse(value)
                    .ok_or_else(|| format!("shadow_filter is hard or cell, got {value}"))?
            }
            "shadow_cell" => self.shadows.cell_metres = num_f32(value)?,
            "shadow_refresh" => self.shadows.refresh_frames = num_u32(value)?.max(1),
            "gi" => self.gi.enabled = on(value)?,
            "gi_refresh" => self.gi.refresh_frames = num_u32(value)?.max(1),
            "budget_shift" => self.gi.budget_shift = num_u32(value)?.min(7),
            "surface_cache.attach_movers" => self.surface_cache.attach_movers = on(value)?,
            "surface_cache.blend" => {
                self.surface_cache.blend = SurfaceCacheBlend::parse(value).ok_or_else(|| {
                    format!("surface_cache.blend is msme or hold, got {value}")
                })?
            }
            "surface_cache.firefly_sigma" => {
                let sigma = num_f32(value)?;
                if !sigma.is_finite() || sigma < 0.0 {
                    return Err(format!(
                        "surface_cache.firefly_sigma is a non-negative number, got {value}"
                    ));
                }
                self.surface_cache.firefly_sigma = sigma;
            }
            "surface_cache.ray_budget" => self.surface_cache.ray_budget = num_u32(value)?,
            "surface_cache.dormant_rays" => self.surface_cache.dormant_rays = num_u32(value)?,
            "world_mode" => self.gi.world_mode = Some(num_u32(value)?.min(3)),
            "bounces" => self.bounce.count = parse_bounces(value)?,
            "probes" => self.probes.enabled = on(value)?,
            "probe_rays" => self.probes.rays = num_u32(value)?,
            "probe_tile" => self.probes.tile = num_u32(value)?,
            "probe_refresh" => self.probes.refresh_frames = num_u32(value)?.max(1),
            "probe_stale" => self.probes.staleness_frames = num_u32(value)?.max(1),
            "world_cache" => self.world_cache.enabled = on(value)?,
            "world_cell" => self.world_cache.cell_metres = num_f32(value)?,
            "world_refresh" => self.world_cache.refresh_frames = num_u32(value)?.max(1),
            "world_stale" => self.world_cache.staleness_frames = num_u32(value)?.max(1),
            "denoise" | "denoiser" => self.denoiser.enabled = on(value)?,
            "denoise_iters" => self.denoiser.iterations = num_u32(value)?,
            "denoise_dirty" => self.denoiser.dirty_fraction = num_f32(value)?,
            "denoise_adaptive" => self.denoiser.adaptive = on(value)?,
            "denoise_variance" => self.denoiser.variance_threshold = num_f32(value)?,
            "compose_reuse" => self.compose_reuse = on(value)?,
            "sky" => self.sky = on(value)?,
            "emissive" => self.emissive = on(value)?,
            "prefill" => self.prefill = on(value)?,
            "ambient" => {
                self.ambient.mode = AmbientMode::parse(value).ok_or_else(|| {
                    format!("ambient is off, constant, hemisphere, day, or curve, got {value}")
                })?
            }
            "ambient_color" => self.ambient.color = parse_rgb(value)?,
            "ambient_intensity" => self.ambient.intensity = num_f32(value)?,
            "ambient_ground" => self.ambient.ground = parse_rgb(value)?,
            "night_color" => self.ambient.night_color = parse_rgb(value)?,
            "night_intensity" => self.ambient.night_intensity = num_f32(value)?,
            "day_color" => self.ambient.day_color = parse_rgb(value)?,
            "day_intensity" => self.ambient.day_intensity = num_f32(value)?,
            "full_effort" => {
                if on(value)? {
                    self.apply_full_effort();
                } else {
                    *self = Self::preset(LightingPreset::High);
                }
            }
            "dawn_color" => self.ambient.dawn_color = parse_rgb(value)?,
            "dawn_intensity" => self.ambient.dawn_intensity = num_f32(value)?,
            "noon_color" => self.ambient.noon_color = parse_rgb(value)?,
            "noon_intensity" => self.ambient.noon_intensity = num_f32(value)?,
            "dusk_color" => self.ambient.dusk_color = parse_rgb(value)?,
            "dusk_intensity" => self.ambient.dusk_intensity = num_f32(value)?,
            "time_of_day" => self.time_of_day = num_f32(value)?,
            "lamp_pick" => {
                self.lamp_pick.mode = LampPickMode::parse(value).ok_or_else(|| {
                    format!(
                        "lamp_pick is off, hits, pixels, both, or reuse (experimental), got {value}"
                    )
                })?
            }
            "lamp_pick_candidates" => self.lamp_pick.candidates = num_u32(value)?,
            "lamp_pick_rays" | "lamp_pick_samples" => self.lamp_pick.rays = num_u32(value)?,
            "lamp_pick_radius" => self.lamp_pick.radius = num_u32(value)?,
            "lamp_pick_history" => self.lamp_pick.history = num_u32(value)?,
            "lamp_pick_unbiased" => self.lamp_pick.unbiased = on(value)?,
            "lamp_pick_reuse" => {
                if on(value)? {
                    self.lamp_pick.mode = LampPickMode::Reuse;
                } else if self.lamp_pick.mode == LampPickMode::Reuse {
                    self.lamp_pick.mode = LampPickMode::Both;
                }
            }
            _ => return Err(format!("unknown lighting setting {key}")),
        }
        self.sanitize();
        if key != "preset" || value.eq_ignore_ascii_case("custom") {
            self.sync_preset_field();
        }
        Ok(())
    }

    /// Launch locks overlay this value.
    ///
    /// `--preset`, `--bounces`, and `--ambient` win every frame.
    /// `GENOS_LIGHTING_PRESET`, `GENOS_LIGHTING_BOUNCES`, and
    /// `GENOS_LIGHTING_AMBIENT` apply only when that argument was omitted, and
    /// then they win every frame. `--preset` or a set `GENOS_LIGHTING_PRESET`
    /// replaces the struct, then the other locks overlay one key. `--time-of-day`
    /// is the program's clock: it keeps `GENOS_LIGHTING_TIME` from replacing
    /// `time_of_day`. When that argument was omitted, a set `GENOS_LIGHTING_TIME`
    /// holds that hour. The scene clock survives a preset replace unless the
    /// time mirror is the lock. The other variables overlay one key each. A set
    /// `GENOS_LIGHTING_COMPOSE_ALL` forces compose reuse off.
    pub fn apply_env(&mut self) {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let preset = std::env::var("GENOS_LIGHTING_PRESET").ok();
        let bounces = std::env::var("GENOS_LIGHTING_BOUNCES").ok();
        let ambient = std::env::var("GENOS_LIGHTING_AMBIENT").ok();
        let time = std::env::var("GENOS_LIGHTING_TIME").ok();
        let attach = std::env::var("GENOS_LIGHTING_SURFACE_CACHE_ATTACH_MOVERS").ok();
        let sc_local = std::env::var("GENOS_LIGHTING_SC_LOCAL").ok();
        let blend = std::env::var("GENOS_LIGHTING_SURFACE_CACHE_BLEND").ok();
        let firefly = std::env::var("GENOS_LIGHTING_SURFACE_CACHE_FIREFLY_SIGMA").ok();
        let ray_budget = std::env::var("GENOS_LIGHTING_SURFACE_CACHE_RAY_BUDGET").ok();
        let dormant_rays = std::env::var("GENOS_LIGHTING_SURFACE_CACHE_DORMANT_RAYS").ok();
        let Ok(launch) = resolve_lighting_launch(
            &refs,
            &LightingMirrors {
                preset: preset.as_deref(),
                bounces: bounces.as_deref(),
                ambient: ambient.as_deref(),
                time_of_day: time.as_deref(),
                attach_movers: attach.as_deref(),
                sc_local: sc_local.as_deref(),
                blend: blend.as_deref(),
                firefly_sigma: firefly.as_deref(),
                ray_budget: ray_budget.as_deref(),
                dormant_rays: dormant_rays.as_deref(),
            },
        ) else {
            return;
        };
        self.apply_launch(&launch);
        let keys = [
            ("GENOS_LIGHTING_DIRECT", "direct"),
            ("GENOS_LIGHTING_DIRECT_LIGHTS", "direct_lights"),
            ("GENOS_LIGHTING_DIRECT_REFRESH", "direct_refresh"),
            ("GENOS_LIGHTING_DIRECT_SCALE", "direct_scale"),
            ("GENOS_LIGHTING_SHADOWS", "shadows"),
            ("GENOS_LIGHTING_SHADOW_FILTER", "shadow_filter"),
            ("GENOS_LIGHTING_SHADOW_CELL", "shadow_cell"),
            ("GENOS_LIGHTING_SHADOW_REFRESH", "shadow_refresh"),
            ("GENOS_LIGHTING_GI", "gi"),
            ("GENOS_LIGHTING_GI_REFRESH", "gi_refresh"),
            ("GENOS_LIGHTING_SC_BUDGET_SHIFT", "budget_shift"),
            ("GENOS_LIGHTING_WORLD", "world_mode"),
            ("GENOS_LIGHTING_PROBES", "probes"),
            ("GENOS_LIGHTING_PROBE_RAYS", "probe_rays"),
            ("GENOS_LIGHTING_TILE", "probe_tile"),
            ("GENOS_LIGHTING_PROBE_REFRESH", "probe_refresh"),
            ("GENOS_LIGHTING_PROBE_STALE", "probe_stale"),
            ("GENOS_LIGHTING_WORLD_CACHE", "world_cache"),
            ("GENOS_LIGHTING_WORLD_CELL", "world_cell"),
            ("GENOS_LIGHTING_WORLD_REFRESH", "world_refresh"),
            ("GENOS_LIGHTING_WORLD_STALE", "world_stale"),
            ("GENOS_LIGHTING_DENOISE", "denoise"),
            ("GENOS_LIGHTING_DENOISE_ITERS", "denoise_iters"),
            ("GENOS_LIGHTING_ATROUS_DIRTY", "denoise_dirty"),
            ("GENOS_LIGHTING_DENOISE_ADAPTIVE", "denoise_adaptive"),
            ("GENOS_LIGHTING_DENOISE_VARIANCE", "denoise_variance"),
            ("GENOS_LIGHTING_COMPOSE_REUSE", "compose_reuse"),
            ("GENOS_LIGHTING_SKY", "sky"),
            ("GENOS_LIGHTING_EMISSIVE", "emissive"),
            ("GENOS_LIGHTING_PREFILL", "prefill"),
            ("GENOS_LIGHTING_AMBIENT_COLOR", "ambient_color"),
            ("GENOS_LIGHTING_AMBIENT_INTENSITY", "ambient_intensity"),
            ("GENOS_LIGHTING_AMBIENT_GROUND", "ambient_ground"),
            ("GENOS_LIGHTING_NIGHT_COLOR", "night_color"),
            ("GENOS_LIGHTING_NIGHT_INTENSITY", "night_intensity"),
            ("GENOS_LIGHTING_DAY_COLOR", "day_color"),
            ("GENOS_LIGHTING_DAY_INTENSITY", "day_intensity"),
            ("GENOS_LIGHTING_DAWN_COLOR", "dawn_color"),
            ("GENOS_LIGHTING_DAWN_INTENSITY", "dawn_intensity"),
            ("GENOS_LIGHTING_NOON_COLOR", "noon_color"),
            ("GENOS_LIGHTING_NOON_INTENSITY", "noon_intensity"),
            ("GENOS_LIGHTING_DUSK_COLOR", "dusk_color"),
            ("GENOS_LIGHTING_DUSK_INTENSITY", "dusk_intensity"),
            ("GENOS_LIGHTING_LAMP_PICK", "lamp_pick"),
            (
                "GENOS_LIGHTING_LAMP_PICK_CANDIDATES",
                "lamp_pick_candidates",
            ),
            ("GENOS_LIGHTING_LAMP_PICK_RAYS", "lamp_pick_rays"),
            ("GENOS_LIGHTING_LAMP_PICK_SAMPLES", "lamp_pick_samples"),
            ("GENOS_LIGHTING_LAMP_PICK_RADIUS", "lamp_pick_radius"),
            ("GENOS_LIGHTING_LAMP_PICK_HISTORY", "lamp_pick_history"),
            ("GENOS_LIGHTING_LAMP_PICK_UNBIASED", "lamp_pick_unbiased"),
            ("GENOS_LIGHTING_LAMP_PICK_REUSE", "lamp_pick_reuse"),
        ];
        for (var, key) in keys {
            if let Ok(value) = std::env::var(var) {
                let _ = self.set_key(key, &value);
            }
        }
        if std::env::var_os("GENOS_LIGHTING_COMPOSE_ALL").is_some() {
            self.compose_reuse = false;
            self.sync_preset_field();
        }
        if env_flag("GENOS_LIGHTING_FULL_EFFORT") {
            self.apply_full_effort();
        }
    }

    fn apply_launch(&mut self, launch: &LightingLaunch) {
        let clock = self.time_of_day;
        if let Some(preset) = launch.preset {
            *self = Self::preset(preset);
        }
        if let Some(count) = launch.bounces {
            self.bounce.count = count;
        }
        if let Some(mode) = launch.ambient {
            self.ambient.mode = mode;
        }
        if let Some(attach) = launch.attach_movers {
            self.surface_cache.attach_movers = attach;
        }
        if let Some(blend) = launch.blend {
            self.surface_cache.blend = blend;
        }
        if let Some(sigma) = launch.firefly_sigma {
            self.surface_cache.firefly_sigma = sigma;
        }
        if let Some(budget) = launch.ray_budget {
            self.surface_cache.ray_budget = budget;
        }
        if let Some(rays) = launch.dormant_rays {
            self.surface_cache.dormant_rays = rays;
        }
        self.time_of_day = launch.time_mirror.unwrap_or(clock);
        self.sanitize();
        self.sync_preset_field();
    }

    /// Which named GPU buckets this value records. A bucket left out is not
    /// dispatched, so its GPU time stays 0.
    pub fn gpu_buckets(&self, frame: u32) -> GpuBuckets {
        GpuBuckets {
            direct: self.direct_this_frame(frame),
            shadows: self.shadows.enabled && (self.direct.enabled || self.gi_passes()),
            place: self.probes_this_frame(frame),
            probe_trace: self.probes_this_frame(frame),
            gather: self.probes_this_frame(frame),
            filter: self.probes_this_frame(frame),
            compact: self.cache_round_this_frame(frame),
            cache: self.cache_round_this_frame(frame),
            cache_rays: self.cache_rays_this_frame(frame),
            atrous: self.atrous_this_frame(frame),
            // True when hits are shaded. The separate hit-light dispatch is only
            // recorded for cache rays (bounce 2 and up). One bounce shades kept
            // hits inside the direct dispatch.
            hit_light: self.gi.enabled
                && self.bounce.count > 0
                && (self.probes.enabled || self.cache_rays_this_frame(frame)),
            compose_indirect: self.compose_reuse && self.world_mode() == 3,
            prefill: self.prefill_this_frame(),
            sky: self.sky && self.gi.enabled,
            emissive: self.emissive,
            bounce_reads: self.gi.enabled && self.bounce.count > 0,
        }
    }
}

/// Passes and trace sites recorded for one frame. A false entry is not dispatched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuBuckets {
    pub direct: bool,
    pub shadows: bool,
    pub place: bool,
    pub probe_trace: bool,
    pub gather: bool,
    pub filter: bool,
    pub compact: bool,
    pub cache: bool,
    pub cache_rays: bool,
    pub atrous: bool,
    pub hit_light: bool,
    pub compose_indirect: bool,
    pub prefill: bool,
    pub sky: bool,
    pub emissive: bool,
    pub bounce_reads: bool,
}

/// Resolved launch locks. `--time-of-day` is not stored: the program that owns
/// the clock writes that hour, and its presence only clears `time_mirror`.
/// `time_mirror` is `GENOS_LIGHTING_TIME` when that argument was omitted.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LightingLaunch {
    preset: Option<LightingPreset>,
    bounces: Option<u32>,
    ambient: Option<AmbientMode>,
    time_mirror: Option<f32>,
    attach_movers: Option<bool>,
    blend: Option<SurfaceCacheBlend>,
    firefly_sigma: Option<f32>,
    ray_budget: Option<u32>,
    dormant_rays: Option<u32>,
}

#[derive(Clone, Copy, Default)]
struct LightingMirrors<'a> {
    preset: Option<&'a str>,
    bounces: Option<&'a str>,
    ambient: Option<&'a str>,
    time_of_day: Option<&'a str>,
    attach_movers: Option<&'a str>,
    /// Previous name of [`Self::attach_movers`]. Used only when that mirror is unset.
    sc_local: Option<&'a str>,
    blend: Option<&'a str>,
    firefly_sigma: Option<&'a str>,
    ray_budget: Option<&'a str>,
    dormant_rays: Option<&'a str>,
}

enum LightingKind {
    Preset,
    Bounces,
    Ambient,
    Time,
    AttachMovers,
    Blend,
    FireflySigma,
    RayBudget,
    DormantRays,
}

enum LightingFlag {
    Preset(LightingPreset),
    Bounces(u32),
    Ambient(AmbientMode),
    Time(f32),
    AttachMovers(bool),
    Blend(SurfaceCacheBlend),
    FireflySigma(f32),
    RayBudget(u32),
    DormantRays(u32),
}

/// `Some(true)` when `arg` is a lighting flag that reads the next argument.
/// `Some(false)` when the flag already includes its value. `None` for any other flag.
pub fn lighting_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, inline) = lighting_dash(arg)?;
    lighting_kind(name)?;
    Some(inline.is_none())
}

/// Reject a bad lighting argument. `next` is the following argument when
/// [`lighting_flag_needs_value`] is `Some(true)`.
pub fn check_lighting_arg(arg: &str, next: Option<&str>) -> Result<(), String> {
    parse_lighting_flag(arg, next).map(|_| ())
}

/// Clock hours from `--time-of-day`, when `arg` is that flag.
/// `Ok(None)` when `arg` is some other lighting flag or not a lighting flag.
pub fn lighting_arg_time_of_day(arg: &str, next: Option<&str>) -> Result<Option<f32>, String> {
    match parse_lighting_flag(arg, next)? {
        Some(LightingFlag::Time(hours)) => Ok(Some(hours)),
        _ => Ok(None),
    }
}

fn resolve_lighting_launch(
    args: &[&str],
    env: &LightingMirrors<'_>,
) -> Result<LightingLaunch, String> {
    let mut chosen = ChosenLighting::default();
    let mut i = 0;
    while i < args.len() {
        let next = args.get(i + 1).copied();
        match parse_lighting_flag(args[i], next)? {
            None => i += 1,
            Some(flag) => {
                let extra = lighting_flag_needs_value(args[i]).unwrap_or(false);
                chosen.set(flag);
                i += 1 + usize::from(extra);
            }
        }
    }
    Ok(LightingLaunch {
        preset: chosen
            .preset
            .or_else(|| env.preset.and_then(LightingPreset::parse)),
        bounces: chosen
            .bounces
            .or_else(|| env.bounces.and_then(|value| parse_bounces(value).ok())),
        ambient: chosen
            .ambient
            .or_else(|| env.ambient.and_then(AmbientMode::parse)),
        time_mirror: if chosen.time.is_some() {
            None
        } else {
            env.time_of_day.and_then(parse_clock)
        },
        attach_movers: chosen.attach_movers.or_else(|| {
            env.attach_movers
                .and_then(|value| on(value).ok())
                .or_else(|| env.sc_local.and_then(|value| on(value).ok()))
        }),
        blend: chosen
            .blend
            .or_else(|| env.blend.and_then(SurfaceCacheBlend::parse)),
        firefly_sigma: chosen.firefly_sigma.or_else(|| {
            env.firefly_sigma.and_then(|value| parse_firefly_sigma(value).ok())
        }),
        ray_budget: chosen
            .ray_budget
            .or_else(|| env.ray_budget.and_then(|value| num_u32(value).ok())),
        dormant_rays: chosen
            .dormant_rays
            .or_else(|| env.dormant_rays.and_then(|value| num_u32(value).ok())),
    })
}

#[derive(Default)]
struct ChosenLighting {
    preset: Option<LightingPreset>,
    bounces: Option<u32>,
    ambient: Option<AmbientMode>,
    time: Option<f32>,
    attach_movers: Option<bool>,
    blend: Option<SurfaceCacheBlend>,
    firefly_sigma: Option<f32>,
    ray_budget: Option<u32>,
    dormant_rays: Option<u32>,
}

impl ChosenLighting {
    fn set(&mut self, flag: LightingFlag) {
        match flag {
            LightingFlag::Preset(preset) => self.preset = Some(preset),
            LightingFlag::Bounces(count) => self.bounces = Some(count),
            LightingFlag::Ambient(mode) => self.ambient = Some(mode),
            LightingFlag::Time(hours) => self.time = Some(hours),
            LightingFlag::AttachMovers(on) => self.attach_movers = Some(on),
            LightingFlag::Blend(blend) => self.blend = Some(blend),
            LightingFlag::FireflySigma(sigma) => self.firefly_sigma = Some(sigma),
            LightingFlag::RayBudget(count) => self.ray_budget = Some(count),
            LightingFlag::DormantRays(count) => self.dormant_rays = Some(count),
        }
    }
}

fn lighting_dash(arg: &str) -> Option<(&str, Option<&str>)> {
    let rest = arg.strip_prefix("--")?;
    if rest.is_empty() {
        return None;
    }
    match rest.split_once('=') {
        Some((name, value)) => Some((name, Some(value))),
        None => Some((rest, None)),
    }
}

fn lighting_kind(name: &str) -> Option<LightingKind> {
    match name {
        "preset" => Some(LightingKind::Preset),
        "bounces" => Some(LightingKind::Bounces),
        "ambient" => Some(LightingKind::Ambient),
        "time-of-day" => Some(LightingKind::Time),
        "surface-cache-attach-movers" => Some(LightingKind::AttachMovers),
        "surface-cache-blend" => Some(LightingKind::Blend),
        "surface-cache-firefly-sigma" => Some(LightingKind::FireflySigma),
        "surface-cache-ray-budget" => Some(LightingKind::RayBudget),
        "surface-cache-dormant-rays" => Some(LightingKind::DormantRays),
        _ => None,
    }
}

fn parse_lighting_flag(arg: &str, next: Option<&str>) -> Result<Option<LightingFlag>, String> {
    let Some((name, inline)) = lighting_dash(arg) else {
        return Ok(None);
    };
    let Some(kind) = lighting_kind(name) else {
        return Ok(None);
    };
    let flag = format!("--{name}");
    let value = match inline {
        Some(value) => value,
        None => next.ok_or_else(|| format!("{flag} needs a value"))?,
    };
    if value.is_empty() {
        return Err(format!("{flag} needs a value"));
    }
    let parsed = match kind {
        LightingKind::Preset => LightingFlag::Preset(
            LightingPreset::parse(value)
                .ok_or_else(|| format!("{flag} {value} is not a preset"))?,
        ),
        LightingKind::Bounces => LightingFlag::Bounces(
            parse_bounces(value)
                .map_err(|_| format!("{flag} {value} is not 0, 1, 2, or unlimited"))?,
        ),
        LightingKind::Ambient => {
            LightingFlag::Ambient(AmbientMode::parse(value).ok_or_else(|| {
                format!("{flag} {value} is not off, constant, hemisphere, day, or curve")
            })?)
        }
        LightingKind::Time => LightingFlag::Time(
            parse_clock(value).ok_or_else(|| format!("{flag} {value} is not a time of day"))?,
        ),
        LightingKind::AttachMovers => LightingFlag::AttachMovers(
            on(value).map_err(|_| format!("{flag} {value} is not on or off"))?,
        ),
        LightingKind::Blend => LightingFlag::Blend(
            SurfaceCacheBlend::parse(value)
                .ok_or_else(|| format!("{flag} {value} is not msme or hold"))?,
        ),
        LightingKind::FireflySigma => LightingFlag::FireflySigma(parse_firefly_sigma(value).map_err(
            |_| format!("{flag} {value} is not a non-negative number"),
        )?),
        LightingKind::RayBudget => LightingFlag::RayBudget(
            num_u32(value).map_err(|_| format!("{flag} {value} is not a number"))?,
        ),
        LightingKind::DormantRays => LightingFlag::DormantRays(
            num_u32(value).map_err(|_| format!("{flag} {value} is not a number"))?,
        ),
    };
    Ok(Some(parsed))
}

/// Hours, or `HH:MM` (minutes in `0..60`).
fn parse_clock(value: &str) -> Option<f32> {
    if value.is_empty() {
        return None;
    }
    if let Some((hours, minutes)) = value.split_once(':') {
        let hours: f32 = hours.parse().ok()?;
        let minutes: f32 = minutes.parse().ok()?;
        if !hours.is_finite() || !minutes.is_finite() || !(0.0..60.0).contains(&minutes) {
            return None;
        }
        return Some(hours + minutes / 60.0);
    }
    let hours: f32 = value.parse().ok()?;
    hours.is_finite().then_some(hours)
}

fn frame_due(frame: u32, every: u32) -> bool {
    let every = every.max(1);
    frame.saturating_sub(1) % every == 0
}

fn bit(v: bool) -> u32 {
    u32::from(v)
}

/// A set variable turns the flag on, except the explicit off words.
fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => !matches!(value.trim(), "0" | "false" | "off" | "no"),
        Err(_) => false,
    }
}

fn on(value: &str) -> Result<bool, String> {
    match value {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(format!("expected on or off, got {value}")),
    }
}

fn num_u32(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .or_else(|_| value.parse::<f32>().map(|v| v.round().max(0.0) as u32))
        .map_err(|_| format!("expected a number, got {value}"))
}

fn parse_firefly_sigma(value: &str) -> Result<f32, String> {
    let sigma = num_f32(value)?;
    if sigma.is_finite() && sigma >= 0.0 {
        Ok(sigma)
    } else {
        Err(format!("expected a non-negative number, got {value}"))
    }
}

fn num_f32(value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("expected a number, got {value}"))
}

fn parse_bounces(value: &str) -> Result<u32, String> {
    match value.to_ascii_lowercase().as_str() {
        "unlimited" | "inf" | "max" => Ok(LIGHTING_BOUNCES_UNLIMITED),
        "0" | "1" | "2" => num_u32(value),
        _ => Err(format!("bounces is 0, 1, 2, or unlimited, got {value}")),
    }
}

fn bounces_text(count: u32) -> String {
    if count == LIGHTING_BOUNCES_UNLIMITED {
        "unlimited".into()
    } else {
        count.to_string()
    }
}

fn num_text(v: f32) -> String {
    format!("{v:?}")
}

fn rgb(c: [f32; 3]) -> String {
    format!("{},{},{}", num_text(c[0]), num_text(c[1]), num_text(c[2]))
}

fn parse_rgb(value: &str) -> Result<[f32; 3], String> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(format!("expected r,g,b, got {value}"));
    }
    Ok([num_f32(parts[0])?, num_f32(parts[1])?, num_f32(parts[2])?])
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[cfg(test)]
mod launch_tests {
    use super::*;

    fn empty() -> LightingMirrors<'static> {
        LightingMirrors::default()
    }

    fn hours_near(value: f32, hours: f32) {
        assert!((value - hours).abs() < 1.0e-4, "{value} is not {hours}");
    }

    #[test]
    fn arguments_win_and_env_is_the_mirror() {
        let launch = resolve_lighting_launch(&[], &empty()).unwrap();
        assert!(launch.preset.is_none());
        assert!(launch.bounces.is_none());
        assert!(launch.ambient.is_none());
        assert!(launch.time_mirror.is_none());
        let mut settings = LightingSettings::default();
        settings.time_of_day = 9.5;
        settings.apply_launch(&launch);
        assert_eq!(settings.preset_label(), "high");
        hours_near(settings.time_of_day, 9.5);

        let mut env = empty();
        env.preset = Some("potato");
        env.bounces = Some("0");
        env.ambient = Some("curve");
        env.time_of_day = Some("18:24");
        let from_env = resolve_lighting_launch(&[], &env).unwrap();
        let mut settings = LightingSettings::default();
        settings.time_of_day = 9.5;
        settings.apply_launch(&from_env);
        assert!(!settings.gi.enabled);
        assert_eq!(settings.bounce.count, 0);
        assert_eq!(settings.ambient.mode, AmbientMode::Curve);
        hours_near(settings.time_of_day, 18.4);

        let forced = resolve_lighting_launch(
            &[
                "--preset",
                "day-cycle",
                "--bounces=1",
                "--ambient",
                "hemisphere",
                "--time-of-day=06:00",
            ],
            &env,
        )
        .unwrap();
        assert_eq!(forced.preset, Some(LightingPreset::DayCycle));
        assert_eq!(forced.bounces, Some(1));
        assert_eq!(forced.ambient, Some(AmbientMode::Hemisphere));
        assert!(forced.time_mirror.is_none());
        let mut settings = LightingSettings::default();
        settings.time_of_day = 9.5;
        settings.apply_launch(&forced);
        assert!(!settings.gi.enabled);
        assert_eq!(settings.bounce.count, 1);
        assert_eq!(settings.ambient.mode, AmbientMode::Hemisphere);
        hours_near(settings.time_of_day, 9.5);
        assert_eq!(settings.preset_label(), "custom");

        let held = resolve_lighting_launch(&["--preset=day-cycle"], &empty()).unwrap();
        let mut settings = LightingSettings::default();
        settings.time_of_day = 9.5;
        settings.apply_launch(&held);
        assert_eq!(settings.preset_label(), "day-cycle");
        hours_near(settings.time_of_day, 9.5);

        let unlimited = resolve_lighting_launch(&["--bounces", "unlimited"], &env).unwrap();
        assert_eq!(unlimited.bounces, Some(LIGHTING_BOUNCES_UNLIMITED));
        assert_eq!(unlimited.preset, Some(LightingPreset::Potato));

        let mut ignored = empty();
        ignored.bounces = Some("4");
        ignored.ambient = Some("nope");
        ignored.time_of_day = Some("nope");
        ignored.preset = Some("nope");
        let soft = resolve_lighting_launch(&[], &ignored).unwrap();
        assert!(soft.preset.is_none() && soft.bounces.is_none() && soft.ambient.is_none());
        assert!(soft.time_mirror.is_none());

        assert!(lighting_flag_needs_value("--bounces").unwrap());
        assert!(lighting_flag_needs_value("--bounces=0").is_some_and(|needs| !needs));
        assert!(lighting_flag_needs_value("--scale").is_none());
        assert_eq!(
            check_lighting_arg("--bounces", None).unwrap_err(),
            "--bounces needs a value"
        );
        assert_eq!(
            check_lighting_arg("--bounces", Some("4")).unwrap_err(),
            "--bounces 4 is not 0, 1, 2, or unlimited"
        );
        assert_eq!(
            check_lighting_arg("--ambient", Some("nope")).unwrap_err(),
            "--ambient nope is not off, constant, hemisphere, day, or curve"
        );
        assert_eq!(
            check_lighting_arg("--preset=", None).unwrap_err(),
            "--preset needs a value"
        );
        assert_eq!(
            check_lighting_arg("--preset", Some("nope")).unwrap_err(),
            "--preset nope is not a preset"
        );
        assert_eq!(
            check_lighting_arg("--time-of-day", Some("nope")).unwrap_err(),
            "--time-of-day nope is not a time of day"
        );
        assert_eq!(
            check_lighting_arg("--time-of-day", Some("18:60")).unwrap_err(),
            "--time-of-day 18:60 is not a time of day"
        );
        assert_eq!(
            lighting_arg_time_of_day("--time-of-day", Some("22:00")).unwrap(),
            Some(22.0)
        );
        assert_eq!(
            lighting_arg_time_of_day("--ambient=off", None).unwrap(),
            None
        );
        assert!(check_lighting_arg("--ambient=off", None).is_ok());
        assert!(check_lighting_arg("--time-of-day=18:24", None).is_ok());
    }

    #[test]
    fn attach_movers_argument_wins_over_both_mirrors() {
        let mut settings = LightingSettings::default();
        assert!(settings.surface_cache.attach_movers);
        let off =
            resolve_lighting_launch(&["--surface-cache-attach-movers", "off"], &empty()).unwrap();
        settings.apply_launch(&off);
        assert!(!settings.surface_cache.attach_movers);

        let mut env = empty();
        env.attach_movers = Some("off");
        env.sc_local = Some("1");
        let from_new = resolve_lighting_launch(&[], &env).unwrap();
        settings = LightingSettings::default();
        settings.apply_launch(&from_new);
        assert!(!settings.surface_cache.attach_movers);

        env.attach_movers = None;
        env.sc_local = Some("0");
        let from_old = resolve_lighting_launch(&[], &env).unwrap();
        settings = LightingSettings::default();
        settings.apply_launch(&from_old);
        assert!(!settings.surface_cache.attach_movers);

        let cli =
            resolve_lighting_launch(&["--surface-cache-attach-movers=on"], &env).unwrap();
        settings.apply_launch(&cli);
        assert!(settings.surface_cache.attach_movers);
        assert!(check_lighting_arg("--surface-cache-attach-movers", Some("maybe")).is_err());
    }

    #[test]
    fn blend_and_firefly_arguments_win_over_the_mirrors() {
        let mut settings = LightingSettings::default();
        assert_eq!(settings.surface_cache.blend, SurfaceCacheBlend::Msme);
        assert!((settings.surface_cache.firefly_sigma - 8.0).abs() < 1.0e-6);

        let mut env = empty();
        env.blend = Some("hold");
        env.firefly_sigma = Some("4");
        let from_env = resolve_lighting_launch(&[], &env).unwrap();
        settings.apply_launch(&from_env);
        assert_eq!(settings.surface_cache.blend, SurfaceCacheBlend::Hold);
        assert!((settings.surface_cache.firefly_sigma - 4.0).abs() < 1.0e-6);

        let cli = resolve_lighting_launch(
            &[
                "--surface-cache-blend",
                "msme",
                "--surface-cache-firefly-sigma=12",
            ],
            &env,
        )
        .unwrap();
        settings.apply_launch(&cli);
        assert_eq!(settings.surface_cache.blend, SurfaceCacheBlend::Msme);
        assert!((settings.surface_cache.firefly_sigma - 12.0).abs() < 1.0e-6);

        assert!(check_lighting_arg("--surface-cache-blend", Some("snap")).is_err());
        assert!(check_lighting_arg("--surface-cache-firefly-sigma", Some("-1")).is_err());
        assert!(check_lighting_arg("--surface-cache-firefly-sigma", Some("0")).is_ok());

        let rays = resolve_lighting_launch(
            &["--surface-cache-ray-budget", "4096", "--surface-cache-dormant-rays=4"],
            &empty(),
        )
        .unwrap();
        settings = LightingSettings::default();
        settings.apply_launch(&rays);
        assert_eq!(settings.surface_cache.ray_budget, 4096);
        assert_eq!(settings.surface_cache.dormant_rays, 4);
        assert_eq!(settings.surface_cache.blend, SurfaceCacheBlend::Msme);
    }
}
