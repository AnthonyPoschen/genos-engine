//! Packed mesh and light inputs for the GPU.
//!
//! The CPU writes positions, normals, colors, texture ids, and lamp parameters.
//! It does not evaluate probe radiance or the final lit color. The learned
//! picture still has to hold on the GPU: no lamp is black, a wall blocks the
//! lamp, a face is lit only from just outside when it points at the lamp, a
//! lamp above an object colors the floor all around, a side lamp does not
//! color the far side, and a lit wall brightens the near floor shadow.

use genos_scene::Shape;

use crate::particles::{self, Card, FireLight, Puff};
use crate::world::{
    transform_pose, DrawKind, FixedPart, ParticleFrame, ParticleImage, ShaderSpace, World,
};

pub const MAX_LAMPS: usize = 4;
pub const MAX_OCCLUDERS: usize = 16;
pub const MAX_OBJECTS: usize = 32;
pub const FIELD_GRID: u32 = 128;

/// One stored triangle corner. `shade` is 1 when the GPU lights the vertex.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub albedo: [f32; 3],
    pub normal: [f32; 3],
    pub shade: f32,
    /// Atlas coordinates. A negative x means the fragment keeps the albedo.
    pub uv: [f32; 2],
}

const _: () = assert!(std::mem::size_of::<GpuVertex>() == 48);
const ATLAS_EDGE: u32 = 256;

#[derive(Clone, Copy)]
pub struct GpuObject {
    pub first: u32,
    pub count: u32,
    pub texture: u32,
    pub color: [f32; 3],
}

#[derive(Clone, Copy)]
pub struct GpuLamp {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

#[derive(Clone, Copy)]
pub struct GpuOcc {
    pub center: [f32; 3],
    pub shape: f32,
    pub half_x: f32,
    pub height: f32,
    pub half_z: f32,
    pub radius: f32,
    pub albedo: [f32; 3],
    /// Nepers per meter. The light shader reads only `albedo`.
    pub absorption: f32,
    /// Diffuse reflectance. Below zero selects the game default.
    pub reflectance: f32,
    /// Surface-color mix for bounce. Below zero selects a full tint.
    pub color_mix: f32,
}

pub struct Pack {
    pub verts: Vec<GpuVertex>,
    pub objects: Vec<GpuObject>,
    pub lamps: Vec<GpuLamp>,
    pub occs: Vec<GpuOcc>,
    pub mesh_key: u64,
    pub light_key: u64,
    pub count_x: u32,
    pub count_z: u32,
    pub spacing: f32,
    pub origin_x: f32,
    pub origin_z: f32,
    pub near_end: f32,
    pub far_end: f32,
    pub world_end: f32,
    pub eye: [f32; 3],
    pub floor_center: [f32; 3],
    pub floor_half_x: f32,
    pub floor_half_z: f32,
    pub floor_color: [f32; 3],
    pub fire: FireLight,
    pub puffs: Vec<Puff>,
    /// Particle image the fragment shader samples. Empty width means no image.
    pub image: Vec<u8>,
}

pub fn pack_frame(world: &World, view_proj: &[f32; 16], eye: [f32; 3]) -> Pack {
    let mut verts = Vec::new();
    let mut objects = Vec::new();
    let mut occs = Vec::new();
    let (fire, puffs) = particles::medium(world, eye);
    let atlas = build_atlas(world, view_proj);
    for (index, object) in world.objects.iter().enumerate() {
        if object.affects_light {
            if let Some(occ) = occluder(world, object) {
                if occs.len() < MAX_OCCLUDERS {
                    occs.push(occ);
                }
            }
        }
        if object.hidden || !crate::world::in_view(view_proj, object.bounds) {
            continue;
        }
        if objects.len() >= MAX_OBJECTS {
            continue;
        }
        let first = verts.len() as u32;
        push_object(&mut verts, world, object, eye, atlas.place(index));
        let count = verts.len() as u32 - first;
        if count == 0 {
            continue;
        }
        objects.push(GpuObject {
            first,
            count,
            texture: 0,
            color: object_color(world, object),
        });
    }
    let lamps = lamps_of(world);
    let (count_x, count_z, spacing, origin_x, origin_z, near_end, far_end, world_end) =
        field_grid(world);
    let mesh_key = hash_mesh(&verts, &objects, &atlas.bytes);
    let light_key = hash_light(
        &lamps,
        &occs,
        count_x,
        count_z,
        eye,
        &fire,
        &puffs,
        world.scene.floor.position.x,
        world.scene.floor.position.z,
        world.scene.floor.half_x,
        world.scene.floor.half_z,
        world.scene.floor.color,
    );
    Pack {
        verts,
        objects,
        lamps,
        occs,
        mesh_key,
        light_key,
        count_x,
        count_z,
        spacing,
        origin_x,
        origin_z,
        near_end,
        far_end,
        world_end,
        eye,
        floor_center: [
            world.scene.floor.position.x,
            world.scene.floor.position.y,
            world.scene.floor.position.z,
        ],
        floor_half_x: world.scene.floor.half_x,
        floor_half_z: world.scene.floor.half_z,
        floor_color: world.scene.floor.color,
        fire,
        puffs,
        image: atlas.bytes,
    }
}

/// Bytes matching the std430 scene block the shaders read.
pub fn scene_bytes(pack: &Pack) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(48 + 64 + 64 + 16 * 48 + 32 * 32);
    push_u32(&mut bytes, pack.lamps.len() as u32);
    push_u32(&mut bytes, pack.occs.len() as u32);
    push_u32(&mut bytes, pack.objects.len() as u32);
    push_u32(&mut bytes, pack.count_x);
    push_u32(&mut bytes, pack.count_z);
    push_f32(&mut bytes, pack.spacing);
    push_f32(&mut bytes, pack.origin_x);
    push_f32(&mut bytes, pack.origin_z);
    push_f32(&mut bytes, pack.near_end);
    push_f32(&mut bytes, pack.far_end);
    push_f32(&mut bytes, pack.world_end);
    push_u32(&mut bytes, 0);
    for index in 0..MAX_LAMPS {
        let lamp = pack.lamps.get(index);
        let pos = lamp.map(|lamp| lamp.pos).unwrap_or([0.0; 3]);
        let color = lamp.map(|lamp| lamp.color).unwrap_or([0.0; 3]);
        push_f32(&mut bytes, pos[0]);
        push_f32(&mut bytes, pos[1]);
        push_f32(&mut bytes, pos[2]);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, color[0]);
        push_f32(&mut bytes, color[1]);
        push_f32(&mut bytes, color[2]);
        push_f32(&mut bytes, 0.0);
    }
    for index in 0..MAX_OCCLUDERS {
        let occ = pack.occs.get(index).copied().unwrap_or(GpuOcc {
            center: [0.0; 3],
            shape: 0.0,
            half_x: 0.0,
            height: 0.0,
            half_z: 0.0,
            radius: 0.0,
            albedo: [0.0; 3],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        });
        push_f32(&mut bytes, occ.center[0]);
        push_f32(&mut bytes, occ.center[1]);
        push_f32(&mut bytes, occ.center[2]);
        push_f32(&mut bytes, occ.shape);
        push_f32(&mut bytes, occ.half_x);
        push_f32(&mut bytes, occ.height);
        push_f32(&mut bytes, occ.half_z);
        push_f32(&mut bytes, occ.radius);
        push_f32(&mut bytes, occ.albedo[0]);
        push_f32(&mut bytes, occ.albedo[1]);
        push_f32(&mut bytes, occ.albedo[2]);
        push_f32(&mut bytes, occ.absorption);
        push_f32(&mut bytes, occ.reflectance);
        push_f32(&mut bytes, occ.color_mix);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 0.0);
    }
    for index in 0..MAX_OBJECTS {
        let object = pack.objects.get(index);
        push_u32(&mut bytes, object.map(|item| item.first).unwrap_or(0));
        push_u32(&mut bytes, object.map(|item| item.count).unwrap_or(0));
        push_u32(&mut bytes, object.map(|item| item.texture).unwrap_or(0));
        push_u32(&mut bytes, 0);
        let color = object.map(|item| item.color).unwrap_or([0.0; 3]);
        push_f32(&mut bytes, color[0]);
        push_f32(&mut bytes, color[1]);
        push_f32(&mut bytes, color[2]);
        push_f32(&mut bytes, 0.0);
    }
    push_f32(&mut bytes, pack.eye[0]);
    push_f32(&mut bytes, pack.eye[1]);
    push_f32(&mut bytes, pack.eye[2]);
    push_f32(&mut bytes, pack.puffs.len() as f32);
    push_f32(&mut bytes, pack.fire.position[0]);
    push_f32(&mut bytes, pack.fire.position[1]);
    push_f32(&mut bytes, pack.fire.position[2]);
    push_f32(&mut bytes, pack.fire.strength);
    push_f32(&mut bytes, pack.fire.color[0]);
    push_f32(&mut bytes, pack.fire.color[1]);
    push_f32(&mut bytes, pack.fire.color[2]);
    push_f32(&mut bytes, 0.0);
    for index in 0..8 {
        let puff = pack.puffs.get(index);
        let center = puff.map(|puff| puff.center).unwrap_or([0.0; 3]);
        let density = puff.map(|puff| puff.density).unwrap_or(0.0);
        let radius = puff.map(|puff| puff.radius).unwrap_or(0.0);
        push_f32(&mut bytes, center[0]);
        push_f32(&mut bytes, center[1]);
        push_f32(&mut bytes, center[2]);
        push_f32(&mut bytes, density);
        push_f32(&mut bytes, radius);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 0.0);
    }
    push_f32(&mut bytes, pack.floor_center[0]);
    push_f32(&mut bytes, pack.floor_center[1]);
    push_f32(&mut bytes, pack.floor_center[2]);
    push_f32(&mut bytes, pack.floor_half_x);
    push_f32(&mut bytes, pack.floor_half_z);
    push_f32(&mut bytes, pack.floor_color[0]);
    push_f32(&mut bytes, pack.floor_color[1]);
    push_f32(&mut bytes, pack.floor_color[2]);
    bytes
}

pub fn overlay_vertex(pos: [f32; 3], color: [f32; 3]) -> GpuVertex {
    GpuVertex {
        pos,
        albedo: color,
        normal: [0.0; 3],
        shade: 0.0,
        uv: [-1.0, -1.0],
    }
}

fn field_grid(world: &World) -> (u32, u32, f32, f32, f32, f32, f32, f32) {
    let spacing = 0.16_f32;
    let origin_x = world.scene.floor.position.x - world.scene.floor.half_x;
    let origin_z = world.scene.floor.position.z - world.scene.floor.half_z;
    let span_x = world.scene.floor.half_x * 2.0;
    let span_z = world.scene.floor.half_z * 2.0;
    let count_x = ((span_x / spacing).ceil() as u32).clamp(1, FIELD_GRID);
    let count_z = ((span_z / spacing).ceil() as u32).clamp(1, FIELD_GRID);
    let span = span_x.max(span_z).max(1.0);
    let near_end = (span * 0.12).max(spacing * 4.0);
    let far_end = (span * 0.85).max(near_end * 2.0);
    let world_end = (span * 4.0).max(far_end * 2.0);
    (
        count_x, count_z, spacing, origin_x, origin_z, near_end, far_end, world_end,
    )
}

fn lamps_of(world: &World) -> Vec<GpuLamp> {
    world
        .scene
        .lights
        .iter()
        .take(MAX_LAMPS)
        .map(|light| GpuLamp {
            pos: [light.position.x, light.position.y, light.position.z],
            color: light.color,
        })
        .collect()
}

fn occluder(world: &World, object: &crate::world::Object) -> Option<GpuOcc> {
    match &object.kind {
        DrawKind::Fixed(FixedPart::Floor) => None,
        DrawKind::Fixed(FixedPart::Wall(index)) => {
            world.scene.walls.get(*index).map(|wall| GpuOcc {
                center: [wall.position.x, wall.height * 0.5, wall.position.z],
                shape: 0.0,
                half_x: wall.half_x,
                height: wall.height,
                half_z: wall.half_z,
                radius: 0.0,
                albedo: wall.color,
                absorption: wall.absorption,
                reflectance: wall.reflectance,
                color_mix: wall.color_mix,
            })
        }
        DrawKind::Fixed(FixedPart::Solid(index)) => {
            world.scene.solids.get(*index).map(|solid| solid_occ(solid))
        }
        _ => Some(GpuOcc {
            center: object.bounds.center,
            shape: 0.0,
            half_x: object.bounds.half[0],
            height: object.bounds.half[1] * 2.0,
            half_z: object.bounds.half[2],
            radius: 0.0,
            albedo: object_color(world, object),
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }),
    }
}

fn solid_occ(solid: &genos_scene::Solid) -> GpuOcc {
    let half = solid.size * 0.5;
    let shape = match solid.shape {
        Shape::Square => 0.0,
        Shape::Circle => 1.0,
    };
    GpuOcc {
        center: [solid.position.x, solid.height * 0.5, solid.position.z],
        shape,
        half_x: half,
        height: solid.height,
        half_z: half,
        radius: half,
        albedo: solid.color,
        absorption: solid.absorption,
        reflectance: solid.reflectance,
        color_mix: solid.color_mix,
    }
}

fn object_color(world: &World, object: &crate::world::Object) -> [f32; 3] {
    match &object.kind {
        DrawKind::Fixed(FixedPart::Floor) => world.scene.floor.color,
        DrawKind::Fixed(FixedPart::Wall(index)) => world
            .scene
            .walls
            .get(*index)
            .map(|wall| wall.color)
            .unwrap_or([1.0; 3]),
        DrawKind::Fixed(FixedPart::Solid(index)) => world
            .scene
            .solids
            .get(*index)
            .map(|solid| solid.color)
            .unwrap_or([1.0; 3]),
        DrawKind::Mesh { color, .. }
        | DrawKind::Particles { color, .. }
        | DrawKind::Shader { color, .. } => *color,
    }
}

fn push_object(
    out: &mut Vec<GpuVertex>,
    world: &World,
    object: &crate::world::Object,
    eye: [f32; 3],
    place: Option<AtlasPlace>,
) {
    match &object.kind {
        DrawKind::Fixed(FixedPart::Floor) => push_floor(out, world),
        DrawKind::Fixed(FixedPart::Wall(index)) => {
            if let Some(wall) = world.scene.walls.get(*index) {
                push_box(
                    out,
                    wall.color,
                    [wall.position.x, wall.height * 0.5, wall.position.z],
                    [wall.half_x, wall.height * 0.5, wall.half_z],
                    false,
                );
            }
        }
        DrawKind::Fixed(FixedPart::Solid(index)) => {
            if let Some(solid) = world.scene.solids.get(*index) {
                match solid.shape {
                    Shape::Square => {
                        let half = solid.size * 0.5;
                        push_box(
                            out,
                            solid.color,
                            [solid.position.x, solid.height * 0.5, solid.position.z],
                            [half, solid.height * 0.5, half],
                            true,
                        );
                    }
                    Shape::Circle => push_cylinder(
                        out,
                        solid.color,
                        solid.position.x,
                        solid.position.z,
                        solid.size * 0.5,
                        solid.height,
                    ),
                }
            }
        }
        DrawKind::Mesh {
            vertices,
            color,
            pose,
        } => push_mesh(out, vertices, *color, pose),
        DrawKind::Particles {
            density,
            points,
            size,
            ..
        } => {
            if *density > 0.0 {
                push_fog_billboard(out, points, *size, eye);
            } else {
                push_particle_cards(out, object, eye, place);
            }
        }
        DrawKind::Shader {
            space: ShaderSpace::Mesh,
            displacement: Some(map),
            color,
        } => push_height(out, map, *color),
        DrawKind::Shader { .. } => {}
    }
}

fn push_floor(out: &mut Vec<GpuVertex>, world: &World) {
    let floor = &world.scene.floor;
    let x0 = floor.position.x - floor.half_x;
    let x1 = floor.position.x + floor.half_x;
    let z0 = floor.position.z - floor.half_z;
    let z1 = floor.position.z + floor.half_z;
    let color = floor.color;
    let n = [0.0, 1.0, 0.0];
    push_tri(out, [x0, 0.0, z0], [x1, 0.0, z0], [x1, 0.0, z1], n, color);
    push_tri(out, [x0, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1], n, color);
}

fn push_box(
    out: &mut Vec<GpuVertex>,
    color: [f32; 3],
    center: [f32; 3],
    half: [f32; 3],
    top: bool,
) {
    let x0 = center[0] - half[0];
    let x1 = center[0] + half[0];
    let y0 = center[1] - half[1];
    let y1 = center[1] + half[1];
    let z0 = center[2] - half[2];
    let z1 = center[2] + half[2];
    let face = |out: &mut Vec<GpuVertex>, a, b, c, d, n| {
        push_tri(out, a, b, c, n, color);
        push_tri(out, a, c, d, n, color);
    };
    face(
        out,
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
        [0.0, 0.0, 1.0],
    );
    face(
        out,
        [x1, y0, z0],
        [x0, y0, z0],
        [x0, y1, z0],
        [x1, y1, z0],
        [0.0, 0.0, -1.0],
    );
    face(
        out,
        [x1, y0, z1],
        [x1, y0, z0],
        [x1, y1, z0],
        [x1, y1, z1],
        [1.0, 0.0, 0.0],
    );
    face(
        out,
        [x0, y0, z0],
        [x0, y0, z1],
        [x0, y1, z1],
        [x0, y1, z0],
        [-1.0, 0.0, 0.0],
    );
    if top {
        face(
            out,
            [x0, y1, z1],
            [x1, y1, z1],
            [x1, y1, z0],
            [x0, y1, z0],
            [0.0, 1.0, 0.0],
        );
    }
}

fn push_cylinder(
    out: &mut Vec<GpuVertex>,
    color: [f32; 3],
    x: f32,
    z: f32,
    radius: f32,
    height: f32,
) {
    let sides = 64u32;
    for i in 0..sides {
        let a0 = (i as f32) / sides as f32 * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32) / sides as f32 * std::f32::consts::TAU;
        let n0 = [a0.cos(), 0.0, a0.sin()];
        let n1 = [a1.cos(), 0.0, a1.sin()];
        let p0 = [x + n0[0] * radius, 0.0, z + n0[2] * radius];
        let p1 = [x + n1[0] * radius, 0.0, z + n1[2] * radius];
        let p2 = [p1[0], height, p1[2]];
        let p3 = [p0[0], height, p0[2]];
        // Radial normals meet at the shared edge, so the side has no bright facet line.
        let side = |out: &mut Vec<GpuVertex>, a, b, c, na, nb, nc| {
            for (pos, normal) in [(a, na), (b, nb), (c, nc)] {
                out.push(GpuVertex {
                    pos,
                    albedo: color,
                    normal,
                    shade: 1.0,
                    uv: [-1.0, -1.0],
                });
            }
        };
        side(out, p0, p1, p2, n0, n1, n1);
        side(out, p0, p2, p3, n0, n1, n0);
        // The cap sits just inside the side so a brighter top pixel cannot
        // stick out as a one-pixel ridge.
        let inset = (radius - 0.01).max(0.0);
        let t0 = [x + a0.cos() * inset, height, z + a0.sin() * inset];
        let t1 = [x + a1.cos() * inset, height, z + a1.sin() * inset];
        push_tri(out, [x, height, z], t0, t1, [0.0, 1.0, 0.0], color);
    }
}

fn push_mesh(out: &mut Vec<GpuVertex>, vertices: &[[f32; 3]], color: [f32; 3], pose: &[f32; 16]) {
    for tri in vertices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let a = transform_pose(pose, tri[0]);
        let b = transform_pose(pose, tri[1]);
        let c = transform_pose(pose, tri[2]);
        let n = normal_of(a, b, c);
        push_tri(out, a, b, c, n, color);
    }
}

#[derive(Clone, Copy)]
struct AtlasPlace {
    x: u32,
    y: u32,
    atlas_w: u32,
    atlas_h: u32,
}

struct Atlas {
    bytes: Vec<u8>,
    places: Vec<Option<AtlasPlace>>,
}

impl Atlas {
    fn place(&self, index: usize) -> Option<AtlasPlace> {
        self.places.get(index).copied().flatten()
    }
}

fn build_atlas(world: &World, view_proj: &[f32; 16]) -> Atlas {
    let mut sources: Vec<Option<&ParticleImage>> = Vec::new();
    for object in &world.objects {
        let image = match &object.kind {
            DrawKind::Particles { image, density, .. }
                if *density <= 0.0
                    && !object.hidden
                    && crate::world::in_view(view_proj, object.bounds) =>
            {
                image.as_ref()
            }
            _ => None,
        };
        sources.push(image);
    }
    let mut cursor_x = 0u32;
    let mut cursor_y = 0u32;
    let mut row_h = 0u32;
    let mut used_w = 0u32;
    let mut used_h = 0u32;
    let mut spots: Vec<Option<(u32, u32)>> = Vec::new();
    for image in &sources {
        let Some(image) = image else {
            spots.push(None);
            continue;
        };
        if image.width == 0
            || image.height == 0
            || image.width > ATLAS_EDGE
            || image.height > ATLAS_EDGE
        {
            spots.push(None);
            continue;
        }
        if cursor_x + image.width > ATLAS_EDGE {
            cursor_y += row_h;
            cursor_x = 0;
            row_h = 0;
        }
        if cursor_y + image.height > ATLAS_EDGE {
            spots.push(None);
            continue;
        }
        spots.push(Some((cursor_x, cursor_y)));
        cursor_x += image.width;
        row_h = row_h.max(image.height);
        used_w = used_w.max(cursor_x);
        used_h = used_h.max(cursor_y + image.height);
    }
    if used_w == 0 || used_h == 0 {
        return Atlas {
            bytes: image_bytes(0, 0, &[]),
            places: vec![None; sources.len()],
        };
    }
    let mut pixels = vec![0u8; (used_w * used_h * 4) as usize];
    for (image, spot) in sources.iter().zip(&spots) {
        let (Some(image), Some((x, y))) = (image, spot) else {
            continue;
        };
        for row in 0..image.height {
            for col in 0..image.width {
                let src = ((row * image.width + col) * 4) as usize;
                let dst = (((y + row) * used_w + x + col) * 4) as usize;
                if src + 3 < image.pixels.len() && dst + 3 < pixels.len() {
                    pixels[dst..dst + 4].copy_from_slice(&image.pixels[src..src + 4]);
                }
            }
        }
    }
    let places = spots
        .into_iter()
        .map(|spot| {
            spot.map(|(x, y)| AtlasPlace {
                x,
                y,
                atlas_w: used_w,
                atlas_h: used_h,
            })
        })
        .collect();
    Atlas {
        bytes: image_bytes(used_w, used_h, &pixels),
        places,
    }
}

fn image_bytes(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_u32(&mut bytes, width);
    push_u32(&mut bytes, height);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    let count = (width * height) as usize;
    for index in 0..count {
        let offset = index * 4;
        let r = pixels.get(offset).copied().unwrap_or(0) as u32;
        let g = pixels.get(offset + 1).copied().unwrap_or(0) as u32;
        let b = pixels.get(offset + 2).copied().unwrap_or(0) as u32;
        let a = pixels.get(offset + 3).copied().unwrap_or(0) as u32;
        push_u32(&mut bytes, r | (g << 8) | (b << 16) | (a << 24));
    }
    bytes
}

fn push_particle_cards(
    out: &mut Vec<GpuVertex>,
    object: &crate::world::Object,
    eye: [f32; 3],
    place: Option<AtlasPlace>,
) {
    for card in particles::object_cards(object, eye) {
        // 1.25 marks a thin card. The fragment lights either face.
        let shade = if card.lit { 1.25 } else { 0.0 };
        let uv = [
            atlas_uv(&card, 0, place),
            atlas_uv(&card, 1, place),
            atlas_uv(&card, 2, place),
            atlas_uv(&card, 3, place),
        ];
        push_tri_uv(
            out,
            card.corners[0],
            card.corners[1],
            card.corners[2],
            card.normal,
            card.albedo,
            shade,
            [uv[0], uv[1], uv[2]],
        );
        push_tri_uv(
            out,
            card.corners[0],
            card.corners[2],
            card.corners[3],
            card.normal,
            card.albedo,
            shade,
            [uv[0], uv[2], uv[3]],
        );
    }
}

fn atlas_uv(card: &Card, index: usize, place: Option<AtlasPlace>) -> [f32; 2] {
    let Some(place) = place else {
        return [-1.0, -1.0];
    };
    if !card.textured || place.atlas_w == 0 || place.atlas_h == 0 {
        return [-1.0, -1.0];
    }
    let corner = card.uv[index];
    if corner[0] < 0.0 {
        return [-1.0, -1.0];
    }
    let frame: ParticleFrame = card.frame;
    let px = place.x as f32 + frame.x as f32 + corner[0] * frame.width as f32;
    let py = place.y as f32 + frame.y as f32 + corner[1] * frame.height as f32;
    [px / place.atlas_w as f32, py / place.atlas_h as f32]
}

fn push_fog_billboard(out: &mut Vec<GpuVertex>, points: &[[f32; 3]], radius: f32, eye: [f32; 3]) {
    if points.is_empty() {
        return;
    }
    let mut center = [0.0; 3];
    for point in points {
        for axis in 0..3 {
            center[axis] += point[axis];
        }
    }
    let scale = 1.0 / points.len() as f32;
    for value in &mut center {
        *value *= scale;
    }
    let (right, up) = billboard_axes(eye, center);
    // The card has to be nearer than the flames or the depth test hides the fog.
    let mut toward = [eye[0] - center[0], eye[1] - center[1], eye[2] - center[2]];
    let reach = length3(toward).max(0.001);
    toward = scale3(toward, 1.2 / reach);
    let place = [
        center[0] + toward[0],
        center[1] + toward[1],
        center[2] + toward[2],
    ];
    let mut half_w = radius;
    let mut half_h = radius;
    for point in points {
        let dx = point[0] - center[0];
        let dy = point[1] - center[1];
        let dz = point[2] - center[2];
        let along_r = (dx * right[0] + dy * right[1] + dz * right[2]).abs() + radius;
        let along_u = (dx * up[0] + dy * up[1] + dz * up[2]).abs() + radius;
        half_w = half_w.max(along_r);
        half_h = half_h.max(along_u);
    }
    let corner = |rx: f32, uy: f32| {
        [
            place[0] + right[0] * rx + up[0] * uy,
            place[1] + right[1] * rx + up[1] * uy,
            place[2] + right[2] * rx + up[2] * uy,
        ]
    };
    let n = [eye[0] - place[0], eye[1] - place[1], eye[2] - place[2]];
    push_tri_shade(
        out,
        corner(-half_w, -half_h),
        corner(half_w, -half_h),
        corner(half_w, half_h),
        n,
        [0.0; 3],
        2.0,
    );
    push_tri_shade(
        out,
        corner(-half_w, -half_h),
        corner(half_w, half_h),
        corner(-half_w, half_h),
        n,
        [0.0; 3],
        2.0,
    );
}

fn billboard_axes(eye: [f32; 3], point: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let mut forward = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]];
    let flen = length3(forward).max(1.0e-4);
    forward = scale3(forward, 1.0 / flen);
    let mut right = [-forward[2], 0.0, forward[0]];
    let rlen = length3(right);
    if rlen < 1.0e-4 {
        right = [1.0, 0.0, 0.0];
    } else {
        right = scale3(right, 1.0 / rlen);
    }
    let up = [
        forward[1] * right[2] - forward[2] * right[1],
        forward[2] * right[0] - forward[0] * right[2],
        forward[0] * right[1] - forward[1] * right[0],
    ];
    (right, up)
}

fn length3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn scale3(v: [f32; 3], scale: f32) -> [f32; 3] {
    [v[0] * scale, v[1] * scale, v[2] * scale]
}

fn push_height(out: &mut Vec<GpuVertex>, map: &crate::world::Displacement, color: [f32; 3]) {
    let width = map.width.max(2);
    let height = map.height.max(2);
    let at = |ix: u32, iz: u32| {
        let u = ix as f32 / (width - 1) as f32;
        let v = iz as f32 / (height - 1) as f32;
        let sample = map
            .heights
            .get((iz * map.width + ix) as usize)
            .copied()
            .unwrap_or(0) as f32
            / 255.0;
        [
            map.origin[0] + u * map.scale[0],
            map.origin[1] + sample * map.scale[1],
            map.origin[2] + v * map.scale[2],
        ]
    };
    for iz in 0..height - 1 {
        for ix in 0..width - 1 {
            let a = at(ix, iz);
            let b = at(ix + 1, iz);
            let c = at(ix + 1, iz + 1);
            let d = at(ix, iz + 1);
            let n = [0.0, 1.0, 0.0];
            push_tri(out, a, b, c, n, color);
            push_tri(out, a, c, d, n, color);
        }
    }
}

fn push_tri(
    out: &mut Vec<GpuVertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
) {
    push_tri_shade(out, a, b, c, normal, color, 1.0);
}

fn push_tri_shade(
    out: &mut Vec<GpuVertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    shade: f32,
) {
    push_tri_uv(
        out,
        a,
        b,
        c,
        normal,
        color,
        shade,
        [[-1.0, -1.0], [-1.0, -1.0], [-1.0, -1.0]],
    );
}

fn push_tri_uv(
    out: &mut Vec<GpuVertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    shade: f32,
    uv: [[f32; 2]; 3],
) {
    for (pos, uv) in [(a, uv[0]), (b, uv[1]), (c, uv[2])] {
        out.push(GpuVertex {
            pos,
            albedo: color,
            normal,
            shade,
            uv,
        });
    }
}

fn normal_of(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ux = b[0] - a[0];
    let uy = b[1] - a[1];
    let uz = b[2] - a[2];
    let vx = c[0] - a[0];
    let vy = c[1] - a[1];
    let vz = c[2] - a[2];
    let nx = uy * vz - uz * vy;
    let ny = uz * vx - ux * vz;
    let nz = ux * vy - uy * vx;
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    if len < 1.0e-8 {
        [0.0, 1.0, 0.0]
    } else {
        [nx / len, ny / len, nz / len]
    }
}

fn hash_mesh(verts: &[GpuVertex], objects: &[GpuObject], image: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    verts.len().hash(&mut hasher);
    objects.len().hash(&mut hasher);
    for object in objects {
        object.first.hash(&mut hasher);
        object.count.hash(&mut hasher);
        object.texture.hash(&mut hasher);
        for channel in object.color {
            channel.to_bits().hash(&mut hasher);
        }
    }
    for vertex in verts {
        for value in vertex
            .pos
            .into_iter()
            .chain(vertex.albedo)
            .chain(vertex.normal)
        {
            value.to_bits().hash(&mut hasher);
        }
        vertex.shade.to_bits().hash(&mut hasher);
        vertex.uv[0].to_bits().hash(&mut hasher);
        vertex.uv[1].to_bits().hash(&mut hasher);
    }
    image.len().hash(&mut hasher);
    for (index, byte) in image.iter().enumerate() {
        if index % 64 == 0 {
            byte.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn hash_light(
    lamps: &[GpuLamp],
    occs: &[GpuOcc],
    count_x: u32,
    count_z: u32,
    eye: [f32; 3],
    fire: &FireLight,
    puffs: &[Puff],
    floor_x: f32,
    floor_z: f32,
    floor_half_x: f32,
    floor_half_z: f32,
    floor_color: [f32; 3],
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    count_x.hash(&mut hasher);
    count_z.hash(&mut hasher);
    floor_x.to_bits().hash(&mut hasher);
    floor_z.to_bits().hash(&mut hasher);
    floor_half_x.to_bits().hash(&mut hasher);
    floor_half_z.to_bits().hash(&mut hasher);
    for value in floor_color {
        value.to_bits().hash(&mut hasher);
    }
    lamps.len().hash(&mut hasher);
    for lamp in lamps {
        for value in lamp.pos.into_iter().chain(lamp.color) {
            value.to_bits().hash(&mut hasher);
        }
    }
    occs.len().hash(&mut hasher);
    for occ in occs {
        occ.shape.to_bits().hash(&mut hasher);
        for value in occ
            .center
            .into_iter()
            .chain([occ.half_x, occ.height, occ.half_z, occ.radius])
            .chain(occ.albedo)
            .chain([occ.absorption, occ.reflectance, occ.color_mix])
        {
            value.to_bits().hash(&mut hasher);
        }
    }
    for value in eye {
        value.to_bits().hash(&mut hasher);
    }
    for value in fire
        .position
        .into_iter()
        .chain(fire.color)
        .chain([fire.strength])
    {
        value.to_bits().hash(&mut hasher);
    }
    puffs.len().hash(&mut hasher);
    for puff in puffs {
        for value in puff.center.into_iter().chain([puff.radius, puff.density]) {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_ne_bytes());
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_ne_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use genos_scene::{view_proj, Camera, Floor, Light, Scene, Shape, Solid, Vec3, Wall};

    #[test]
    fn packed_objects_name_their_vertex_ranges() {
        let scene = Scene {
            floor: Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 4.0,
                half_z: 4.0,
                color: [1.0, 1.0, 1.0],
            },
            walls: vec![Wall {
                position: Vec3::new(1.0, 0.0, 0.0),
                half_x: 0.2,
                half_z: 1.0,
                height: 2.0,
                color: [0.8, 0.8, 0.8],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            solids: vec![Solid {
                shape: Shape::Square,
                position: Vec3::new(-1.0, 0.0, 0.5),
                size: 1.0,
                height: 1.0,
                color: [1.0, 0.0, 0.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            lights: vec![Light {
                position: Vec3::new(0.0, 4.0, 0.0),
                color: [1.0, 1.0, 1.0],
            }],
        };
        let world = World::from_scene(scene);
        let camera = Camera::opening();
        let view = view_proj(&camera, 16.0 / 9.0);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        let pack = pack_frame(&world, &view, eye);
        assert!(pack.objects.len() >= 2);
        let mut cursor = 0u32;
        for object in &pack.objects {
            assert_eq!(object.first, cursor, "object ranges must be packed");
            assert!(object.count > 0);
            assert_eq!(object.texture, 0, "an empty texture reference stays zero");
            cursor += object.count;
        }
        assert_eq!(cursor, pack.verts.len() as u32);
        let bytes = scene_bytes(&pack);
        let obj_count = u32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(obj_count, pack.objects.len() as u32);
        let mut moved = world.clone();
        moved.scene.solids[0].position.x += 1.5;
        let again = pack_frame(&moved, &view, eye);
        assert_ne!(
            pack.mesh_key, again.mesh_key,
            "moving a solid kept the mesh key"
        );
    }
}
