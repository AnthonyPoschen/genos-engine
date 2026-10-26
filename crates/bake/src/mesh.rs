#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::assert_is_empty, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::chunks_exact_to_as_chunks, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::needless_for_each, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::wildcard_enum_match_arm, reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md")]

//! The bake's input: object-space triangles and the material data cards capture.

use genos_load::{shade_sets, shade_uv, AlphaMode, GltfScene, Image, PbrMaterial, Primitive, ShadeSample};
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
    /// TEXCOORD sets parallel to `triangles`. Empty when every slot can use
    /// `MeshTriangle::uvs` (set 0). Otherwise one entry per triangle, indexed
    /// by TEXCOORD number. `None` is a set the primitive did not have.
    pub uv_sets: Vec<Vec<Option<[[f32; 2]; 3]>>>,
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
        let mut uv_sets = Vec::new();
        let keep_sets = mesh.primitives.iter().any(|p| {
            p.texcoord_sets
                .iter()
                .skip(1)
                .any(|set| !set.is_empty())
        });
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
                let sets = if keep_sets {
                    triangle_sets(p, [at(0), at(1), at(2)])
                } else {
                    Vec::new()
                };
                if let Some(cutoff) = mask {
                    if let Some(m) = scene.materials.get(material as usize) {
                        for captured in clip_mask(&tri, &sets, cutoff, &scene.images, m, 3) {
                            triangles.push(captured.tri);
                            if keep_sets {
                                uv_sets.push(captured.sets);
                            }
                        }
                    }
                } else {
                    triangles.push(tri);
                    if keep_sets {
                        uv_sets.push(sets);
                    }
                }
            }
        }
        BakeMesh {
            name: mesh.name.clone().unwrap_or_else(|| format!("mesh{index}")),
            triangles,
            uv_sets,
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
            uv_sets: Vec::new(),
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

    /// The shared shade at `uv`. Every texture slot is read at that coordinate.
    pub fn shade(&self, material: u32, uv: [f32; 2]) -> ShadeSample {
        let Some(m) = self.materials.get(material as usize) else {
            return ShadeSample::paint([0.8; 3]);
        };
        shade_uv(&m.pbr, self.images, uv)
    }

    /// Diffuse albedo (base colour x (1 - metallic), linear) and emission at a hit.
    ///
    /// Every slot is read at `uv`. A hit that should honour `TextureSlot::texcoord`
    /// uses [`Self::shade_hit`].
    pub fn surface(&self, material: u32, uv: [f32; 2]) -> ([f32; 3], [f32; 3]) {
        let sample = self.shade(material, uv);
        (sample.diffuse_albedo(), sample.emissive)
    }

    /// The shared shade at barycentric `(u, v)` on `triangle`.
    ///
    /// Each slot is read at `TEXCOORD_<slot.texcoord>`. A missing set falls
    /// back to set 0. [`Self::shade`] still samples every slot at one UV.
    pub fn shade_hit(&self, triangle: usize, u: f32, v: f32) -> ShadeSample {
        let Some(tri) = self.triangles.get(triangle) else {
            return ShadeSample::paint([0.8; 3]);
        };
        let Some(material) = self.materials.get(tri.material as usize) else {
            return ShadeSample::paint([0.8; 3]);
        };
        let sets = match self.uv_sets.get(triangle) {
            Some(sets) => sets.as_slice(),
            None => &[],
        };
        shade_bary(&material.pbr, self.images, tri, sets, u, v)
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
        for sets in &self.uv_sets {
            for set in sets {
                match set {
                    Some(corners) => {
                        h.bytes(&[1]);
                        corners.iter().flatten().for_each(|&x| h.f32(x));
                    }
                    None => h.bytes(&[0]),
                }
            }
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

struct Captured {
    tri: MeshTriangle,
    sets: Vec<Option<[[f32; 2]; 3]>>,
}

/// Subdivide a masked triangle and drop children whose samples fall under `cutoff`.
///
/// Depth 3 is enough for foliage holes in the SDF. A child that is fully opaque or
/// fully clear stops early, so only the edge of a hole keeps splitting.
fn clip_mask(
    tri: &MeshTriangle,
    sets: &[Option<[[f32; 2]; 3]>],
    cutoff: f32,
    images: &[Image],
    material: &PbrMaterial,
    depth: u32,
) -> Vec<Captured> {
    let alpha = |u: f32, v: f32| shade_bary(material, images, tri, sets, u, v).alpha;
    let samples = [
        alpha(0.0, 0.0),
        alpha(1.0, 0.0),
        alpha(0.0, 1.0),
        alpha(1.0 / 3.0, 1.0 / 3.0),
    ];
    let clear = |a: f32| a < cutoff;
    let keep = || Captured {
        tri: *tri,
        sets: sets.to_vec(),
    };
    if samples.iter().all(|a| !clear(*a)) {
        return vec![keep()];
    }
    if samples.iter().all(|a| clear(*a)) {
        return Vec::new();
    }
    if depth == 0 {
        return if clear(samples[3]) {
            Vec::new()
        } else {
            vec![keep()]
        };
    }
    let children = split4(tri);
    let child_sets = split_sets(sets);
    children
        .into_iter()
        .zip(child_sets)
        .flat_map(|(child, sets)| clip_mask(&child, &sets, cutoff, images, material, depth - 1))
        .collect()
}

fn triangle_sets(primitive: &Primitive, corners: [usize; 3]) -> Vec<Option<[[f32; 2]; 3]>> {
    let triple = |set: &[[f32; 2]]| -> Option<[[f32; 2]; 3]> {
        if set.len() != primitive.positions.len() {
            return None;
        }
        Some([0, 1, 2].map(|k| set[corners[k]]))
    };
    if primitive.texcoord_sets.is_empty() {
        return vec![triple(&primitive.texcoords)];
    }
    primitive.texcoord_sets.iter().map(|set| triple(set)).collect()
}

fn shade_bary(
    material: &PbrMaterial,
    images: &[Image],
    tri: &MeshTriangle,
    sets: &[Option<[[f32; 2]; 3]>],
    u: f32,
    v: f32,
) -> ShadeSample {
    if sets.is_empty() {
        return shade_uv(material, images, tri.uv_at(u, v));
    }
    let w = 1.0 - u - v;
    let mut uvs = [None; 8];
    let count = sets.len().min(uvs.len());
    for (index, set) in sets.iter().take(count).enumerate() {
        uvs[index] = set.map(|corners| {
            std::array::from_fn(|axis| {
                w * corners[0][axis] + u * corners[1][axis] + v * corners[2][axis]
            })
        });
    }
    shade_sets(material, images, &uvs[..count])
}

fn split_sets(sets: &[Option<[[f32; 2]; 3]>]) -> [Vec<Option<[[f32; 2]; 3]>>; 4] {
    let mut out = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for set in sets {
        let parts = match set {
            None => [None, None, None, None],
            Some(corners) => {
                let mid = |a: [f32; 2], b: [f32; 2]| {
                    std::array::from_fn(|axis| 0.5 * (a[axis] + b[axis]))
                };
                let m01 = mid(corners[0], corners[1]);
                let m12 = mid(corners[1], corners[2]);
                let m20 = mid(corners[2], corners[0]);
                [
                    Some([corners[0], m01, m20]),
                    Some([m01, corners[1], m12]),
                    Some([m20, m12, corners[2]]),
                    Some([m01, m12, m20]),
                ]
            }
        };
        for (child, part) in out.iter_mut().zip(parts) {
            child.push(part);
        }
    }
    out
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

    #[test]
    fn a_hit_reads_each_slot_texcoord() {
        let mut scene = GltfScene::default();
        scene.images.push(Image {
            width: 2,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255,
            ],
        });
        scene.images.push(Image {
            width: 2,
            height: 2,
            pixels: vec![
                0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255,
            ],
        });
        scene.materials.push(PbrMaterial {
            base_color_texture: Some(TextureSlot {
                image: 0,
                texcoord: 0,
                sampler: Sampler::default(),
                scale: 1.0,
            }),
            metallic: 0.0,
            emissive: [1.0, 0.0, 0.0],
            emissive_texture: Some(TextureSlot {
                image: 1,
                texcoord: 1,
                sampler: Sampler::default(),
                scale: 1.0,
            }),
            ..PbrMaterial::default()
        });
        let set0 = vec![[0.25, 0.25], [0.75, 0.25], [0.25, 0.75]];
        let set1 = vec![[0.75, 0.75], [0.25, 0.75], [0.75, 0.25]];
        scene.meshes.push(GltfMesh {
            name: Some("panel".into()),
            primitives: vec![Primitive {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                texcoords: set0.clone(),
                texcoord_sets: vec![set0, set1],
                indices: vec![0, 1, 2],
                material: Some(0),
                ..Primitive::default()
            }],
        });
        let mesh = BakeMesh::from_gltf(&scene, 0);
        let hit = mesh.shade_hit(0, 0.0, 0.0);
        assert!((hit.diffuse_albedo()[0] - 1.0).abs() < 1.0e-5);
        assert!((hit.emissive[0] - 1.0).abs() < 1.0e-5);
        let (_, explicit) = mesh.surface(mesh.triangles[0].material, [0.25, 0.25]);
        assert!(explicit[0] < 1.0e-5);
        assert_eq!(mesh.uv_sets.len(), mesh.triangles.len());
    }
}
