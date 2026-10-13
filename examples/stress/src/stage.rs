//! The running stress scene: the building at one scale, the lamp mix, the clocks, and
//! the [`World`] the renderer draws.

use genos_render::World;

use crate::building::{self, Building, LightMix};

/// Sections across and down. One section is [`building::SECTION`] metres square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    pub cols: u32,
    pub rows: u32,
}

impl Scale {
    /// One section, 25 m × 25 m.
    pub const SMALL: Self = Self { cols: 1, rows: 1 };
    /// 4 × 4 sections, 100 m × 100 m: 16× the floor area of small. From the benchmark
    /// view in the corner section the far corner is 120 m away, past the 50 m probe
    /// tier radius and at the 120 m reach of the world probe volume.
    pub const BIG: Self = Self { cols: 4, rows: 4 };

    /// `small`, `big`, `N` (N × N sections) or `NxM`.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "small" => return Ok(Self::SMALL),
            "big" => return Ok(Self::BIG),
            _ => {}
        }
        let bad = || format!("unknown scale {text}: use small, big, N or NxM");
        let (cols, rows) = match text.split_once('x') {
            Some((a, b)) => (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?),
            None => {
                let n: u32 = text.parse().map_err(|_| bad())?;
                (n, n)
            }
        };
        if cols == 0 || rows == 0 || cols * rows > 64 {
            return Err(bad());
        }
        Ok(Self { cols, rows })
    }

    pub fn label(self) -> String {
        match self {
            Self::SMALL => "small".into(),
            Self::BIG => "big".into(),
            Self { cols, rows } => format!("{cols}x{rows}"),
        }
    }

    pub fn sections(self) -> u32 {
        self.cols * self.rows
    }
}

pub struct Stage {
    pub building: Building,
    pub world: World,
    pub scale: Scale,
    pub seed: u64,
    pub mix: LightMix,
    /// Multiplies the lamp total.
    pub power: f32,
    /// Seconds on the lamp paths.
    pub lamp_clock: f32,
    /// Seconds on the box paths.
    pub box_clock: f32,
    /// Fraction of a day: 0 sunrise, 0.25 noon, 0.5 sunset.
    pub day: f32,
    /// Seconds per day at speed 1.
    pub day_seconds: f32,
    pub sun_speed: f32,
    pub sun_frozen: bool,
    /// The sky follows the sun; off leaves only the sun and the lamps.
    pub sky_on: bool,
    pub boxes_still: bool,
    /// The lamp paths' clock stands (moving lamps hold where they are).
    pub lamps_still: bool,
    /// Count pixels whose surface-cache texel had no finished set. F8 toggles it.
    pub unfilled_count: bool,
    /// Magenta tint on those pixels. Shift+F8 toggles it and turns counting on.
    pub unfilled_tint: bool,
    /// A scene file in place of the building (--scene): its walls, solids and
    /// lights as they are; nothing moves, the sun, sky and lamp knobs do nothing.
    pub room: Option<genos_scene::Scene>,
    /// Move the scene's point lamp, then hold it (--lamp-stop). Catch-up starts
    /// on the first draw that holds.
    pub lamp_stop: bool,
    lamp_stop_frame: u32,
}

/// Draws at the file position before the point lamp steps.
const LAMP_STOP_SETTLE: u32 = 12;
/// Draws that step the lamp. The next draw holds the last step.
const LAMP_STOP_MOVE: u32 = 8;
const LAMP_STOP_STEP: f32 = 0.08;

/// Metres the point lamp has moved by this draw. `0` through the settle
/// draws, then one step per draw, then a constant.
pub fn lamp_stop_shift(frame: u32) -> f32 {
    if frame <= LAMP_STOP_SETTLE {
        0.0
    } else if frame <= LAMP_STOP_SETTLE + LAMP_STOP_MOVE {
        LAMP_STOP_STEP * (frame - LAMP_STOP_SETTLE) as f32
    } else {
        LAMP_STOP_STEP * LAMP_STOP_MOVE as f32
    }
}

impl Stage {
    pub fn new(
        scale: Scale,
        seed: u64,
        mix: LightMix,
        power: f32,
        day: f32,
        day_seconds: f32,
    ) -> Self {
        let building = Building::new(scale.cols, scale.rows, seed);
        let world = World::from_scene(building.scene.clone());
        let mut stage = Self {
            building,
            world,
            scale,
            seed,
            mix,
            power,
            lamp_clock: 0.0,
            box_clock: 0.0,
            day,
            day_seconds: day_seconds.max(1.0),
            sun_speed: 1.0,
            sun_frozen: false,
            sky_on: true,
            boxes_still: false,
            lamps_still: false,
            unfilled_count: false,
            unfilled_tint: false,
            room: None,
            lamp_stop: false,
            lamp_stop_frame: 0,
        };
        stage.apply();
        stage
    }

    /// Rebuild the building at another scale. The lamp mix and clocks carry over.
    pub fn set_scale(&mut self, scale: Scale) {
        if scale == self.scale || self.room.is_some() {
            return;
        }
        self.scale = scale;
        self.building = Building::new(scale.cols, scale.rows, self.seed);
        self.world = World::from_scene(self.building.scene.clone());
        self.apply();
    }

    /// Back to time 0 on every path, the sun at `day`.
    pub fn rewind(&mut self, day: f32) {
        self.lamp_clock = 0.0;
        self.box_clock = 0.0;
        self.day = day;
        self.apply();
    }

    pub fn advance(&mut self, dt: f32) {
        if !self.lamps_still {
            self.lamp_clock += dt;
        }
        if !self.boxes_still {
            self.box_clock += dt;
        }
        if !self.sun_frozen {
            self.day = (self.day + dt * self.sun_speed / self.day_seconds).rem_euclid(1.0);
        }
        if self.lamp_stop {
            self.lamp_stop_frame = self.lamp_stop_frame.saturating_add(1);
        }
        self.apply();
    }

    /// Put a scene file in place of the building (--scene).
    pub fn set_room(&mut self, room: genos_scene::Scene) {
        self.world = World::from_scene(room.clone());
        self.room = Some(room);
        self.apply();
    }

    /// Write the boxes, lamps, sun and sky for the current clocks into the world.
    pub fn apply(&mut self) {
        // The stress clock is the scene time of day (same hours as the debug knob).
        // 0 on the stage clock is sunrise, 06:00.
        let hours = ((self.day.rem_euclid(1.0) as f64) * 24.0 + 6.0) % 24.0;
        self.world.scene.lighting.time_of_day = hours as f32;
        if let Some(room) = &self.room {
            self.world.scene.lights = room.lights.clone();
            self.world.scene.sky = room.sky.clone();
            if self.lamp_stop {
                let shift = lamp_stop_shift(self.lamp_stop_frame);
                if let Some(lamp) = self
                    .world
                    .scene
                    .lights
                    .iter_mut()
                    .find(|lamp| lamp.direction == genos_scene::Vec3::ZERO)
                {
                    lamp.position.x += shift;
                }
            }
            return;
        }
        self.building
            .place_movers(&mut self.world.scene.solids, self.box_clock);
        let mut lights = self.building.lamps(self.mix, self.lamp_clock, self.power);
        lights.extend(building::sun(self.day));
        self.world.scene.lights = lights;
        self.world.scene.sky = if self.sky_on {
            building::sky(self.day)
        } else {
            None
        };
    }

    /// Lamps actually placed (the building may have fewer slots than asked).
    pub fn lamp_count(&self) -> usize {
        self.mix.count.min(self.building.max_lamps(self.mix.layout))
    }

    pub fn occluders(&self) -> usize {
        self.world.scene.walls.len() + self.world.scene.solids.len()
    }
}

#[cfg(test)]
mod lamp_stop_shift_test {
    use super::{lamp_stop_shift, LAMP_STOP_MOVE, LAMP_STOP_SETTLE};

    #[test]
    fn the_lamp_holds_on_the_draw_after_the_last_step() {
        assert_eq!(lamp_stop_shift(1), 0.0);
        assert_eq!(lamp_stop_shift(LAMP_STOP_SETTLE), 0.0);
        assert!(lamp_stop_shift(LAMP_STOP_SETTLE + 1) > lamp_stop_shift(LAMP_STOP_SETTLE));
        let last = LAMP_STOP_SETTLE + LAMP_STOP_MOVE;
        assert_ne!(lamp_stop_shift(last), lamp_stop_shift(last - 1));
        assert_eq!(lamp_stop_shift(last), lamp_stop_shift(last + 1));
        assert_eq!(lamp_stop_shift(last + 8), lamp_stop_shift(last));
    }
}
