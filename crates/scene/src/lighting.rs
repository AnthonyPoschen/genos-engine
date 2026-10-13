//! Lighting settings: the reference [`crate::EngineSettings`] value.
//!
//! A game, a scene file, a script, MCP, and the debug menu edit this one struct.
//!
//! The renderer reads this and skips the GPU passes and allocations a disabled
//! feature owns. A quality knob changes how much work a feature does when it is
//! on. Presets fill the struct. The text form is what scene files and
//! `GENOS_LIGHTING_*` environment variables share with the debug menu.
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
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "potato" => Some(Self::Potato),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub fn all() -> &'static [LightingPreset] {
        &[Self::Potato, Self::Low, Self::Medium, Self::High]
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
}

impl AmbientMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Constant => "constant",
            Self::Hemisphere => "hemisphere",
            Self::Day => "day",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "0" => Some(Self::Off),
            "constant" | "flat" => Some(Self::Constant),
            "hemisphere" | "hemi" => Some(Self::Hemisphere),
            "day" | "day_night" | "curve" => Some(Self::Day),
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

/// Indirect bounces carried by the cache.
///
/// 0 is GI with direct light only: ray hits do not read the cache, and the
/// cache-ray dispatches are not recorded. 1 lets probes and pixels read the
/// cache, but cache rays themselves do not (one extra bounce). 2 and above allow
/// cache-to-cache feedback. [`LIGHTING_BOUNCES_UNLIMITED`] is the tuned path.
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

/// Where the a-trous wavelet runs.
///
/// `Full` filters every surface-cache texel. `Half` filters the bounce field on
/// the even-even lattice, then upsamples. Direct light is not an input either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenoiseResolution {
    Full,
    Half,
}

impl DenoiseResolution {
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Half => "half",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "full" => Some(Self::Full),
            "half" | "half-res" | "half_res" => Some(Self::Half),
            _ => None,
        }
    }
}

/// SVGF a-trous filter of the surface cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiserSettings {
    pub enabled: bool,
    /// A-trous iterations. 1, 3, or 5. The push reads the last iteration.
    pub iterations: u32,
    /// Dirty-list fraction before the dense sweep. 0 is always dense, 1 keeps lists.
    pub dirty_fraction: f32,
    /// Full-resolution wavelet, or half-resolution bounce filter plus upsample.
    pub resolution: DenoiseResolution,
    /// Drop later wavelet iterations where the texel's history is stable.
    pub adaptive: bool,
    /// Relative luminance deviation `sqrt(variance) / mean` under which a texel
    /// counts as stable. 0 leaves the decision to sample age.
    pub variance_threshold: f32,
    /// Shared-memory tiles for wavelet levels 1-5. The scalar kernel stays available.
    pub tiles: bool,
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
    /// Day end of the curve (sun overhead).
    pub day_color: [f32; 3],
    pub day_intensity: f32,
}

/// Everything the lighting frame reads. PartialEq so a live edit reallocates
/// only when a size knob changes.
#[derive(Clone, Debug, PartialEq)]
pub struct LightingSettings {
    pub preset: LightingPreset,
    pub direct: DirectSettings,
    pub shadows: ShadowSettings,
    pub gi: GiSettings,
    pub bounce: BounceSettings,
    pub probes: ProbeSettings,
    pub world_cache: WorldCacheSettings,
    pub denoiser: DenoiserSettings,
    pub compose_reuse: bool,
    pub sky: bool,
    pub emissive: bool,
    pub prefill: bool,
    pub ambient: AmbientSettings,
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
            LightingPreset::High => {}
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
                // The filter does not run (no surface cache). The cheap mode is
                // recorded so turning the cache on does not jump to full cost.
                s.denoiser.resolution = DenoiseResolution::Half;
                s.denoiser.adaptive = true;
                s.denoiser.tiles = true;
                s.prefill = false;
                s.gi.refresh_frames = 2;
                s.gi.budget_shift = 2;
                s.ambient.mode = AmbientMode::Day;
                s.ambient.intensity = 0.35;
            }
            LightingPreset::Medium => {
                s.direct.max_lights = 8;
                s.bounce.count = 2;
                s.probes.rays = 32;
                s.denoiser.iterations = 3;
                s.denoiser.resolution = DenoiseResolution::Half;
                s.denoiser.adaptive = true;
                s.denoiser.tiles = true;
                s.gi.budget_shift = 1;
                s.ambient.mode = AmbientMode::Day;
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
                // High stays on the full-resolution wavelet until it is benchmarked.
                resolution: DenoiseResolution::Full,
                adaptive: false,
                variance_threshold: 0.02,
                // High stays on the scalar kernel until the tiles are benchmarked.
                tiles: false,
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
            },
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
    }

    /// World mode the shaders see. A forced mode wins. Otherwise the surface
    /// cache is mode 3 and a probe-only GI is mode 0.
    pub fn world_mode(&self) -> u32 {
        if !self.gi.enabled {
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

    pub fn gi_passes(&self) -> bool {
        self.gi.enabled
            && (self.probes.enabled || self.world_cache.enabled || self.bounce.count > 0)
    }

    /// The million-slot cache and its key table. Freed when GI is off.
    pub fn allocates_cache(&self) -> bool {
        self.gi.enabled && (self.probes.enabled || self.world_cache.enabled)
    }

    /// Probe and cache-ray work buffer. Freed when GI is off.
    pub fn allocates_probe_work(&self) -> bool {
        self.gi.enabled && self.probes.enabled
    }

    pub fn direct_this_frame(&self, frame: u32) -> bool {
        self.direct.enabled && frame_due(frame, self.direct.refresh_frames)
    }

    pub fn probes_this_frame(&self, frame: u32) -> bool {
        self.gi.enabled && self.probes.enabled && frame_due(frame, self.probes.refresh_frames)
    }

    pub fn cache_round_this_frame(&self, frame: u32) -> bool {
        self.gi.enabled
            && (self.world_cache.enabled || self.probes.enabled)
            && frame_due(frame, self.gi.refresh_frames)
    }

    /// Cache-ray dispatches (the extra bounce). Skipped at bounce count 0.
    pub fn cache_rays_this_frame(&self, frame: u32) -> bool {
        self.cache_round_this_frame(frame) && self.bounce.count > 0
    }

    pub fn atrous_this_frame(&self, frame: u32) -> bool {
        self.denoiser.enabled
            && self.world_mode() == 3
            && self.cache_round_this_frame(frame)
            && self.bounce.count > 0
    }

    pub fn prefill_this_frame(&self) -> bool {
        self.prefill && self.gi.enabled && self.world_cache.enabled && self.bounce.count > 0
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
        }
    }

    /// `potato`, `low`, `medium`, `high`, or `custom` when the knobs are not
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
        let _ = writeln!(out, "denoise_res={}", self.denoiser.resolution.name());
        let _ = writeln!(out, "denoise_adaptive={}", bit(self.denoiser.adaptive));
        let _ = writeln!(
            out,
            "denoise_variance={}",
            num_text(self.denoiser.variance_threshold)
        );
        let _ = writeln!(out, "denoise_tiles={}", bit(self.denoiser.tiles));
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
            "denoise_res" | "denoise_resolution" => {
                self.denoiser.resolution = DenoiseResolution::parse(value)
                    .ok_or_else(|| format!("denoise_res is full or half, got {value}"))?
            }
            "denoise_adaptive" => self.denoiser.adaptive = on(value)?,
            "denoise_variance" => self.denoiser.variance_threshold = num_f32(value)?,
            "denoise_tiles" => self.denoiser.tiles = on(value)?,
            "compose_reuse" => self.compose_reuse = on(value)?,
            "sky" => self.sky = on(value)?,
            "emissive" => self.emissive = on(value)?,
            "prefill" => self.prefill = on(value)?,
            "ambient" => {
                self.ambient.mode = AmbientMode::parse(value).ok_or_else(|| {
                    format!("ambient is off, constant, hemisphere, or day, got {value}")
                })?
            }
            "ambient_color" => self.ambient.color = parse_rgb(value)?,
            "ambient_intensity" => self.ambient.intensity = num_f32(value)?,
            "ambient_ground" => self.ambient.ground = parse_rgb(value)?,
            "night_color" => self.ambient.night_color = parse_rgb(value)?,
            "night_intensity" => self.ambient.night_intensity = num_f32(value)?,
            "day_color" => self.ambient.day_color = parse_rgb(value)?,
            "day_intensity" => self.ambient.day_intensity = num_f32(value)?,
            _ => return Err(format!("unknown lighting setting {key}")),
        }
        self.sanitize();
        if key != "preset" || value.eq_ignore_ascii_case("custom") {
            self.sync_preset_field();
        }
        Ok(())
    }

    /// Environment variables overlay this value. A set `GENOS_LIGHTING_PRESET`
    /// replaces the struct, then each other variable overlays one knob.
    /// Unset variables leave the struct alone, so a scene file and a live edit
    /// survive. This is the launch lock: a variable that is set wins every frame.
    pub fn apply_env(&mut self) {
        if let Ok(value) = std::env::var("GENOS_LIGHTING_PRESET") {
            if let Some(preset) = LightingPreset::parse(&value) {
                *self = Self::preset(preset);
            }
        }
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
            ("GENOS_LIGHTING_BOUNCES", "bounces"),
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
            ("GENOS_LIGHTING_DENOISE_RES", "denoise_res"),
            ("GENOS_LIGHTING_DENOISE_ADAPTIVE", "denoise_adaptive"),
            ("GENOS_LIGHTING_DENOISE_VARIANCE", "denoise_variance"),
            ("GENOS_LIGHTING_DENOISE_TILES", "denoise_tiles"),
            ("GENOS_LIGHTING_COMPOSE_REUSE", "compose_reuse"),
            ("GENOS_LIGHTING_SKY", "sky"),
            ("GENOS_LIGHTING_EMISSIVE", "emissive"),
            ("GENOS_LIGHTING_PREFILL", "prefill"),
            ("GENOS_LIGHTING_AMBIENT", "ambient"),
            ("GENOS_LIGHTING_AMBIENT_COLOR", "ambient_color"),
            ("GENOS_LIGHTING_AMBIENT_INTENSITY", "ambient_intensity"),
            ("GENOS_LIGHTING_AMBIENT_GROUND", "ambient_ground"),
            ("GENOS_LIGHTING_NIGHT_COLOR", "night_color"),
            ("GENOS_LIGHTING_NIGHT_INTENSITY", "night_intensity"),
            ("GENOS_LIGHTING_DAY_COLOR", "day_color"),
            ("GENOS_LIGHTING_DAY_INTENSITY", "day_intensity"),
        ];
        for (var, key) in keys {
            if let Ok(value) = std::env::var(var) {
                let _ = self.set_key(key, &value);
            }
        }
        if std::env::var_os("GENOS_LIGHTING_COMPOSE_ALL").is_some() {
            self.compose_reuse = false;
        }
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
            cache: self.cache_round_this_frame(frame)
                && (self.world_cache.enabled || self.bounce.count > 0),
            cache_rays: self.cache_rays_this_frame(frame),
            atrous: self.atrous_this_frame(frame),
            hit_light: self.gi_passes()
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

fn frame_due(frame: u32, every: u32) -> bool {
    let every = every.max(1);
    frame.saturating_sub(1) % every == 0
}

fn bit(v: bool) -> u32 {
    u32::from(v)
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

fn num_f32(value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("expected a number, got {value}"))
}

fn parse_bounces(value: &str) -> Result<u32, String> {
    match value {
        "unlimited" | "inf" | "max" => Ok(LIGHTING_BOUNCES_UNLIMITED),
        _ => num_u32(value),
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
