//! Opt-in load of a `genos-cook` directory (ADR 0035).
//!
//! `--cooked DIR` / [`RenderSettings::cooked_dir`] selects it. Unset, the glTF
//! path is unchanged. A bad hash logs `assets fallback <asset> <reason>` and
//! that asset loads from source.

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
    Surface {
        color: color_rect,
        normal: normal_rect,
        cutoff: match m.alpha_mode {
            genos_load::AlphaMode::Opaque => 0.0,
            genos_load::AlphaMode::Mask(c) => c,
            genos_load::AlphaMode::Blend => 0.5,
        },
        two_sided: m.double_sided,
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
