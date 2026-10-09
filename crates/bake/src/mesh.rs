//! The bake's input: object-space triangles and the material data cards capture.

use genos_load::{GltfScene, Image, PbrMaterial, TextureSlot};
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
            for t in p.indices.chunks_exact(3) {
                let at = |k: usize| t[k] as usize;
                let mut tri =
                    MeshTriangle::flat([0, 1, 2].map(|k| p.positions[at(k)]), material, two_sided);
                tri.normals = [0, 1, 2].map(|k| p.normals[at(k)]);
                if !p.texcoords.is_empty() {
                    tri.uvs = [0, 1, 2].map(|k| p.texcoords[at(k)]);
                }
                if tri.area() > 0.0 {
                    triangles.push(tri);
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
