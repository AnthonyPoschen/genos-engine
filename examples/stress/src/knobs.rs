//! The stress stage's settings as debug knobs: the tools, scripts and the panel's
//! sliders all set them through here.

use genos_debug::{Host, Knob};
use genos_scene::{
    AmbientMode, LampPickMode, LightingPreset, LightingSettings, Scene, ShadowFilter,
    SurfaceCacheBlend,
};

use crate::building::{self, Layout};
use crate::stage::{Scale, Stage};

/// Clock hours to the stage's day fraction (0 is sunrise at 06:00).
pub fn day_from_hours(hours: f64) -> f32 {
    (((hours - 6.0) / 24.0) as f32).rem_euclid(1.0)
}

pub fn hours(day: f32) -> f64 {
    ((day.rem_euclid(1.0) as f64) * 24.0 + 6.0) % 24.0
}

impl Host for Stage {
    fn scene(&mut self) -> &mut Scene {
        &mut self.world.scene
    }

    fn knobs(&self) -> Vec<Knob> {
        let max_lamps = self.building.max_lamps(self.mix.layout) as f64;
        let (hh, mm) = building::clock(self.day);
        let mut time = Knob::number(
            "time_of_day",
            "Time of day",
            hours(self.day),
            (0.0, 24.0),
            0.1,
            " h",
        );
        time.text = format!("{:.1} h ({hh:02}:{mm:02})", hours(self.day));
        let mut speed = Knob::number(
            "sun_speed",
            "Sun speed",
            self.sun_speed as f64,
            (0.0, 64.0),
            0.5,
            "",
        );
        speed.text = format!("x{}", self.sun_speed);
        let lighting = &self.world.scene.lighting;
        let mut knobs = vec![
            time,
            speed,
            Knob::choice(
                "sun",
                "Sun",
                usize::from(self.sun_frozen),
                &["run", "freeze"],
            ),
            Knob::switch("sky", "Sky", self.sky_on),
            Knob::choice(
                "boxes",
                "Boxes",
                usize::from(self.boxes_still),
                &["move", "still"],
            ),
            Knob::choice(
                "lamps",
                "Lamps",
                usize::from(self.lamps_still),
                &["move", "still"],
            ),
            Knob::switch("unfilled", "Unfilled", self.unfilled_count),
            Knob::switch("unfilled_tint", "Unfilled tint", self.unfilled_tint),
            Knob::number(
                "lamp_clock",
                "Lamp clock",
                self.lamp_clock as f64,
                (0.0, 1.0e6),
                1.0e-5,
                " s",
            ),
            Knob::number(
                "box_clock",
                "Box clock",
                self.box_clock as f64,
                (0.0, 1.0e6),
                1.0e-5,
                " s",
            ),
            Knob::number(
                "lights",
                "Lights",
                self.mix.count as f64,
                (0.0, max_lamps),
                1.0,
                "",
            ),
            Knob::number(
                "dynamic",
                "Dynamic",
                self.mix.dynamic_pct as f64,
                (0.0, 100.0),
                1.0,
                " %",
            ),
            Knob::number(
                "power",
                "Lamp power",
                self.power as f64,
                (0.0, 4.0),
                0.05,
                "",
            ),
            Knob::choice(
                "scale",
                "Scale",
                usize::from(self.scale == Scale::BIG),
                &["small", "big"],
            ),
            Knob::choice(
                "layout",
                "Lamps in",
                usize::from(self.mix.layout == Layout::First),
                &["spread", "first"],
            ),
        ];
        knobs.extend(lighting_knobs(lighting));
        knobs
    }

    fn set_knob(&mut self, name: &str, v: f64) -> Result<(), String> {
        if apply_lighting_knob(&mut self.world.scene.lighting, name, v)? {
            self.apply();
            return Ok(());
        }
        match name {
            "time_of_day" => self.day = day_from_hours(v),
            "sun_speed" => self.sun_speed = v as f32,
            "sun" => self.sun_frozen = v >= 0.5,
            "sky" => self.sky_on = v >= 0.5,
            "boxes" => self.boxes_still = v >= 0.5,
            "lamps" => self.lamps_still = v >= 0.5,
            "unfilled" => self.unfilled_count = v >= 0.5,
            "unfilled_tint" => {
                self.unfilled_tint = v >= 0.5;
                if self.unfilled_tint {
                    self.unfilled_count = true;
                }
            }
            // Scripts put the moving lamps and boxes back exactly where they started.
            "lamp_clock" => self.lamp_clock = v as f32,
            "box_clock" => self.box_clock = v as f32,
            "lights" => self.mix.count = v as usize,
            "dynamic" => self.mix.dynamic_pct = v as u32,
            "power" => self.power = v as f32,
            "scale" => self.set_scale(if v >= 0.5 { Scale::BIG } else { Scale::SMALL }),
            "layout" => {
                self.mix.layout = if v >= 0.5 {
                    Layout::First
                } else {
                    Layout::Spread
                }
            }
            _ => return Err(format!("unknown knob {name}")),
        }
        self.apply();
        Ok(())
    }
}

/// The lighting knobs shared with the lighting debug binary.
pub fn lighting_knobs(lighting: &LightingSettings) -> Vec<Knob> {
    vec![
        lighting_preset_knob(lighting),
        Knob::choice(
            "lighting_bounces",
            "Bounces",
            bounce_index(lighting.bounce.count),
            &["0", "1", "2", "unlimited"],
        ),
        Knob::switch("lighting_direct", "Direct", lighting.direct.enabled),
        Knob::number(
            "lighting_direct_lights",
            "Max lights",
            lighting.direct.max_lights as f64,
            (0.0, 64.0),
            1.0,
            "",
        ),
        Knob::number(
            "lighting_direct_refresh",
            "Direct refresh",
            lighting.direct.refresh_frames as f64,
            (1.0, 30.0),
            1.0,
            " frames",
        ),
        Knob::number(
            "lighting_direct_scale",
            "Direct scale",
            lighting.direct.resolution_scale as f64,
            (1.0, 8.0),
            1.0,
            "",
        ),
        Knob::choice(
            "lighting_lamp_pick",
            "Lamp pick",
            lamp_pick_index(lighting.lamp_pick.mode),
            &["off", "hits", "pixels", "both", "reuse (experimental)"],
        ),
        Knob::number(
            "lighting_lamp_pick_samples",
            "Lamp samples",
            lighting.lamp_pick.rays as f64,
            (1.0, 4.0),
            1.0,
            "",
        ),
        Knob::switch(
            "lighting_lamp_pick_reuse",
            "Lamp reuse",
            lighting.lamp_pick.reuse(),
        ),
        Knob::number(
            "lighting_lamp_pick_candidates",
            "Lamp candidates",
            lighting.lamp_pick.candidates as f64,
            (1.0, 32.0),
            1.0,
            "",
        ),
        Knob::number(
            "lighting_lamp_pick_radius",
            "Lamp radius",
            lighting.lamp_pick.radius as f64,
            (0.0, 64.0),
            1.0,
            " px",
        ),
        Knob::number(
            "lighting_lamp_pick_history",
            "Lamp history",
            lighting.lamp_pick.history as f64,
            (1.0, 64.0),
            1.0,
            "",
        ),
        Knob::switch(
            "lighting_lamp_pick_unbiased",
            "Lamp unbiased",
            lighting.lamp_pick.unbiased,
        ),
        Knob::switch("lighting_shadows", "Shadows", lighting.shadows.enabled),
        Knob::choice(
            "lighting_shadow_filter",
            "Shadow filter",
            usize::from(lighting.shadows.filter == ShadowFilter::Cell),
            &["hard", "cell"],
        ),
        Knob::switch("lighting_gi", "GI", lighting.gi.enabled),
        Knob::number(
            "lighting_gi_refresh",
            "GI refresh",
            lighting.gi.refresh_frames as f64,
            (1.0, 30.0),
            1.0,
            " frames",
        ),
        Knob::number(
            "lighting_budget_shift",
            "Budget shift",
            lighting.gi.budget_shift as f64,
            (0.0, 7.0),
            1.0,
            "",
        ),
        Knob::switch(
            "lighting_surface_cache_attach_movers",
            "Attach movers",
            lighting.surface_cache.attach_movers,
        ),
        Knob::choice(
            "lighting_surface_cache_blend",
            "Surface cache blend",
            usize::from(lighting.surface_cache.blend == SurfaceCacheBlend::Hold),
            &["msme", "hold"],
        ),
        Knob::number(
            "lighting_surface_cache_firefly_sigma",
            "Firefly sigma",
            lighting.surface_cache.firefly_sigma as f64,
            (0.0, 32.0),
            0.5,
            " σ",
        ),
        Knob::number(
            "lighting_surface_cache_ray_budget",
            "Ray budget",
            lighting.surface_cache.ray_budget as f64,
            (0.0, 1_048_576.0),
            1024.0,
            "",
        ),
        Knob::number(
            "lighting_surface_cache_dormant_rays",
            "Dormant rays",
            lighting.surface_cache.dormant_rays as f64,
            (0.0, 16.0),
            1.0,
            "",
        ),
        Knob::switch("lighting_probes", "Probes", lighting.probes.enabled),
        Knob::number(
            "lighting_probe_rays",
            "Probe rays",
            lighting.probes.rays as f64,
            (4.0, 64.0),
            4.0,
            "",
        ),
        Knob::number(
            "lighting_probe_stale",
            "Probe stale",
            lighting.probes.staleness_frames as f64,
            (1.0, 900.0),
            1.0,
            " frames",
        ),
        Knob::switch(
            "lighting_world_cache",
            "World cache",
            lighting.world_cache.enabled,
        ),
        Knob::number(
            "lighting_world_stale",
            "World stale",
            lighting.world_cache.staleness_frames as f64,
            (1.0, 1800.0),
            1.0,
            " frames",
        ),
        Knob::switch("lighting_denoise", "Denoise", lighting.denoiser.enabled),
        Knob::choice(
            "lighting_denoise_iters",
            "Denoise iterations",
            denoise_index(lighting.denoiser.iterations),
            &["1", "3", "5"],
        ),
        Knob::switch(
            "lighting_denoise_adaptive",
            "Adaptive denoise",
            lighting.denoiser.adaptive,
        ),
        Knob::number(
            "lighting_denoise_variance",
            "Denoise variance",
            lighting.denoiser.variance_threshold as f64,
            (0.0, 0.1),
            0.005,
            "",
        ),
        Knob::choice(
            "lighting_denoise_launch",
            "Denoise launch",
            usize::from(lighting.denoiser.screen),
            &["hash", "screen"],
        ),
        Knob::switch(
            "lighting_compose_reuse",
            "Compose reuse",
            lighting.compose_reuse,
        ),
        Knob::switch("lighting_sky", "Lighting sky", lighting.sky),
        Knob::switch("lighting_emissive", "Emissive", lighting.emissive),
        Knob::switch("lighting_prefill", "Prefill", lighting.prefill),
        Knob::switch(
            "lighting_full_effort",
            "Full effort",
            lighting.is_full_effort(),
        ),
        Knob::choice(
            "lighting_ambient",
            "Ambient",
            ambient_index(lighting.ambient.mode),
            &["off", "constant", "hemisphere", "day", "curve"],
        ),
    ]
}

fn lighting_preset_knob(settings: &LightingSettings) -> Knob {
    let label = settings.preset_label();
    let index = LightingPreset::all()
        .iter()
        .position(|preset| preset.name() == label)
        .unwrap_or(LightingPreset::all().len());
    let mut names: Vec<&str> = LightingPreset::all()
        .iter()
        .map(|preset| preset.name())
        .collect();
    names.push("custom");
    Knob::choice("lighting_preset", "Lighting", index, &names)
}

fn lamp_pick_index(mode: LampPickMode) -> usize {
    match mode {
        LampPickMode::Off => 0,
        LampPickMode::Hits => 1,
        LampPickMode::Pixels => 2,
        LampPickMode::Both => 3,
        LampPickMode::Reuse => 4,
    }
}

fn bounce_index(count: u32) -> usize {
    match count {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => 3,
    }
}

fn denoise_index(iterations: u32) -> usize {
    match iterations {
        0 | 1 => 0,
        2 | 3 => 1,
        _ => 2,
    }
}

fn ambient_index(mode: AmbientMode) -> usize {
    match mode {
        AmbientMode::Off => 0,
        AmbientMode::Constant => 1,
        AmbientMode::Hemisphere => 2,
        AmbientMode::Day => 3,
        AmbientMode::Curve => 4,
    }
}

/// Write one stress knob through [`LightingSettings::set_key`].
///
/// Returns false when `name` is not a lighting knob, so the stage can keep its
/// own names. A lighting name that is not a key fails.
pub fn apply_lighting_knob(
    settings: &mut LightingSettings,
    name: &str,
    v: f64,
) -> Result<bool, String> {
    let whole = v.round().max(0.0) as u32;
    let (key, value) = match name {
        "lighting_preset" => {
            return match LightingPreset::all().get(v as usize) {
                Some(preset) => settings.set_key("preset", preset.name()).map(|()| true),
                None => Ok(true),
            };
        }
        "lighting_bounces" => (
            "bounces",
            match whole {
                0 => "0".to_string(),
                1 => "1".to_string(),
                2 => "2".to_string(),
                _ => "unlimited".to_string(),
            },
        ),
        "lighting_direct" => ("direct", on_off(v)),
        "lighting_direct_lights" => ("direct_lights", whole.to_string()),
        "lighting_direct_refresh" => ("direct_refresh", whole.max(1).to_string()),
        "lighting_direct_scale" => ("direct_scale", whole.to_string()),
        "lighting_lamp_pick" => (
            "lamp_pick",
            ["off", "hits", "pixels", "both", "reuse (experimental)"]
                .get(whole as usize)
                .copied()
                .unwrap_or("off")
                .to_string(),
        ),
        "lighting_lamp_pick_samples" => ("lamp_pick_samples", whole.max(1).to_string()),
        "lighting_lamp_pick_reuse" => ("lamp_pick_reuse", on_off(v)),
        "lighting_lamp_pick_candidates" => ("lamp_pick_candidates", whole.max(1).to_string()),
        "lighting_lamp_pick_radius" => ("lamp_pick_radius", whole.to_string()),
        "lighting_lamp_pick_history" => ("lamp_pick_history", whole.max(1).to_string()),
        "lighting_lamp_pick_unbiased" => ("lamp_pick_unbiased", on_off(v)),
        "lighting_shadows" => ("shadows", on_off(v)),
        "lighting_shadow_filter" => (
            "shadow_filter",
            if v >= 0.5 { "cell" } else { "hard" }.to_string(),
        ),
        "lighting_gi" => ("gi", on_off(v)),
        "lighting_gi_refresh" => ("gi_refresh", whole.max(1).to_string()),
        "lighting_budget_shift" => ("budget_shift", whole.to_string()),
        "lighting_surface_cache_attach_movers" => ("surface_cache.attach_movers", on_off(v)),
        "lighting_surface_cache_blend" => (
            "surface_cache.blend",
            if v >= 0.5 { "hold" } else { "msme" }.to_string(),
        ),
        "lighting_surface_cache_firefly_sigma" => ("surface_cache.firefly_sigma", format!("{v}")),
        "lighting_surface_cache_ray_budget" => ("surface_cache.ray_budget", whole.to_string()),
        "lighting_surface_cache_dormant_rays" => ("surface_cache.dormant_rays", whole.to_string()),
        "lighting_probes" => ("probes", on_off(v)),
        "lighting_probe_rays" => ("probe_rays", whole.to_string()),
        "lighting_probe_stale" => ("probe_stale", whole.max(1).to_string()),
        "lighting_world_cache" => ("world_cache", on_off(v)),
        "lighting_world_stale" => ("world_stale", whole.max(1).to_string()),
        "lighting_denoise" => ("denoise", on_off(v)),
        "lighting_denoise_iters" => (
            "denoise_iters",
            match whole {
                0 => "1",
                1 => "3",
                _ => "5",
            }
            .to_string(),
        ),
        "lighting_denoise_adaptive" => ("denoise_adaptive", on_off(v)),
        "lighting_denoise_variance" => ("denoise_variance", format!("{v}")),
        "lighting_denoise_launch" => (
            "denoise_launch",
            if whole == 0 { "hash" } else { "screen" }.to_string(),
        ),
        "lighting_denoise_screen" => ("denoise_screen", on_off(v)),
        "lighting_compose_reuse" => ("compose_reuse", on_off(v)),
        "lighting_sky" => ("sky", on_off(v)),
        "lighting_emissive" => ("emissive", on_off(v)),
        "lighting_prefill" => ("prefill", on_off(v)),
        "lighting_full_effort" => ("full_effort", on_off(v)),
        "lighting_ambient" => (
            "ambient",
            ["off", "constant", "hemisphere", "day", "curve"]
                .get(v as usize)
                .copied()
                .unwrap_or("day")
                .to_string(),
        ),
        _ => return Ok(false),
    };
    settings.set_key(key, &value)?;
    Ok(true)
}

fn on_off(v: f64) -> String {
    if v >= 0.5 {
        "on".into()
    } else {
        "off".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::LightMix;

    /// Every knob `gi-v2` published on the stress stage, including the preset
    /// and bounce knobs and full effort.
    const GI_V2_KNOBS: &[&str] = &[
        "time_of_day",
        "sun_speed",
        "sun",
        "sky",
        "boxes",
        "lamps",
        "unfilled",
        "unfilled_tint",
        "lamp_clock",
        "box_clock",
        "lights",
        "dynamic",
        "power",
        "scale",
        "layout",
        "lighting_preset",
        "lighting_bounces",
        "lighting_direct",
        "lighting_direct_lights",
        "lighting_direct_refresh",
        "lighting_direct_scale",
        "lighting_shadows",
        "lighting_shadow_filter",
        "lighting_gi",
        "lighting_gi_refresh",
        "lighting_budget_shift",
        "lighting_surface_cache_attach_movers",
        "lighting_surface_cache_blend",
        "lighting_surface_cache_firefly_sigma",
        "lighting_surface_cache_ray_budget",
        "lighting_surface_cache_dormant_rays",
        "lighting_probes",
        "lighting_probe_rays",
        "lighting_probe_stale",
        "lighting_world_cache",
        "lighting_world_stale",
        "lighting_denoise",
        "lighting_denoise_iters",
        "lighting_denoise_adaptive",
        "lighting_denoise_variance",
        "lighting_denoise_launch",
        "lighting_compose_reuse",
        "lighting_sky",
        "lighting_emissive",
        "lighting_prefill",
        "lighting_full_effort",
        "lighting_ambient",
    ];

    #[test]
    fn the_stage_keeps_every_gi_v2_knob() {
        let mut stage = Stage::new(
            Scale::SMALL,
            1,
            LightMix {
                count: 5,
                dynamic_pct: 50,
                layout: Layout::Spread,
            },
            1.0,
            0.12,
            120.0,
        );
        for name in GI_V2_KNOBS {
            let value = stage
                .knobs()
                .into_iter()
                .find(|knob| knob.name == *name)
                .unwrap_or_else(|| panic!("missing {name}"))
                .value;
            stage.set_knob(name, value).expect(name);
        }
    }
}
