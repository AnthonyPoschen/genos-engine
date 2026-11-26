#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! A glTF model as world-space triangles for the CPU tracers.
//!
//! Every mesh instance is placed by its node transform and the caller's pose.
//! A hit reads the shared shade (`genos_load::shade_sets`) at the interpolated
//! UV: the albedo is the diffuse albedo, and the emission is the textured one.
//!
//! Conventions, matched to the G-buffer (`gltf_draw.rs`, `gbuffer.frag`):
//! - A texel is a hole when the base-colour texel alpha is under the cutoff.
//!   The cutoff is `alphaCutoff` for MASK, 0.5 for BLEND and none for OPAQUE.
//!   The test runs at each hit, and a ray continues through a hole.
//! - The raster does not cull, so every triangle stops a ray from both sides.
//!   The normal faces the ray, for `doubleSided` and one-sided materials alike.
//!
//! The normal map and the occlusion texture are not read: the path tracer uses
//! the vertex normal and traces its own occlusion. A skinned mesh stands in its
//! bind pose.

use genos_load::{shade_sets, transform_point, AlphaMode, GltfScene, Image, PbrMaterial};
use genos_load::{ShadeSample, TextureSlot};
use genos_mesh::{Aabb, Bvh, MeshTriangle, RayHit};

use crate::trace::Hit;

/// TEXCOORD sets a hit interpolates (glTF names 0 through 7).
const MAX_SETS: usize = 8;
/// Holes one ray may pass before it counts as a miss.
const MAX_HOLES: u32 = 256;

/// One TEXCOORD set per entry, at the three corners. `None` is an absent set.
type CornerSets = Vec<Option<[[f32; 2]; 3]>>;

/// Triangles that share one hit rule, with their hierarchy.
#[derive(Clone, Default)]
struct Part {
    triangles: Vec<MeshTriangle>,
    /// Parallel to `triangles`, or empty when every slot reads set 0.
    sets: Vec<CornerSets>,
    bvh: Bvh,
}

#[derive(Clone)]
struct SurfaceShade {
    pbr: PbrMaterial,
    /// A texel whose alpha is under this is a hole. 0 has no holes.
    cutoff: f32,
}

/// The triangles of a glTF scene in world space, with textured materials.
#[derive(Clone)]
pub struct GltfSurface {
    /// Triangles that always stop a ray.
    solid: Part,
    /// Triangles whose material can have holes.
    holed: Part,
    materials: Vec<SurfaceShade>,
    /// Indexed as `GltfScene::images`. An image no kept slot reads is empty.
    images: Vec<Image>,
    bounds: Aabb,
    key: u64,
}

impl std::fmt::Debug for GltfSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GltfSurface")
            .field("triangles", &self.triangle_count())
            .field("holed", &self.holed.triangles.len())
            .field("materials", &self.materials.len())
            .field("bounds", &self.bounds)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl GltfSurface {
    /// Every mesh instance of `scene`, placed by `place` (column-major, applied
    /// after the node transforms, as `World::add_gltf` does). The images the
    /// albedo, the alpha and the emission read are copied once here.
    #[must_use]
    pub fn from_gltf(scene: &GltfScene, place: &[f32; 16]) -> GltfSurface {
        let mut materials: Vec<SurfaceShade> = scene.materials.iter().map(surface_shade).collect();
        let default = materials.len();
        materials.push(surface_shade(&PbrMaterial::default()));
        let mut keep = vec![false; scene.images.len()];
        for material in &materials {
            let slots = [
                material.pbr.base_color_texture,
                material.pbr.metallic_roughness_texture,
                material.pbr.emissive_texture,
            ];
            for slot in slots.into_iter().flatten() {
                if let Some(flag) = keep.get_mut(slot.image) {
                    *flag = true;
                }
            }
        }
        let images: Vec<Image> = scene
            .images
            .iter()
            .zip(&keep)
            .map(|(image, keep)| {
                if *keep {
                    image.clone()
                } else {
                    Image {
                        width: 0,
                        height: 0,
                        pixels: Vec::new(),
                    }
                }
            })
            .collect();
        let keep_sets = materials.iter().any(|m| reads_later_set(&m.pbr));
        let mut solid = Part::default();
        let mut holed = Part::default();
        for instance in &scene.instances {
            let Some(mesh) = scene.meshes.get(instance.mesh) else {
                continue;
            };
            let pose = mul(place, &instance.transform);
            for primitive in &mesh.primitives {
                let index = primitive.material.map_or(default, |m| m.min(default));
                let Ok(slot) = u32::try_from(index) else {
                    continue;
                };
                let shade = &materials[index];
                let part = if shade.cutoff > 0.0 && shade.pbr.base_color_texture.is_some() {
                    &mut holed
                } else {
                    &mut solid
                };
                let count = primitive.positions.len();
                let corner = |i: u32| usize::try_from(i).ok().filter(|at| *at < count);
                for t in primitive.indices.as_chunks::<3>().0 {
                    let (Some(a), Some(b), Some(c)) = (corner(t[0]), corner(t[1]), corner(t[2]))
                    else {
                        continue;
                    };
                    let at = [a, b, c];
                    let positions = at.map(|k| transform_point(&pose, primitive.positions[k]));
                    let mut tri = MeshTriangle::flat(positions, slot, true);
                    if tri.area() <= 0.0 {
                        continue;
                    }
                    if primitive.normals.len() == count {
                        let face = tri.face_normal();
                        tri.normals = at.map(|k| world_normal(&pose, primitive.normals[k], face));
                    }
                    if primitive.texcoords.len() == count {
                        tri.uvs = at.map(|k| primitive.texcoords[k]);
                    }
                    part.triangles.push(tri);
                    if keep_sets {
                        part.sets.push(corner_sets(primitive, at));
                    }
                }
            }
        }
        solid.bvh = Bvh::build(&solid.triangles);
        holed.bvh = Bvh::build(&holed.triangles);
        // `Aabb::union` with an empty box would reach infinity.
        let bounds = match (solid.bvh.bounds(), holed.bvh.bounds()) {
            (a, b) if b.is_empty() => a,
            (a, b) if a.is_empty() => b,
            (a, b) => a.union(&b),
        };
        let key = content_key(&solid, &holed, &materials, &images);
        GltfSurface {
            solid,
            holed,
            materials,
            images,
            bounds,
            key,
        }
    }

    /// Whether the material a triangle names is `doubleSided` in the file.
    /// The material can have holes: a texel alpha under its cutoff lets a ray
    /// through (MASK or BLEND with a base-colour texture).
    #[must_use]
    pub fn holed(&self, material: u32) -> bool {
        usize::try_from(material)
            .ok()
            .and_then(|i| self.materials.get(i))
            .is_some_and(|m| m.cutoff > 0.0 && m.pbr.base_color_texture.is_some())
    }

    /// The diffuse albedo of `tri` (one of [`GltfSurface::triangles`]) at
    /// barycentric `u`, `v`: its material's base colour, with its texture read
    /// at the first TEXCOORD set. No ray: the voxel field's points take their
    /// albedo from here.
    #[must_use]
    pub fn albedo_on(&self, tri: &MeshTriangle, u: f32, v: f32) -> [f32; 3] {
        let material = &self.materials[usize::try_from(tri.material).unwrap_or(0).min(self.materials.len() - 1)];
        shade_sets(&material.pbr, &self.images, &[Some(tri.uv_at(u, v))]).diffuse_albedo()
    }

    /// Every triangle stops a ray from both sides either way.
    #[must_use]
    pub fn double_sided(&self, material: u32) -> bool {
        usize::try_from(material).ok().and_then(|i| self.materials.get(i)).is_some_and(|m| m.pbr.double_sided)
    }

    /// Every triangle in world space, solid ones first.
    pub fn triangles(&self) -> impl Iterator<Item = &MeshTriangle> {
        self.solid.triangles.iter().chain(&self.holed.triangles)
    }

    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.solid.triangles.len() + self.holed.triangles.len()
    }

    /// World-space bounds of every triangle.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// A hash of the triangles, the materials and the texels. Two surfaces
    /// with the same key trace the same picture.
    #[must_use]
    pub fn key(&self) -> u64 {
        self.key
    }

    /// The first surface along `dir` within `reach`. The normal faces the ray.
    #[must_use]
    pub fn trace(&self, origin: [f32; 3], dir: [f32; 3], reach: f32) -> Option<Hit> {
        let solid = self
            .solid
            .bvh
            .intersect(&self.solid.triangles, origin, dir, reach);
        let limit = solid.map_or(reach, |hit| hit.t);
        if let Some((t, hit, shade)) = self.first_holed(origin, dir, limit) {
            return Some(report(&self.holed, origin, dir, t, &hit, shade));
        }
        let hit = solid?;
        let shade = self.shade(&self.solid, &hit).0;
        Some(report(&self.solid, origin, dir, hit.t, &hit, shade))
    }

    /// True when something lies between `from` and `to`. A hole does not count.
    #[must_use]
    pub fn occluded(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        let d: [f32; 3] = std::array::from_fn(|a| to[a] - from[a]);
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist < 1.0e-3 {
            return false;
        }
        let dir = d.map(|c| c / dist);
        let reach = dist - 1.0e-3;
        self.solid
            .bvh
            .occluded(&self.solid.triangles, from, dir, reach)
            || self.first_holed(from, dir, reach).is_some()
    }

    /// The nearest holed triangle whose texel is not a hole: the distance from
    /// `origin`, the hit and its shade.
    fn first_holed(
        &self,
        origin: [f32; 3],
        dir: [f32; 3],
        reach: f32,
    ) -> Option<(f32, RayHit, ShadeSample)> {
        if self.holed.triangles.is_empty() {
            return None;
        }
        let mut walked = 0.0f32;
        for _ in 0..MAX_HOLES {
            let from: [f32; 3] = std::array::from_fn(|a| origin[a] + dir[a] * walked);
            let hit = self
                .holed
                .bvh
                .intersect(&self.holed.triangles, from, dir, reach - walked)?;
            let (shade, cutoff) = self.shade(&self.holed, &hit);
            if shade.texel_alpha >= cutoff {
                return Some((walked + hit.t, hit, shade));
            }
            walked += hit.t;
        }
        None
    }

    /// The shared shade at a hit, and the material's cutoff.
    fn shade(&self, part: &Part, hit: &RayHit) -> (ShadeSample, f32) {
        let at = usize::try_from(hit.triangle).unwrap_or(usize::MAX);
        let tri = &part.triangles[at];
        let material = &self.materials[usize::try_from(tri.material).unwrap_or(0)];
        let Some(sets) = part.sets.get(at) else {
            let uv = [Some(tri.uv_at(hit.u, hit.v))];
            return (
                shade_sets(&material.pbr, &self.images, &uv),
                material.cutoff,
            );
        };
        let w = 1.0 - hit.u - hit.v;
        let mut uvs = [None; MAX_SETS];
        let count = sets.len().min(MAX_SETS);
        for (uv, set) in uvs.iter_mut().zip(sets) {
            *uv = set.map(|corners| {
                std::array::from_fn(|axis| {
                    w * corners[0][axis] + hit.u * corners[1][axis] + hit.v * corners[2][axis]
                })
            });
        }
        (
            shade_sets(&material.pbr, &self.images, &uvs[..count]),
            material.cutoff,
        )
    }
}

/// The hit a tracer gets: the normal faces the ray.
fn report(
    part: &Part,
    origin: [f32; 3],
    dir: [f32; 3],
    t: f32,
    hit: &RayHit,
    shade: ShadeSample,
) -> Hit {
    let tri = &part.triangles[usize::try_from(hit.triangle).unwrap_or(usize::MAX)];
    // The face normal picks the side; the shading normal stays on that side.
    let mut face = tri.face_normal();
    if dot(face, dir) > 0.0 {
        face = face.map(|c| -c);
    }
    let mut normal = tri.normal_at(hit.u, hit.v);
    let along = dot(normal, face);
    if along < 0.0 {
        normal = normal.map(|c| -c);
    } else if along.abs() < 1.0e-6 {
        normal = face;
    }
    Hit {
        point: std::array::from_fn(|a| origin[a] + dir[a] * t),
        normal,
        albedo: shade.diffuse_albedo(),
        emissive: shade.emissive,
    }
}

impl crate::world::World {
    /// Build the [`GltfSurface`] of `scene` at `place` and keep it for the CPU
    /// reference tracer (`World::gltf_surface`). Call it beside `add_gltf_traced`
    /// or `add_cooked`, with the same pose. A second call replaces the first.
    pub fn keep_gltf_surface(&mut self, scene: &GltfScene, place: &[f32; 16]) {
        self.gltf_surface = Some(std::sync::Arc::new(GltfSurface::from_gltf(scene, place)));
    }
}

/// The material a hit shades with: the slots the path tracer does not use are
/// dropped, so a hit reads three textures at most.
fn surface_shade(material: &PbrMaterial) -> SurfaceShade {
    SurfaceShade {
        pbr: PbrMaterial {
            normal_texture: None,
            occlusion_texture: None,
            ..material.clone()
        },
        cutoff: match material.alpha_mode {
            AlphaMode::Opaque => 0.0,
            AlphaMode::Mask(cutoff) => cutoff,
            AlphaMode::Blend => 0.5,
        },
    }
}

fn reads_later_set(material: &PbrMaterial) -> bool {
    let later = |slot: Option<TextureSlot>| slot.is_some_and(|s| s.texcoord > 0);
    later(material.base_color_texture)
        || later(material.metallic_roughness_texture)
        || later(material.emissive_texture)
}

fn corner_sets(primitive: &genos_load::Primitive, at: [usize; 3]) -> CornerSets {
    let triple =
        |set: &[[f32; 2]]| (set.len() == primitive.positions.len()).then(|| at.map(|k| set[k]));
    if primitive.texcoord_sets.is_empty() {
        return vec![triple(&primitive.texcoords)];
    }
    primitive
        .texcoord_sets
        .iter()
        .map(|set| triple(set))
        .collect()
}

/// `a * b`, column-major.
fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    std::array::from_fn(|i| {
        let (col, row) = (i / 4, i % 4);
        (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
    })
}

/// A normal through the inverse transpose of `pose` (its cofactors), unit
/// length. A normal that vanishes becomes `face`.
fn world_normal(pose: &[f32; 16], n: [f32; 3], face: [f32; 3]) -> [f32; 3] {
    let c0 = [pose[0], pose[1], pose[2]];
    let c1 = [pose[4], pose[5], pose[6]];
    let c2 = [pose[8], pose[9], pose[10]];
    let (r0, r1, r2) = (cross(c1, c2), cross(c2, c0), cross(c0, c1));
    let det = dot(c0, r0);
    let sign = if det < 0.0 { -1.0 } else { 1.0 };
    let out: [f32; 3] =
        std::array::from_fn(|a| sign * (n[0] * r0[a] + n[1] * r1[a] + n[2] * r2[a]));
    let len = dot(out, out).sqrt();
    if len > 1.0e-12 {
        out.map(|c| c / len)
    } else {
        face
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// FNV-1a over what changes a trace.
fn content_key(solid: &Part, holed: &Part, materials: &[SurfaceShade], images: &[Image]) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    for part in [solid, holed] {
        h.bytes(&part.triangles.len().to_le_bytes());
        for t in &part.triangles {
            for v in t.positions.iter().chain(&t.normals).flatten() {
                h.bytes(&v.to_bits().to_le_bytes());
            }
            for v in t.uvs.iter().flatten() {
                h.bytes(&v.to_bits().to_le_bytes());
            }
            h.bytes(&t.material.to_le_bytes());
        }
        for set in part.sets.iter().flatten() {
            for v in set.iter().flatten().flatten() {
                h.bytes(&v.to_bits().to_le_bytes());
            }
        }
    }
    for m in materials {
        h.bytes(format!("{:?}", m.pbr).as_bytes());
    }
    for image in images {
        h.bytes(&image.width.to_le_bytes());
        h.bytes(&image.height.to_le_bytes());
        h.bytes(&image.pixels);
    }
    h.0
}

struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::identity_pose;
    use genos_load::{GltfMesh, MeshInstance, Primitive, Sampler};

    /// A 2 m square at z = 0 facing +z. The texture's left half (x < 0) is
    /// clear and its right half is opaque red that glows green.
    fn panel(alpha_mode: AlphaMode, double_sided: bool) -> GltfScene {
        let mut scene = GltfScene::default();
        scene.images.push(Image {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 255],
        });
        scene.images.push(Image {
            width: 1,
            height: 1,
            pixels: vec![0, 255, 0, 255],
        });
        let slot = |image| {
            Some(TextureSlot {
                image,
                texcoord: 0,
                sampler: Sampler::default(),
                scale: 1.0,
            })
        };
        scene.materials.push(PbrMaterial {
            base_color_texture: slot(0),
            metallic: 0.5,
            emissive: [1.0, 2.0, 1.0],
            emissive_texture: slot(1),
            alpha_mode,
            double_sided,
            ..PbrMaterial::default()
        });
        scene.meshes.push(GltfMesh {
            name: Some("panel".into()),
            primitives: vec![Primitive {
                positions: vec![
                    [-1.0, -1.0, 0.0],
                    [1.0, -1.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [-1.0, 1.0, 0.0],
                ],
                normals: vec![[0.0, 0.0, 1.0]; 4],
                texcoords: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                indices: vec![0, 1, 2, 0, 2, 3],
                material: Some(0),
                ..Primitive::default()
            }],
        });
        scene.instances.push(MeshInstance {
            mesh: 0,
            node_name: None,
            transform: identity_pose(),
            skin: None,
        });
        scene
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1.0e-5)
    }

    #[test]
    fn a_ray_passes_a_masked_hole_and_stops_at_an_opaque_texel() {
        let surface = GltfSurface::from_gltf(&panel(AlphaMode::Mask(0.5), true), &identity_pose());
        assert_eq!(surface.triangle_count(), 2);
        let toward = [0.0, 0.0, -1.0];
        assert!(surface.trace([-0.5, 0.0, 2.0], toward, 10.0).is_none());
        let hit = surface
            .trace([0.5, 0.0, 2.0], toward, 10.0)
            .expect("the opaque half stops the ray");
        assert!(close(hit.point, [0.5, 0.0, 0.0]), "{hit:?}");
        assert!(close(hit.normal, [0.0, 0.0, 1.0]), "{hit:?}");
        // Base red × (1 − 0.5 metallic); the emission is factor × texel.
        assert!(close(hit.albedo, [0.5, 0.0, 0.0]), "{hit:?}");
        assert!(close(hit.emissive, [0.0, 2.0, 0.0]), "{hit:?}");
        assert!(!surface.occluded([-0.5, 0.0, 2.0], [-0.5, 0.0, -2.0]));
        assert!(surface.occluded([0.5, 0.0, 2.0], [0.5, 0.0, -2.0]));
    }

    #[test]
    fn the_back_of_a_one_sided_face_stops_a_ray_and_its_normal_faces_the_ray() {
        let surface = GltfSurface::from_gltf(&panel(AlphaMode::Opaque, false), &identity_pose());
        // OPAQUE has no holes: the clear half stops the ray too.
        let hit = surface
            .trace([-0.5, 0.0, -2.0], [0.0, 0.0, 1.0], 10.0)
            .expect("the back face stops the ray");
        assert!(close(hit.normal, [0.0, 0.0, -1.0]), "{hit:?}");
        assert!(surface.occluded([-0.5, 0.0, -2.0], [-0.5, 0.0, 2.0]));
    }

    #[test]
    fn the_placement_moves_the_triangles_and_the_key() {
        let scene = panel(AlphaMode::Opaque, true);
        let mut place = identity_pose();
        place[12] = 10.0;
        let here = GltfSurface::from_gltf(&scene, &identity_pose());
        let there = GltfSurface::from_gltf(&scene, &place);
        assert!(there
            .trace([0.5, 0.0, 2.0], [0.0, 0.0, -1.0], 10.0)
            .is_none());
        assert!(there
            .trace([10.5, 0.0, 2.0], [0.0, 0.0, -1.0], 10.0)
            .is_some());
        assert_ne!(here.key(), there.key());
        assert!(close(there.bounds().min, [9.0, -1.0, 0.0]));
    }
}
