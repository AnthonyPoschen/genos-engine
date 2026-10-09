//! Place a glTF scene in the world as mesh draws (GI v2 phase 1).
//!
//! Each primitive of each instance becomes one `DrawKind::Mesh` with its world pose
//! and its material's mean diffuse albedo and emitted light over the surface (see
//! [`surface_mean`]). Per-texel materials in the raster
//! come with the G-buffer pass; until then a textured surface draws in its mean
//! colour. Lighting still uses the old tracer: a mesh that `affects_light` stands
//! in as its bounding box (see `World::light_scene`).

use genos_load::{GltfScene, Image, PbrMaterial, Primitive, TextureSlot};

use crate::world::{transform_pose, Bounds, DrawKind, Object, World};

impl World {
    /// Add every mesh instance of `scene`, placed by `place` (column-major, applied
    /// after the glTF node transforms). Returns the new objects' indices.
    pub fn add_gltf(
        &mut self,
        scene: &GltfScene,
        place: &[f32; 16],
        affects_light: bool,
    ) -> Vec<usize> {
        self.add_gltf_parts(scene, place, affects_light)
            .into_iter()
            .map(|p| p.object)
            .collect()
    }

    /// Add the scene as [`World::add_gltf`] does, and give every mesh instance its
    /// baked SDF (`World::fields`) for the GI v2 software tracer. The draws do not
    /// stand in as boxes for the old tracer. Returns the new objects' indices.
    pub fn add_gltf_traced(
        &mut self,
        scene: &GltfScene,
        place: &[f32; 16],
        settings: &genos_bake::BakeSettings,
    ) -> Result<Vec<usize>, genos_bake::BakeError> {
        let parts = self.add_gltf_parts(scene, place, false);
        let mut sdfs: Vec<Option<std::sync::Arc<genos_bake::MeshSdf>>> =
            vec![None; scene.meshes.len()];
        let mut instance_at = usize::MAX;
        for (k, part) in parts.iter().enumerate() {
            if part.instance == instance_at {
                continue;
            }
            instance_at = part.instance;
            let mesh = scene.instances[part.instance].mesh;
            if sdfs[mesh].is_none() {
                let baked = genos_bake::load_or_bake(
                    &genos_bake::BakeMesh::from_gltf(scene, mesh),
                    settings,
                )?;
                sdfs[mesh] = Some(std::sync::Arc::new(baked.sdf));
            }
            let mine: Vec<&GltfPart> = parts[k..]
                .iter()
                .take_while(|p| p.instance == part.instance)
                .collect();
            let weight: f32 = mine
                .iter()
                .map(|p| p.triangles as f32)
                .sum::<f32>()
                .max(1.0);
            let albedo = std::array::from_fn(|c| {
                mine.iter()
                    .map(|p| p.color[c] * p.triangles as f32)
                    .sum::<f32>()
                    / weight
            });
            let emission = std::array::from_fn(|c| {
                mine.iter()
                    .map(|p| p.emission[c] * p.triangles as f32)
                    .sum::<f32>()
                    / weight
            });
            if let Some(sdf) = &sdfs[mesh] {
                self.fields.push(crate::mesh_field::FieldInstance {
                    sdf: sdf.clone(),
                    pose: part.pose,
                    albedo,
                    emission,
                    object: Some(part.object),
                });
            }
        }
        Ok(parts.into_iter().map(|p| p.object).collect())
    }

    fn add_gltf_parts(
        &mut self,
        scene: &GltfScene,
        place: &[f32; 16],
        affects_light: bool,
    ) -> Vec<GltfPart> {
        let mut added = Vec::new();
        for (instance_index, instance) in scene.instances.iter().enumerate() {
            let pose = mul(place, &instance.transform);
            let Some(mesh) = scene.meshes.get(instance.mesh) else {
                continue;
            };
            for primitive in &mesh.primitives {
                let vertices: Vec<[f32; 3]> = primitive
                    .indices
                    .iter()
                    .map(|&i| primitive.positions[i as usize])
                    .collect();
                if vertices.is_empty() {
                    continue;
                }
                let material = primitive.material.and_then(|m| scene.materials.get(m));
                let (color, glow) = material.map_or(([1.0; 3], [0.0; 3]), |m| {
                    surface_mean(m, &scene.images, primitive)
                });
                let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                for v in &vertices {
                    let p = transform_pose(&pose, *v);
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
                added.push(GltfPart {
                    instance: instance_index,
                    object: self.objects.len(),
                    pose,
                    color,
                    emission: glow,
                    triangles: vertices.len() / 3,
                });
                self.objects.push(Object {
                    hidden: false,
                    affects_light,
                    bounds: Bounds {
                        center: std::array::from_fn(|a| 0.5 * (lo[a] + hi[a])),
                        half: std::array::from_fn(|a| (0.5 * (hi[a] - lo[a])).max(1.0e-3)),
                    },
                    kind: DrawKind::Mesh {
                        vertices,
                        color,
                        pose,
                        emission: glow,
                    },
                });
            }
        }
        added
    }
}

/// One primitive draw of one glTF instance.
struct GltfPart {
    instance: usize,
    object: usize,
    pose: [f32; 16],
    color: [f32; 3],
    emission: [f32; 3],
    triangles: usize,
}

/// Mean diffuse albedo and emitted light of one primitive (linear), its textures
/// reduced to one colour by sampling them where the surface actually maps
/// (area-weighted points on its triangles), so unused atlas space does not count.
///
/// The engine has no specular yet, so a surface's whole reflectance shows as
/// diffuse: its hemispherical albedo, which for metal and dielectric alike is about
/// the base colour (a metal's reflectance is its base colour; a dielectric loses a
/// few percent to its specular). Metallic and roughness only shape the lobe, which
/// the engine cannot draw, so they do not darken the mean. (Dropping metal to
/// zero, as a diffuse-only reading of the glTF model does, turned DamagedHelmet
/// black.)
pub fn surface_mean(m: &PbrMaterial, images: &[Image], prim: &Primitive) -> ([f32; 3], [f32; 3]) {
    let textured = m.base_color_texture.is_some()
        || m.metallic_roughness_texture.is_some()
        || m.emissive_texture.is_some();
    if !textured || prim.texcoords.len() < prim.positions.len() {
        return (diffuse(m, images), emission(m, images));
    }
    let lut = srgb_lut();
    let tris: Vec<[usize; 3]> = prim
        .indices
        .chunks_exact(3)
        .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
        .filter(|t| t.iter().all(|&i| i < prim.positions.len()))
        .collect();
    let areas: Vec<f64> = tris
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| prim.positions[i]);
            let u: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
            let v: [f32; 3] = std::array::from_fn(|k| c[k] - a[k]);
            let x = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            0.5 * ((x[0] * x[0] + x[1] * x[1] + x[2] * x[2]) as f64).sqrt()
        })
        .collect();
    let total: f64 = areas.iter().sum();
    if tris.is_empty() || total <= 0.0 {
        return (diffuse(m, images), emission(m, images));
    }
    // Points inside each triangle (barycentric), weighted by its share of the area.
    const POINTS: [[f32; 3]; 4] = [
        [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
        [2.0 / 3.0, 1.0 / 6.0, 1.0 / 6.0],
        [1.0 / 6.0, 2.0 / 3.0, 1.0 / 6.0],
        [1.0 / 6.0, 1.0 / 6.0, 2.0 / 3.0],
    ];
    let (mut dsum, mut esum) = ([0.0f64; 3], [0.0f64; 3]);
    for (t, area) in tris.iter().zip(&areas) {
        let w = area / total / POINTS.len() as f64;
        for b in POINTS {
            let uv: [f32; 2] = std::array::from_fn(|k| {
                b[0] * prim.texcoords[t[0]][k]
                    + b[1] * prim.texcoords[t[1]][k]
                    + b[2] * prim.texcoords[t[2]][k]
            });
            let (d, e) = texel_light(m, images, &lut, uv[0], uv[1]);
            for c in 0..3 {
                dsum[c] += w * d[c] as f64;
                esum[c] += w * e[c] as f64;
            }
        }
    }
    (dsum.map(|v| v as f32), esum.map(|v| v as f32))
}

/// Diffuse albedo and emitted light of a material at one texture coordinate.
fn texel_light(
    m: &PbrMaterial,
    images: &[Image],
    lut: &[f32],
    u: f32,
    v: f32,
) -> ([f32; 3], [f32; 3]) {
    let srgb = |slot: Option<TextureSlot>| {
        slot.and_then(|t| slot_texel(&t, images, u, v))
            .map_or([1.0; 3], |p| {
                [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]]
            })
    };
    let base = srgb(m.base_color_texture);
    let glow = srgb(m.emissive_texture);
    (
        std::array::from_fn(|c| m.base_color[c] * base[c]),
        std::array::from_fn(|c| m.emissive[c] * glow[c] * m.emissive_strength),
    )
}

/// Mean diffuse albedo of a material over its whole texture space (see
/// [`surface_mean`], used when a primitive has no texture coordinates).
pub fn diffuse(m: &PbrMaterial, images: &[Image]) -> [f32; 3] {
    let lut = srgb_lut();
    // A grid over texture space, fine enough for a mean and cheap on 4k textures.
    const GRID: u32 = 256;
    let mut sum = [0.0f64; 3];
    for gy in 0..GRID {
        for gx in 0..GRID {
            let u = (gx as f32 + 0.5) / GRID as f32;
            let v = (gy as f32 + 0.5) / GRID as f32;
            let (d, _) = texel_light(m, images, &lut, u, v);
            for c in 0..3 {
                sum[c] += d[c] as f64;
            }
        }
    }
    let n = (GRID * GRID) as f64;
    sum.map(|v| (v / n) as f32)
}

/// Mean emitted light of a material (linear): factor, whole texture and strength.
pub fn emission(m: &PbrMaterial, images: &[Image]) -> [f32; 3] {
    let tex = m
        .emissive_texture
        .and_then(|t| images.get(t.image))
        .map_or([1.0; 3], mean_linear);
    std::array::from_fn(|c| m.emissive[c] * tex[c] * m.emissive_strength)
}

const REPEAT: u32 = 10497;
const MIRRORED_REPEAT: u32 = 33648;

/// glTF wrap modes: repeat, mirrored repeat, else clamp to edge.
fn wrap(t: f32, mode: u32) -> f32 {
    match mode {
        REPEAT => t - t.floor(),
        MIRRORED_REPEAT => {
            let f = t.rem_euclid(2.0);
            if f > 1.0 {
                2.0 - f
            } else {
                f
            }
        }
        _ => t.clamp(0.0, 1.0),
    }
}

fn texel_wrapped(img: &Image, u: f32, v: f32, wrap_s: u32, wrap_t: u32) -> [u8; 4] {
    let (u, v) = (wrap(u, wrap_s), wrap(v, wrap_t));
    let x = ((u * img.width as f32) as u32).min(img.width.saturating_sub(1));
    let y = ((v * img.height as f32) as u32).min(img.height.saturating_sub(1));
    let i = ((y * img.width + x) * 4) as usize;
    match img.pixels.get(i..i + 4) {
        Some(p) => [p[0], p[1], p[2], p[3]],
        None => [0; 4],
    }
}

/// The texel a texture slot shows at (u, v), with its sampler's wrap modes.
fn slot_texel(slot: &TextureSlot, images: &[Image], u: f32, v: f32) -> Option<[u8; 4]> {
    let img = images.get(slot.image)?;
    Some(texel_wrapped(
        img,
        u,
        v,
        slot.sampler.wrap_s,
        slot.sampler.wrap_t,
    ))
}

fn srgb_lut() -> Vec<f32> {
    (0..256)
        .map(|v| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect()
}

/// Mean linear colour of an sRGB image.
pub fn mean_linear(img: &Image) -> [f32; 3] {
    let lut = srgb_lut();
    let mut sum = [0.0f64; 3];
    for p in img.pixels.chunks_exact(4) {
        for c in 0..3 {
            sum[c] += lut[p[c] as usize] as f64;
        }
    }
    let n = (img.pixels.len() / 4).max(1) as f64;
    sum.map(|s| (s / n) as f32)
}

fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::identity_pose;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../load/fixtures")
            .join(name)
    }

    #[test]
    fn a_gltf_cube_becomes_a_mesh_draw_with_its_pose_and_colour() {
        let scene = genos_load::load_gltf_file(fixture("cube.gltf")).unwrap();
        let mut world = World::from_scene(genos_scene::Scene {
            floor: genos_scene::Floor {
                position: genos_scene::Vec3::ZERO,
                half_x: 5.0,
                half_z: 5.0,
                color: [1.0; 3],
            },
            walls: Vec::new(),
            solids: Vec::new(),
            lights: Vec::new(),
            ceiling: None,
            sky: None,
        });
        let before = world.objects.len();
        let mut place = identity_pose();
        place[13] = 1.0; // lift 1 m
        let added = world.add_gltf(&scene, &place, true);
        assert_eq!(added, vec![before]);
        let o = &world.objects[before];
        let DrawKind::Mesh {
            vertices,
            color,
            pose,
            ..
        } = &o.kind
        else {
            panic!("not a mesh");
        };
        assert_eq!(vertices.len(), 36);
        // base 0.8, 0.1, 0.1; metallic 0.25 does not darken it (no specular yet)
        assert!((color[0] - 0.8).abs() < 1e-6 && (color[1] - 0.1).abs() < 1e-6);
        // parent T(10,0,0), child R90 S2, then lifted: the cube spans x 8..12, y -1..3.
        assert!(
            (o.bounds.center[0] - 10.0).abs() < 1e-4 && (o.bounds.center[1] - 1.0).abs() < 1e-4
        );
        assert!((o.bounds.half[0] - 2.0).abs() < 1e-4 && (o.bounds.half[1] - 2.0).abs() < 1e-4);
        assert!((pose[13] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn textured_colour_is_the_mean_linear_texel() {
        let img = Image {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 0, 255],
        };
        assert_eq!(mean_linear(&img), [0.5, 0.0, 0.0]);
    }
}
