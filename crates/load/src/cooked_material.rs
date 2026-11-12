//! Proposed `.gmat` record (ADR 0035 amendment A).
//!
//! `GMT1`, version, asset id, image widths and heights (no pixels), the
//! `PbrMaterial` list, and one mean and emission per primitive. A SHA-256
//! trailer covers every byte before it. [`load_covered`] reads that record
//! with `.gcook` and `.gtex` and does not enter the glTF decoder.
//!
//! `genos-cook` does not write this file. The renderer does not read it.
//! The draw path stays the accepted one until the amendment is accepted.

use std::path::{Path, PathBuf};

use crate::{
    hex, sha256, AlphaMode, CookedError, CookedImage, GltfMesh, GltfScene, MeshInstance,
    PbrMaterial, Sampler, TextureSlot,
};

/// `GMT1`.
pub const MATERIAL_MAGIC: &[u8; 4] = b"GMT1";
/// Record version. A different version is corrupt.
pub const MATERIAL_VERSION: u32 = 1;

/// Mean diffuse albedo and emission for one primitive, as `surface_mean` returns them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimitiveShade {
    pub mean: [f32; 3],
    pub emission: [f32; 3],
}

/// Materials and image sizes for one cooked asset. No image pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct CookedMaterials {
    pub asset_id: [u8; 32],
    /// Width and height of each source image, in image order.
    pub image_size: Vec<[u32; 2]>,
    pub materials: Vec<PbrMaterial>,
    /// Empty, or one entry per primitive in mesh order.
    pub primitive_shade: Vec<PrimitiveShade>,
}

/// What [`load_covered`] assembles without calling the glTF importer.
#[derive(Clone, Debug, PartialEq)]
pub struct CoveredAsset {
    pub meshes: Vec<GltfMesh>,
    pub instances: Vec<MeshInstance>,
    pub materials: Vec<PbrMaterial>,
    pub image_size: Vec<[u32; 2]>,
    pub cooked: Vec<CookedImage>,
    pub primitive_shade: Vec<PrimitiveShade>,
}

/// [`load_covered_or_source`] either used the package or the source file.
#[derive(Clone, Debug, PartialEq)]
pub struct CoveredFallback {
    pub asset: CoveredAsset,
    /// `assets fallback …` lines.
    pub fallback: Vec<String>,
    /// The source path was handed to the glTF loader.
    pub used_source: bool,
}

/// Where the material record for a source lives under a cook directory.
#[must_use]
pub fn materials_path(dir: &Path, asset_id: &[u8; 32]) -> PathBuf {
    dir.join("cooked").join(format!("{}.gmat", hex(asset_id)))
}

/// Encode a material record, including its trailer.
///
/// # Errors
///
/// A length that does not fit in `u64`, or a texture slot whose image index
/// does not fit in `u32`.
pub fn encode_materials(materials: &CookedMaterials) -> Result<Vec<u8>, CookedError> {
    let mut w = W(Vec::new());
    w.0.extend_from_slice(MATERIAL_MAGIC);
    w.u32(MATERIAL_VERSION);
    w.0.extend_from_slice(&materials.asset_id);
    w.len(materials.image_size.len())?;
    for [width, height] in &materials.image_size {
        w.u32(*width);
        w.u32(*height);
    }
    w.len(materials.materials.len())?;
    for material in &materials.materials {
        w.opt_str(material.name.as_deref())?;
        w.floats(&material.base_color);
        w.slot(material.base_color_texture)?;
        w.f32(material.metallic);
        w.f32(material.roughness);
        w.slot(material.metallic_roughness_texture)?;
        w.slot(material.normal_texture)?;
        w.slot(material.occlusion_texture)?;
        w.floats(&material.emissive);
        w.slot(material.emissive_texture)?;
        w.f32(material.emissive_strength);
        let (tag, cutoff) = alpha_tag(material.alpha_mode);
        w.u32(tag);
        w.f32(cutoff);
        w.0.push(u8::from(material.double_sided));
    }
    w.len(materials.primitive_shade.len())?;
    for shade in &materials.primitive_shade {
        w.floats(&shade.mean);
        w.floats(&shade.emission);
    }
    let trailer = sha256(&w.0);
    w.0.extend_from_slice(&trailer);
    Ok(w.0)
}

/// Decode a material record after checking its trailer.
///
/// # Errors
///
/// A bad magic, version, length, utf-8 string, or trailer.
pub fn decode_materials(bytes: &[u8]) -> Result<CookedMaterials, CookedError> {
    let body = checked_body(bytes)?;
    let mut r = R { b: body, at: 0 };
    if r.take(4)? != MATERIAL_MAGIC {
        return Err(CookedError::Corrupt("gmat magic"));
    }
    if r.u32()? != MATERIAL_VERSION {
        return Err(CookedError::Corrupt("gmat version"));
    }
    let asset_id = r.array()?;
    let mut image_size = Vec::new();
    for _ in 0..r.len()? {
        image_size.push([r.u32()?, r.u32()?]);
    }
    let mut materials = Vec::new();
    for _ in 0..r.len()? {
        materials.push(r.material()?);
    }
    let mut primitive_shade = Vec::new();
    for _ in 0..r.len()? {
        primitive_shade.push(PrimitiveShade {
            mean: r.float3()?,
            emission: r.float3()?,
        });
    }
    if r.at != body.len() {
        return Err(CookedError::Corrupt("gmat trailing"));
    }
    Ok(CookedMaterials {
        asset_id,
        image_size,
        materials,
        primitive_shade,
    })
}

/// Load geometry, cooked images and materials for `asset_id`.
///
/// The manifest line must name `cook=` and `mat=`. `gtex=` is optional.
/// This function does not call the glTF importer and does not read the source
/// file or the cached source objects.
///
/// # Errors
///
/// A missing file is I/O. A bad hash, magic, or a shade count that is neither
/// zero nor the primitive count is corrupt.
pub fn load_covered(dir: &Path, asset_id: &[u8; 32]) -> Result<CoveredAsset, CookedError> {
    let package = crate::Package::open(dir)?;
    let line = package
        .line_for(asset_id)
        .ok_or(CookedError::Corrupt("asset is not in the package"))?;
    let refs = manifest_refs(line)?;
    let geom_bytes = read_hashed(
        &crate::cooked_path(dir, asset_id),
        refs.cook,
        "record hash differs from the manifest",
    )?;
    let geom = crate::decode_geometry(&geom_bytes)?;
    if geom.asset_id != *asset_id {
        return Err(CookedError::Corrupt("gcook asset id"));
    }
    let cooked = match refs.gtex {
        None => Vec::new(),
        Some(expect) => {
            let bytes = read_hashed(
                &crate::textures_path(dir, asset_id),
                expect,
                "texture hash differs from the manifest",
            )?;
            let images = crate::decode_images(&bytes)?;
            if images.asset_id != *asset_id {
                return Err(CookedError::Corrupt("gtex asset id"));
            }
            images.images
        }
    };
    let mat_bytes = read_hashed(
        &materials_path(dir, asset_id),
        refs.mat,
        "material hash differs from the manifest",
    )?;
    let materials = decode_materials(&mat_bytes)?;
    if materials.asset_id != *asset_id {
        return Err(CookedError::Corrupt("gmat asset id"));
    }
    check_images(&cooked, &materials.image_size)?;
    let prims: usize = geom.meshes.iter().map(|mesh| mesh.primitives.len()).sum();
    if !materials.primitive_shade.is_empty() && materials.primitive_shade.len() != prims {
        return Err(CookedError::Corrupt("primitive shade count"));
    }
    Ok(CoveredAsset {
        meshes: geom.meshes,
        instances: geom.instances,
        materials: materials.materials,
        image_size: materials.image_size,
        cooked,
        primitive_shade: materials.primitive_shade,
    })
}

struct ManifestRefs<'a> {
    cook: &'a str,
    gtex: Option<&'a str>,
    mat: &'a str,
}

fn manifest_refs(line: &str) -> Result<ManifestRefs<'_>, CookedError> {
    let mut cook = None;
    let mut gtex = None;
    let mut mat = None;
    for part in line.split(' ').skip(1) {
        if let Some(value) = part.strip_prefix("cook=") {
            cook = Some(value);
        } else if let Some(value) = part.strip_prefix("gtex=") {
            gtex = Some(value);
        } else if let Some(value) = part.strip_prefix("mat=") {
            mat = Some(value);
        }
    }
    Ok(ManifestRefs {
        cook: cook.ok_or(CookedError::Corrupt("manifest line"))?,
        gtex,
        mat: mat.ok_or(CookedError::Corrupt("no material record"))?,
    })
}

fn read_hashed(path: &Path, expect: &str, why: &'static str) -> Result<Vec<u8>, CookedError> {
    let bytes = std::fs::read(path)?;
    if hex(&sha256(&bytes)) != expect {
        return Err(CookedError::Corrupt(why));
    }
    Ok(bytes)
}

fn check_images(cooked: &[CookedImage], image_size: &[[u32; 2]]) -> Result<(), CookedError> {
    for image in cooked {
        let index =
            usize::try_from(image.image).map_err(|_| CookedError::Corrupt("image index"))?;
        let size = image_size
            .get(index)
            .ok_or(CookedError::Corrupt("cooked image index"))?;
        if image.width != size[0] || image.height != size[1] {
            return Err(CookedError::Corrupt(
                "image size differs from the cooked texture",
            ));
        }
    }
    Ok(())
}

/// [`load_covered`] when the package checks, otherwise the source glTF.
///
/// A package error appends one `assets fallback <source> <reason>` line and
/// loads `source`. The source load is the importer. An empty asset is returned
/// when that load fails too; the frame is not failed.
#[must_use]
pub fn load_covered_or_source(dir: &Path, asset_id: &[u8; 32], source: &Path) -> CoveredFallback {
    match load_covered(dir, asset_id) {
        Ok(asset) => CoveredFallback {
            asset,
            fallback: Vec::new(),
            used_source: false,
        },
        Err(err) => {
            let mut fallback = vec![format!(
                "assets fallback {} {}",
                source.display(),
                reason(&err)
            )];
            match crate::load_gltf_file(source) {
                Ok(scene) => CoveredFallback {
                    asset: from_scene(&scene),
                    fallback,
                    used_source: true,
                },
                Err(load_err) => {
                    fallback.push(format!("assets fallback {} {load_err}", source.display()));
                    CoveredFallback {
                        asset: CoveredAsset {
                            meshes: Vec::new(),
                            instances: Vec::new(),
                            materials: Vec::new(),
                            image_size: Vec::new(),
                            cooked: Vec::new(),
                            primitive_shade: Vec::new(),
                        },
                        fallback,
                        used_source: true,
                    }
                }
            }
        }
    }
}

fn from_scene(scene: &GltfScene) -> CoveredAsset {
    CoveredAsset {
        meshes: scene.meshes.clone(),
        instances: scene.instances.clone(),
        materials: scene.materials.clone(),
        image_size: scene
            .images
            .iter()
            .map(|image| [image.width, image.height])
            .collect(),
        cooked: Vec::new(),
        primitive_shade: Vec::new(),
    }
}

fn reason(err: &CookedError) -> String {
    match err {
        CookedError::Io(io) => io.to_string(),
        CookedError::Corrupt(why) => (*why).to_string(),
    }
}

fn alpha_tag(mode: AlphaMode) -> (u32, f32) {
    match mode {
        AlphaMode::Opaque => (0, 0.0),
        AlphaMode::Mask(cutoff) => (1, cutoff),
        AlphaMode::Blend => (2, 0.0),
    }
}

fn alpha_mode(tag: u32, cutoff: f32) -> Result<AlphaMode, CookedError> {
    match tag {
        0 => Ok(AlphaMode::Opaque),
        1 => Ok(AlphaMode::Mask(cutoff)),
        2 => Ok(AlphaMode::Blend),
        _ => Err(CookedError::Corrupt("alpha mode")),
    }
}

fn checked_body(bytes: &[u8]) -> Result<&[u8], CookedError> {
    let split = bytes
        .len()
        .checked_sub(32)
        .ok_or(CookedError::Corrupt("short"))?;
    let (body, trailer) = bytes.split_at(split);
    if sha256(body) != trailer {
        return Err(CookedError::Corrupt("content hash mismatch"));
    }
    Ok(body)
}

struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn len(&mut self, n: usize) -> Result<(), CookedError> {
        let n = u64::try_from(n).map_err(|_| CookedError::Corrupt("length"))?;
        self.0.extend_from_slice(&n.to_le_bytes());
        Ok(())
    }
    fn str(&mut self, s: &str) -> Result<(), CookedError> {
        self.len(s.len())?;
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
    fn opt_str(&mut self, s: Option<&str>) -> Result<(), CookedError> {
        self.0.push(u8::from(s.is_some()));
        self.str(s.unwrap_or(""))
    }
    fn floats<const N: usize>(&mut self, v: &[f32; N]) {
        for x in v {
            self.f32(*x);
        }
    }
    fn slot(&mut self, slot: Option<TextureSlot>) -> Result<(), CookedError> {
        let present = slot.is_some();
        self.0.push(u8::from(present));
        let slot = slot.unwrap_or(TextureSlot {
            image: 0,
            texcoord: 0,
            sampler: Sampler {
                mag_filter: 0,
                min_filter: 0,
                wrap_s: 0,
                wrap_t: 0,
            },
            scale: 0.0,
        });
        let image = u32::try_from(slot.image).map_err(|_| CookedError::Corrupt("image index"))?;
        self.u32(image);
        self.u32(slot.texcoord);
        self.u32(slot.sampler.mag_filter);
        self.u32(slot.sampler.min_filter);
        self.u32(slot.sampler.wrap_s);
        self.u32(slot.sampler.wrap_t);
        self.f32(slot.scale);
        Ok(())
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CookedError> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(CookedError::Corrupt("short"))?;
        let s = self
            .b
            .get(self.at..end)
            .ok_or(CookedError::Corrupt("short"))?;
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, CookedError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CookedError::Corrupt("short"))?,
        ))
    }
    fn f32(&mut self) -> Result<f32, CookedError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn len(&mut self) -> Result<usize, CookedError> {
        let n = u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| CookedError::Corrupt("short"))?,
        );
        let n = usize::try_from(n).map_err(|_| CookedError::Corrupt("length"))?;
        if n > self.b.len().saturating_sub(self.at) {
            return Err(CookedError::Corrupt("length"));
        }
        Ok(n)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CookedError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CookedError::Corrupt("short"))
    }
    fn str(&mut self) -> Result<String, CookedError> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| CookedError::Corrupt("utf8"))
    }
    fn opt_str(&mut self) -> Result<Option<String>, CookedError> {
        let flag = self.take(1)?[0];
        let s = self.str()?;
        match flag {
            0 => Ok(None),
            1 => Ok(Some(s)),
            _ => Err(CookedError::Corrupt("string flag")),
        }
    }
    fn float3(&mut self) -> Result<[f32; 3], CookedError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    fn slot(&mut self) -> Result<Option<TextureSlot>, CookedError> {
        let flag = self.take(1)?[0];
        let image =
            usize::try_from(self.u32()?).map_err(|_| CookedError::Corrupt("image index"))?;
        let texcoord = self.u32()?;
        let sampler = Sampler {
            mag_filter: self.u32()?,
            min_filter: self.u32()?,
            wrap_s: self.u32()?,
            wrap_t: self.u32()?,
        };
        let scale = self.f32()?;
        match flag {
            0 => Ok(None),
            1 => Ok(Some(TextureSlot {
                image,
                texcoord,
                sampler,
                scale,
            })),
            _ => Err(CookedError::Corrupt("texture slot")),
        }
    }
    fn material(&mut self) -> Result<PbrMaterial, CookedError> {
        let name = self.opt_str()?;
        let base_color = [self.f32()?, self.f32()?, self.f32()?, self.f32()?];
        let base_color_texture = self.slot()?;
        let metallic = self.f32()?;
        let roughness = self.f32()?;
        let metallic_roughness_texture = self.slot()?;
        let normal_texture = self.slot()?;
        let occlusion_texture = self.slot()?;
        let emissive = self.float3()?;
        let emissive_texture = self.slot()?;
        let emissive_strength = self.f32()?;
        let tag = self.u32()?;
        let cutoff = self.f32()?;
        let sided = self.take(1)?[0];
        let double_sided = match sided {
            0 => false,
            1 => true,
            _ => return Err(CookedError::Corrupt("double sided")),
        };
        Ok(PbrMaterial {
            name,
            base_color,
            base_color_texture,
            metallic,
            roughness,
            metallic_roughness_texture,
            normal_texture,
            occlusion_texture,
            emissive,
            emissive_texture,
            emissive_strength,
            alpha_mode: alpha_mode(tag, cutoff)?,
            double_sided,
        })
    }
}
