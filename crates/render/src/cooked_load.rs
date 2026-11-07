//! Opt-in load of a `genos-cook` directory (ADR 0035).
//!
//! `--cooked DIR` / [`RenderSettings::cooked_dir`] selects it. Unset, the glTF
//! path is unchanged. A manifest line with `mat=` loads through [`genos_load::load_covered`]:
//! geometry, materials, image blocks and the stored shade, with no glTF decode.
//! [`World::add_covered`] is that load when the caller already has the asset id,
//! so the source file can be absent. A line without `mat=` keeps today's path.
//! A bad hash logs `assets fallback <asset> <reason>` and, when the source file
//! is still there, loads that asset from source.

use std::path::Path;
use std::time::Instant;

use genos_load::{
    decode_geometry, decode_images, image_spaces, sha256, CookedImage, GltfScene, MipSpace, Package,
};

use crate::material_stream::MipLevel;
use crate::world::{MaterialAtlas, Surface, World};

/// Where an opt-in cooked package lives. Unset keeps the glTF and PNG path.
#[derive(Clone, Debug, Default)]
pub struct RenderSettings {
    pub cooked_dir: Option<std::path::PathBuf>,
}

/// What [`World::add_cooked`] did.
#[derive(Clone, Debug)]
pub struct CookedReport {
    pub objects: Vec<usize>,
    /// `assets fallback …` lines.
    pub fallback: Vec<String>,
    /// Wall time to read and place the asset, milliseconds.
    pub load_ms: f64,
    /// Block bytes registered for the colour and normal atlases. 0 on the RGBA path.
    pub texture_bytes: u64,
}

const BC7_SRGB: u32 = 146;
const BC5: u32 = 141;

impl World {
    /// Place `source` from a cook directory when the package checks.
    ///
    /// `bc` is [`crate::Renderer::bc_textures`]. Without it, block textures are
    /// not uploaded. A line with `mat=` does not decode the glTF. A line
    /// without it still does. A hash or decode failure loads the whole asset
    /// from glTF when `source` can be read.
    pub fn add_cooked(
        &mut self,
        dir: &Path,
        source: &Path,
        place: &[f32; 16],
        affects_light: bool,
        bc: bool,
    ) -> CookedReport {
        let start = Instant::now();
        if let Some(id) = covered_id(dir, source) {
            return self.place_covered(dir, &id, Some(source), place, affects_light, bc);
        }
        let mut fallback = Vec::new();
        let scene = match load_scene(dir, source, bc, &mut fallback) {
            Ok(scene) => scene,
            Err(reason) => {
                fallback.push(format!("assets fallback {} {reason}", source.display()));
                let Ok(scene) = genos_load::load_gltf_file(source) else {
                    return CookedReport {
                        objects: Vec::new(),
                        fallback,
                        load_ms: elapsed_ms(start),
                        texture_bytes: 0,
                    };
                };
                let objects = self.add_gltf(&scene, place, affects_light);
                return CookedReport {
                    objects,
                    fallback,
                    load_ms: elapsed_ms(start),
                    texture_bytes: 0,
                };
            }
        };
        let objects = if scene.cooked.is_empty() || !bc {
            self.add_gltf(&scene.scene, place, affects_light)
        } else {
            self.add_gltf_cooked(&scene.scene, &scene.cooked, &[], place, affects_light)
        };
        let texture_bytes = block_bytes(&self.color_atlas) + block_bytes(&self.normal_atlas);
        CookedReport {
            objects,
            fallback,
            load_ms: elapsed_ms(start),
            texture_bytes,
        }
    }

    /// Place one cooked asset by id. Does not read the source file and does
    /// not enter the glTF decoder. A missing or bad record logs
    /// `assets fallback <asset-id> <reason>` and adds no objects.
    pub fn add_covered(
        &mut self,
        dir: &Path,
        asset_id: &[u8; 32],
        place: &[f32; 16],
        affects_light: bool,
        bc: bool,
    ) -> CookedReport {
        self.place_covered(dir, asset_id, None, place, affects_light, bc)
    }
}

fn covered_id(dir: &Path, source: &Path) -> Option<[u8; 32]> {
    let bytes = std::fs::read(source).ok()?;
    let id = sha256(&bytes);
    let package = Package::open(dir).ok()?;
    let line = package.line_for(&id)?;
    line.split(' ')
        .any(|part| part.starts_with("mat="))
        .then_some(id)
}

fn cooked_reason(err: &genos_load::CookedError) -> String {
    match err {
        genos_load::CookedError::Io(io) => io.to_string(),
        genos_load::CookedError::Corrupt(why) => (*why).to_string(),
    }
}

fn scene_from_covered(asset: &genos_load::CoveredAsset) -> GltfScene {
    GltfScene {
        meshes: asset.meshes.clone(),
        materials: asset.materials.clone(),
        images: asset
            .image_size
            .iter()
            .map(|[width, height]| genos_load::Image {
                width: *width,
                height: *height,
                pixels: Vec::new(),
            })
            .collect(),
        instances: asset.instances.clone(),
        ..GltfScene::default()
    }
}

impl World {
    fn place_covered(
        &mut self,
        dir: &Path,
        asset_id: &[u8; 32],
        source: Option<&Path>,
        place: &[f32; 16],
        affects_light: bool,
        bc: bool,
    ) -> CookedReport {
        let start = Instant::now();
        let name = source.map_or_else(
            || genos_load::hex(asset_id),
            |path| path.display().to_string(),
        );
        let asset = match genos_load::load_covered(dir, asset_id) {
            Ok(asset) => asset,
            Err(err) => {
                return self.fallback_gltf(
                    source,
                    place,
                    affects_light,
                    start,
                    vec![format!("assets fallback {name} {}", cooked_reason(&err))],
                );
            }
        };
        let prims: usize = asset.meshes.iter().map(|mesh| mesh.primitives.len()).sum();
        if asset.primitive_shade.len() != prims {
            return self.fallback_gltf(
                source,
                place,
                affects_light,
                start,
                vec![format!("assets fallback {name} no stored shade")],
            );
        }
        let mut fallback = Vec::new();
        let has_gtex = Package::open(dir)
            .ok()
            .and_then(|package| package.line_for(asset_id).map(str::to_string))
            .is_some_and(|line| line.split(' ').any(|part| part.starts_with("gtex=")));
        if !has_gtex {
            fallback.push(format!("assets fallback {} no gtex", hex_short(asset_id)));
        }
        if !bc && has_gtex {
            fallback.push("assets fallback textures no-bc".to_string());
        }
        let scene = scene_from_covered(&asset);
        let use_blocks = bc && !asset.cooked.is_empty();
        let objects = if use_blocks {
            self.add_gltf_cooked(
                &scene,
                &asset.cooked,
                &asset.primitive_shade,
                place,
                affects_light,
            )
        } else {
            self.add_gltf_shaded(&scene, &asset.primitive_shade, place, affects_light)
        };
        let texture_bytes = block_bytes(&self.color_atlas) + block_bytes(&self.normal_atlas);
        CookedReport {
            objects,
            fallback,
            load_ms: elapsed_ms(start),
            texture_bytes,
        }
    }

    fn fallback_gltf(
        &mut self,
        source: Option<&Path>,
        place: &[f32; 16],
        affects_light: bool,
        start: Instant,
        fallback: Vec<String>,
    ) -> CookedReport {
        let Some(source) = source else {
            return CookedReport {
                objects: Vec::new(),
                fallback,
                load_ms: elapsed_ms(start),
                texture_bytes: 0,
            };
        };
        let Ok(scene) = genos_load::load_gltf_file(source) else {
            return CookedReport {
                objects: Vec::new(),
                fallback,
                load_ms: elapsed_ms(start),
                texture_bytes: 0,
            };
        };
        let objects = self.add_gltf(&scene, place, affects_light);
        CookedReport {
            objects,
            fallback,
            load_ms: elapsed_ms(start),
            texture_bytes: 0,
        }
    }
}

struct Loaded {
    scene: GltfScene,
    cooked: Vec<CookedImage>,
}

fn load_scene(
    dir: &Path,
    source: &Path,
    bc: bool,
    fallback: &mut Vec<String>,
) -> Result<Loaded, String> {
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    let id = sha256(&bytes);
    let package = Package::open(dir).map_err(|e| e.to_string())?;
    let line = package
        .line_for(&id)
        .ok_or_else(|| "asset is not in the package".to_string())?;
    if let Err(err) = package.verify(dir) {
        return Err(err.to_string());
    }
    let geom = decode_geometry(
        &std::fs::read(genos_load::cooked_path(dir, &id)).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if !line.contains(" gtex=") {
        fallback.push(format!("assets fallback {} no gtex", hex_short(&id)));
    }
    let mut scene = genos_load::load_gltf_file(source).map_err(|e| e.to_string())?;
    scene.meshes = geom.meshes;
    scene.instances = geom.instances;
    if !bc {
        fallback.push("assets fallback textures no-bc".to_string());
        return Ok(Loaded {
            scene,
            cooked: Vec::new(),
        });
    }
    if !line.contains(" gtex=") {
        return Ok(Loaded {
            scene,
            cooked: Vec::new(),
        });
    }
    let images = decode_images(
        &std::fs::read(genos_load::textures_path(dir, &id)).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(Loaded {
        scene,
        cooked: images.images,
    })
}

fn hex_short(id: &[u8; 32]) -> String {
    genos_load::hex(id)[..8].to_string()
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

fn block_bytes(atlas: &MaterialAtlas) -> u64 {
    atlas
        .block_mips
        .iter()
        .map(|m| u64::try_from(m.rgba.len()).unwrap_or(0))
        .sum()
}

/// Shelf of 4×4 blocks. Rects stay on a 4-texel grid so every mip is whole blocks
/// until a rect would drop below one block.
pub(crate) struct BlockShelf {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    row_h: u32,
    placed: Vec<([u32; 4], Vec<Vec<u8>>)>,
    format: u32,
}

impl BlockShelf {
    fn new(format: u32) -> Self {
        Self {
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            row_h: 0,
            placed: Vec::new(),
            format,
        }
    }

    fn place(&mut self, image: &CookedImage) -> [u32; 4] {
        let (w, h) = (image.width.max(4), image.height.max(4));
        let (w, h) = (w.div_ceil(4) * 4, h.div_ceil(4) * 4);
        if self.x + w > 4096 {
            self.y += self.row_h;
            self.x = 0;
            self.row_h = 0;
        }
        let at = [self.x, self.y, w, h];
        self.x += w;
        self.row_h = self.row_h.max(h);
        self.width = self.width.max(self.x);
        self.height = self.height.max(self.y + self.row_h);
        self.placed.push((at, image.levels.clone()));
        at
    }

    pub(crate) fn finish(self) -> MaterialAtlas {
        if self.placed.is_empty() || self.width == 0 || self.height == 0 {
            return MaterialAtlas::empty();
        }
        let mut levels = 1u32;
        while self.width >> levels >= 4 && self.height >> levels >= 4 {
            let ok = self.placed.iter().all(|([x, y, w, h], src)| {
                let (lw, lh) = (w >> levels, h >> levels);
                lw >= 4
                    && lh >= 4
                    && (x >> levels).is_multiple_of(4)
                    && (y >> levels).is_multiple_of(4)
                    && src.len() > usize::try_from(levels).unwrap_or(0)
            });
            if !ok {
                break;
            }
            levels += 1;
            if levels > 16 {
                break;
            }
        }
        let mut mips = Vec::new();
        for level in 0..levels {
            let (aw, ah) = (self.width >> level, self.height >> level);
            let blocks_x = usize::try_from(aw / 4).unwrap_or(0);
            let blocks_y = usize::try_from(ah / 4).unwrap_or(0);
            let mut bytes = vec![0u8; blocks_x * blocks_y * 16];
            for ([x, y, w, h], src) in &self.placed {
                let Some(level_bytes) = src.get(usize::try_from(level).unwrap_or(0)) else {
                    continue;
                };
                blit_blocks(
                    &mut bytes,
                    blocks_x,
                    x >> level,
                    y >> level,
                    w >> level,
                    h >> level,
                    level_bytes,
                );
            }
            mips.push(MipLevel {
                width: aw,
                height: ah,
                rgba: bytes,
            });
        }
        let mut atlas = MaterialAtlas {
            width: self.width,
            height: self.height,
            rgba: Vec::new(),
            key: 0,
            block_format: self.format,
            block_mips: mips,
        };
        atlas.rehash();
        atlas
    }
}

fn blit_blocks(
    dst: &mut [u8],
    dst_blocks_x: usize,
    origin_x: u32,
    origin_y: u32,
    texels_w: u32,
    texels_h: u32,
    src: &[u8],
) {
    let blocks_w = usize::try_from(texels_w / 4).unwrap_or(0);
    let blocks_h = usize::try_from(texels_h / 4).unwrap_or(0);
    let bx = usize::try_from(origin_x / 4).unwrap_or(0);
    let by = usize::try_from(origin_y / 4).unwrap_or(0);
    for row in 0..blocks_h {
        let n = blocks_w * 16;
        let from = row * n;
        let to = ((by + row) * dst_blocks_x + bx) * 16;
        let Some(src_row) = src.get(from..from + n) else {
            return;
        };
        let Some(dst_row) = dst.get_mut(to..to + n) else {
            return;
        };
        dst_row.copy_from_slice(src_row);
    }
}

pub(crate) fn block_surface(
    scene: &GltfScene,
    images: &[CookedImage],
    material: usize,
    color: &mut Option<BlockShelf>,
    normal: &mut Option<BlockShelf>,
) -> Surface {
    let spaces = image_spaces(scene.images.len(), &scene.materials);
    let m = &scene.materials[material];
    let color_rect = m
        .base_color_texture
        .and_then(|slot| place_kind(images, &spaces, slot.image, MipSpace::Srgb, BC7_SRGB, color))
        .unwrap_or([0; 4]);
    let normal_rect = m
        .normal_texture
        .and_then(|slot| place_kind(images, &spaces, slot.image, MipSpace::Normal, BC5, normal))
        .unwrap_or([0; 4]);
    let mut flags = 0;
    if color_rect[2] > 0 {
        flags |= crate::world::MAT_BASE;
    }
    Surface {
        color: color_rect,
        normal: normal_rect,
        cutoff: match m.alpha_mode {
            genos_load::AlphaMode::Opaque => 0.0,
            genos_load::AlphaMode::Mask(c) => c,
            genos_load::AlphaMode::Blend => 0.5,
        },
        two_sided: m.double_sided,
        roughness: m.roughness,
        metallic: if flags & crate::world::MAT_BASE != 0 {
            m.metallic
        } else {
            0.0
        },
        material_flags: flags,
    }
}

fn place_kind(
    images: &[CookedImage],
    spaces: &[MipSpace],
    index: usize,
    space: MipSpace,
    format: u32,
    shelf: &mut Option<BlockShelf>,
) -> Option<[u32; 4]> {
    if spaces.get(index).copied() != Some(space) {
        return None;
    }
    let image = images
        .iter()
        .find(|i| i.image == u32::try_from(index).unwrap_or(u32::MAX))?;
    if image.format != format || image.levels.is_empty() {
        return None;
    }
    Some(
        shelf
            .get_or_insert_with(|| BlockShelf::new(format))
            .place(image),
    )
}

#[cfg(test)]
mod shade_record {
    #![expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]

    use std::path::Path;

    use genos_load::{decode_materials, encode_materials, load_gltf_file, CookedMaterials};

    use crate::gltf_draw::surface_mean;

    /// The shade stored for a later cook is the mean the draw uses today.
    #[test]
    fn stored_shade_is_the_renderer_mean() {
        let scene =
            load_gltf_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/fixtures/tri.gltf"))
                .unwrap();
        let mut primitive_shade = Vec::new();
        for mesh in &scene.meshes {
            for primitive in &mesh.primitives {
                let material = &scene.materials[primitive.material.unwrap()];
                let (mean, emission) = surface_mean(material, &scene.images, primitive);
                primitive_shade.push(genos_load::PrimitiveShade { mean, emission });
            }
        }
        assert_eq!(primitive_shade.len(), 1);
        let record = CookedMaterials {
            asset_id: [9; 32],
            image_size: scene
                .images
                .iter()
                .map(|image| [image.width, image.height])
                .collect(),
            materials: scene.materials.clone(),
            primitive_shade: primitive_shade.clone(),
        };
        let loaded = decode_materials(&encode_materials(&record).unwrap()).unwrap();
        assert_eq!(loaded.primitive_shade, primitive_shade);
        assert_eq!(loaded.materials, scene.materials);
    }

    /// The cook's stored shade is the mean the draw uses, including a second mesh.
    #[test]
    fn primitive_shades_match_surface_mean_across_meshes() {
        let scene =
            load_gltf_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/fixtures/tri.gltf"))
                .unwrap();
        assert_shades(&scene);
        assert_shades(&two_mesh_scene());
    }

    fn assert_shades(scene: &genos_load::GltfScene) {
        let shades = genos_load::primitive_shades(scene);
        let mut expect = Vec::new();
        for mesh in &scene.meshes {
            for primitive in &mesh.primitives {
                let (mean, emission) = primitive
                    .material
                    .and_then(|index| scene.materials.get(index))
                    .map_or(([1.0; 3], [0.0; 3]), |material| {
                        surface_mean(material, &scene.images, primitive)
                    });
                expect.push(genos_load::PrimitiveShade { mean, emission });
            }
        }
        assert_eq!(shades, expect);
    }

    fn two_mesh_scene() -> genos_load::GltfScene {
        use genos_load::{GltfMesh, GltfScene, Image, PbrMaterial, Primitive, TextureSlot};
        let image = Image {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let slot = TextureSlot {
            image: 0,
            texcoord: 0,
            sampler: genos_load::Sampler::default(),
            scale: 1.0,
        };
        let red = PbrMaterial {
            base_color: [0.2, 0.4, 0.6, 1.0],
            metallic: 0.0,
            roughness: 1.0,
            ..PbrMaterial::default()
        };
        let textured = PbrMaterial {
            base_color: [1.0, 1.0, 1.0, 1.0],
            base_color_texture: Some(slot),
            metallic: 0.25,
            emissive: [0.5, 0.0, 0.0],
            emissive_texture: Some(TextureSlot {
                texcoord: 1,
                ..slot
            }),
            emissive_strength: 2.0,
            double_sided: true,
            ..PbrMaterial::default()
        };
        let tri = |material: usize, set1: [f32; 2]| Primitive {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            texcoords: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            texcoord_sets: vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], vec![set1; 3]],
            tangents: Vec::new(),
            indices: vec![0, 1, 2],
            material: Some(material),
        };
        GltfScene {
            images: vec![image],
            materials: vec![red, textured],
            meshes: vec![
                GltfMesh {
                    name: Some("a".into()),
                    primitives: vec![tri(0, [0.25, 0.25]), tri(1, [0.75, 0.25])],
                },
                GltfMesh {
                    name: Some("b".into()),
                    primitives: vec![tri(1, [0.75, 0.75])],
                },
            ],
            ..GltfScene::default()
        }
    }
}

#[cfg(test)]
mod covered_world {
    #![expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]

    use std::path::{Path, PathBuf};

    use genos_bake::{BakeMode, BakeSettings};
    use genos_cook::{cook_asset, load_cooked, package, package_with, Threads};
    use genos_load::{
        importer_entries, load_gltf_file, reset_importer_entries, sha256, write_atomic,
    };
    use genos_scene::{Floor, Scene, Vec3};

    use crate::world::{identity_pose, DrawKind, World};

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("genos-covered-world-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bare_world() -> World {
        World::from_scene(Scene {
            floor: Floor {
                position: Vec3::ZERO,
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
            reflections: genos_scene::ReflectionSettings::default(),
            hdr: genos_scene::HdrSettings::legacy_bytes(),
            fog: genos_scene::FogSettings::default(),
            menu: genos_scene::MenuSettings::default(),
        })
    }

    type Paint = ([f32; 3], [f32; 3], Option<usize>);

    fn paint(world: &World, index: usize) -> Result<Paint, &'static str> {
        if let DrawKind::Mesh {
            color,
            emission,
            surface,
            ..
        } = &world.objects[index].kind
        {
            Ok((*color, *emission, *surface))
        } else {
            Err("not a mesh")
        }
    }

    /// Two meshes. Mesh 0 is instanced twice and is an untextured red.
    /// Mesh 1 is instanced twice, samples `quad.png` as emission, and is double sided.
    fn write_two_mesh(dir: &Path) -> PathBuf {
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/fixtures/quad.png"),
            src.join("quad.png"),
        )
        .unwrap();
        let mut bin = Vec::new();
        for p in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for v in p {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        let idx0 = bin.len();
        for i in [0_u16, 1, 2] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let pos1 = bin.len();
        for p in [[2.0_f32, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]] {
            for v in p {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        let uv = bin.len();
        for p in [[0.0_f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            for v in p {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        let idx1 = bin.len();
        for i in [0_u16, 1, 2] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        std::fs::write(src.join("mesh.bin"), &bin).unwrap();
        let gltf = format!(
            r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0, 1, 2, 3]}}],
  "nodes": [{{"mesh": 0}}, {{"mesh": 0}}, {{"mesh": 1}}, {{"mesh": 1}}],
  "meshes": [
    {{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1, "material": 0}}]}},
    {{"primitives": [{{"attributes": {{"POSITION": 2, "TEXCOORD_0": 3}}, "indices": 4, "material": 1}}]}}
  ],
  "materials": [
    {{"pbrMetallicRoughness": {{"baseColorFactor": [1, 0, 0, 1], "metallicFactor": 0, "roughnessFactor": 1}}, "emissiveFactor": [0.2, 0, 0]}},
    {{"doubleSided": true, "pbrMetallicRoughness": {{"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0.3, "baseColorTexture": {{"index": 0}}}}, "emissiveFactor": [1, 0, 0], "emissiveTexture": {{"index": 0}}}}
  ],
  "textures": [{{"source": 0}}],
  "images": [{{"uri": "quad.png"}}],
  "buffers": [{{"uri": "mesh.bin", "byteLength": {len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": {idx0}, "byteLength": 6}},
    {{"buffer": 0, "byteOffset": {pos1}, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": {uv}, "byteLength": 24}},
    {{"buffer": 0, "byteOffset": {idx1}, "byteLength": 6}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}},
    {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2"}},
    {{"bufferView": 4, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ]
}}"#,
            len = bin.len(),
        );
        let path = src.join("meshes.gltf");
        std::fs::write(&path, gltf).unwrap();
        path
    }

    fn cooks(source: &Path, out: &Path) -> [u8; 32] {
        let settings = BakeSettings {
            mode: BakeMode::OnLoad,
            cache_dir: out.join("bake"),
            ..BakeSettings::default()
        };
        let asset = cook_asset(source, out, &settings, true, Threads::ONE).unwrap();
        let record = load_cooked(&asset.record.1).unwrap();
        let gtex = sha256(&std::fs::read(&asset.textures.unwrap().1).unwrap());
        let mat = sha256(&std::fs::read(&asset.materials.1).unwrap());
        let text = package_with(&[(&record, Some(gtex), Some(mat))]).text();
        write_atomic(&out.join("package.manifest"), text.as_bytes()).unwrap();
        record.asset_id
    }

    fn source_paint(scene: &genos_load::GltfScene) -> Vec<Paint> {
        let mut oracle = bare_world();
        let ids = oracle.add_gltf(scene, &identity_pose(), false);
        assert_eq!(ids.len(), 4);
        let paint: Vec<_> = ids.iter().map(|id| paint(&oracle, *id).unwrap()).collect();
        assert_eq!(paint[0].0, paint[1].0);
        assert_eq!(paint[0].1, paint[1].1);
        assert_eq!(paint[2].1, paint[3].1);
        assert_ne!(paint[0].1, paint[2].1);
        assert!(
            paint[2].1[0] > 0.0,
            "emissive texture mean {:?}",
            paint[2].1
        );
        paint
    }

    fn drop_mat_token(manifest: &str) -> String {
        let token = manifest
            .lines()
            .nth(1)
            .and_then(|line| line.split(' ').find_map(|part| part.strip_prefix("mat=")))
            .unwrap();
        manifest.replace(&format!(" mat={token}"), "")
    }

    fn remove_source(source: &Path, out: &Path) {
        for name in ["meshes.gltf", "mesh.bin", "quad.png"] {
            std::fs::remove_file(source.parent().unwrap().join(name)).unwrap();
        }
        let objects = out.join("objects");
        if objects.is_dir() {
            for entry in std::fs::read_dir(&objects).unwrap() {
                std::fs::write(entry.unwrap().path(), b"not-a-gltf").unwrap();
            }
        }
    }

    #[test]
    fn covered_world_loads_two_meshes_after_the_source_is_gone() {
        let dir = temp("world");
        let source = write_two_mesh(&dir);
        let out = dir.join("cook");
        let oracle_paint = source_paint(&load_gltf_file(&source).unwrap());
        let id = cooks(&source, &out);

        let manifest = std::fs::read_to_string(out.join("package.manifest")).unwrap();
        write_atomic(
            &out.join("package.manifest"),
            drop_mat_token(&manifest).as_bytes(),
        )
        .unwrap();
        reset_importer_entries();
        let mut legacy = bare_world();
        let legacy_report = legacy.add_cooked(&out, &source, &identity_pose(), false, true);
        assert!(
            legacy_report.fallback.is_empty(),
            "{:?}",
            legacy_report.fallback
        );
        assert!(importer_entries() >= 1, "a line without mat= still imports");
        write_atomic(&out.join("package.manifest"), manifest.as_bytes()).unwrap();

        reset_importer_entries();
        let mut present = bare_world();
        let present_report = present.add_cooked(&out, &source, &identity_pose(), false, true);
        assert!(
            present_report.fallback.is_empty(),
            "{:?}",
            present_report.fallback
        );
        assert_eq!(
            importer_entries(),
            0,
            "mat= does not enter the glTF decoder"
        );

        remove_source(&source, &out);
        reset_importer_entries();
        let mut covered = bare_world();
        let report = covered.add_covered(&out, &id, &identity_pose(), false, true);
        assert!(report.fallback.is_empty(), "{:?}", report.fallback);
        assert_eq!(importer_entries(), 0);
        assert_eq!(report.objects.len(), 4);
        assert!(report.texture_bytes > 0);
        for (index, object) in report.objects.iter().enumerate() {
            assert_eq!(
                paint(&covered, *object).unwrap(),
                oracle_paint[index],
                "object {index}"
            );
        }
        let shared = oracle_paint[2].2.unwrap();
        assert_eq!(
            paint(&covered, report.objects[2]).unwrap().2,
            paint(&covered, report.objects[3]).unwrap().2
        );
        assert!(covered.surfaces[shared].two_sided);
        assert!((covered.surfaces[shared].roughness - 0.3).abs() < 1.0e-5);

        let missing = covered.add_cooked(&out, &source, &identity_pose(), false, true);
        assert_eq!(missing.objects.len(), 0);
        assert_ne!(missing.fallback.len(), 0);
        flip_material(&out, &id);
        reset_importer_entries();
        let mut broken = bare_world();
        let bad = broken.add_covered(&out, &id, &identity_pose(), false, true);
        assert_eq!(bad.objects.len(), 0);
        assert!(
            bad.fallback
                .iter()
                .any(|line| line.contains("material hash differs from the manifest")),
            "{:?}",
            bad.fallback
        );
        assert_eq!(importer_entries(), 0, "no source remains to import");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_material_record_adds_nothing_and_does_not_import() {
        let dir = temp("missing-mat");
        let source = write_two_mesh(&dir);
        let out = dir.join("cook");
        let id = cooks(&source, &out);
        std::fs::remove_file(genos_load::materials_path(&out, &id)).unwrap();
        remove_source(&source, &out);
        reset_importer_entries();
        let mut world = bare_world();
        let report = world.add_covered(&out, &id, &identity_pose(), false, true);
        assert_eq!(report.objects.len(), 0);
        assert_ne!(report.fallback.len(), 0);
        assert_eq!(importer_entries(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn flip_material(out: &Path, id: &[u8; 32]) {
        let mat_path = genos_load::materials_path(out, id);
        let mut damaged = std::fs::read(&mat_path).unwrap();
        let at = damaged.len() / 2;
        damaged[at] ^= 0x01;
        std::fs::write(&mat_path, damaged).unwrap();
    }

    #[test]
    fn a_line_without_mat_is_unchanged_for_package_verify() {
        let dir = temp("legacy-verify");
        let source = write_two_mesh(&dir);
        let out = dir.join("cook");
        let settings = BakeSettings {
            mode: BakeMode::OnLoad,
            cache_dir: out.join("bake"),
            ..BakeSettings::default()
        };
        let asset = cook_asset(&source, &out, &settings, false, Threads::ONE).unwrap();
        let record = load_cooked(&asset.record.1).unwrap();
        let text = package(&[(&record, None)]).text();
        assert!(!text.contains("mat="));
        write_atomic(&out.join("package.manifest"), text.as_bytes()).unwrap();
        assert_eq!(
            genos_load::Package::open(&out)
                .unwrap()
                .verify(&out)
                .unwrap(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
