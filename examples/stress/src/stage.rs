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
    /// Lighting path: false the current one, true GI v2. `gi_v2_set` is a pending
    /// switch the frame loop hands to the renderer.
    pub gi_v2: bool,
    pub gi_v2_set: Option<bool>,
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
            gi_v2: std::env::var("GENOS_GI").is_ok_and(|v| v == "v2"),
            gi_v2_set: None,
        };
        stage.apply();
        stage
    }

    /// Rebuild the building at another scale. The lamp mix and clocks carry over.
    pub fn set_scale(&mut self, scale: Scale) {
        if scale == self.scale {
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
        self.apply();
    }

    /// Write the boxes, lamps, sun and sky for the current clocks into the world.
    pub fn apply(&mut self) {
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
