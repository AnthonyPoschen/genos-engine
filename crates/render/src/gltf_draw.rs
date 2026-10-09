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
        let means: Vec<[f32; 3]> = scene.images.iter().map(mean_linear).collect();
        let mut added = Vec::new();
        for instance in &scene.instances {
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
                let color = material.map_or([1.0; 3], |m| diffuse(m, &means));
                let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                for v in &vertices {
                    let p = transform_pose(&pose, *v);
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
                added.push(self.objects.len());
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
                    },
                });
            }
        }
        added
    }
}

/// Mean diffuse albedo of a material, with textures reduced to their mean colour.
pub fn diffuse(m: &PbrMaterial, image_means: &[[f32; 3]]) -> [f32; 3] {
    let tex = m
        .base_color_texture
        .and_then(|t| image_means.get(t.image))
        .copied()
        .unwrap_or([1.0; 3]);
    // Metallic lives in the blue channel of the metallic-roughness texture (linear).
    let metal_tex = m
        .metallic_roughness_texture
        .and_then(|t| image_means.get(t.image))
        .map_or(1.0, |mean| srgb_to_linear_inverse(mean[2]));
    let k = 1.0 - (m.metallic * metal_tex).clamp(0.0, 1.0);
    std::array::from_fn(|c| m.base_color[c] * tex[c] * k)
}

/// `mean_linear` decodes sRGB; a data texture (metallic) wants the stored value.
/// Re-encoding the mean is close enough for a mean colour.
fn srgb_to_linear_inverse(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Mean linear colour of an sRGB image.
pub fn mean_linear(img: &Image) -> [f32; 3] {
    let lut: Vec<f32> = (0..256)
        .map(|v| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect();
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
        } = &o.kind
        else {
            panic!("not a mesh");
        };
        assert_eq!(vertices.len(), 36);
        // base 0.8, 0.1, 0.1 x (1 - 0.25)
        assert!((color[0] - 0.6).abs() < 1e-6 && (color[1] - 0.075).abs() < 1e-6);
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
