//! The stress stage's settings as debug knobs: the tools, scripts and the panel's
//! sliders all set them through here.

use genos_debug::{Host, Knob};
use genos_scene::{LightingPreset, LightingSettings, Scene};

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
        vec![
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
            lighting_preset_knob(&self.world.scene.lighting),
            Knob::choice(
                "lighting_bounces",
                "Bounces",
                bounce_index(self.world.scene.lighting.bounce.count),
                &["0", "1", "2", "unlimited"],
            ),
            Knob::switch("lighting_direct", "Direct", self.world.scene.lighting.direct.enabled),
            Knob::switch("lighting_shadows", "Shadows", self.world.scene.lighting.shadows.enabled),
            Knob::switch("lighting_gi", "GI", self.world.scene.lighting.gi.enabled),
            Knob::switch("lighting_probes", "Probes", self.world.scene.lighting.probes.enabled),
            Knob::switch(
                "lighting_world_cache",
                "World cache",
                self.world.scene.lighting.world_cache.enabled,
            ),
            Knob::switch("lighting_denoise", "Denoise", self.world.scene.lighting.denoiser.enabled),
            Knob::switch(
                "lighting_compose_reuse",
                "Compose reuse",
                self.world.scene.lighting.compose_reuse,
            ),
            Knob::switch("lighting_sky", "Lighting sky", self.world.scene.lighting.sky),
            Knob::switch("lighting_emissive", "Emissive", self.world.scene.lighting.emissive),
            Knob::switch("lighting_prefill", "Prefill", self.world.scene.lighting.prefill),
        ]
    }

    fn set_knob(&mut self, name: &str, v: f64) -> Result<(), String> {
        match name {
            "time_of_day" => self.day = day_from_hours(v),
            "sun_speed" => self.sun_speed = v as f32,
            "sun" => self.sun_frozen = v >= 0.5,
            "sky" => self.sky_on = v >= 0.5,
            "boxes" => self.boxes_still = v >= 0.5,
            "lamps" => self.lamps_still = v >= 0.5,
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
            "lighting_preset" => {
                if let Some(preset) = LightingPreset::all().get(v as usize) {
                    self.world.scene.lighting = LightingSettings::preset(*preset);
                }
            }
            "lighting_bounces" => {
                self.world.scene.lighting.bounce.count = match v.round() as u32 {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    _ => genos_scene::LIGHTING_BOUNCES_UNLIMITED,
                };
            }
            "lighting_direct" => self.world.scene.lighting.direct.enabled = v >= 0.5,
            "lighting_shadows" => self.world.scene.lighting.shadows.enabled = v >= 0.5,
            "lighting_gi" => self.world.scene.lighting.gi.enabled = v >= 0.5,
            "lighting_probes" => self.world.scene.lighting.probes.enabled = v >= 0.5,
            "lighting_world_cache" => self.world.scene.lighting.world_cache.enabled = v >= 0.5,
            "lighting_denoise" => self.world.scene.lighting.denoiser.enabled = v >= 0.5,
            "lighting_compose_reuse" => self.world.scene.lighting.compose_reuse = v >= 0.5,
            "lighting_sky" => self.world.scene.lighting.sky = v >= 0.5,
            "lighting_emissive" => self.world.scene.lighting.emissive = v >= 0.5,
            "lighting_prefill" => self.world.scene.lighting.prefill = v >= 0.5,
            _ => return Err(format!("unknown knob {name}")),
        }
        self.apply();
        Ok(())
    }
}

fn lighting_preset_knob(settings: &LightingSettings) -> Knob {
    let index = LightingPreset::all()
        .iter()
        .position(|preset| LightingSettings::preset(*preset) == *settings)
        .unwrap_or(4);
    Knob::choice(
        "lighting_preset",
        "Lighting",
        index,
        &["potato", "low", "medium", "high", "custom"],
    )
}

fn bounce_index(count: u32) -> usize {
    match count {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => 3,
    }
}
