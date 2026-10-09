//! The stress stage's settings as debug knobs: the tools, scripts and the panel's
//! sliders all set them through here.

use genos_debug::{Host, Knob};
use genos_scene::Scene;

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
            Knob::choice("gi", "GI", usize::from(self.gi_v2), &["v1", "v2"]),
            Knob::choice(
                "layout",
                "Lamps in",
                usize::from(self.mix.layout == Layout::First),
                &["spread", "first"],
            ),
        ]
    }

    fn set_knob(&mut self, name: &str, v: f64) -> Result<(), String> {
        match name {
            "time_of_day" => self.day = day_from_hours(v),
            "sun_speed" => self.sun_speed = v as f32,
            "sun" => self.sun_frozen = v >= 0.5,
            "sky" => self.sky_on = v >= 0.5,
            "boxes" => self.boxes_still = v >= 0.5,
            "lights" => self.mix.count = v as usize,
            "dynamic" => self.mix.dynamic_pct = v as u32,
            "power" => self.power = v as f32,
            "scale" => self.set_scale(if v >= 0.5 { Scale::BIG } else { Scale::SMALL }),
            "gi" => {
                self.gi_v2 = v >= 0.5;
                self.gi_v2_set = Some(self.gi_v2);
            }
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
