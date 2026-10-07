use genos_math::Vec3;

/// Diffuse reflectance used when a material does not set one.
/// This is `1 / π`. A white surface returns that share of the irradiance.
pub const DEFAULT_REFLECTANCE: f32 = std::f32::consts::FRAC_1_PI;
/// Lamps and suns the renderer lights a frame with.
pub const MAX_LAMPS: usize = 4;
/// Walls and solids the renderer traces light against.
pub const MAX_OCCLUDERS: usize = 16;

/// Resolve a material reflectance. A negative value selects the game default.
pub fn reflectance_of(value: f32) -> f32 {
    if value < 0.0 {
        DEFAULT_REFLECTANCE
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Resolve how strongly the surface color tints a bounce. A negative value means full color.
pub fn color_mix_of(value: f32) -> f32 {
    if value < 0.0 {
        1.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Light leaving a diffuse hit.
///
/// The lamp and the previous bounce are one irradiance. The surface color tints both.
/// `color_mix` is 1 for that full tint, and 0 to keep the arriving color.
/// A negative `reflectance` uses [`DEFAULT_REFLECTANCE`].
pub fn bounce_radiance(
    albedo: [f32; 3],
    reflectance: f32,
    color_mix: f32,
    direct: f32,
    incoming: [f32; 3],
) -> [f32; 3] {
    let mix = color_mix_of(color_mix);
    let tint = [
        1.0 + (albedo[0] - 1.0) * mix,
        1.0 + (albedo[1] - 1.0) * mix,
        1.0 + (albedo[2] - 1.0) * mix,
    ];
    let reflect = reflectance_of(reflectance);
    [
        tint[0] * reflect * (direct + incoming[0]),
        tint[1] * reflect * (direct + incoming[1]),
        tint[2] * reflect * (direct + incoming[2]),
    ]
}

/// A square footprint or a circle footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Square,
    Circle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    pub position: Vec3,
    pub half_x: f32,
    pub half_z: f32,
    pub height: f32,
    pub color: [f32; 3],
    /// Nepers per meter along the straight path. Zero leaves the level unchanged.
    pub absorption: f32,
    /// Share of arriving light that leaves the surface. Below zero uses the game default.
    pub reflectance: f32,
    /// How much of `color` tints the bounce. Below zero uses the full surface color.
    pub color_mix: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Solid {
    pub shape: Shape,
    pub position: Vec3,
    pub size: f32,
    pub height: f32,
    pub color: [f32; 3],
    /// Nepers per meter along the straight path. Zero leaves the level unchanged.
    pub absorption: f32,
    /// Share of arriving light that leaves the surface. Below zero uses the game default.
    pub reflectance: f32,
    /// How much of `color` tints the bounce. Below zero uses the full surface color.
    pub color_mix: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub position: Vec3,
    pub color: [f32; 3],
    /// Direction the rays travel. Zero keeps a point lamp at `position`.
    pub direction: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Floor {
    pub position: Vec3,
    pub half_x: f32,
    pub half_z: f32,
    pub color: [f32; 3],
}

/// A flat roof over the whole floor footprint, facing down. It closes a room so no
/// light escapes upward.
#[derive(Clone, Debug, PartialEq)]
pub struct Ceiling {
    /// Height of the underside above the floor, in meters.
    pub height: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub floor: Floor,
    pub walls: Vec<Wall>,
    pub solids: Vec<Solid>,
    pub lights: Vec<Light>,
    /// None leaves the scene open to the sky.
    pub ceiling: Option<Ceiling>,
}

impl Scene {
    pub fn solid_by_color(&self, color: [f32; 3]) -> Option<&Solid> {
        self.solids.iter().find(|solid| {
            (solid.color[0] - color[0]).abs() < 0.01
                && (solid.color[1] - color[1]).abs() < 0.01
                && (solid.color[2] - color[2]).abs() < 0.01
        })
    }

    /// True when two wall rectangles on the ground plane overlap or touch.
    pub fn walls_share_a_corner(&self) -> bool {
        for (i, a) in self.walls.iter().enumerate() {
            for b in self.walls.iter().skip(i + 1) {
                let overlap_x = (a.position.x - b.position.x).abs() <= a.half_x + b.half_x + 0.05;
                let overlap_z = (a.position.z - b.position.z).abs() <= a.half_z + b.half_z + 0.05;
                if overlap_x && overlap_z {
                    return true;
                }
            }
        }
        false
    }
}

impl Solid {
    pub fn contains_xz(&self, x: f32, z: f32) -> bool {
        let dx = x - self.position.x;
        let dz = z - self.position.z;
        let half = self.size * 0.5;
        match self.shape {
            Shape::Square => dx.abs() <= half && dz.abs() <= half,
            Shape::Circle => dx * dx + dz * dz <= half * half,
        }
    }
}
