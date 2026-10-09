//! Place a glTF scene in the world as mesh draws (GI v2 phase 1).
//!
//! Each primitive of each instance becomes one `DrawKind::Mesh` with its world pose
//! and the material's mean diffuse albedo: base colour factor x the base colour
//! texture's mean linear colour x (1 - metallic). Per-texel materials in the raster
//! come with the G-buffer pass; until then a textured surface draws in its mean
//! colour. Lighting still uses the old tracer: a mesh that `affects_light` stands
//! in as its bounding box (see `World::light_scene`).

use genos_load::{GltfScene, Image, PbrMaterial};

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
        let colors: Vec<[f32; 3]> = scene
            .materials
            .iter()
            .map(|m| diffuse(m, &scene.images))
            .collect();
        let glows: Vec<[f32; 3]> = scene
            .materials
            .iter()
            .map(|m| emission(m, &scene.images))
            .collect();
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
                let color = primitive
                    .material
                    .and_then(|m| colors.get(m).copied())
                    .unwrap_or([1.0; 3]);
                let glow = primitive
                    .material
                    .and_then(|m| glows.get(m).copied())
                    .unwrap_or([0.0; 3]);
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

/// Mean diffuse albedo of a material, with textures reduced to one colour.
///
/// The engine has no specular yet, so a surface's whole reflectance has to show as
/// diffuse. Dielectric texels reflect their base colour. Metal texels have no
/// diffuse lobe; a rough metal scatters its base colour broadly, close to diffuse,
/// while a mirror-like one sends it into a lobe the engine cannot draw, so metal
/// counts as base colour times roughness. Base colour and metallic-roughness are
/// sampled at the same texture coordinates and averaged together, since the mean of
/// a product is not the product of the means (metal parts are often the dark parts).
pub fn diffuse(m: &PbrMaterial, images: &[Image]) -> [f32; 3] {
    let base_img = m.base_color_texture.and_then(|t| images.get(t.image));
    let mr_img = m
        .metallic_roughness_texture
        .and_then(|t| images.get(t.image));
    let lut = srgb_lut();
    // A grid over texture space, fine enough for a mean and cheap on 4k textures.
    const GRID: u32 = 256;
    let mut sum = [0.0f64; 3];
    for gy in 0..GRID {
        for gx in 0..GRID {
            let u = (gx as f32 + 0.5) / GRID as f32;
            let v = (gy as f32 + 0.5) / GRID as f32;
            let base = base_img.map_or([1.0; 3], |img| {
                let p = texel(img, u, v);
                [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]]
            });
            let (rough, metal) = mr_img.map_or((1.0, 1.0), |img| {
                let p = texel(img, u, v);
                (p[1] as f32 / 255.0, p[2] as f32 / 255.0)
            });
            let metal = (m.metallic * metal).clamp(0.0, 1.0);
            let rough = (m.roughness * rough).clamp(0.0, 1.0);
            let k = 1.0 - metal + metal * rough;
            for c in 0..3 {
                sum[c] += (base[c] * k) as f64;
            }
        }
    }
    let n = (GRID * GRID) as f64;
    std::array::from_fn(|c| m.base_color[c] * (sum[c] / n) as f32)
}

/// Mean emitted light of a material (linear): factor, texture and strength.
pub fn emission(m: &PbrMaterial, images: &[Image]) -> [f32; 3] {
    let tex = m
        .emissive_texture
        .and_then(|t| images.get(t.image))
        .map_or([1.0; 3], mean_linear);
    std::array::from_fn(|c| m.emissive[c] * tex[c] * m.emissive_strength)
}

fn texel(img: &Image, u: f32, v: f32) -> [u8; 4] {
    let x = ((u * img.width as f32) as u32).min(img.width.saturating_sub(1));
    let y = ((v * img.height as f32) as u32).min(img.height.saturating_sub(1));
    let i = ((y * img.width + x) * 4) as usize;
    match img.pixels.get(i..i + 4) {
        Some(p) => [p[0], p[1], p[2], p[3]],
        None => [0; 4],
    }
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
        // base 0.8, 0.1, 0.1 x (1 - metal + metal x roughness) = x (1 - 0.25 + 0.125)
        assert!((color[0] - 0.7).abs() < 1e-6 && (color[1] - 0.0875).abs() < 1e-6);
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
