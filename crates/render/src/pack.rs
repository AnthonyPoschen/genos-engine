//! Packed mesh and light inputs for the GPU.
//!
//! The CPU writes positions, normals, colors, texture ids, and lamp parameters.
//! It does not evaluate probe radiance or the final lit color. The learned
//! picture still has to hold on the GPU: no lamp is black, a wall blocks the
//! lamp, a face is lit only from just outside when it points at the lamp, a
//! lamp above an object colors the floor all around, a side lamp does not
//! color the far side, and a lit wall brightens the near floor shadow.

use genos_scene::Shape;

use crate::budget::FrameMemory;
use crate::particles::{self, Card, FireLight, Puff};
use crate::world::{
    transform_pose, DrawKind, FixedPart, ParticleFrame, ParticleImage, ShaderSpace, World,
};

pub use genos_scene::{MAX_LAMPS, MAX_OCCLUDERS};
/// Objects the draw rasterizes in one frame.
pub const MAX_OBJECTS: usize = 8192;
/// Bytes of the resident scene block: the fixed fields, then the lamps, occluders and
/// their grids (see [`scene_bytes`]).
pub const SCENE_CAPACITY: usize = 8 << 20;
/// Light at which a point lamp stops: half an 8-bit step of the picture on white
/// paint (the dither hides that much). Shortcut: a lamp adds nothing past
/// [`lamp_range`], losing at most this much light per lamp on an unblocked face; in
/// return a pixel or a probe ray visits only the lamps whose range covers it, so the
/// cost follows the lamps nearby instead of every lamp in the scene.
pub const LAMP_CUTOFF: f32 = 0.5 / 255.0;
/// Lamp brightness unit the shaders use (`LAMP_UNIT` in scene_rays.glsl):
/// irradiance of a unit lamp 1 m away, facing it.
pub const LAMP_UNIT: f32 = 72.0;
/// Occluder cell, in metres. The cell does not grow.
pub(crate) const OCC_CELL: f32 = 2.0;
/// Smallest lamp grid cell, in metres.
const LAMP_CELL: f32 = 4.0;
/// Most lamp grid cells along one axis.
const LAMP_CELLS: f32 = 128.0;
pub const FIELD_GRID: u32 = 128;
/// The gather keeps one field until the player or a lamp moves this far, in meters.
/// A shorter move does not rebuild. Screen rectangles are not part of this decision.
pub const FIELD_PLACE: f32 = 1.0;

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
    /// True when `pos` is the direction the rays travel, not a point.
    pub directional: bool,
    /// Metres past which the lamp adds nothing ([`lamp_range`]). 0 for a sun.
    pub range: f32,
}

/// Distance at which a point lamp of `color` falls to [`LAMP_CUTOFF`] on white paint
/// facing it: the brightest channel times [`LAMP_UNIT`] over the squared distance,
/// times the paint's albedo over π.
pub fn lamp_range(color: [f32; 3]) -> f32 {
    let peak = color.iter().fold(0.0_f32, |m, c| m.max(*c));
    if peak <= 0.0 {
        return 0.0;
    }
    (peak * LAMP_UNIT * genos_scene::DEFAULT_BRDF / LAMP_CUTOFF).sqrt()
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
    /// Diffuse reflectance as an albedo, 0 to 1 (the shaders divide by π). Below zero
    /// selects the game default, white paint 0.8.
    pub reflectance: f32,
    /// Surface-color mix for bounce. Below zero selects a full tint.
    pub color_mix: f32,
    /// Turn about +Y in radians (a square solid's yaw).
    pub yaw: f32,
}

impl GpuOcc {
    /// Half extents on X and on Z of the ground box around the occluder.
    pub fn reach(&self) -> [f32; 2] {
        if self.shape > 0.5 {
            return [self.radius, self.radius];
        }
        let (s, c) = self.yaw.sin_cos();
        let (s, c) = (s.abs(), c.abs());
        [
            c * self.half_x + s * self.half_z,
            s * self.half_x + c * self.half_z,
        ]
    }
}

pub struct Pack {
    pub objects: Vec<GpuObject>,
    pub lamps: Vec<GpuLamp>,
    pub occs: Vec<GpuOcc>,
    pub light_key: u64,
    pub count_x: u32,
    pub count_z: u32,
    pub spacing: f32,
    pub origin_x: f32,
    pub origin_z: f32,
    pub near_end: f32,
    pub far_end: f32,
    pub world_end: f32,
    /// Near, far, and world cascades. The grids stay on the world.
    pub(crate) cascades: [crate::field::CascadePlan; 3],
    pub eye: [f32; 3],
    pub floor_center: [f32; 3],
    pub floor_half_x: f32,
    pub floor_half_z: f32,
    pub floor_color: [f32; 3],
    /// Underside height and color of the roof over the floor. Height 0 leaves it open.
    pub ceiling: [f32; 4],
    /// Radiance of the sky around the scene (genos_scene::Sky); zero for none.
    pub sky: [f32; 3],
    pub fire: FireLight,
    pub puffs: Vec<Puff>,
    pub(crate) view_right: [f32; 3],
    pub(crate) view_up: [f32; 3],
    pub(crate) view_forward: [f32; 3],
    pub(crate) view_tan: f32,
    pub(crate) view_aspect: f32,
    pub(crate) grid_w: u32,
    pub(crate) grid_h: u32,
    /// Near-field rays per pixel plus one, 0 for the shader's own count (scene.frag
    /// NEAR_RAYS). `GENOS_NEAR_RAYS` sets it, so the near field's share of the raster
    /// time can be measured (0 turns it off). The shader reads it from `view_grid.z`.
    pub(crate) near_rays: u32,
    /// Debug word the shader reads from `grid_at.w` (gpu debug_view); 0 is the picture.
    pub(crate) debug_view: u32,
    /// XZ of the last sun's travel direction. Kept after the sun sets so the
    /// ground it used to reach can drop that bounce. `view_up.w` and `sky.w`.
    pub(crate) last_sun: [f32; 2],
    /// Shapes and particle spans in draw order.
    pub draws: Vec<PackedDraw>,
    /// Cells over the occluders and the lamps the light passes walk.
    pub grid: SceneGrid,
}

/// The sparse occluder directory and the lamp grid, as the shaders read them
/// (`scene_rays.glsl`). `words` holds the directory, the lamp cell table, the sun
/// list and the lamp lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneGrid {
    /// Occluder box: origin x, origin z, cell size, origin y.
    pub occ: [f32; 4],
    /// Lamp grid: low x, low z, cell size, notice band.
    pub lamp: [f32; 4],
    /// Cells on x and z of the occluder box, then of the lamp grid.
    pub dims: [u32; 4],
    /// Words where the occluder directory, the lamp cells and the sun list start.
    pub at: [u32; 3],
    pub suns: u32,
    pub words: Vec<u32>,
}

/// Cells of `cell` metres over `lo..hi` on one axis, at least one, at most `cap` (the
/// cell grows to fit).
fn grid_axis(lo: f32, hi: f32, cell: f32, cap: f32) -> (f32, u32) {
    let span = (hi - lo).max(1.0e-3);
    let cell = cell.max(span / cap);
    (cell, ((span / cell).ceil() as u32).max(1))
}

/// Store occluder cells in the camera box and point lamps by their range.
/// A ray jumps to the next stored cell. A point lamp visits the cells its range meets.
pub fn build_grid(lamps: &[GpuLamp], occs: &[GpuOcc], eye: [f32; 3], far: f32) -> SceneGrid {
    let mut grid = SceneGrid::default();
    let (origin, dims, mut words) = crate::occ_grid::occ_directory(occs, eye, far);
    grid.occ = [origin[0], origin[2], OCC_CELL, origin[1]];
    grid.dims[0] = dims[0];
    grid.dims[1] = dims[2];
    // Point lamps by range. Suns go in their own list.
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    let mut suns = Vec::new();
    for (index, lamp) in lamps.iter().enumerate() {
        if lamp.directional {
            suns.push(index as u32);
            continue;
        }
        if lamp.range <= 0.0 {
            continue;
        }
        lo = [
            lo[0].min(lamp.pos[0] - lamp.range),
            lo[1].min(lamp.pos[2] - lamp.range),
        ];
        hi = [
            hi[0].max(lamp.pos[0] + lamp.range),
            hi[1].max(lamp.pos[2] + lamp.range),
        ];
    }
    let mut lamp_lists: Vec<Vec<u32>> = Vec::new();
    if lo[0] <= hi[0] {
        let (cx, _) = grid_axis(lo[0], hi[0], LAMP_CELL, LAMP_CELLS);
        let (cz, _) = grid_axis(lo[1], hi[1], LAMP_CELL, LAMP_CELLS);
        let cell = cx.max(cz);
        let nx = (((hi[0] - lo[0]) / cell).ceil() as u32).max(1);
        let nz = (((hi[1] - lo[1]) / cell).ceil() as u32).max(1);
        grid.lamp = [lo[0], lo[1], cell, crate::probe_tier::notice_band()];
        grid.dims[2] = nx;
        grid.dims[3] = nz;
        lamp_lists = vec![Vec::new(); (nx * nz) as usize];
        for (index, lamp) in lamps.iter().enumerate() {
            if lamp.directional || lamp.range <= 0.0 {
                continue;
            }
            let (px, pz, r) = (lamp.pos[0], lamp.pos[2], lamp.range);
            let x0 = (((px - r - lo[0]) / cell).floor().max(0.0) as u32).min(nx - 1);
            let x1 = (((px + r - lo[0]) / cell).floor().max(0.0) as u32).min(nx - 1);
            let z0 = (((pz - r - lo[1]) / cell).floor().max(0.0) as u32).min(nz - 1);
            let z1 = (((pz + r - lo[1]) / cell).floor().max(0.0) as u32).min(nz - 1);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    // Keep the lamp only where its range circle meets the cell.
                    let cx0 = lo[0] + x as f32 * cell;
                    let cz0 = lo[1] + z as f32 * cell;
                    let dx = (cx0 - px).max(px - (cx0 + cell)).max(0.0);
                    let dz = (cz0 - pz).max(pz - (cz0 + cell)).max(0.0);
                    if dx * dx + dz * dz <= r * r {
                        lamp_lists[(z * nx + x) as usize].push(index as u32);
                    }
                }
            }
        }
    }
    // The directory already fills `words`. The lamp table follows it, then the sun
    // list, then the lamp lists. Occluder list words stay inside the directory.
    let occ_len = words.len() as u32;
    let lamp_cells = lamp_lists.len() as u32;
    grid.at = [0, occ_len, occ_len + lamp_cells * 2];
    grid.suns = suns.len() as u32;
    let lamp_table = words.len();
    words.resize(lamp_table + (lamp_cells * 2) as usize, 0);
    words.extend_from_slice(&suns);
    for (cell, list) in lamp_lists.iter().enumerate() {
        words[lamp_table + cell * 2] = words.len() as u32;
        words[lamp_table + cell * 2 + 1] = list.len() as u32;
        words.extend_from_slice(list);
    }
    grid.words = words;
    // The gather reads the notice band from lamp_grid.w even when no point lamp is binned.
    grid.lamp[3] = crate::probe_tier::notice_band();
    grid
}

/// One resident shape, or one particle span that changes every frame.
pub enum PackedDraw {
    Shape {
        key: u64,
        model: [f32; 16],
        color: [f32; 3],
        /// Emitted light (linear); GI v2 only.
        emission: [f32; 3],
        source: ShapeSource,
    },
    Dynamic(Vec<GpuVertex>),
}

/// Local geometry for a shape block. The pose and the color stay on the instance.
pub enum ShapeSource {
    Floor {
        half_x: f32,
        half_z: f32,
    },
    /// The floor rectangle facing down.
    Ceiling {
        half_x: f32,
        half_z: f32,
    },
    /// `top` and `bottom` add those faces. A box on the floor needs no bottom.
    Box {
        half: [f32; 3],
        top: bool,
        bottom: bool,
    },
    Cylinder {
        radius: f32,
        height: f32,
    },
    Mesh(Vec<[f32; 3]>),
    Baked(Vec<GpuVertex>),
}

pub(crate) fn build_shape(source: &ShapeSource) -> Vec<GpuVertex> {
    let white = [1.0, 1.0, 1.0];
    let mut verts = Vec::new();
    match source {
        ShapeSource::Floor { half_x, half_z } => {
            push_floor_local(&mut verts, *half_x, *half_z, white);
        }
        ShapeSource::Ceiling { half_x, half_z } => {
            push_ceiling_local(&mut verts, *half_x, *half_z, white);
        }
        ShapeSource::Box { half, top, bottom } => {
            push_box(&mut verts, white, [0.0, 0.0, 0.0], *half, *top, *bottom);
        }
        ShapeSource::Cylinder { radius, height } => {
            push_cylinder(&mut verts, white, 0.0, 0.0, *radius, *height);
        }
        ShapeSource::Mesh(positions) => {
            let pose = crate::world::identity_pose();
            push_mesh(&mut verts, positions, white, &pose);
        }
        ShapeSource::Baked(verts_in) => verts = verts_in.clone(),
    }
    verts
}

/// `model` turned by `yaw` radians about +Y in place (column-major): +X goes to
/// (cos, 0, -sin), the same sense as the occluder turn in the light shaders.
pub(crate) fn turned(mut model: [f32; 16], yaw: f32) -> [f32; 16] {
    let (s, c) = yaw.sin_cos();
    model[0] = c;
    model[2] = -s;
    model[8] = s;
    model[10] = c;
    model
}

pub(crate) fn translation(x: f32, y: f32, z: f32) -> [f32; 16] {
    let mut model = crate::world::identity_pose();
    model[12] = x;
    model[13] = y;
    model[14] = z;
    model
}

pub fn pack_frame(
    world: &World,
    view_proj: &[f32; 16],
    eye: [f32; 3],
    memory: &mut FrameMemory,
) -> Pack {
    memory.clear_frames();
    let mut staged: Vec<StagedDraw> = Vec::new();
    let mut shape_bytes = Vec::new();
    let mut occs = Vec::new();
    let (fire, puffs) = particles::medium(world, eye);
    let mut reserved = 0usize;
    for (index, object) in world.objects.iter().enumerate() {
        if object.affects_light {
            if let Some(occ) = occluder(world, object) {
                if occs.len() < MAX_OCCLUDERS {
                    occs.push(occ);
                }
            }
        }
        if object.hidden || !crate::world::in_view(view_proj, object.live_bounds(&world.scene)) {
            continue;
        }
        if reserved >= MAX_OBJECTS {
            continue;
        }
        let color = object_color(world, object);
        if particle_cards(object) {
            staged.push(StagedDraw::Particle { index, color });
            reserved += 1;
            continue;
        }
        let Some(draw) = pack_draw(world, object, eye, None, color) else {
            continue;
        };
        if let PackedDraw::Shape { key, source, .. } = &draw {
            let bytes = memory
                .shape_bytes(*key)
                .unwrap_or_else(|| shape_vertex_bytes(source));
            shape_bytes.push((*key, bytes));
        }
        staged.push(StagedDraw::Ready { draw, color });
        reserved += 1;
    }
    memory.admit_shapes(&shape_bytes);
    let places = resident_atlas(memory, world, view_proj);
    let mut draws = Vec::new();
    let mut objects = Vec::new();
    for item in staged {
        match item {
            StagedDraw::Ready { draw, color } => {
                if let PackedDraw::Shape { key, .. } = &draw {
                    if !memory.contains_shape(*key) {
                        continue;
                    }
                }
                draws.push(draw);
                objects.push(gpu_object(color));
            }
            StagedDraw::Particle { index, color } => {
                let object = &world.objects[index];
                for card in particles::object_cards(object, eye) {
                    if card.textured {
                        memory.note_frame(card.frame);
                    }
                }
                let Some(draw) = pack_draw(
                    world,
                    object,
                    eye,
                    places.get(index).copied().flatten(),
                    color,
                ) else {
                    continue;
                };
                draws.push(draw);
                objects.push(gpu_object(color));
            }
        }
    }
    let lamps = lamps_of(world);
    let cascades = crate::field::cascade_plans(&world.scene.floor);
    let near = cascades[0];
    let count_x = near.count_x;
    let count_z = near.count_z;
    let spacing = near.spacing;
    let origin_x = near.origin_x;
    let origin_z = near.origin_z;
    let near_end = near.t1;
    let far_end = cascades[1].t1;
    let world_end = cascades[2].t1;
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
        ceiling_of(&world.scene),
        sky_of(&world.scene),
    );
    let grid = build_grid(&lamps, &occs, eye, genos_scene::CAMERA_FAR);
    Pack {
        grid,
        objects,
        lamps,
        occs,
        light_key,
        count_x,
        count_z,
        spacing,
        origin_x,
        origin_z,
        near_end,
        far_end,
        world_end,
        cascades,
        eye,
        floor_center: [
            world.scene.floor.position.x,
            world.scene.floor.position.y,
            world.scene.floor.position.z,
        ],
        floor_half_x: world.scene.floor.half_x,
        floor_half_z: world.scene.floor.half_z,
        floor_color: world.scene.floor.color,
        ceiling: ceiling_of(&world.scene),
        sky: sky_of(&world.scene),
        fire,
        puffs,
        view_right: [0.0; 3],
        view_up: [0.0, 1.0, 0.0],
        view_forward: [0.0, 0.0, -1.0],
        view_tan: (30.0_f32.to_radians()).tan(),
        view_aspect: 1.0,
        grid_w: 1,
        grid_h: 1,
        near_rays: near_rays_override(),
        debug_view: 0,
        last_sun: [0.0, -1.0],
        draws,
    }
}

/// `GENOS_NEAR_RAYS` as the scene block wants it: the count plus one, 0 when unset.
fn near_rays_override() -> u32 {
    static RAYS: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *RAYS.get_or_init(|| {
        std::env::var("GENOS_NEAR_RAYS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .map_or(0, |n| n.min(64) + 1)
    })
}

enum StagedDraw {
    Ready { draw: PackedDraw, color: [f32; 3] },
    Particle { index: usize, color: [f32; 3] },
}

fn particle_cards(object: &crate::world::Object) -> bool {
    matches!(object.kind, DrawKind::Particles { density, .. } if density <= 0.0)
}

fn gpu_object(color: [f32; 3]) -> GpuObject {
    GpuObject {
        first: 0,
        count: 1,
        texture: 0,
        color,
    }
}

pub(crate) fn shape_vertex_bytes(source: &ShapeSource) -> u64 {
    (build_shape(source).len() * std::mem::size_of::<GpuVertex>()) as u64
}

fn pack_draw(
    world: &World,
    object: &crate::world::Object,
    eye: [f32; 3],
    place: Option<AtlasPlace>,
    color: [f32; 3],
) -> Option<PackedDraw> {
    match &object.kind {
        DrawKind::Fixed(FixedPart::Floor) => {
            let floor = &world.scene.floor;
            let key = shape_key(1, &[floor.half_x, floor.half_z]);
            Some(PackedDraw::Shape {
                key,
                model: translation(floor.position.x, 0.0, floor.position.z),
                color,
                emission: [0.0; 3],
                source: ShapeSource::Floor {
                    half_x: floor.half_x,
                    half_z: floor.half_z,
                },
            })
        }
        DrawKind::Fixed(FixedPart::Ceiling) => {
            let floor = &world.scene.floor;
            let ceiling = world.scene.ceiling.as_ref()?;
            let key = shape_key(6, &[floor.half_x, floor.half_z]);
            Some(PackedDraw::Shape {
                key,
                model: translation(floor.position.x, ceiling.height, floor.position.z),
                color,
                emission: [0.0; 3],
                source: ShapeSource::Ceiling {
                    half_x: floor.half_x,
                    half_z: floor.half_z,
                },
            })
        }
        DrawKind::Fixed(FixedPart::Wall(index)) => {
            let wall = world.scene.walls.get(*index)?;
            let half = [wall.half_x, wall.height * 0.5, wall.half_z];
            // A wall lower than the eye shows its top; a raised one (lintel, sill, roof
            // slab) shows its underside too. The fourth key value names the faces.
            let bottom = wall.base > 0.0;
            let key = shape_key(
                2,
                &[half[0], half[1], half[2], if bottom { 3.0 } else { 1.0 }],
            );
            Some(PackedDraw::Shape {
                key,
                model: translation(
                    wall.position.x,
                    wall.base + wall.height * 0.5,
                    wall.position.z,
                ),
                color,
                emission: [0.0; 3],
                source: ShapeSource::Box {
                    half,
                    top: true,
                    bottom,
                },
            })
        }
        DrawKind::Fixed(FixedPart::Solid(index)) => {
            let solid = world.scene.solids.get(*index)?;
            match solid.shape {
                Shape::Square => {
                    let half = solid.size * 0.5;
                    let extent = [half, solid.height * 0.5, half];
                    let key = shape_key(2, &[extent[0], extent[1], extent[2], 1.0]);
                    Some(PackedDraw::Shape {
                        key,
                        model: turned(
                            translation(solid.position.x, solid.height * 0.5, solid.position.z),
                            solid.yaw,
                        ),
                        color,
                        emission: [0.0; 3],
                        source: ShapeSource::Box {
                            half: extent,
                            top: true,
                            bottom: false,
                        },
                    })
                }
                Shape::Circle => {
                    let radius = solid.size * 0.5;
                    let key = shape_key(3, &[radius, solid.height]);
                    Some(PackedDraw::Shape {
                        key,
                        model: translation(solid.position.x, 0.0, solid.position.z),
                        color,
                        emission: [0.0; 3],
                        source: ShapeSource::Cylinder {
                            radius,
                            height: solid.height,
                        },
                    })
                }
            }
        }
        DrawKind::Mesh {
            vertices,
            pose,
            emission,
            ..
        } => {
            let key = shape_key_positions(4, vertices);
            Some(PackedDraw::Shape {
                key,
                model: *pose,
                color,
                emission: *emission,
                source: ShapeSource::Mesh(vertices.clone()),
            })
        }
        DrawKind::Particles { .. } => {
            let mut verts = Vec::new();
            push_object(&mut verts, world, object, eye, place);
            if verts.is_empty() {
                None
            } else {
                Some(PackedDraw::Dynamic(verts))
            }
        }
        DrawKind::Shader {
            space: ShaderSpace::Mesh,
            displacement: Some(map),
            ..
        } => {
            let mut baked = Vec::new();
            push_height(&mut baked, map, [1.0, 1.0, 1.0]);
            if baked.is_empty() {
                return None;
            }
            let key = shape_key_verts(5, &baked);
            Some(PackedDraw::Shape {
                key,
                model: crate::world::identity_pose(),
                color,
                emission: [0.0; 3],
                source: ShapeSource::Baked(baked),
            })
        }
        DrawKind::Shader { .. } => None,
    }
}

fn shape_key(tag: u64, values: &[f32]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    for value in values {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn shape_key_positions(tag: u64, positions: &[[f32; 3]]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    for position in positions {
        for value in position {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn shape_key_verts(tag: u64, verts: &[GpuVertex]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    for vertex in verts {
        for value in vertex.pos {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// The roof is a plane like the floor, so the light shaders test it directly and it
/// takes no occluder slot.
fn sky_of(scene: &genos_scene::Scene) -> [f32; 3] {
    scene
        .sky
        .as_ref()
        .map_or([0.0; 3], |sky| sky.color.map(|c| c.max(0.0)))
}

fn ceiling_of(scene: &genos_scene::Scene) -> [f32; 4] {
    match &scene.ceiling {
        Some(ceiling) => [
            ceiling.height,
            ceiling.color[0],
            ceiling.color[1],
            ceiling.color[2],
        ],
        None => [0.0; 4],
    }
}

fn push_ceiling_local(out: &mut Vec<GpuVertex>, half_x: f32, half_z: f32, color: [f32; 3]) {
    let n = [0.0, -1.0, 0.0];
    push_tri(
        out,
        [-half_x, 0.0, -half_z],
        [half_x, 0.0, half_z],
        [half_x, 0.0, -half_z],
        n,
        color,
    );
    push_tri(
        out,
        [-half_x, 0.0, -half_z],
        [-half_x, 0.0, half_z],
        [half_x, 0.0, half_z],
        n,
        color,
    );
}

fn push_floor_local(out: &mut Vec<GpuVertex>, half_x: f32, half_z: f32, color: [f32; 3]) {
    let n = [0.0, 1.0, 0.0];
    push_tri(
        out,
        [-half_x, 0.0, -half_z],
        [half_x, 0.0, -half_z],
        [half_x, 0.0, half_z],
        n,
        color,
    );
    push_tri(
        out,
        [-half_x, 0.0, -half_z],
        [half_x, 0.0, half_z],
        [-half_x, 0.0, half_z],
        n,
        color,
    );
}

/// Corners of the floor, walls, and solids the shaded draw would rasterize.
/// Each triangle is three positions. A second object on the same faces adds its corners.
pub(crate) fn shape_corners(world: &World, view_proj: &[f32; 16]) -> Vec<[f32; 3]> {
    let mut verts = Vec::new();
    for object in &world.objects {
        if object.hidden || !crate::world::in_view(view_proj, object.live_bounds(&world.scene)) {
            continue;
        }
        if !matches!(object.kind, DrawKind::Fixed(_)) {
            continue;
        }
        push_object(&mut verts, world, object, [0.0, 0.0, 0.0], None);
    }
    verts.into_iter().map(|vert| vert.pos).collect()
}

/// Byte offset of the tail in the scene block (scene_data.glsl): the fixed fields end
/// here and the lamps, occluders and grid words follow.
pub const SCENE_TAIL: usize = 704;

/// Bytes matching the std430 scene block the shaders read (scene_data.glsl): the fixed
/// fields, then a tail of 16-byte words with the lamps (two words each), the occluders
/// (four each) and the grid words (four per tail word). Past [`SCENE_CAPACITY`] the
/// last lamps, occluders or grid cells are dropped, never written out of bounds.
pub fn scene_bytes(pack: &Pack) -> Vec<u8> {
    let tail_words =
        |lamps: usize, occs: usize, grid: usize| lamps * 2 + occs * 4 + grid.div_ceil(4);
    let room = (SCENE_CAPACITY - SCENE_TAIL) / 16;
    let mut lamp_count = pack.lamps.len().min(MAX_LAMPS);
    let mut occ_count = pack.occs.len().min(MAX_OCCLUDERS);
    let mut grid = pack.grid.clone();
    if tail_words(lamp_count, occ_count, grid.words.len()) > room {
        // Over capacity: keep the lamps and occluders, drop the grid lists (every cell
        // empty), then trim the lists themselves if even that does not fit.
        grid = build_grid(&[], &[], pack.eye, genos_scene::CAMERA_FAR);
        lamp_count = lamp_count.min(room / 4);
        occ_count = occ_count.min((room - lamp_count * 2) / 4);
    }
    let mut bytes =
        Vec::with_capacity(SCENE_TAIL + tail_words(lamp_count, occ_count, grid.words.len()) * 16);
    push_u32(&mut bytes, lamp_count as u32);
    push_u32(&mut bytes, occ_count as u32);
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
    for cascade in &pack.cascades {
        push_f32(&mut bytes, cascade.spacing);
        push_f32(&mut bytes, cascade.t0);
        push_f32(&mut bytes, cascade.t1);
        push_f32(&mut bytes, cascade.origin_x);
        push_f32(&mut bytes, cascade.origin_z);
        push_f32(&mut bytes, cascade.count_x as f32);
        push_f32(&mut bytes, cascade.count_z as f32);
        push_f32(&mut bytes, cascade.dirs as f32);
        push_f32(&mut bytes, cascade.offset as f32);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 0.0);
        push_f32(&mut bytes, 0.0);
    }
    push_f32(&mut bytes, pack.view_right[0]);
    push_f32(&mut bytes, pack.view_right[1]);
    push_f32(&mut bytes, pack.view_right[2]);
    push_f32(&mut bytes, pack.view_tan);
    push_f32(&mut bytes, pack.view_up[0]);
    push_f32(&mut bytes, pack.view_up[1]);
    push_f32(&mut bytes, pack.view_up[2]);
    // view_up.w: x of the last sun travel direction.
    push_f32(&mut bytes, pack.last_sun[0]);
    push_f32(&mut bytes, pack.view_forward[0]);
    push_f32(&mut bytes, pack.view_forward[1]);
    push_f32(&mut bytes, pack.view_forward[2]);
    push_f32(&mut bytes, pack.view_aspect);
    push_f32(&mut bytes, pack.grid_w as f32);
    push_f32(&mut bytes, pack.grid_h as f32);
    // view_grid.z is the near-field ray override; w is unused.
    push_f32(&mut bytes, pack.near_rays as f32);
    push_f32(&mut bytes, 0.0);
    for value in pack.ceiling {
        push_f32(&mut bytes, value);
    }
    for value in pack.sky {
        push_f32(&mut bytes, value);
    }
    // sky.w: z of the last sun travel direction.
    push_f32(&mut bytes, pack.last_sun[1]);
    for value in grid.occ.into_iter().chain(grid.lamp) {
        push_f32(&mut bytes, value);
    }
    for value in grid.dims {
        push_u32(&mut bytes, value);
    }
    let lamp_at = 0u32;
    let occ_at = lamp_at + lamp_count as u32 * 2;
    let grid_at = occ_at + occ_count as u32 * 4;
    for value in [lamp_at, occ_at, grid_at, grid.suns] {
        push_u32(&mut bytes, value);
    }
    for value in grid.at.into_iter().chain([pack.debug_view]) {
        push_u32(&mut bytes, value);
    }
    debug_assert_eq!(bytes.len(), SCENE_TAIL);
    for lamp in &pack.lamps[..lamp_count] {
        push_f32(&mut bytes, lamp.pos[0]);
        push_f32(&mut bytes, lamp.pos[1]);
        push_f32(&mut bytes, lamp.pos[2]);
        push_f32(&mut bytes, if lamp.directional { 1.0 } else { 0.0 });
        push_f32(&mut bytes, lamp.color[0]);
        push_f32(&mut bytes, lamp.color[1]);
        push_f32(&mut bytes, lamp.color[2]);
        push_f32(&mut bytes, if lamp.directional { 0.0 } else { lamp.range });
    }
    for occ in &pack.occs[..occ_count] {
        let (s, c) = occ.yaw.sin_cos();
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
        push_f32(&mut bytes, c);
        push_f32(&mut bytes, s);
    }
    for word in &grid.words {
        push_u32(&mut bytes, *word);
    }
    while bytes.len() % 16 != 0 {
        push_u32(&mut bytes, 0);
    }
    debug_assert!(
        bytes.len() <= SCENE_CAPACITY,
        "scene block is {} bytes",
        bytes.len()
    );
    bytes
}

pub(crate) fn apply_view(
    pack: &mut Pack,
    camera: &genos_scene::Camera,
    aspect: f32,
    width: u32,
    height: u32,
) {
    let forward = genos_scene::look_direction(camera.yaw, camera.pitch);
    let mut right = forward.cross(genos_scene::Vec3::Y);
    let scale = right.length().max(1.0e-6);
    right = right / scale;
    let up = right.cross(forward);
    let (grid_w, grid_h) = crate::field::screen_grid(width, height);
    pack.view_right = [right.x, right.y, right.z];
    pack.view_up = [up.x, up.y, up.z];
    pack.view_forward = [forward.x, forward.y, forward.z];
    pack.view_tan = (30.0_f32.to_radians()).tan();
    pack.view_aspect = aspect.max(0.01);
    pack.grid_w = grid_w;
    pack.grid_h = grid_h;
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

fn lamps_of(world: &World) -> Vec<GpuLamp> {
    world
        .scene
        .lights
        .iter()
        .take(MAX_LAMPS)
        .map(|light| {
            let span = light.direction.x * light.direction.x
                + light.direction.y * light.direction.y
                + light.direction.z * light.direction.z;
            if span > 1.0e-8 {
                let len = span.sqrt();
                GpuLamp {
                    pos: [
                        light.direction.x / len,
                        light.direction.y / len,
                        light.direction.z / len,
                    ],
                    color: light.color,
                    directional: true,
                    range: 0.0,
                }
            } else {
                GpuLamp {
                    pos: [light.position.x, light.position.y, light.position.z],
                    color: light.color,
                    directional: false,
                    range: lamp_range(light.color),
                }
            }
        })
        .collect()
}

fn occluder(world: &World, object: &crate::world::Object) -> Option<GpuOcc> {
    match &object.kind {
        DrawKind::Fixed(FixedPart::Floor) | DrawKind::Fixed(FixedPart::Ceiling) => None,
        DrawKind::Fixed(FixedPart::Wall(index)) => {
            world.scene.walls.get(*index).map(|wall| GpuOcc {
                center: [
                    wall.position.x,
                    wall.base + wall.height * 0.5,
                    wall.position.z,
                ],
                shape: 0.0,
                half_x: wall.half_x,
                height: wall.height,
                half_z: wall.half_z,
                radius: 0.0,
                albedo: wall.color,
                absorption: wall.absorption,
                reflectance: wall.reflectance,
                color_mix: wall.color_mix,
                yaw: 0.0,
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
            yaw: 0.0,
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
        yaw: match solid.shape {
            Shape::Square => solid.yaw,
            Shape::Circle => 0.0,
        },
    }
}

fn object_color(world: &World, object: &crate::world::Object) -> [f32; 3] {
    match &object.kind {
        DrawKind::Fixed(FixedPart::Floor) => world.scene.floor.color,
        DrawKind::Fixed(FixedPart::Ceiling) => world
            .scene
            .ceiling
            .as_ref()
            .map_or([1.0, 1.0, 1.0], |ceiling| ceiling.color),
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
        DrawKind::Fixed(FixedPart::Ceiling) => {
            if let Some(ceiling) = &world.scene.ceiling {
                let floor = &world.scene.floor;
                let mut local = Vec::new();
                push_ceiling_local(&mut local, floor.half_x, floor.half_z, ceiling.color);
                for mut vertex in local {
                    vertex.pos[0] += floor.position.x;
                    vertex.pos[1] += ceiling.height;
                    vertex.pos[2] += floor.position.z;
                    out.push(vertex);
                }
            }
        }
        DrawKind::Fixed(FixedPart::Wall(index)) => {
            if let Some(wall) = world.scene.walls.get(*index) {
                push_box(
                    out,
                    wall.color,
                    [
                        wall.position.x,
                        wall.base + wall.height * 0.5,
                        wall.position.z,
                    ],
                    [wall.half_x, wall.height * 0.5, wall.half_z],
                    true,
                    wall.base > 0.0,
                );
            }
        }
        DrawKind::Fixed(FixedPart::Solid(index)) => {
            if let Some(solid) = world.scene.solids.get(*index) {
                match solid.shape {
                    Shape::Square => {
                        let half = solid.size * 0.5;
                        let first = out.len();
                        push_box(
                            out,
                            solid.color,
                            [0.0, solid.height * 0.5, 0.0],
                            [half, solid.height * 0.5, half],
                            true,
                            false,
                        );
                        let model = turned(
                            translation(solid.position.x, 0.0, solid.position.z),
                            solid.yaw,
                        );
                        for vert in &mut out[first..] {
                            let [x, y, z] = vert.pos;
                            vert.pos = [
                                model[0] * x + model[8] * z + model[12],
                                y,
                                model[2] * x + model[10] * z + model[14],
                            ];
                            let [nx, ny, nz] = vert.normal;
                            vert.normal = [
                                model[0] * nx + model[8] * nz,
                                ny,
                                model[2] * nx + model[10] * nz,
                            ];
                        }
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
            ..
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
    bottom: bool,
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
    if bottom {
        face(
            out,
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y0, z1],
            [x0, y0, z1],
            [0.0, -1.0, 0.0],
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
    /// Atlas size the frame rectangle was measured in, before a budget scale.
    source_w: u32,
    source_h: u32,
}

struct AtlasLayout {
    width: u32,
    height: u32,
    places: Vec<Option<AtlasPlace>>,
}

fn particle_sources<'a>(world: &'a World, view_proj: &[f32; 16]) -> Vec<Option<&'a ParticleImage>> {
    world
        .objects
        .iter()
        .map(|object| match &object.kind {
            DrawKind::Particles { image, density, .. }
                if *density <= 0.0
                    && !object.hidden
                    && crate::world::in_view(view_proj, object.live_bounds(&world.scene)) =>
            {
                image.as_ref()
            }
            _ => None,
        })
        .collect()
}

fn source_key(sources: &[Option<&ParticleImage>]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for source in sources {
        match source {
            None => 0u8.hash(&mut hasher),
            Some(image) => {
                1u8.hash(&mut hasher);
                image.width.hash(&mut hasher);
                image.height.hash(&mut hasher);
                image.pixels.hash(&mut hasher);
                image.frames.len().hash(&mut hasher);
                for frame in &image.frames {
                    frame.x.hash(&mut hasher);
                    frame.y.hash(&mut hasher);
                    frame.width.hash(&mut hasher);
                    frame.height.hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

fn atlas_layout(sources: &[Option<&ParticleImage>]) -> AtlasLayout {
    let mut cursor_x = 0u32;
    let mut cursor_y = 0u32;
    let mut row_h = 0u32;
    let mut used_w = 0u32;
    let mut used_h = 0u32;
    let mut spots: Vec<Option<(u32, u32)>> = Vec::new();
    for image in sources {
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
        return AtlasLayout {
            width: 0,
            height: 0,
            places: vec![None; sources.len()],
        };
    }
    let places = spots
        .into_iter()
        .map(|spot| {
            spot.map(|(x, y)| AtlasPlace {
                x,
                y,
                atlas_w: used_w,
                atlas_h: used_h,
                source_w: used_w,
                source_h: used_h,
            })
        })
        .collect();
    AtlasLayout {
        width: used_w,
        height: used_h,
        places,
    }
}

/// Copy source pixels into one atlas. A cache hit does not call this.
fn blit_atlas(sources: &[Option<&ParticleImage>], layout: &AtlasLayout) -> Vec<u8> {
    let mut pixels = vec![0u8; (layout.width * layout.height * 4) as usize];
    for (image, place) in sources.iter().zip(&layout.places) {
        let (Some(image), Some(place)) = (image, place) else {
            continue;
        };
        for row in 0..image.height {
            for col in 0..image.width {
                let src = ((row * image.width + col) * 4) as usize;
                let dst = (((place.y + row) * layout.width + place.x + col) * 4) as usize;
                if src + 3 < image.pixels.len() && dst + 3 < pixels.len() {
                    pixels[dst..dst + 4].copy_from_slice(&image.pixels[src..src + 4]);
                }
            }
        }
    }
    pixels
}

fn resident_atlas(
    memory: &mut FrameMemory,
    world: &World,
    view_proj: &[f32; 16],
) -> Vec<Option<AtlasPlace>> {
    let sources = particle_sources(world, view_proj);
    let layout = atlas_layout(&sources);
    if layout.width == 0 || layout.height == 0 {
        return layout.places;
    }
    let key = source_key(&sources);
    if memory.image_matches(key) {
        let (width, height) = memory.image_size();
        memory.admit_image(key, width, height, &[]);
    } else {
        let pixels = blit_atlas(&sources, &layout);
        memory.admit_image(key, layout.width, layout.height, &pixels);
    }
    scale_places(layout, memory)
}

fn scale_places(layout: AtlasLayout, memory: &FrameMemory) -> Vec<Option<AtlasPlace>> {
    if !memory.image_resident() {
        return vec![None; layout.places.len()];
    }
    let (width, height) = memory.image_size();
    if width == layout.width && height == layout.height {
        return layout.places;
    }
    if layout.width == 0 || layout.height == 0 || width == 0 || height == 0 {
        return vec![None; layout.places.len()];
    }
    layout
        .places
        .into_iter()
        .map(|place| {
            place.map(|place| AtlasPlace {
                x: place.x * width / layout.width,
                y: place.y * height / layout.height,
                atlas_w: width,
                atlas_h: height,
                source_w: layout.width,
                source_h: layout.height,
            })
        })
        .collect()
}

pub(crate) fn image_bytes(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
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
    // A budget scale shrinks the atlas. The frame rect is still in source pixels.
    let sx = place.atlas_w as f32 / place.source_w.max(1) as f32;
    let sy = place.atlas_h as f32 / place.source_h.max(1) as f32;
    let px = place.x as f32 + (frame.x as f32 + corner[0] * frame.width as f32) * sx;
    let py = place.y as f32 + (frame.y as f32 + corner[1] * frame.height as f32) * sy;
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
    ceiling: [f32; 4],
    sky: [f32; 3],
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
    for value in floor_color.into_iter().chain(ceiling).chain(sky) {
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

/// Placement captured the last time the field was built.
pub(crate) struct FieldAnchor {
    eye: [f32; 3],
    lamps: Vec<GpuLamp>,
    lamp_count: usize,
    rest: u64,
}

impl FieldAnchor {
    pub(crate) fn capture(pack: &Pack) -> Self {
        let lamp_count = pack.lamps.len().min(MAX_LAMPS);
        let lamps = pack.lamps[..lamp_count].to_vec();
        Self {
            eye: pack.eye,
            lamps,
            lamp_count,
            rest: field_rest(pack),
        }
    }

    /// True when `pack` can keep showing the field this anchor built.
    pub(crate) fn holds(&self, pack: &Pack) -> bool {
        if field_rest(pack) != self.rest || pack.lamps.len() != self.lamp_count {
            return false;
        }
        // The probe grids stay on the world. A camera move does not rebuild them.
        for (lamp, old) in pack.lamps.iter().zip(self.lamps.iter()) {
            if lamp.color.map(f32::to_bits) != old.color.map(f32::to_bits) {
                return false;
            }
            if lamp.directional || old.directional {
                if lamp.directional != old.directional
                    || lamp.pos.map(f32::to_bits) != old.pos.map(f32::to_bits)
                {
                    return false;
                }
                continue;
            }
            let delta = [
                lamp.pos[0] - old.pos[0],
                lamp.pos[1] - old.pos[1],
                lamp.pos[2] - old.pos[2],
            ];
            let dist2 = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
            if dist2 >= FIELD_PLACE * FIELD_PLACE {
                return false;
            }
        }
        true
    }

    pub(crate) fn player(&self) -> [f32; 2] {
        [self.eye[0], self.eye[2]]
    }
}

fn field_rest(pack: &Pack) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    pack.count_x.hash(&mut hasher);
    pack.count_z.hash(&mut hasher);
    for value in pack
        .floor_center
        .into_iter()
        .chain([pack.floor_half_x, pack.floor_half_z])
        .chain(pack.floor_color)
    {
        value.to_bits().hash(&mut hasher);
    }
    pack.lamps.len().hash(&mut hasher);
    for lamp in &pack.lamps {
        for value in lamp.color {
            value.to_bits().hash(&mut hasher);
        }
    }
    pack.occs.len().hash(&mut hasher);
    for occ in &pack.occs {
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
    for value in pack
        .fire
        .position
        .into_iter()
        .chain(pack.fire.color)
        .chain([pack.fire.strength])
    {
        value.to_bits().hash(&mut hasher);
    }
    pack.puffs.len().hash(&mut hasher);
    for puff in &pack.puffs {
        for value in puff.center.into_iter().chain([puff.radius, puff.density]) {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Spacing of the cascade that covers `(x, z)`. The floor uses the near grid. Outside it, the world grid.
pub fn probe_spacing(world: &World, _eye: [f32; 3], x: f32, z: f32) -> f32 {
    let plans = crate::field::cascade_plans(&world.scene.floor);
    let floor = &world.scene.floor;
    let on_floor = (x - floor.position.x).abs() <= floor.half_x
        && (z - floor.position.z).abs() <= floor.half_z;
    if on_floor {
        plans[0].spacing
    } else {
        plans[2].spacing
    }
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
    use crate::FrameMemory;
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
                base: 0.0,
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
                yaw: 0.0,
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

                direction: Vec3::ZERO,
            }],
            ceiling: None,
            sky: None,
        };
        let world = World::from_scene(scene);
        let camera = Camera::opening();
        let view = view_proj(&camera, 16.0 / 9.0);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        let mut memory = FrameMemory::default();
        let pack = pack_frame(&world, &view, eye, &mut memory);
        assert!(pack.objects.len() >= 2);
        for object in &pack.objects {
            assert!(object.count > 0);
            assert_eq!(object.texture, 0, "an empty texture reference stays zero");
        }
        let bytes = scene_bytes(&pack);
        let obj_count = u32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(obj_count, pack.objects.len() as u32);
        let solid = pack.draws.iter().find_map(|draw| match draw {
            PackedDraw::Shape {
                key,
                model,
                color,
                source: ShapeSource::Box { .. },
                ..
            } if color[1] < 0.5 => Some((*key, *model)),
            _ => None,
        });
        let (key, model) = solid.expect("the solid keeps a shape block");
        let mut moved = world.clone();
        moved.scene.solids[0].position.x += 1.5;
        let again = pack_frame(&moved, &view, eye, &mut memory);
        let again_key = again.draws.iter().find_map(|draw| match draw {
            PackedDraw::Shape {
                key: moved_key,
                color,
                source: ShapeSource::Box { .. },
                ..
            } if color[1] < 0.5 => Some(*moved_key),
            _ => None,
        });
        assert_eq!(Some(key), again_key, "a moved solid keeps the shape key");
        let moved_model = again.draws.iter().find_map(|draw| match draw {
            PackedDraw::Shape {
                key: moved_key,
                model,
                source: ShapeSource::Box { .. },
                ..
            } if *moved_key == key => Some(*model),
            _ => None,
        });
        assert_ne!(
            model[12],
            moved_model.expect("the moved solid keeps the same block")[12]
        );
    }

    fn pack_at(eye: [f32; 3], lamp: [f32; 3], solid_x: f32) -> Pack {
        let scene = Scene {
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
                position: Vec3::new(solid_x, 0.0, 0.0),
                size: 1.0,
                height: 1.0,
                color: [1.0, 0.0, 0.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            lights: vec![Light {
                position: Vec3::new(lamp[0], lamp[1], lamp[2]),
                color: [1.0, 1.0, 1.0],

                direction: Vec3::ZERO,
            }],
            ceiling: None,
            sky: None,
        };
        let world = World::from_scene(scene);
        let camera = Camera::opening();
        let view = view_proj(&camera, 1.0);
        let mut memory = FrameMemory::default();
        pack_frame(&world, &view, eye, &mut memory)
    }

    #[test]
    fn a_short_move_keeps_the_field_and_a_real_move_rebuilds_it() {
        let eye = [0.0, 1.7, 0.0];
        let lamp = [0.0, 7.0, 0.0];
        let anchor = FieldAnchor::capture(&pack_at(eye, lamp, 0.0));
        assert!(anchor.holds(&pack_at([0.2, 1.7, -0.1], lamp, 0.0)));
        assert!(anchor.holds(&pack_at(eye, [0.2, 7.0, 0.0], 0.0)));
        assert!(
            anchor.holds(&pack_at([2.0, 1.7, 0.0], lamp, 0.0)),
            "a camera move rebuilt the world grid"
        );
        assert!(!anchor.holds(&pack_at(eye, [0.0, 7.0, 2.0], 0.0)));
        assert!(!anchor.holds(&pack_at(eye, lamp, 1.5)));
        let mut tinted = pack_at(eye, lamp, 0.0);
        tinted.lamps[0].color = [0.2, 0.2, 0.2];
        assert!(!anchor.holds(&tinted));
    }
}
