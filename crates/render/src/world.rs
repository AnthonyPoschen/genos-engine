//! What the renderer can draw, and what it may skip on screen.
//!
//! Fixed parts are the authored floor, walls, and solids. A mesh can move by
//! replacing its pose. Particles are cards. A card can face the camera, sit at
//! an angle, take world light, emit light, and show one texture-map frame.
//! A shader is either a mesh-space grid, optionally displaced by a height map,
//! or a screen-space pass.

use genos_scene::{Scene, Shape, Solid};

/// Axis-aligned box in world space. Y is up.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub center: [f32; 3],
    pub half: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderSpace {
    Mesh,
    Screen,
}

#[derive(Clone, Debug)]
pub struct Displacement {
    pub width: u32,
    pub height: u32,
    pub heights: Vec<u8>,
    pub origin: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug)]
pub enum FixedPart {
    Floor,
    /// The roof over the floor footprint, when the scene has one.
    Ceiling,
    Wall(usize),
    Solid(usize),
}

/// One frame rectangle in a particle image. The origin is the top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleFrame {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Pixels a particle card can show. `frames` play in order for `frame_seconds` each.
#[derive(Clone, Debug)]
pub struct ParticleImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8. The length is `width * height * 4`.
    pub pixels: Vec<u8>,
    pub frames: Vec<ParticleFrame>,
    pub frame_seconds: f32,
}

#[derive(Clone, Debug)]
pub enum DrawKind {
    Fixed(FixedPart),
    Mesh {
        vertices: Vec<[f32; 3]>,
        color: [f32; 3],
        pose: [f32; 16],
    },
    Particles {
        points: Vec<[f32; 3]>,
        /// Age in seconds, one entry per point. A missing age is zero.
        ages: Vec<f32>,
        /// Base albedo. The shade path multiplies this. It is not a built-in flame color.
        color: [f32; 3],
        size: f32,
        /// Light added to the ground fire term. Zero leaves that term unchanged.
        emission: [f32; 3],
        /// Extinction of each point as one fog card. Zero draws a card per point.
        density: f32,
        /// When true, world light multiplies `color`. When false, the card keeps `color`.
        lit: bool,
        /// When true, the card normal points at the eye. `angle` is then unused.
        face_camera: bool,
        /// Radians of yaw away from the camera when `face_camera` is false.
        angle: f32,
        image: Option<ParticleImage>,
    },
    Shader {
        space: ShaderSpace,
        displacement: Option<Displacement>,
        color: [f32; 3],
    },
}

#[derive(Clone, Debug)]
pub struct Object {
    pub hidden: bool,
    /// When false, the object does not occlude and does not bounce light. It can still be drawn.
    pub affects_light: bool,
    pub bounds: Bounds,
    pub kind: DrawKind,
}

impl Object {
    /// Bounds of the object as the scene holds it now. A floor, wall or solid follows
    /// edits to `World::scene` (a game may move or turn a solid every frame); other
    /// kinds keep `bounds`, which the game updates with the pose.
    pub fn live_bounds(&self, scene: &Scene) -> Bounds {
        match &self.kind {
            DrawKind::Fixed(part) => fixed_bounds(scene, part).unwrap_or(self.bounds),
            _ => self.bounds,
        }
    }
}

/// World box of one authored part. A raised wall starts at its base; a turned square
/// solid covers its turned corners.
pub fn fixed_bounds(scene: &Scene, part: &FixedPart) -> Option<Bounds> {
    match part {
        FixedPart::Floor => Some(Bounds {
            center: [scene.floor.position.x, 0.0, scene.floor.position.z],
            half: [scene.floor.half_x, 0.5, scene.floor.half_z],
        }),
        FixedPart::Ceiling => scene.ceiling.as_ref().map(|ceiling| Bounds {
            center: [scene.floor.position.x, ceiling.height, scene.floor.position.z],
            half: [scene.floor.half_x, 0.05, scene.floor.half_z],
        }),
        FixedPart::Wall(index) => scene.walls.get(*index).map(|wall| Bounds {
            center: [wall.position.x, wall.base + wall.height * 0.5, wall.position.z],
            half: [wall.half_x, wall.height * 0.5, wall.half_z],
        }),
        FixedPart::Solid(index) => scene.solids.get(*index).map(|solid| {
            let reach = solid_reach(solid);
            Bounds {
                center: [solid.position.x, solid.height * 0.5, solid.position.z],
                half: [reach, solid.height * 0.5, reach],
            }
        }),
    }
}

/// Half width on X and on Z of the ground box around a solid. A square turned by
/// its yaw reaches out to its corners.
pub fn solid_reach(solid: &Solid) -> f32 {
    let half = solid.size * 0.5;
    match solid.shape {
        Shape::Circle => half,
        Shape::Square => half * (solid.yaw.cos().abs() + solid.yaw.sin().abs()),
    }
}

#[derive(Clone, Debug)]
pub struct World {
    pub scene: Scene,
    pub objects: Vec<Object>,
}

impl World {
    pub fn from_scene(scene: Scene) -> Self {
        let mut parts = vec![FixedPart::Floor];
        if scene.ceiling.is_some() {
            parts.push(FixedPart::Ceiling);
        }
        parts.extend((0..scene.walls.len()).map(FixedPart::Wall));
        parts.extend((0..scene.solids.len()).map(FixedPart::Solid));
        let objects = parts
            .into_iter()
            .map(|part| Object {
                hidden: false,
                affects_light: true,
                bounds: fixed_bounds(&scene, &part).unwrap_or(Bounds {
                    center: [0.0; 3],
                    half: [0.0; 3],
                }),
                kind: DrawKind::Fixed(part),
            })
            .collect();
        Self { scene, objects }
    }

    /// Scene the cascades march. Hidden and off-screen objects stay in it.
    pub fn light_scene(&self) -> Scene {
        let mut scene = self.scene.clone();
        let mut drop_solids = vec![false; scene.solids.len()];
        let mut drop_walls = vec![false; scene.walls.len()];
        let mut extras = Vec::new();
        for object in &self.objects {
            match &object.kind {
                DrawKind::Fixed(FixedPart::Solid(index)) if !object.affects_light => {
                    if *index < drop_solids.len() {
                        drop_solids[*index] = true;
                    }
                }
                DrawKind::Fixed(FixedPart::Wall(index)) if !object.affects_light => {
                    if *index < drop_walls.len() {
                        drop_walls[*index] = true;
                    }
                }
                DrawKind::Fixed(_) => {}
                _ if object.affects_light => extras.push(stand_in(object)),
                _ => {}
            }
        }
        let solids = std::mem::take(&mut scene.solids);
        scene.solids = solids
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !drop_solids[*index])
            .map(|(_, solid)| solid)
            .collect();
        let walls = std::mem::take(&mut scene.walls);
        scene.walls = walls
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !drop_walls[*index])
            .map(|(_, wall)| wall)
            .collect();
        scene.solids.extend(extras);
        scene
    }
}

fn stand_in(object: &Object) -> Solid {
    let color = match &object.kind {
        DrawKind::Mesh { color, .. }
        | DrawKind::Particles { color, .. }
        | DrawKind::Shader { color, .. } => *color,
        DrawKind::Fixed(_) => [1.0, 1.0, 1.0],
    };
    let half = object.bounds.half[0].max(object.bounds.half[2]).max(0.05);
    Solid {
        yaw: 0.0,
        shape: Shape::Square,
        position: genos_scene::Vec3::new(object.bounds.center[0], 0.0, object.bounds.center[2]),
        size: half * 2.0,
        height: (object.bounds.half[1] * 2.0).max(0.05),
        color,
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

pub fn identity_pose() -> [f32; 16] {
    let mut pose = [0.0; 16];
    pose[0] = 1.0;
    pose[5] = 1.0;
    pose[10] = 1.0;
    pose[15] = 1.0;
    pose
}

/// True when the box meets the camera frustum.
pub fn in_view(view_proj: &[f32; 16], bounds: Bounds) -> bool {
    let m = view_proj;
    let row = |r: usize| [m[r], m[4 + r], m[8 + r], m[12 + r]];
    let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    let r0 = row(0);
    let r1 = row(1);
    let r2 = row(2);
    let r3 = row(3);
    let planes = [
        add(r3, r0),
        sub(r3, r0),
        add(r3, r1),
        sub(r3, r1),
        r2,
        sub(r3, r2),
    ];
    planes.iter().all(|plane| !fully_outside(*plane, bounds))
}

fn fully_outside(plane: [f32; 4], bounds: Bounds) -> bool {
    let extent = bounds.half[0] * plane[0].abs()
        + bounds.half[1] * plane[1].abs()
        + bounds.half[2] * plane[2].abs();
    let distance = plane[0] * bounds.center[0]
        + plane[1] * bounds.center[1]
        + plane[2] * bounds.center[2]
        + plane[3];
    distance + extent < 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{build, sample};
    use crate::lighting::Lighting;
    use genos_scene::{view_proj, Camera, Floor, Light, Scene, Shape, Solid, Vec3, Wall};

    fn red_scene() -> Scene {
        Scene {
            floor: Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 8.0,
                half_z: 8.0,
                color: [1.0, 1.0, 1.0],
            },
            walls: Vec::new(),
            solids: vec![Solid {
                yaw: 0.0,
                shape: Shape::Square,
                position: Vec3::new(0.0, 0.0, 0.0),
                size: 1.5,
                height: 1.2,
                color: [1.0, 0.0, 0.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            lights: vec![Light {
                position: Vec3::new(0.0, 7.0, 0.0),
                color: [1.0, 1.0, 1.0],

                direction: Vec3::ZERO,
            }],
            ceiling: None,
            sky: None,
        }
    }

    #[test]
    fn hidden_and_offscreen_objects_stay_in_the_light_and_leave_the_picture() {
        let mut world = World::from_scene(red_scene());
        for object in &mut world.objects {
            if let DrawKind::Fixed(FixedPart::Solid(_)) = object.kind {
                object.hidden = true;
            }
        }
        let mut pose = identity_pose();
        pose[12] = -40.0;
        pose[13] = 1.0;
        pose[14] = 40.0;
        world.objects.push(Object {
            hidden: false,
            affects_light: true,
            bounds: Bounds {
                center: [-40.0, 1.0, 40.0],
                half: [0.4, 0.4, 0.4],
            },
            kind: DrawKind::Mesh {
                vertices: vec![[-0.2, 0.0, -0.2], [0.2, 0.0, -0.2], [0.0, 0.5, 0.2]],
                color: [0.0, 1.0, 1.0],
                pose,
            },
        });
        let camera = Camera::opening();
        let view = view_proj(&camera, 16.0 / 9.0);
        assert!(in_view(
            &view,
            Bounds {
                center: [0.0, 0.0, 0.0],
                half: [8.0, 0.5, 8.0],
            }
        ));
        assert!(!in_view(
            &view,
            Bounds {
                center: [-40.0, 1.0, 40.0],
                half: [0.4, 0.4, 0.4],
            }
        ));

        let field = build(&world.light_scene());
        let mut dark = red_scene();
        dark.solids.clear();
        let dark_field = build(&dark);
        let tint = sample(&field, 1.15, 0.0);
        let plain = sample(&dark_field, 1.15, 0.0);
        assert!(tint[0] > tint[1] && tint[0] > tint[2]);
        assert!(tint[0] > plain[0]);
        assert_eq!(world.light_scene().solids.len(), 2);

        let mut lighting = Lighting::new();
        let verts = lighting.vertices(&world, &view);
        assert!(verts.iter().all(|vertex| {
            let inside_red =
                vertex.pos[0].abs() < 0.7 && vertex.pos[2].abs() < 0.7 && vertex.pos[1] > 0.2;
            let offscreen = vertex.pos[0] < -20.0;
            !inside_red && !offscreen
        }));
    }

    #[test]
    fn a_height_map_builds_mesh_geometry() {
        let world_scene = red_scene();
        let mut world = World::from_scene(world_scene);
        world.objects.push(Object {
            hidden: false,
            affects_light: false,
            bounds: Bounds {
                center: [2.0, 0.5, 2.0],
                half: [1.0, 1.0, 1.0],
            },
            kind: DrawKind::Shader {
                space: ShaderSpace::Mesh,
                displacement: Some(Displacement {
                    width: 2,
                    height: 2,
                    heights: vec![0, 0, 0, 255],
                    origin: [2.0, 0.0, 2.0],
                    scale: [1.0, 2.0, 1.0],
                }),
                color: [1.0, 1.0, 1.0],
            },
        });
        let view = view_proj(&Camera::opening(), 16.0 / 9.0);
        let mut lighting = Lighting::new();
        let verts = lighting.vertices(&world, &view);
        assert!(verts
            .iter()
            .any(|vertex| (vertex.pos[1] - 2.0).abs() < 0.05));
    }

    #[test]
    fn the_back_of_a_wall_is_darker_than_the_lit_face() {
        use genos_scene::Wall;
        let mut scene = red_scene();
        scene.solids.clear();
        scene.walls.push(Wall {
            base: 0.0,
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 1.0,
            half_z: 0.3,
            height: 2.0,
            color: [1.0, 1.0, 1.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        });
        scene.lights[0].position.y = 1.5;
        scene.lights[0].position.z = -4.0;
        let world = World::from_scene(scene);
        let view = view_proj(&Camera::opening(), 16.0 / 9.0);
        let mut lighting = Lighting::new();
        let verts = lighting.vertices(&world, &view);
        let bright =
            |vertex: &crate::mesh::Vertex| vertex.color[0] + vertex.color[1] + vertex.color[2];
        let front = verts
            .iter()
            .filter(|vertex| (vertex.pos[2] + 0.3).abs() < 0.02 && vertex.pos[1] > 0.4)
            .map(bright)
            .fold(0.0_f32, f32::max);
        let back = verts
            .iter()
            .filter(|vertex| (vertex.pos[2] - 0.3).abs() < 0.02 && vertex.pos[1] > 0.4)
            .map(bright)
            .fold(0.0_f32, f32::max);
        assert!(front > 0.04, "lit face is dark: {front}");
        assert!(
            front > back + 0.02,
            "back face is as bright as the front: front {front} back {back}"
        );
        assert!(
            back < 0.05,
            "the unlit side carries the lit side: back {back} front {front}"
        );

        let mut colored = red_scene();
        colored.lights[0].position = Vec3::new(-4.0, 3.0, 0.0);
        let red = &colored.solids[0];
        let far_x = red.position.x + red.size * 0.5 + 0.3;
        let field = build(&colored);
        let far = sample(&field, far_x, red.position.z);
        assert!(
            far[0] < 0.04 && far[0] <= far[1] + 0.02 && far[0] <= far[2] + 0.02,
            "red underglow on the unlit side: {far:?} at x={far_x}"
        );
    }

    #[test]
    fn a_lit_wall_lifts_the_floor_shadow() {
        let scene = |wall: bool| Scene {
            floor: Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 8.0,
                half_z: 8.0,
                color: [1.0, 1.0, 1.0],
            },
            walls: if wall {
                vec![Wall {
                    base: 0.0,
                    position: Vec3::new(3.0, 0.0, 0.0),
                    half_x: 0.2,
                    half_z: 3.0,
                    height: 2.6,
                    color: [1.0, 1.0, 1.0],
                    absorption: 0.0,
                    reflectance: -1.0,
                    color_mix: -1.0,
                }]
            } else {
                Vec::new()
            },
            solids: vec![Solid {
                yaw: 0.0,
                shape: Shape::Square,
                position: Vec3::new(0.0, 0.0, 0.0),
                size: 1.5,
                height: 1.5,
                color: [0.8, 0.2, 0.1],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            lights: vec![Light {
                position: Vec3::new(-5.0, 4.0, 0.0),
                color: [1.0, 1.0, 1.0],

                direction: Vec3::ZERO,
            }],
            ceiling: None,
            sky: None,
        };
        let view = view_proj(&Camera::opening(), 16.0 / 9.0);
        let floor_at = |wall: bool, x: f32, z: f32| {
            let world = World::from_scene(scene(wall));
            let mut lighting = Lighting::new();
            let verts = lighting.vertices(&world, &view);
            verts
                .iter()
                .filter(|vertex| {
                    vertex.pos[1] < 0.02
                        && (vertex.pos[0] - x).abs() < 0.2
                        && (vertex.pos[2] - z).abs() < 0.2
                })
                .map(|vertex| vertex.color[0] + vertex.color[1] + vertex.color[2])
                .fold(0.0_f32, f32::max)
        };
        let near = floor_at(true, 2.4, 0.0);
        let far = floor_at(true, 1.2, 0.0);
        let bare = floor_at(false, 2.4, 0.0);
        let behind = floor_at(true, 3.6, 0.0);
        assert!(
            near > bare,
            "the lit wall did not add bounce: near {near} bare {bare}"
        );
        assert!(
            near + 0.02 >= far,
            "the floor beside the wall is darker than the floor farther out: near {near} far {far}"
        );
        assert!(
            behind <= near + 1.0e-4,
            "the floor behind the wall took the bounce: behind {behind} near {near}"
        );
    }
}

pub fn transform_pose(pose: &[f32; 16], point: [f32; 3]) -> [f32; 3] {
    [
        pose[0] * point[0] + pose[4] * point[1] + pose[8] * point[2] + pose[12],
        pose[1] * point[0] + pose[5] * point[1] + pose[9] * point[2] + pose[13],
        pose[2] * point[0] + pose[6] * point[1] + pose[10] * point[2] + pose[14],
    ]
}
