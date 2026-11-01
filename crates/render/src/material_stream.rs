//! CPU mip chain and the per-frame views the streamer reads.
//!
//! The sampled atlas is the finest resident mip. The transfer queue stretches
//! that mip into a device-local image; the CPU chain is not what the shader
//! loads. A missing page is that coarser mip, never an empty texel.

use genos_load::{Derivatives, TextureView};
use genos_scene::Camera;

use crate::world::{in_view, transform_pose, Bounds, DrawKind, Object, Surface, World};

/// One mip, finest first.
#[derive(Clone, Debug)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Box-filtered chain down to 1×1.
#[derive(Clone, Debug)]
pub struct Chain {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<MipLevel>,
    /// `VkFormat` of `levels`. 37 is RGBA8. A block format's level bytes are the
    /// compressed payload, not `width * height * 4`.
    pub format: i32,
}

impl Chain {
    #[must_use]
    pub fn level_bytes(&self) -> Vec<u64> {
        self.levels
            .iter()
            .map(|level| u64::try_from(level.rgba.len()).unwrap_or(u64::MAX))
            .collect()
    }

    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.level_bytes().into_iter().sum()
    }
}

/// `rgba` is the full image. A short buffer is not a chain.
#[must_use]
pub fn mip_chain(width: u32, height: u32, rgba: &[u8]) -> Option<Chain> {
    let texels = usize::try_from(u64::from(width) * u64::from(height) * 4).ok()?;
    if width == 0 || height == 0 || rgba.len() < texels {
        return None;
    }
    let mut levels = Vec::new();
    let mut level = MipLevel {
        width,
        height,
        rgba: rgba[..texels].to_vec(),
    };
    loop {
        let last = level.width == 1 && level.height == 1;
        let next = if last { None } else { Some(downsample(&level)) };
        levels.push(level);
        match next {
            Some(down) => level = down,
            None => break,
        }
    }
    Some(Chain {
        width,
        height,
        levels,
        format: 37,
    })
}

/// Nearest stretch of `level` onto the full image. A missing level is white,
/// which is the factor stand-in, not black. The frame does not call this;
/// the transfer blit is the stretch the shader sees.
#[cfg(test)]
#[must_use]
pub fn expand_level(chain: &Chain, level: usize) -> Vec<u8> {
    let Some(mip) = chain.levels.get(level) else {
        return vec![255u8; texel_count(chain.width, chain.height)];
    };
    let dest = texel_count(chain.width, chain.height);
    let mut out = vec![255u8; dest];
    let full_w = chain.width.max(1);
    let full_h = chain.height.max(1);
    let mip_w = mip.width.max(1);
    let mip_h = mip.height.max(1);
    for y in 0..full_h {
        for x in 0..full_w {
            let sx = x * mip_w / full_w;
            let sy = y * mip_h / full_h;
            let from = texel_index(sy, mip_w, sx);
            let to = texel_index(y, full_w, x);
            if from + 4 <= mip.rgba.len() && to + 4 <= out.len() {
                out[to..to + 4].copy_from_slice(&mip.rgba[from..from + 4]);
            }
        }
    }
    out
}

/// Colour atlas is index 0 when it exists. The normal atlas follows it.
#[must_use]
pub fn texture_index(color: bool, normal: bool, want_color: bool) -> Option<usize> {
    if want_color {
        color.then_some(0)
    } else if normal {
        Some(usize::from(color))
    } else {
        None
    }
}

/// One view per registered atlas. The nearest visible draw wins.
#[must_use]
pub fn frame_views(
    world: &World,
    camera: &Camera,
    width: u32,
    height: u32,
    view_proj: &[f32; 16],
) -> Vec<TextureView> {
    let color = atlas_live(
        world.color_atlas.width,
        world.color_atlas.height,
        &world.color_atlas.rgba,
    );
    let normal = atlas_live(
        world.normal_atlas.width,
        world.normal_atlas.height,
        &world.normal_atlas.rgba,
    );
    let mut views = Vec::new();
    if let Some(index) = texture_index(color, normal, true) {
        views.push(atlas_view(
            world, camera, width, height, view_proj, index, true,
        ));
    }
    if let Some(index) = texture_index(color, normal, false) {
        views.push(atlas_view(
            world, camera, width, height, view_proj, index, false,
        ));
    }
    let mut index = views.len();
    for streamed in &world.stream_textures {
        if !atlas_live(
            streamed.atlas.width,
            streamed.atlas.height,
            &streamed.atlas.rgba,
        ) {
            continue;
        }
        views.push(streamed_view(
            world, camera, width, height, view_proj, index, streamed,
        ));
        index += 1;
    }
    views
}

fn streamed_view(
    world: &World,
    camera: &Camera,
    width: u32,
    height: u32,
    view_proj: &[f32; 16],
    index: usize,
    streamed: &crate::world::StreamedTexture,
) -> TextureView {
    let far = TextureView {
        index,
        distance: 1.0e6,
        visible: false,
        derivatives: None,
    };
    let Some(object) = world.objects.get(streamed.object) else {
        return far;
    };
    let DrawKind::Mesh {
        vertices,
        texcoords,
        pose,
        ..
    } = &object.kind
    else {
        return far;
    };
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    let distance = bounds_distance(object.bounds, eye);
    let visible = !object.hidden && in_view(view_proj, object.bounds);
    let derivatives = if visible {
        triangle_derivatives(vertices, texcoords, pose, width, height, view_proj)
    } else {
        None
    };
    TextureView {
        index,
        distance,
        visible,
        derivatives,
    }
}

fn atlas_live(width: u32, height: u32, rgba: &[u8]) -> bool {
    let texels = u64::from(width) * u64::from(height) * 4;
    width > 0 && height > 0 && u64::try_from(rgba.len()).unwrap_or(0) >= texels
}

fn atlas_view(
    world: &World,
    camera: &Camera,
    width: u32,
    height: u32,
    view_proj: &[f32; 16],
    index: usize,
    color: bool,
) -> TextureView {
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    let atlas_w = if color {
        world.color_atlas.width
    } else {
        world.normal_atlas.width
    };
    let atlas_h = if color {
        world.color_atlas.height
    } else {
        world.normal_atlas.height
    };
    let ctx = ViewCtx {
        surfaces: &world.surfaces,
        eye,
        width,
        height,
        view_proj,
        color,
        atlas_w,
        atlas_h,
    };
    let mut best: Option<Hit> = None;
    for object in &world.objects {
        let Some(hit) = object_hit(object, &ctx) else {
            continue;
        };
        let replace = match &best {
            None => true,
            Some(so_far) => match (hit.visible, so_far.visible) {
                (true, false) => true,
                (false, true) => false,
                _ => hit.distance < so_far.distance,
            },
        };
        if replace {
            best = Some(hit);
        }
    }
    match best {
        Some(hit) => TextureView {
            index,
            distance: hit.distance,
            visible: hit.visible,
            derivatives: hit.derivatives,
        },
        None => TextureView {
            index,
            distance: 1.0e6,
            visible: false,
            derivatives: None,
        },
    }
}

struct Hit {
    distance: f32,
    visible: bool,
    derivatives: Option<Derivatives>,
}

struct ViewCtx<'a> {
    surfaces: &'a [Surface],
    eye: [f32; 3],
    width: u32,
    height: u32,
    view_proj: &'a [f32; 16],
    color: bool,
    atlas_w: u32,
    atlas_h: u32,
}

fn object_hit(object: &Object, ctx: &ViewCtx<'_>) -> Option<Hit> {
    let DrawKind::Mesh {
        vertices,
        texcoords,
        pose,
        surface,
        ..
    } = &object.kind
    else {
        return None;
    };
    let rect = surface
        .and_then(|slot| ctx.surfaces.get(slot))
        .map(|item| if ctx.color { item.color } else { item.normal })?;
    if rect[2] == 0 || rect[3] == 0 {
        return None;
    }
    let distance = bounds_distance(object.bounds, ctx.eye);
    let visible = !object.hidden && in_view(ctx.view_proj, object.bounds);
    let derivatives = if visible {
        triangle_derivatives(
            vertices,
            texcoords,
            pose,
            ctx.width,
            ctx.height,
            ctx.view_proj,
        )
        .map(|deriv| scale_to_atlas(deriv, rect[2], rect[3], ctx.atlas_w, ctx.atlas_h))
    } else {
        None
    };
    Some(Hit {
        distance,
        visible,
        derivatives,
    })
}

fn triangle_derivatives(
    vertices: &[[f32; 3]],
    texcoords: &[[f32; 2]],
    pose: &[f32; 16],
    width: u32,
    height: u32,
    view_proj: &[f32; 16],
) -> Option<Derivatives> {
    if texcoords.len() != vertices.len() || vertices.len() < 3 {
        return None;
    }
    let mut index = 0;
    while index + 2 < vertices.len() {
        let p0 = project(
            view_proj,
            transform_pose(pose, vertices[index]),
            width,
            height,
        );
        let p1 = project(
            view_proj,
            transform_pose(pose, vertices[index + 1]),
            width,
            height,
        );
        let p2 = project(
            view_proj,
            transform_pose(pose, vertices[index + 2]),
            width,
            height,
        );
        if let (Some(s0), Some(s1), Some(s2)) = (p0, p1, p2) {
            if let Some(deriv) = screen_derivatives(
                s0,
                s1,
                s2,
                texcoords[index],
                texcoords[index + 1],
                texcoords[index + 2],
            ) {
                return Some(deriv);
            }
        }
        index += 3;
    }
    None
}

fn screen_derivatives(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    uv0: [f32; 2],
    uv1: [f32; 2],
    uv2: [f32; 2],
) -> Option<Derivatives> {
    let edge_x = p1[0] - p0[0];
    let edge_y = p1[1] - p0[1];
    let other_x = p2[0] - p0[0];
    let other_y = p2[1] - p0[1];
    let det = edge_x * other_y - other_x * edge_y;
    if !det.is_finite() || det.abs() < 1.0e-4 {
        return None;
    }
    let du1 = uv1[0] - uv0[0];
    let dv1 = uv1[1] - uv0[1];
    let du2 = uv2[0] - uv0[0];
    let dv2 = uv2[1] - uv0[1];
    Some(Derivatives {
        dudx: (du1 * other_y - du2 * edge_y) / det,
        dvdx: (dv1 * other_y - dv2 * edge_y) / det,
        dudy: (edge_x * du2 - other_x * du1) / det,
        dvdy: (edge_x * dv2 - other_x * dv1) / det,
    })
}

fn scale_to_atlas(
    deriv: Derivatives,
    rect_w: u32,
    rect_h: u32,
    atlas_w: u32,
    atlas_h: u32,
) -> Derivatives {
    let across = u32_to_f32(rect_w) / u32_to_f32(atlas_w.max(1));
    let down = u32_to_f32(rect_h) / u32_to_f32(atlas_h.max(1));
    Derivatives {
        dudx: deriv.dudx * across,
        dvdx: deriv.dvdx * down,
        dudy: deriv.dudy * across,
        dvdy: deriv.dvdy * down,
    }
}

fn project(view_proj: &[f32; 16], world: [f32; 3], width: u32, height: u32) -> Option<[f32; 2]> {
    let x = world[0];
    let y = world[1];
    let z = world[2];
    let clip_w = view_proj[3] * x + view_proj[7] * y + view_proj[11] * z + view_proj[15];
    if !clip_w.is_finite() || clip_w <= 1.0e-4 {
        return None;
    }
    let clip_x = view_proj[0] * x + view_proj[4] * y + view_proj[8] * z + view_proj[12];
    let clip_y = view_proj[1] * x + view_proj[5] * y + view_proj[9] * z + view_proj[13];
    let ndc_x = clip_x / clip_w;
    let ndc_y = clip_y / clip_w;
    Some([
        (ndc_x * 0.5 + 0.5) * u32_to_f32(width.max(1)),
        (ndc_y * 0.5 + 0.5) * u32_to_f32(height.max(1)),
    ])
}

fn bounds_distance(bounds: Bounds, eye: [f32; 3]) -> f32 {
    let dx = bounds.center[0] - eye[0];
    let dy = bounds.center[1] - eye[1];
    let dz = bounds.center[2] - eye[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn downsample(level: &MipLevel) -> MipLevel {
    let width = (level.width / 2).max(1);
    let height = (level.height / 2).max(1);
    let mut rgba = vec![0u8; texel_count(width, height)];
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0u32; 4];
            for oy in 0..2 {
                for ox in 0..2 {
                    let sx = (x * 2 + ox).min(level.width.saturating_sub(1));
                    let sy = (y * 2 + oy).min(level.height.saturating_sub(1));
                    let from = texel_index(sy, level.width, sx);
                    sum[0] = sum[0].saturating_add(u32::from(level.rgba[from]));
                    sum[1] = sum[1].saturating_add(u32::from(level.rgba[from + 1]));
                    sum[2] = sum[2].saturating_add(u32::from(level.rgba[from + 2]));
                    sum[3] = sum[3].saturating_add(u32::from(level.rgba[from + 3]));
                }
            }
            let to = texel_index(y, width, x);
            rgba[to] = u8::try_from(sum[0] / 4).unwrap_or(255);
            rgba[to + 1] = u8::try_from(sum[1] / 4).unwrap_or(255);
            rgba[to + 2] = u8::try_from(sum[2] / 4).unwrap_or(255);
            rgba[to + 3] = u8::try_from(sum[3] / 4).unwrap_or(255);
        }
    }
    MipLevel {
        width,
        height,
        rgba,
    }
}

fn texel_count(width: u32, height: u32) -> usize {
    usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap_or(0)
}

fn texel_index(y: u32, width: u32, x: u32) -> usize {
    usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4).unwrap_or(0)
}

fn u32_to_f32(value: u32) -> f32 {
    let mut out = 0.0f32;
    let mut bit = 1.0f32;
    let mut left = value;
    while left > 0 {
        if left & 1 == 1 {
            out += bit;
        }
        left >>= 1;
        bit *= 2.0;
    }
    out
}

#[cfg(test)]
mod tests {
    use genos_load::Derivatives;

    use super::*;

    #[test]
    fn a_flat_image_stays_flat_down_the_chain_and_stretches_back() {
        let chain = mip_chain(
            2,
            2,
            &[
                10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255,
            ],
        )
        .unwrap_or(Chain {
            width: 0,
            height: 0,
            levels: Vec::new(),
            format: 37,
        });
        assert_eq!(chain.levels.len(), 2);
        assert_eq!(chain.levels[1].rgba, vec![10, 20, 30, 255]);
        let full = expand_level(&chain, 1);
        assert_eq!(full.len(), 16);
        assert_eq!(&full[0..4], &[10, 20, 30, 255]);
        assert_eq!(expand_level(&chain, 0), chain.levels[0].rgba);
    }

    #[test]
    fn a_screen_edge_is_the_uv_slope() {
        let deriv = screen_derivatives(
            [0.0, 0.0],
            [10.0, 0.0],
            [0.0, 10.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
        )
        .unwrap_or(Derivatives {
            dudx: -1.0,
            dvdx: -1.0,
            dudy: -1.0,
            dvdy: -1.0,
        });
        assert_eq!(deriv.dudx.to_bits(), 0.1f32.to_bits());
        assert_eq!(deriv.dvdy.to_bits(), 0.1f32.to_bits());
        assert_eq!(deriv.dvdx.to_bits(), 0.0f32.to_bits());
        assert_eq!(deriv.dudy.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn the_normal_atlas_follows_the_colour_atlas() {
        assert_eq!(texture_index(true, true, true), Some(0));
        assert_eq!(texture_index(true, true, false), Some(1));
        assert_eq!(texture_index(false, true, false), Some(0));
        assert_eq!(texture_index(false, false, true), None);
    }
}
