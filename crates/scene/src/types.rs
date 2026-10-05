/// A square footprint or a circle footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Square,
    Circle,
}

#[derive(Clone, Debug)]
pub struct Wall {
    pub x: f32,
    pub z: f32,
    pub half_x: f32,
    pub half_z: f32,
    pub height: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Solid {
    pub shape: Shape,
    pub x: f32,
    pub z: f32,
    pub size: f32,
    pub height: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Light {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Floor {
    pub x: f32,
    pub z: f32,
    pub half_x: f32,
    pub half_z: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub floor: Floor,
    pub walls: Vec<Wall>,
    pub solids: Vec<Solid>,
    pub lights: Vec<Light>,
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
                let overlap_x = (a.x - b.x).abs() <= a.half_x + b.half_x + 0.05;
                let overlap_z = (a.z - b.z).abs() <= a.half_z + b.half_z + 0.05;
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
        let dx = x - self.x;
        let dz = z - self.z;
        let half = self.size * 0.5;
        match self.shape {
            Shape::Square => dx.abs() <= half && dz.abs() <= half,
            Shape::Circle => dx * dx + dz * dz <= half * half,
        }
    }
}
