#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md"
)]

//! What the renderer can draw, and what it may skip on screen.
//!
//! Fixed parts are the authored floor, walls, and solids. A mesh can move by
//! replacing its pose. Particles are cards. A card can face the camera, sit at
//! an angle, take world light, emit light, and show one texture-map frame.
//! A shader is either a mesh-space grid, optionally displaced by a height map,
//! or a screen-space pass.

use genos_scene::{Scene, Shape, Solid, Vec3};

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

/// One shelf-packed RGBA8 image the material shaders sample.
///
/// `key` is 0 when the atlas is empty. A non-empty atlas never uses key 0.
#[derive(Clone, Debug, Default)]
pub struct MaterialAtlas {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, length `width * height * 4`. Top to bottom.
    /// Empty when [`Self::block_format`] is a block format.
    pub rgba: Vec<u8>,
    pub key: u64,
    /// `VkFormat`. 0 is RGBA8 in `rgba`. 146 is BC7 sRGB, 141 is BC5.
    pub block_format: u32,
    /// Prebuilt mips, finest first. Block bytes, not RGBA. The streamer copies
    /// these and does not filter them (ADR 0035).
    pub block_mips: Vec<crate::material_stream::MipLevel>,
}

impl MaterialAtlas {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let mut atlas = Self {
            width,
            height,
            rgba,
            key: 0,
            block_format: 0,
            block_mips: Vec::new(),
        };
        atlas.rehash();
        atlas
    }

    pub(crate) fn rehash(&mut self) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        if self.width == 0
            || self.height == 0
            || (self.rgba.is_empty() && self.block_mips.is_empty())
        {
            self.key = 0;
            return;
        }
        let mut hasher = DefaultHasher::new();
        self.width.hash(&mut hasher);
        self.height.hash(&mut hasher);
        self.block_format.hash(&mut hasher);
        self.rgba.hash(&mut hasher);
        for mip in &self.block_mips {
            mip.width.hash(&mut hasher);
            mip.height.hash(&mut hasher);
            mip.rgba.hash(&mut hasher);
        }
        let key = hasher.finish();
        self.key = if key == 0 { 1 } else { key };
    }
}

/// G-buffer material flag: rgb is base colour, readers apply `(1 − metallic)`.
pub const MAT_BASE: u32 = 1;
/// Colour-atlas alpha is metallic (opaque materials with a metallic-roughness map).
pub const MAT_ALPHA_METAL: u32 = 2;
/// Normal-atlas alpha is roughness.
pub const MAT_NORMAL_ROUGH: u32 = 4;
/// Normal atlas is a flat normal plus roughness. Do not perturb.
pub const MAT_FLAT_NORMAL: u32 = 8;

/// A rectangle in a [`MaterialAtlas`], in texels. `w == 0` means no texture.
#[derive(Clone, Copy, Debug, Default)]
pub struct Surface {
    pub color: [u32; 4],
    pub normal: [u32; 4],
    /// Discard below this alpha. 0 keeps the whole surface.
    pub cutoff: f32,
    pub two_sided: bool,
    /// Roughness factor used when the normal atlas does not carry roughness.
    pub roughness: f32,
    /// Metallic factor used when the colour atlas alpha is not metallic.
    pub metallic: f32,
    /// [`MAT_BASE`], [`MAT_ALPHA_METAL`], [`MAT_NORMAL_ROUGH`], [`MAT_FLAT_NORMAL`].
    pub material_flags: u32,
}

#[derive(Clone, Debug)]
pub enum DrawKind {
    Fixed(FixedPart),
    Mesh {
        vertices: Vec<[f32; 3]>,
        /// Object-space normals, one per vertex. Empty means a flat face normal.
        normals: Vec<[f32; 3]>,
        /// Texture coordinates, one per vertex. Empty means the vertex albedo.
        texcoords: Vec<[f32; 2]>,
        color: [f32; 3],
        pose: [f32; 16],
        /// Emitted light (linear). Only GI draws it.
        emission: [f32; 3],
        /// Index into [`World::surfaces`]. `None` is a flat colour.
        surface: Option<usize>,
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
        FixedPart::Floor => {
            let place = scene.world_point(scene.floor_handle(), scene.floor.position);
            Some(Bounds {
                center: [place.x, place.y, place.z],
                half: [scene.floor.half_x, 0.5, scene.floor.half_z],
            })
        }
        FixedPart::Ceiling => scene.ceiling.as_ref().map(|ceiling| {
            let place = scene.world_point(scene.floor_handle(), scene.floor.position);
            Bounds {
                center: [place.x, ceiling.height, place.z],
                half: [scene.floor.half_x, 0.05, scene.floor.half_z],
            }
        }),
        FixedPart::Wall(index) => {
            let handle = scene.wall_handle(*index)?;
            let wall = scene.wall_at(*index)?;
            let place = scene.world_point(
                handle,
                Vec3::new(
                    wall.position.x,
                    wall.position.y + wall.base + wall.height * 0.5,
                    wall.position.z,
                ),
            );
            Some(Bounds {
                center: [place.x, place.y, place.z],
                half: [wall.half_x, wall.height * 0.5, wall.half_z],
            })
        }
        FixedPart::Solid(index) => {
            let handle = scene.solid_handle(*index)?;
            let solid = scene.solid_at(*index)?;
            let place = scene.world_point(
                handle,
                Vec3::new(
                    solid.position.x,
                    solid.position.y + solid.height * 0.5,
                    solid.position.z,
                ),
            );
            let reach = solid_reach(solid);
            Some(Bounds {
                center: [place.x, place.y, place.z],
                half: [reach, solid.height * 0.5, reach],
            })
        }
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

/// A planar mirror, a portal opening, or a water surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReflectorKind {
    Mirror,
    Portal,
    Water,
}

/// One reflector the frame may draw. An empty list leaves the picture alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflector {
    pub kind: ReflectorKind,
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    /// Where a portal opens. Mirrors and water leave this at zero.
    pub portal_to: [f32; 3],
    /// Column-major orthonormal basis of the opening. Identity for a mirror.
    pub from_basis: [f32; 9],
    /// Column-major orthonormal basis of the destination.
    pub to_basis: [f32; 9],
    /// Tangent of the window quad. A zero vector is derived from the normal.
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub half_u: f32,
    pub half_v: f32,
    /// Colour written when the view is refused or the reflection misses.
    pub albedo: [f32; 3],
    /// Window pixels `x, y, w, h`. `w == 0` means the whole window.
    pub scissor: [u32; 4],
}

impl Default for Reflector {
    fn default() -> Self {
        Self {
            kind: ReflectorKind::Mirror,
            origin: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            portal_to: [0.0; 3],
            from_basis: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            to_basis: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0],
            bitangent: [0.0, 0.0, 1.0],
            half_u: 1.0,
            half_v: 1.0,
            albedo: [0.5, 0.5, 0.55],
            scissor: [0; 4],
        }
    }
}

/// A finite plane the water shader raycasts. The same quad is drawn as a mesh
/// so the depth buffer and the reflection views see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadePlane {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub half_u: f32,
    pub half_v: f32,
    pub color: [f32; 3],
}

/// Which scene a `Scene` is, from its fixed layout: the floor, walls and
/// ceiling in full, and each solid's shape and size but not its place, turn or
/// colour (solids move). Lights, sky and settings are left out: the renderer
/// follows those incrementally. Two builds of the same scene agree, so
/// rebuilding a world does not drop the surface cache. Never 0, so a renderer
/// that has drawn nothing yet is told apart.
fn scene_identity(scene: &Scene) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let f = |values: &[f32], h: &mut std::collections::hash_map::DefaultHasher| {
        for v in values {
            v.to_bits().hash(h);
        }
    };
    let fl = &scene.floor;
    f(
        &[
            fl.position.x,
            fl.position.y,
            fl.position.z,
            fl.half_x,
            fl.half_z,
        ],
        &mut h,
    );
    f(&fl.color, &mut h);
    scene.wall_len().hash(&mut h);
    for w in scene.walls() {
        f(
            &[
                w.position.x,
                w.position.y,
                w.position.z,
                w.half_x,
                w.half_z,
                w.height,
                w.base,
            ],
            &mut h,
        );
        f(&w.color, &mut h);
    }
    scene.solid_len().hash(&mut h);
    for s in scene.solids() {
        format!("{:?}", s.shape).hash(&mut h);
        f(&[s.size, s.height], &mut h);
    }
    match &scene.ceiling {
        Some(c) => {
            1u8.hash(&mut h);
            f(&[c.height], &mut h);
            f(&c.color, &mut h);
        }
        None => 0u8.hash(&mut h),
    }
    h.finish().max(1)
}

#[derive(Clone, Debug)]
pub struct World {
    pub scene: Scene,
    pub objects: Vec<Object>,
    /// Planar mirrors, portals, and water. Empty skips the reflection passes.
    pub reflectors: Vec<Reflector>,
    /// Planes the water shader raycasts, and meshes the reflection views draw.
    pub shade_planes: Vec<ShadePlane>,
    /// Draw the GGX energy swatch in the corner of the presented frame.
    pub reflection_swatch: bool,
    /// Sky radiance a water reflection uses when no plane is hit.
    pub reflection_sky: [f32; 3],
    /// Meshes the GPU software tracer sees through their SDFs (GI).
    pub fields: Vec<crate::mesh_field::FieldInstance>,
    /// Per-texel materials. Indexed by [`DrawKind::Mesh::surface`].
    pub surfaces: Vec<Surface>,
    /// Base colour (sRGB RGB, linear alpha) for every surface that has one.
    pub color_atlas: MaterialAtlas,
    /// Tangent-space normals (linear RGB) for every surface that has one.
    pub normal_atlas: MaterialAtlas,
    /// More mip chains the frame streams, after the colour and normal atlases.
    /// Each names the object that supplies its distance and derivatives.
    pub stream_textures: Vec<StreamedTexture>,
    /// Which scene this is, from its fixed layout (see `scene_identity`). The
    /// same scene built twice gets the same value. The renderer drops the surface cache when the
    /// value changes. The light key cannot do that: it changes every frame with
    /// the fire and the puffs.
    identity: u64,
}

/// A mip chain the frame registers, viewed through one object.
#[derive(Clone, Debug)]
pub struct StreamedTexture {
    pub atlas: MaterialAtlas,
    pub object: usize,
}

impl World {
    pub fn from_scene(mut scene: Scene) -> Self {
        scene.sync_floor();
        let identity = scene_identity(&scene);
        let mut parts = vec![FixedPart::Floor];
        if scene.ceiling.is_some() {
            parts.push(FixedPart::Ceiling);
        }
        parts.extend((0..scene.wall_len()).map(FixedPart::Wall));
        parts.extend((0..scene.solid_len()).map(FixedPart::Solid));
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
        Self {
            scene,
            objects,
            reflectors: Vec::new(),
            shade_planes: Vec::new(),
            reflection_swatch: false,
            reflection_sky: [0.45, 0.58, 0.72],
            fields: Vec::new(),
            surfaces: Vec::new(),
            color_atlas: MaterialAtlas::empty(),
            normal_atlas: MaterialAtlas::empty(),
            stream_textures: Vec::new(),
            identity,
        }
    }

    /// See the `identity` field.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// Scene the lighting traces. Hidden and off-screen objects stay in it.
    pub fn light_scene(&self) -> Scene {
        let mut scene = self.scene.clone();
        let mut drop_solids = vec![false; scene.solid_len()];
        let mut drop_walls = vec![false; scene.wall_len()];
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
        let walls: Vec<_> = scene
            .walls()
            .cloned()
            .enumerate()
            .filter(|(index, _)| !drop_walls[*index])
            .map(|(_, wall)| wall)
            .collect();
        let mut solids: Vec<_> = scene
            .solids()
            .cloned()
            .enumerate()
            .filter(|(index, _)| !drop_solids[*index])
            .map(|(_, solid)| solid)
            .collect();
        solids.extend(extras);
        let lights = scene.lights_cloned();
        scene = scene.replace_parts(walls, solids, lights);
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
    use genos_scene::{view_proj, Camera, Floor, Light, Scene, Shape, Solid, Vec3};

    pub(super) fn red_scene() -> Scene {
        {
            let mut __scene = Scene::from_lists(
                Floor {
                    position: Vec3::new(0.0, 0.0, 0.0),
                    half_x: 8.0,
                    half_z: 8.0,
                    color: [1.0, 1.0, 1.0],
                },
                Vec::new(),
                vec![Solid {
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
                vec![Light {
                    position: Vec3::new(0.0, 7.0, 0.0),
                    color: [1.0, 1.0, 1.0],

                    direction: Vec3::ZERO,
                }],
            );
            __scene.ceiling = None;
            __scene.sky = None;
            __scene.lighting = genos_scene::LightingSettings::default();
            __scene.debug = genos_scene::DebugSettings::default();
            __scene.fps_counter = genos_scene::FpsCounterSettings::default();
            __scene.event_loop = genos_scene::EventLoopSettings::default();
            __scene.reflections = genos_scene::ReflectionSettings::default();
            __scene.hdr = genos_scene::HdrSettings::legacy_bytes();
            __scene.fog = genos_scene::FogSettings::default();
            __scene.menu = genos_scene::MenuSettings::default();
            __scene
        }
    }

    #[test]
    fn hidden_and_offscreen_objects_stay_in_the_light() {
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
                normals: Vec::new(),
                texcoords: Vec::new(),
                color: [0.0, 1.0, 1.0],
                pose,
                emission: [0.0; 3],
                surface: None,
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

        assert_eq!(world.light_scene().solid_len(), 2);
    }
}

pub fn transform_pose(pose: &[f32; 16], point: [f32; 3]) -> [f32; 3] {
    [
        pose[0] * point[0] + pose[4] * point[1] + pose[8] * point[2] + pose[12],
        pose[1] * point[0] + pose[5] * point[1] + pose[9] * point[2] + pose[13],
        pose[2] * point[0] + pose[6] * point[1] + pose[10] * point[2] + pose[14],
    ]
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn the_same_scene_built_twice_is_one_world() {
        let a = World::from_scene(super::tests::red_scene());
        let b = World::from_scene(super::tests::red_scene());
        assert_eq!(a.identity(), b.identity());
        assert_ne!(a.identity(), 0);
    }

    #[test]
    fn a_moved_lamp_or_solid_keeps_the_world_but_a_new_floor_does_not() {
        let base = super::tests::red_scene();
        let mut lamp = base.clone();
        if let Some(l) = lamp.light_at_mut(0) {
            l.position.x += 1.5;
        }
        lamp.clear_lights();
        let mut solid = base.clone();
        if let Some(s) = solid.solid_at_mut(0) {
            s.position.x += 0.5;
        }
        let mut floor = base.clone();
        floor.floor.half_x += 1.0;
        let id = World::from_scene(base).identity();
        assert_eq!(World::from_scene(lamp).identity(), id);
        assert_eq!(World::from_scene(solid).identity(), id);
        assert_ne!(World::from_scene(floor).identity(), id);
    }
}
