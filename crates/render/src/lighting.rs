//! Illumination for one draw.
//!
//! The caller passes lights and objects. This engine decides which lamp reaches
//! which surface, builds the bounce field once, and keeps that result until a
//! light or an occluder changes.

use std::hash::Hash;

use genos_scene::{view_proj, Camera, Light, Scene, Shape, Solid, Wall};

use crate::field::{self, Field};
use crate::mesh::{self, LitVertex, Vertex};
use crate::world::{Bounds, Displacement, DrawKind, FixedPart, ShaderSpace, World};

pub struct Lighting {
    key: u64,
    draw_key: u64,
    field: Option<Field>,
    shaded: Vec<Shaded>,
    gathers: u32,
    rays: u64,
    /// Baked triangles for the current visible set. The camera does not change these colors.
    cached: Vec<Vertex>,
    cached_scene: u64,
    cached_draw: u64,
    cached_visible: u64,
    generation: u64,
    mesh_builds: u32,
}

struct Shaded {
    hidden: bool,
    bounds: Bounds,
    vertices: Vec<LitVertex>,
}

impl Lighting {
    pub fn new() -> Self {
        Self {
            key: 0,
            draw_key: 0,
            field: None,
            shaded: Vec::new(),
            gathers: 0,
            rays: 0,
            cached: Vec::new(),
            cached_scene: 0,
            cached_draw: 0,
            cached_visible: 0,
            generation: 0,
            mesh_builds: 0,
        }
    }

    pub fn gather_count(&self) -> u32 {
        self.gathers
    }

    pub fn ray_count(&self) -> u64 {
        self.rays
    }

    /// Times the visible mesh was rebuilt. A camera move does not increment this.
    pub fn mesh_builds(&self) -> u32 {
        self.mesh_builds
    }

    /// Spacing of the one range used for this scene. It does not change with the camera.
    pub fn spacing(&mut self, world: &World) -> f32 {
        self.ensure(world);
        self.field.as_ref().unwrap().near.spacing
    }

    /// Triangles for objects the camera should show. Bounce comes from the cached field.
    pub fn vertices(&mut self, world: &World, view_proj: &[f32; 16]) -> Vec<Vertex> {
        self.compose(world, view_proj)
    }

    /// The path `Renderer::draw` uses. Aspect is the window width divided by its height.
    pub fn vertices_for_camera(
        &mut self,
        world: &World,
        camera: &Camera,
        aspect: f32,
    ) -> Vec<Vertex> {
        self.compose(world, &view_proj(camera, aspect))
    }

    /// Bake or reuse the visible mesh. Returns the generation `Renderer` uploads.
    pub(crate) fn prepare(&mut self, world: &World, view_proj: &[f32; 16]) -> u64 {
        self.sync(world, view_proj);
        self.generation
    }

    pub(crate) fn vertex_len(&self) -> usize {
        self.cached.len()
    }

    pub(crate) fn copy_vertices(&self) -> Vec<Vertex> {
        self.cached.clone()
    }

    fn compose(&mut self, world: &World, view_proj: &[f32; 16]) -> Vec<Vertex> {
        self.sync(world, view_proj);
        self.cached.clone()
    }

    fn sync(&mut self, world: &World, view_proj: &[f32; 16]) {
        self.ensure(world);
        let visible = visible_key(&self.shaded, view_proj);
        if self.generation != 0
            && self.cached_scene == self.key
            && self.cached_draw == self.draw_key
            && self.cached_visible == visible
        {
            return;
        }
        self.fill_cached(view_proj);
        self.cached_scene = self.key;
        self.cached_draw = self.draw_key;
        self.cached_visible = visible;
    }

    fn fill_cached(&mut self, view_proj: &[f32; 16]) {
        let field = self.field.as_ref().unwrap();
        let mut cached = Vec::new();
        for item in &self.shaded {
            if item.hidden || !crate::world::in_view(view_proj, item.bounds) {
                continue;
            }
            cached.reserve(item.vertices.len());
            for lit in &item.vertices {
                let bounce = field::sample(field, lit.pos[0], lit.pos[2]);
                cached.push(Vertex {
                    pos: lit.pos,
                    color: mesh::compose(lit.albedo, lit.direct, bounce),
                });
            }
        }
        self.cached = cached;
        self.mesh_builds += 1;
        self.generation = self.generation.wrapping_add(1).max(1);
    }

    /// Direct light at a point, using the same rules as a shaded surface.
    pub fn light_at(&mut self, world: &World, x: f32, y: f32, z: f32, normal: [f32; 3]) -> f32 {
        self.ensure(world);
        field::illuminate_facing(&world.light_scene(), x, y, z, normal)
    }

    fn ensure(&mut self, world: &World) {
        let scene = world.light_scene();
        let key = scene_key(&scene);
        let draw = draw_key(world);
        if self.field.is_some() && key == self.key && draw == self.draw_key {
            return;
        }
        if self.field.is_none() || key != self.key {
            let field = field::build(&scene);
            self.rays += field::ray_count(&field);
            self.gathers += 1;
            self.field = Some(field);
            self.key = key;
        }
        let field = self.field.as_ref().unwrap();
        self.shaded = mesh::shade_world(world, &scene, field)
            .into_iter()
            .map(|item| Shaded {
                hidden: item.hidden,
                bounds: item.bounds,
                vertices: item.vertices,
            })
            .collect();
        self.draw_key = draw;
    }
}

fn visible_key(items: &[Shaded], view_proj: &[f32; 16]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut hasher = DefaultHasher::new();
    items.len().hash(&mut hasher);
    for item in items {
        let show = !item.hidden && crate::world::in_view(view_proj, item.bounds);
        show.hash(&mut hasher);
    }
    hasher.finish()
}

fn draw_key(world: &World) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut hasher = DefaultHasher::new();
    world.objects.len().hash(&mut hasher);
    for object in &world.objects {
        object.hidden.hash(&mut hasher);
        object.affects_light.hash(&mut hasher);
        for value in object.bounds.center {
            value.to_bits().hash(&mut hasher);
        }
        for value in object.bounds.half {
            value.to_bits().hash(&mut hasher);
        }
        hash_kind(world, &object.kind, &mut hasher);
    }
    hasher.finish()
}

fn hash_kind(world: &World, kind: &DrawKind, hasher: &mut impl std::hash::Hasher) {
    match kind {
        DrawKind::Fixed(FixedPart::Floor) => {
            0u8.hash(hasher);
            hash_floor_geom(&world.scene.floor, hasher);
        }
        DrawKind::Fixed(FixedPart::Wall(index)) => {
            1u8.hash(hasher);
            index.hash(hasher);
            if let Some(wall) = world.scene.walls.get(*index) {
                hash_wall(wall, hasher);
            }
        }
        DrawKind::Fixed(FixedPart::Solid(index)) => {
            2u8.hash(hasher);
            index.hash(hasher);
            if let Some(solid) = world.scene.solids.get(*index) {
                hash_solid(solid, hasher);
            }
        }
        DrawKind::Mesh {
            vertices,
            color,
            pose,
        } => {
            3u8.hash(hasher);
            vertices.len().hash(hasher);
            for point in vertices {
                point[0].to_bits().hash(hasher);
                point[1].to_bits().hash(hasher);
                point[2].to_bits().hash(hasher);
            }
            color[0].to_bits().hash(hasher);
            color[1].to_bits().hash(hasher);
            color[2].to_bits().hash(hasher);
            for value in pose {
                value.to_bits().hash(hasher);
            }
        }
        DrawKind::Particles {
            points,
            color,
            size,
        } => {
            4u8.hash(hasher);
            points.len().hash(hasher);
            for point in points {
                point[0].to_bits().hash(hasher);
                point[1].to_bits().hash(hasher);
                point[2].to_bits().hash(hasher);
            }
            color[0].to_bits().hash(hasher);
            color[1].to_bits().hash(hasher);
            color[2].to_bits().hash(hasher);
            size.to_bits().hash(hasher);
        }
        DrawKind::Shader {
            space,
            displacement,
            color,
        } => {
            5u8.hash(hasher);
            match space {
                ShaderSpace::Mesh => 0u8.hash(hasher),
                ShaderSpace::Screen => 1u8.hash(hasher),
            }
            hash_displacement(displacement, hasher);
            color[0].to_bits().hash(hasher);
            color[1].to_bits().hash(hasher);
            color[2].to_bits().hash(hasher);
        }
    }
}

fn hash_displacement(map: &Option<Displacement>, hasher: &mut impl std::hash::Hasher) {
    let Some(map) = map else {
        0u8.hash(hasher);
        return;
    };
    1u8.hash(hasher);
    map.width.hash(hasher);
    map.height.hash(hasher);
    map.heights.len().hash(hasher);
    for byte in &map.heights {
        byte.hash(hasher);
    }
    for value in map.origin {
        value.to_bits().hash(hasher);
    }
    for value in map.scale {
        value.to_bits().hash(hasher);
    }
}

fn scene_key(scene: &Scene) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut hasher = DefaultHasher::new();
    fn bits(value: f32, hasher: &mut DefaultHasher) {
        value.to_bits().hash(hasher);
    }
    fn color(value: [f32; 3], hasher: &mut DefaultHasher) {
        bits(value[0], hasher);
        bits(value[1], hasher);
        bits(value[2], hasher);
    }
    bits(scene.floor.position.x, &mut hasher);
    bits(scene.floor.position.z, &mut hasher);
    bits(scene.floor.half_x, &mut hasher);
    bits(scene.floor.half_z, &mut hasher);
    color(scene.floor.color, &mut hasher);
    scene.lights.len().hash(&mut hasher);
    for light in &scene.lights {
        hash_light(light, &mut hasher);
    }
    scene.walls.len().hash(&mut hasher);
    for wall in &scene.walls {
        hash_wall(wall, &mut hasher);
    }
    scene.solids.len().hash(&mut hasher);
    for solid in &scene.solids {
        hash_solid(solid, &mut hasher);
    }
    hasher.finish()
}

fn hash_floor_geom(floor: &genos_scene::Floor, hasher: &mut impl std::hash::Hasher) {
    floor.position.x.to_bits().hash(hasher);
    floor.position.y.to_bits().hash(hasher);
    floor.position.z.to_bits().hash(hasher);
    floor.half_x.to_bits().hash(hasher);
    floor.half_z.to_bits().hash(hasher);
    floor.color[0].to_bits().hash(hasher);
    floor.color[1].to_bits().hash(hasher);
    floor.color[2].to_bits().hash(hasher);
}

fn hash_light(light: &Light, hasher: &mut impl std::hash::Hasher) {
    light.position.x.to_bits().hash(hasher);
    light.position.y.to_bits().hash(hasher);
    light.position.z.to_bits().hash(hasher);
    light.color[0].to_bits().hash(hasher);
    light.color[1].to_bits().hash(hasher);
    light.color[2].to_bits().hash(hasher);
}

fn hash_wall(wall: &Wall, hasher: &mut impl std::hash::Hasher) {
    wall.position.x.to_bits().hash(hasher);
    wall.position.z.to_bits().hash(hasher);
    wall.half_x.to_bits().hash(hasher);
    wall.half_z.to_bits().hash(hasher);
    wall.height.to_bits().hash(hasher);
    wall.color[0].to_bits().hash(hasher);
    wall.color[1].to_bits().hash(hasher);
    wall.color[2].to_bits().hash(hasher);
}

fn hash_solid(solid: &Solid, hasher: &mut impl std::hash::Hasher) {
    match solid.shape {
        Shape::Square => 0u8.hash(hasher),
        Shape::Circle => 1u8.hash(hasher),
    }
    solid.position.x.to_bits().hash(hasher);
    solid.position.z.to_bits().hash(hasher);
    solid.size.to_bits().hash(hasher);
    solid.height.to_bits().hash(hasher);
    solid.color[0].to_bits().hash(hasher);
    solid.color[1].to_bits().hash(hasher);
    solid.color[2].to_bits().hash(hasher);
}
