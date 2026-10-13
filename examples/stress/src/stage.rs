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
    /// A scene file in place of the building (--scene). The file is the world.
    /// Later applies do not put its lights back, so a script can move a lamp.
    pub room: Option<genos_scene::Scene>,
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
            room: None,
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
        self.apply();
    }

    /// Put a scene file in place of the building (--scene).
    pub fn set_room(&mut self, room: genos_scene::Scene) {
        self.world = World::from_scene(room.clone());
        self.room = Some(room);
        self.apply();
    }

    /// Write the boxes, lamps, sun and sky for the current clocks into the world.
    /// A scene file is already the world. Its lights stay where the script put
    /// them: copying the file back every frame undoes `light(path)`.
    pub fn apply(&mut self) {
        if self.room.is_some() {
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
mod tests {
    use super::*;
    use crate::building::{Layout, LightMix};

    #[test]
    fn a_scene_file_keeps_a_moved_lamp() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/bounce_corner.rhai");
        let scene = genos_scene::load_path(&path).expect("corner scene");
        assert_eq!(scene.walls.len(), 2, "two walls");
        assert_eq!(scene.solids.len(), 3, "three solids");
        assert_eq!(scene.lights.len(), 1, "one lamp");
        assert!(scene.sky.is_none(), "no sky");
        let mut stage = Stage::new(
            Scale::SMALL,
            1,
            LightMix {
                count: 5,
                dynamic_pct: 50,
                layout: Layout::Spread,
            },
            1.0,
            0.25,
            120.0,
        );
        stage.set_room(scene);
        assert_eq!(stage.occluders(), 5, "the room replaced the building");
        assert_eq!(stage.world.scene.lights.len(), 1);
        stage.world.scene.lights[0].position = genos_scene::Vec3::new(0.85, 1.55, 1.15);
        stage.advance(1.0 / 60.0);
        let p = stage.world.scene.lights[0].position;
        assert!(
            (p.x - 0.85).abs() < 1.0e-4 && (p.z - 1.15).abs() < 1.0e-4,
            "apply put the scene-file lamp back at {p:?}"
        );
    }
}
