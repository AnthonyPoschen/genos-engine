//! Opt-in load of a `genos-cook` directory (ADR 0035).
//!
//! `--cooked DIR` / [`RenderSettings::cooked_dir`] selects it. Unset, the glTF
//! path is unchanged. A bad `.gcook` or `.gtex` hash logs `assets fallback <asset> <reason>`
//! and that asset loads from source. A bad `.gskin` logs the same line and the
//! mesh stays the rigid `.gcook` geometry.

use std::path::Path;
use std::time::Instant;

use genos_load::{
    decode_geometry, decode_images, decode_skin, image_spaces, sha256, skin_path, CookedImage,
    DecodedSkin, GltfScene, MipSpace, Package,
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
    /// `bc` is [`crate::Renderer::bc_textures`]. Without it, geometry still comes
    /// from `.gcook` and textures stay on the source path. A hash or decode
    /// failure loads the whole asset from glTF.
    pub fn add_cooked(
        &mut self,
        dir: &Path,
        source: &Path,
        place: &[f32; 16],
        affects_light: bool,
        bc: bool,
    ) -> CookedReport {
        let start = Instant::now();
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
            self.add_gltf_cooked(&scene.scene, &scene.cooked, place, affects_light)
        };
        let texture_bytes = block_bytes(&self.color_atlas) + block_bytes(&self.normal_atlas);
        CookedReport {
            objects,
            fallback,
            load_ms: elapsed_ms(start),
            texture_bytes,
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
    if let Err(err) = package.verify_geometry(dir) {
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
    scene.skins.clear();
    scene.clips.clear();
    scene.skin_notes.clear();
    if let Some(expected) = line
        .split(' ')
        .find_map(|token| token.strip_prefix("skin="))
    {
        match read_skin(dir, &id, expected) {
            Ok(record) => {
                if let Err(reason) = attach_skin(&mut scene, record) {
                    fallback.push(format!("assets fallback {} {reason}", source.display()));
                }
            }
            Err(reason) => {
                fallback.push(format!("assets fallback {} {reason}", source.display()));
            }
        }
    }
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

fn read_skin(dir: &Path, id: &[u8; 32], expected: &str) -> Result<DecodedSkin, String> {
    let bytes = std::fs::read(skin_path(dir, id)).map_err(|_| "gskin missing".to_string())?;
    if genos_load::hex(&sha256(&bytes)) != expected {
        return Err("gskin hash".into());
    }
    decode_skin(&bytes).map_err(|err| err.to_string())
}

fn attach_skin(scene: &mut GltfScene, skin: DecodedSkin) -> Result<(), &'static str> {
    let count: usize = scene.meshes.iter().map(|mesh| mesh.primitives.len()).sum();
    if count != skin.primitives.len() || scene.instances.len() != skin.instance_skins.len() {
        return Err("gskin shape");
    }
    let mut offset = 0;
    for mesh in &scene.meshes {
        for primitive in &mesh.primitives {
            let cooked = skin.primitives.get(offset).ok_or("gskin shape")?;
            offset += 1;
            if cooked.joints.len() != cooked.weights.len()
                || (!cooked.joints.is_empty() && cooked.joints.len() != primitive.positions.len())
            {
                return Err("gskin influences");
            }
        }
    }
    let mut cooked = skin.primitives.into_iter();
    for mesh in &mut scene.meshes {
        for primitive in &mut mesh.primitives {
            let next = cooked.next().ok_or("gskin shape")?;
            primitive.joints = next.joints;
            primitive.weights = next.weights;
        }
    }
    for (instance, index) in scene.instances.iter_mut().zip(skin.instance_skins) {
        instance.skin = usize::try_from(index).ok();
    }
    scene.skins = skin.skins;
    scene.clips = skin.clips;
    scene.skin_notes = skin.notes;
    Ok(())
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
#[expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]
mod skin_fallback {
    use std::path::PathBuf;

    use genos_bake::{BakeMode, BakeSettings};
    use genos_cook::{cook_asset, load_cooked, package_files, PackageFile, Threads};
    use genos_load::sha256;

    use super::load_scene;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("genos-skin-fallback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn align(bin: &mut Vec<u8>) {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
    }

    fn put_f32(bin: &mut Vec<u8>, values: &[f32]) -> usize {
        align(bin);
        let offset = bin.len();
        for value in values {
            bin.extend(value.to_le_bytes());
        }
        offset
    }

    fn put_u16(bin: &mut Vec<u8>, values: &[u16]) -> usize {
        align(bin);
        let offset = bin.len();
        for value in values {
            bin.extend(value.to_le_bytes());
        }
        offset
    }

    fn glb() -> Vec<u8> {
        let mut bin = Vec::new();
        let positions = put_f32(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        let joints = put_u16(&mut bin, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let weights = put_f32(
            &mut bin,
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        );
        let indices = put_u16(&mut bin, &[0, 1, 2]);
        let byte_length = bin.len();
        let acc = |offset: usize, count: usize, ctype: &str, kind: &str| {
            format!(
                r#"{{"bufferView":0,"byteOffset":{offset},"componentType":{ctype},"count":{count},"type":"{kind}"}}"#
            )
        };
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "scene": 0,
                "scenes": [{{"nodes": [0]}}],
                "nodes": [{{"name": "hips", "mesh": 0, "skin": 0}}],
                "skins": [{{"joints": [0]}}],
                "meshes": [{{"primitives": [{{
                    "attributes": {{"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}},
                    "indices": 3
                }}]}}],
                "buffers": [{{"byteLength": {byte_length}}}],
                "bufferViews": [{{"buffer": 0, "byteLength": {byte_length}}}],
                "accessors": [{accessors}]
            }}"#,
            accessors = [
                acc(positions, 3, "5126", "VEC3"),
                acc(joints, 3, "5123", "VEC4"),
                acc(weights, 3, "5126", "VEC4"),
                acc(indices, 3, "5123", "SCALAR"),
            ]
            .join(","),
        );
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = u32::try_from(12 + 8 + json.len() + 8 + bin.len()).unwrap();
        let mut out = Vec::new();
        out.extend(0x4654_6C67u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend(total.to_le_bytes());
        out.extend(u32::try_from(json.len()).unwrap().to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(json);
        out.extend(u32::try_from(bin.len()).unwrap().to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes());
        out.extend(bin);
        out
    }

    #[test]
    fn a_bad_gskin_keeps_the_rigid_mesh() {
        let dir = temp();
        let source = dir.join("src.glb");
        std::fs::write(&source, glb()).unwrap();
        let out = dir.join("out");
        let settings = BakeSettings {
            mode: BakeMode::OnLoad,
            cache_dir: dir.join("bake"),
            ..BakeSettings::default()
        };
        let cooked = cook_asset(&source, &out, &settings, false, Threads::ONE).unwrap();
        let (_, skin_path) = cooked.skin.unwrap();
        let record = load_cooked(&cooked.record.1).unwrap();
        let bytes = std::fs::read(&skin_path).unwrap();
        let text = package_files(&[PackageFile {
            cooked: &record,
            gtex: None,
            skin: Some(sha256(&bytes)),
        }])
        .text();
        std::fs::write(out.join("package.manifest"), text).unwrap();

        let mut fallback = Vec::new();
        let loaded = load_scene(&out, &source, false, &mut fallback).unwrap();
        assert_eq!(loaded.scene.skins.len(), 1);
        assert_eq!(loaded.scene.instances[0].skin, Some(0));
        assert_eq!(loaded.scene.meshes[0].primitives[0].joints.len(), 3);
        assert_eq!(loaded.scene.meshes[0].primitives[0].positions.len(), 3);
        assert!(fallback.iter().all(|line| !line.contains("gskin")));

        let mut broken = bytes;
        let last = broken.len() - 1;
        broken[last] ^= 0xff;
        std::fs::write(&skin_path, broken).unwrap();
        fallback.clear();
        let rigid = load_scene(&out, &source, false, &mut fallback).unwrap();
        assert_eq!(rigid.scene.skins.len(), 0);
        assert_eq!(rigid.scene.clips.len(), 0);
        assert_eq!(rigid.scene.instances[0].skin, None);
        assert_eq!(rigid.scene.meshes[0].primitives[0].joints.len(), 0);
        assert_eq!(rigid.scene.meshes[0].primitives[0].positions.len(), 3);
        assert!(fallback
            .iter()
            .any(|line| { line.contains("assets fallback") && line.contains("gskin") }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
