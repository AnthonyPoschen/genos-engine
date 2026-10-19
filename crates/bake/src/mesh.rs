#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::assert_is_empty, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::chunks_exact_to_as_chunks, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::needless_for_each, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::wildcard_enum_match_arm, reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md")]

//! The bake's input: object-space triangles and the material data cards capture.

use genos_load::{AlphaMode, GltfScene, Image, PbrMaterial, TextureSlot};
use genos_mesh::MeshTriangle;

use crate::BakeSettings;

/// A material reduced to what GI reads: diffuse albedo and emission, with textures.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeMaterial {
    pub pbr: PbrMaterial,
}

/// One mesh in object space.
#[derive(Clone, Debug)]
pub struct BakeMesh<'a> {
    pub name: String,
    pub triangles: Vec<MeshTriangle>,
    pub materials: Vec<BakeMaterial>,
    /// Images the materials' texture slots index.
    pub images: &'a [Image],
}

impl<'a> BakeMesh<'a> {
    /// Mesh `index` of a glTF scene, in its own space (instances share the bake).
    /// Primitives without a material use the glTF default material.
    pub fn from_gltf(scene: &'a GltfScene, index: usize) -> BakeMesh<'a> {
        let mesh = &scene.meshes[index];
        let mut materials: Vec<BakeMaterial> = scene
            .materials
            .iter()
            .map(|m| BakeMaterial { pbr: m.clone() })
            .collect();
        let default = materials.len() as u32;
        materials.push(BakeMaterial {
            pbr: PbrMaterial::default(),
        });
        let mut triangles = Vec::new();
        for p in &mesh.primitives {
            let material = p.material.map_or(default, |m| m as u32);
            let two_sided = scene
                .materials
                .get(material as usize)
                .is_some_and(|m| m.double_sided);
            let mask = scene
                .materials
                .get(material as usize)
                .and_then(|m| match m.alpha_mode {
                    AlphaMode::Mask(cutoff) => Some(cutoff),
                    _ => None,
                });
            for t in p.indices.chunks_exact(3) {
                let at = |k: usize| t[k] as usize;
                if (0..3).any(|k| at(k) >= p.positions.len()) {
                    continue;
                }
                let mut tri =
                    MeshTriangle::flat([0, 1, 2].map(|k| p.positions[at(k)]), material, two_sided);
                if p.normals.len() == p.positions.len() {
                    tri.normals = [0, 1, 2].map(|k| p.normals[at(k)]);
                }
                if p.texcoords.len() == p.positions.len() {
                    tri.uvs = [0, 1, 2].map(|k| p.texcoords[at(k)]);
                }
                if tri.area() <= 0.0 {
                    continue;
                }
                match mask {
                    Some(cutoff) => {
                        if let Some(m) = scene.materials.get(material as usize) {
                            triangles.extend(clip_mask(&tri, cutoff, &scene.images, m, 3));
                        }
                    }
                    None => triangles.push(tri),
                }
            }
        }
        BakeMesh {
            name: mesh.name.clone().unwrap_or_else(|| format!("mesh{index}")),
            triangles,
            materials,
            images: &scene.images,
        }
    }

    /// A plain mesh with one flat material per slot (analytic shapes).
    pub fn plain(
        name: &str,
        triangles: Vec<MeshTriangle>,
        albedo: &[[f32; 3]],
    ) -> BakeMesh<'static> {
        let materials = albedo
            .iter()
            .map(|a| BakeMaterial {
                pbr: PbrMaterial {
                    base_color: [a[0], a[1], a[2], 1.0],
                    metallic: 0.0,
                    ..PbrMaterial::default()
                },
            })
            .collect();
        BakeMesh {
            name: name.to_string(),
            triangles,
            materials,
            images: &[],
        }
    }

    /// Object-space bounds.
    pub fn bounds(&self) -> genos_mesh::Aabb {
        let mut b = genos_mesh::Aabb::EMPTY;
        for t in &self.triangles {
            b = b.union(&t.bounds());
        }
        b
    }

    /// Diffuse albedo (base colour x (1 - metallic), linear) and emission at a hit.
    pub fn surface(&self, material: u32, uv: [f32; 2]) -> ([f32; 3], [f32; 3]) {
        let Some(m) = self.materials.get(material as usize) else {
            return ([0.8; 3], [0.0; 3]);
        };
        let m = &m.pbr;
        let base = self.sample(m.base_color_texture, uv, true);
        let mr = self.sample(m.metallic_roughness_texture, uv, false);
        let metallic = (m.metallic * mr[2]).clamp(0.0, 1.0);
        let albedo = std::array::from_fn(|c| m.base_color[c] * base[c] * (1.0 - metallic));
        let e = self.sample(m.emissive_texture, uv, true);
        let emissive = std::array::from_fn(|c| m.emissive[c] * e[c] * m.emissive_strength);
        (albedo, emissive)
    }

    /// Nearest texel with repeat wrapping (cards are coarser than texels, and the
    /// card capture averages several samples). White when there is no texture.
    fn sample(&self, slot: Option<TextureSlot>, uv: [f32; 2], srgb: bool) -> [f32; 4] {
        let Some(img) = slot.and_then(|s| self.images.get(s.image)) else {
            return [1.0; 4];
        };
        let wrap = |t: f32, n: u32| ((t - t.floor()) * n as f32).min(n as f32 - 1.0) as u32;
        let (x, y) = (wrap(uv[0], img.width), wrap(uv[1], img.height));
        let p = img.pixel(x, y).unwrap_or([255; 4]);
        std::array::from_fn(|c| {
            let v = p[c] as f32 / 255.0;
            if srgb && c < 3 {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            } else {
                v
            }
        })
    }

    /// FNV-1a over everything that changes the result: triangles, materials,
    /// the texels they use, and the settings. Not over the cache mode or path.
    pub fn content_key(&self, s: &BakeSettings) -> u64 {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        h.bytes(&crate::format::VERSION.to_le_bytes());
        for v in [s.voxels_per_side, s.voxel_min, s.voxel_max, s.card_texel] {
            h.f32(v);
        }
        h.bytes(&s.card_max_texels.to_le_bytes());
        for t in &self.triangles {
            for v in t.positions.iter().chain(&t.normals) {
                v.iter().for_each(|&x| h.f32(x));
            }
            t.uvs.iter().flatten().for_each(|&x| h.f32(x));
            h.bytes(&t.material.to_le_bytes());
            h.bytes(&[t.two_sided as u8]);
        }
        for m in &self.materials {
            h.bytes(format!("{:?}", m.pbr).as_bytes());
        }
        for img in self.images {
            h.bytes(&img.width.to_le_bytes());
            h.bytes(&img.height.to_le_bytes());
            h.bytes(&img.pixels);
        }
        h.0
    }
}

/// Subdivide a masked triangle and drop children whose samples fall under `cutoff`.
///
/// Depth 3 is enough for foliage holes in the SDF. A child that is fully opaque or
/// fully clear stops early, so only the edge of a hole keeps splitting.
fn clip_mask(
    tri: &MeshTriangle,
    cutoff: f32,
    images: &[Image],
    material: &PbrMaterial,
    depth: u32,
) -> Vec<MeshTriangle> {
    let alpha = |uv: [f32; 2]| sample_alpha(images, material, uv);
    let samples = [
        alpha(tri.uvs[0]),
        alpha(tri.uvs[1]),
        alpha(tri.uvs[2]),
        alpha(tri.uv_at(1.0 / 3.0, 1.0 / 3.0)),
    ];
    let clear = |a: f32| a < cutoff;
    if samples.iter().all(|a| !clear(*a)) {
        return vec![*tri];
    }
    if samples.iter().all(|a| clear(*a)) {
        return Vec::new();
    }
    if depth == 0 {
        return if clear(samples[3]) {
            Vec::new()
        } else {
            vec![*tri]
        };
    }
    split4(tri)
        .into_iter()
        .flat_map(|child| clip_mask(&child, cutoff, images, material, depth - 1))
        .collect()
}

fn sample_alpha(images: &[Image], material: &PbrMaterial, uv: [f32; 2]) -> f32 {
    let Some(slot) = material.base_color_texture else {
        return material.base_color[3];
    };
    let Some(img) = images.get(slot.image) else {
        return material.base_color[3];
    };
    if img.width == 0 || img.height == 0 {
        return material.base_color[3];
    }
    let wrap = |t: f32, n: u32| ((t - t.floor()) * n as f32).min(n as f32 - 1.0) as u32;
    let texel = img
        .pixel(wrap(uv[0], img.width), wrap(uv[1], img.height))
        .unwrap_or([255; 4]);
    (texel[3] as f32 / 255.0) * material.base_color[3]
}

fn split4(tri: &MeshTriangle) -> [MeshTriangle; 4] {
    let mid_p = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|k| 0.5 * (a[k] + b[k]));
    let mid_n = |a: [f32; 3], b: [f32; 3]| {
        let m: [f32; 3] = std::array::from_fn(|k| 0.5 * (a[k] + b[k]));
        let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
        if len > 1.0e-8 {
            m.map(|v| v / len)
        } else {
            m
        }
    };
    let mid_u = |a: [f32; 2], b: [f32; 2]| std::array::from_fn(|k| 0.5 * (a[k] + b[k]));
    let corner = |i: usize, j: usize| {
        (
            mid_p(tri.positions[i], tri.positions[j]),
            mid_n(tri.normals[i], tri.normals[j]),
            mid_u(tri.uvs[i], tri.uvs[j]),
        )
    };
    let c = |i: usize| (tri.positions[i], tri.normals[i], tri.uvs[i]);
    let m01 = corner(0, 1);
    let m12 = corner(1, 2);
    let m20 = corner(2, 0);
    let child = |a: ([f32; 3], [f32; 3], [f32; 2]),
                 b: ([f32; 3], [f32; 3], [f32; 2]),
                 c: ([f32; 3], [f32; 3], [f32; 2])| MeshTriangle {
        positions: [a.0, b.0, c.0],
        normals: [a.1, b.1, c.1],
        uvs: [a.2, b.2, c.2],
        material: tri.material,
        two_sided: tri.two_sided,
    };
    [
        child(c(0), m01, m20),
        child(m01, c(1), m12),
        child(m20, m12, c(2)),
        child(m01, m12, m20),
    ]
}

struct Fnv(u64);
impl Fnv {
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_bits().to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_load::{AlphaMode, GltfMesh, GltfScene, MeshInstance, Primitive, Sampler, TextureSlot};

    fn mask_scene(alpha: u8) -> GltfScene {
        let mut scene = GltfScene::default();
        scene.images.push(Image {
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, alpha],
        });
        scene.materials.push(PbrMaterial {
            base_color_texture: Some(TextureSlot {
                image: 0,
                texcoord: 0,
                sampler: Sampler::default(),
                scale: 1.0,
            }),
            alpha_mode: AlphaMode::Mask(0.5),
            ..PbrMaterial::default()
        });
        scene.meshes.push(GltfMesh {
            name: Some("leaf".into()),
            primitives: vec![Primitive {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                texcoords: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                indices: vec![0, 1, 2],
                material: Some(0),
                ..Primitive::default()
            }],
        });
        scene.instances.push(MeshInstance {
            mesh: 0,
            node_name: None,
            transform: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        });
        scene
    }

    #[test]
    fn a_masked_clear_triangle_is_dropped_and_an_opaque_one_stays() {
        assert!(BakeMesh::from_gltf(&mask_scene(0), 0).triangles.is_empty());
        assert_eq!(BakeMesh::from_gltf(&mask_scene(255), 0).triangles.len(), 1);
    }
}
