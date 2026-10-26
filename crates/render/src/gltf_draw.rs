#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::chunks_exact_to_as_chunks, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::items_after_statements, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::many_single_char_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::map_unwrap_or, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![expect(clippy::too_many_lines, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::type_complexity, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::unreadable_literal, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! Place a glTF scene in the world as mesh draws.
//!
//! Each primitive of each instance becomes one `DrawKind::Mesh` with its world pose,
//! object-space normals, texture coordinates, and a shelf in the world's material
//! atlases. The instance colour is the base-colour factor. The colour atlas stores
//! sRGB of linear(base texture) × (1 − metallic), so the shader's multiply is the
//! diffuse albedo from [`genos_load::shade_uv`] and metal is not applied twice.
//! There is no specular lobe; roughness stays on the shade sample and is not
//! lit. A mesh with no texture keeps [`surface_mean`], which is that same shade,
//! so metal darkens it. The G-buffer and the world-space cache multiply this
//! albedo; they do not keep a second material formula. The path tracer's Lambert
//! weight is the shade's diffuse albedo.
//!
//! A mesh that `affects_light` stands in as its bounding box for the analytic
//! tracer (`World::light_scene`). `add_gltf_traced` also stores the baked SDF.

use std::collections::HashMap;

use genos_load::{shade_uv, AlphaMode, GltfScene, Image, PbrMaterial, Primitive};

use crate::world::{
    transform_pose, Bounds, DrawKind, MaterialAtlas, Object, Surface, World,
};

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
    /// baked SDF (`World::fields`) for the GI software tracer. The draws do not
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
        let mut color_shelf = Shelf::from_atlas(&self.color_atlas);
        let mut normal_shelf = Shelf::from_atlas(&self.normal_atlas);
        let mut color_changed = false;
        let mut normal_changed = false;
        let mut ready: HashMap<usize, Option<usize>> = HashMap::new();
        let mut added = Vec::new();
        let (mut scene_lo, mut scene_hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for (instance_index, instance) in scene.instances.iter().enumerate() {
            let pose = mul(place, &instance.transform);
            let Some(mesh) = scene.meshes.get(instance.mesh) else {
                continue;
            };
            for primitive in &mesh.primitives {
                let (vertices, normals, texcoords) = expand(primitive);
                if vertices.is_empty() {
                    continue;
                }
                let material = primitive.material.and_then(|m| scene.materials.get(m));
                let (mean, glow) = material.map_or(([1.0; 3], [0.0; 3]), |m| {
                    surface_mean(m, &scene.images, primitive)
                });
                let surface = primitive.material.and_then(|index| {
                    *ready.entry(index).or_insert_with(|| {
                        let m = scene.materials.get(index)?;
                        let built = build_surface(m, &scene.images, &mut color_shelf, &mut normal_shelf);
                        if built.color[2] > 0 {
                            color_changed = true;
                        }
                        if built.normal[2] > 0 {
                            normal_changed = true;
                        }
                        if built.color[2] > 0
                            || built.normal[2] > 0
                            || built.cutoff > 0.0
                            || built.two_sided
                        {
                            let at = self.surfaces.len();
                            self.surfaces.push(built);
                            Some(at)
                        } else {
                            None
                        }
                    })
                });
                let has_uv = texcoords.len() == vertices.len() && !texcoords.is_empty();
                let textured = has_uv && surface.is_some_and(|s| self.surfaces[s].color[2] > 0);
                let draw_color = if textured {
                    material
                        .map(|m| [m.base_color[0], m.base_color[1], m.base_color[2]])
                        .unwrap_or([1.0; 3])
                } else {
                    mean
                };
                let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                for v in &vertices {
                    let p = transform_pose(&pose, *v);
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                        scene_lo[a] = scene_lo[a].min(p[a]);
                        scene_hi[a] = scene_hi[a].max(p[a]);
                    }
                }
                added.push(GltfPart {
                    instance: instance_index,
                    object: self.objects.len(),
                    pose,
                    color: mean,
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
                        normals,
                        texcoords: if has_uv { texcoords } else { Vec::new() },
                        color: draw_color,
                        pose,
                        emission: glow,
                        surface,
                    },
                });
            }
        }
        if scene_lo[0].is_finite() {
            let side = (0..3)
                .map(|a| scene_hi[a] - scene_lo[a])
                .fold(0.0_f32, f32::max);
            assert!(
                side <= 80.0,
                "glTF world extent {side} m exceeds 80 m; the mesh is probably still in centimetres"
            );
        }
        if color_changed {
            self.color_atlas = color_shelf.finish();
        }
        if normal_changed {
            self.normal_atlas = normal_shelf.finish();
        }
        added
    }
}

fn expand(primitive: &Primitive) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>) {
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    let mut texcoords = Vec::new();
    let have_normals = primitive.normals.len() == primitive.positions.len();
    let have_uv = primitive.texcoords.len() == primitive.positions.len();
    for &index in &primitive.indices {
        let i = index as usize;
        if i >= primitive.positions.len() {
            continue;
        }
        vertices.push(primitive.positions[i]);
        if have_normals {
            normals.push(primitive.normals[i]);
        }
        if have_uv {
            texcoords.push(primitive.texcoords[i]);
        }
    }
    if !have_normals {
        normals.clear();
    }
    if !have_uv {
        texcoords.clear();
    }
    (vertices, normals, texcoords)
}

/// Shelf width for material atlases. Images wider than this are scaled down.
const SHELF_WIDTH: u32 = 4096;

struct Shelf {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    x: u32,
    y: u32,
    row_h: u32,
}

impl Shelf {
    fn new() -> Self {
        Self {
            width: SHELF_WIDTH,
            height: 0,
            rgba: Vec::new(),
            x: 0,
            y: 0,
            row_h: 0,
        }
    }

    fn from_atlas(atlas: &MaterialAtlas) -> Self {
        if atlas.width == 0 || atlas.height == 0 {
            return Self::new();
        }
        Self {
            width: atlas.width.max(SHELF_WIDTH),
            height: atlas.height,
            rgba: atlas.rgba.clone(),
            x: 0,
            y: atlas.height,
            row_h: 0,
        }
    }

    fn place(&mut self, w: u32, h: u32) -> (u32, u32) {
        if self.x + w > self.width {
            self.y += self.row_h;
            self.x = 0;
            self.row_h = 0;
        }
        let at = (self.x, self.y);
        self.x += w;
        self.row_h = self.row_h.max(h);
        let need = self.y + self.row_h;
        if need > self.height {
            self.rgba.resize((self.width * need) as usize * 4, 0);
            self.height = need;
        }
        at
    }

    fn blit(&mut self, x: u32, y: u32, w: u32, h: u32, src: &[u8]) {
        for row in 0..h {
            let dst = ((y + row) * self.width + x) as usize * 4;
            let start = (row * w) as usize * 4;
            let n = w as usize * 4;
            self.rgba[dst..dst + n].copy_from_slice(&src[start..start + n]);
        }
    }

    fn finish(self) -> MaterialAtlas {
        if self.height == 0 {
            MaterialAtlas::empty()
        } else {
            MaterialAtlas::from_rgba(self.width, self.height, self.rgba)
        }
    }
}

fn build_surface(
    m: &PbrMaterial,
    images: &[Image],
    color: &mut Shelf,
    normal: &mut Shelf,
) -> Surface {
    let color_rect = place_image(color, bake_diffuse(m, images));
    let normal_rect = place_image(normal, copy_normal(m, images));
    Surface {
        color: color_rect,
        normal: normal_rect,
        cutoff: alpha_cutoff(m.alpha_mode),
        two_sided: m.double_sided,
    }
}

fn alpha_cutoff(mode: AlphaMode) -> f32 {
    match mode {
        AlphaMode::Opaque => 0.0,
        AlphaMode::Mask(cutoff) => cutoff,
        AlphaMode::Blend => 0.5,
    }
}

fn place_image(shelf: &mut Shelf, image: Option<(u32, u32, Vec<u8>)>) -> [u32; 4] {
    let Some((w, h, rgba)) = image else {
        return [0; 4];
    };
    let (w, h, rgba) = fit_shelf(w, h, rgba);
    let (x, y) = shelf.place(w, h);
    shelf.blit(x, y, w, h, &rgba);
    [x, y, w, h]
}

fn fit_shelf(w: u32, h: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
    if w <= SHELF_WIDTH && h <= SHELF_WIDTH && w > 0 && h > 0 {
        return (w, h, rgba);
    }
    let scale = (SHELF_WIDTH as f32 / w.max(h).max(1) as f32).min(1.0);
    let nw = ((w as f32 * scale).round() as u32).clamp(1, SHELF_WIDTH);
    let nh = ((h as f32 * scale).round() as u32).clamp(1, SHELF_WIDTH);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let sx = (x * w / nw).min(w.saturating_sub(1));
            let sy = (y * h / nh).min(h.saturating_sub(1));
            let s = ((sy * w + sx) * 4) as usize;
            let d = ((y * nw + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    (nw, nh, out)
}

/// sRGB bytes of linear(base) × (1 − metallic). Alpha stays the base texture's alpha.
/// The base-colour factor is the instance colour, not baked here. The product is
/// [`genos_load::shade_uv`] with that factor held at 1, so the shader's multiply
/// is the shade's diffuse albedo.
fn bake_diffuse(m: &PbrMaterial, images: &[Image]) -> Option<(u32, u32, Vec<u8>)> {
    let slot = m.base_color_texture?;
    let img = images.get(slot.image)?;
    let pixels = (img.width as usize).saturating_mul(img.height as usize).saturating_mul(4);
    if img.width == 0 || img.height == 0 || img.pixels.len() < pixels {
        return None;
    }
    let mut unfactored = m.clone();
    unfactored.base_color = [1.0, 1.0, 1.0, m.base_color[3]];
    let mut out = vec![0u8; pixels];
    for y in 0..img.height {
        for x in 0..img.width {
            let u = (x as f32 + 0.5) / img.width as f32;
            let v = (y as f32 + 0.5) / img.height as f32;
            let i = ((y * img.width + x) * 4) as usize;
            let sample = shade_uv(&unfactored, images, [u, v]);
            let albedo = sample.diffuse_albedo();
            for c in 0..3 {
                out[i + c] = linear_to_srgb_u8(albedo[c]);
            }
            out[i + 3] = img.pixels[i + 3];
        }
    }
    Some((img.width, img.height, out))
}

fn copy_normal(m: &PbrMaterial, images: &[Image]) -> Option<(u32, u32, Vec<u8>)> {
    let slot = m.normal_texture?;
    let img = images.get(slot.image)?;
    let pixels = (img.width as usize).saturating_mul(img.height as usize).saturating_mul(4);
    if img.width == 0 || img.height == 0 || img.pixels.len() < pixels {
        return None;
    }
    let mut out = img.pixels[..pixels].to_vec();
    for px in out.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Some((img.width, img.height, out))
}

fn linear_to_srgb_u8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5).clamp(0.0, 255.0) as u8
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

/// Mean diffuse albedo and emitted light of one primitive (linear).
///
/// The mean is [`genos_load::shade_uv`]: base colour times (1 - metallic), and
/// emission with strength. A metal's Lambert picture is darker than a specular
/// reference until that lobe exists. Painting the metal with its base colour
/// fails `metal_diffuse`.
pub fn surface_mean(m: &PbrMaterial, images: &[Image], prim: &Primitive) -> ([f32; 3], [f32; 3]) {
    let textured = m.base_color_texture.is_some()
        || m.metallic_roughness_texture.is_some()
        || m.emissive_texture.is_some();
    if !textured || prim.texcoords.len() < prim.positions.len() {
        return (diffuse(m, images), emission(m, images));
    }
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
            let (d, e) = texel_light(m, images, uv[0], uv[1]);
            for c in 0..3 {
                dsum[c] += w * d[c] as f64;
                esum[c] += w * e[c] as f64;
            }
        }
    }
    (dsum.map(|v| v as f32), esum.map(|v| v as f32))
}

/// Diffuse albedo and emitted light of a material at one texture coordinate.
fn texel_light(m: &PbrMaterial, images: &[Image], u: f32, v: f32) -> ([f32; 3], [f32; 3]) {
    let sample = shade_uv(m, images, [u, v]);
    (sample.diffuse_albedo(), sample.emissive)
}

/// Mean diffuse albedo of a material over its whole texture space (see
/// [`surface_mean`], used when a primitive has no texture coordinates).
pub fn diffuse(m: &PbrMaterial, images: &[Image]) -> [f32; 3] {
    // A grid over texture space, fine enough for a mean and cheap on 4k textures.
    const GRID: u32 = 256;
    let mut sum = [0.0f64; 3];
    for gy in 0..GRID {
        for gx in 0..GRID {
            let u = (gx as f32 + 0.5) / GRID as f32;
            let v = (gy as f32 + 0.5) / GRID as f32;
            let (d, _) = texel_light(m, images, u, v);
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
            lighting: genos_scene::LightingSettings::default(),
            debug: genos_scene::DebugSettings::default(),
            fps_counter: genos_scene::FpsCounterSettings::default(),
            event_loop: genos_scene::EventLoopSettings::default(),
            menu: genos_scene::MenuSettings::default(),
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
        // base 0.8, 0.1, 0.1 times (1 - metallic 0.25). The shade darkens the metal.
        assert!((color[0] - 0.6).abs() < 1e-5 && (color[1] - 0.075).abs() < 1e-5);
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

    fn empty_world() -> World {
        World::from_scene(genos_scene::Scene {
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
            lighting: genos_scene::LightingSettings::default(),
            debug: genos_scene::DebugSettings::default(),
            fps_counter: genos_scene::FpsCounterSettings::default(),
            event_loop: genos_scene::EventLoopSettings::default(),
            menu: genos_scene::MenuSettings::default(),
        })
    }

    fn textured_quad(metallic: f32, alpha_mode: genos_load::AlphaMode) -> genos_load::GltfScene {
        use genos_load::{GltfMesh, MeshInstance, Sampler, TextureSlot};
        let mut scene = genos_load::GltfScene::default();
        scene.images.push(Image {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 128],
        });
        scene.materials.push(genos_load::PbrMaterial {
            base_color: [0.5, 0.5, 0.5, 1.0],
            base_color_texture: Some(TextureSlot {
                image: 0,
                texcoord: 0,
                sampler: Sampler::default(),
                scale: 1.0,
            }),
            metallic,
            alpha_mode,
            double_sided: true,
            ..genos_load::PbrMaterial::default()
        });
        scene.meshes.push(GltfMesh {
            name: Some("quad".into()),
            primitives: vec![genos_load::Primitive {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                texcoords: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                indices: vec![0, 1, 2],
                material: Some(0),
                ..genos_load::Primitive::default()
            }],
        });
        scene.instances.push(MeshInstance {
            mesh: 0,
            node_name: None,
            transform: identity_pose(),
        });
        scene
    }

    #[test]
    fn a_textured_mesh_keeps_texcoords_and_bakes_metal_into_the_atlas() {
        let scene = textured_quad(0.0, genos_load::AlphaMode::Mask(0.5));
        let mut world = empty_world();
        let added = world.add_gltf(&scene, &identity_pose(), false);
        let DrawKind::Mesh {
            texcoords,
            normals,
            color,
            surface,
            ..
        } = &world.objects[added[0]].kind
        else {
            panic!("not a mesh");
        };
        assert_eq!(texcoords.len(), 3);
        assert_eq!(normals.len(), 3);
        assert!((color[0] - 0.5).abs() < 1e-6, "instance colour is the factor");
        let surface = world.surfaces[surface.unwrap()];
        assert!(surface.two_sided);
        assert!((surface.cutoff - 0.5).abs() < 1e-6);
        assert_eq!(surface.color, [0, 0, 2, 1]);
        let px = |x: u32| {
            let i = (x * 4) as usize;
            [
                world.color_atlas.rgba[i],
                world.color_atlas.rgba[i + 1],
                world.color_atlas.rgba[i + 2],
                world.color_atlas.rgba[i + 3],
            ]
        };
        assert_eq!(px(0), [255, 0, 0, 255]);
        assert_eq!(px(1), [0, 0, 255, 128]);
        assert_ne!(world.color_atlas.key, 0);
    }

    #[test]
    fn metal_is_baked_into_the_diffuse_atlas_and_not_into_an_untextured_mean() {
        let scene = textured_quad(1.0, genos_load::AlphaMode::Opaque);
        let mut world = empty_world();
        world.add_gltf(&scene, &identity_pose(), false);
        assert_eq!(world.color_atlas.rgba[0], 0);
        assert_eq!(world.color_atlas.rgba[1], 0);
        assert_eq!(world.color_atlas.rgba[2], 0);
    }

    #[test]
    fn a_second_gltf_appends_a_shelf_under_the_first() {
        let scene = textured_quad(0.0, genos_load::AlphaMode::Opaque);
        let mut world = empty_world();
        world.add_gltf(&scene, &identity_pose(), false);
        let first = world.color_atlas.height;
        world.add_gltf(&scene, &identity_pose(), false);
        let DrawKind::Mesh { surface, .. } = &world.objects.last().unwrap().kind else {
            panic!("not a mesh");
        };
        assert!(world.surfaces[surface.unwrap()].color[1] >= first);
    }

    #[test]
    #[should_panic(expected = "80 m")]
    fn a_centimetre_scale_mesh_is_refused() {
        use genos_load::{GltfMesh, MeshInstance};
        let mut scene = genos_load::GltfScene::default();
        scene.meshes.push(GltfMesh {
            name: None,
            primitives: vec![genos_load::Primitive {
                positions: vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                indices: vec![0, 1, 2],
                ..genos_load::Primitive::default()
            }],
        });
        scene.instances.push(MeshInstance {
            mesh: 0,
            node_name: None,
            transform: identity_pose(),
        });
        empty_world().add_gltf(&scene, &identity_pose(), false);
    }
}
