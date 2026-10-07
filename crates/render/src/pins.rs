//! Surface probes for the two finest radiance cascades.
//!
//! A probe sits at the center of its world cell on a floor or wall face.
//! Cascade 0 uses 0.5 m cells. Cascade 1 uses 1 m cells. The position is a
//! function of the cell, so a camera move does not slide it. Cells are kept
//! in a window around the camera. Each solid carries its own face lattice on
//! cascade 0. Those points follow the solid and are not snapped to the world
//! cells. Cascade 2 is cast from the camera on every call.

use std::collections::HashMap;

use genos_scene::{Camera, Scene, Shape, Solid};

use crate::field::{screen_grid, SCREEN_MAX_H, SCREEN_MAX_W};

/// World-cell size of cascade 0.
pub const FINE_SPACING: f32 = 0.5;
/// World-cell size of cascade 1.
pub const NEXT_SPACING: f32 = 1.0;
/// Safety cap for cascade 0, including each solid's face lattice.
pub const PROBE_CAP0: u32 = 2048;
/// Safety cap for cascade 1. Direction storage must end before [`PIN_POS0`].
pub const PROBE_CAP1: u32 = 1024;
/// Hash cube for cascade 0. 32 cells at 0.5 m is a 16 m window.
pub const HASH_DIM0: u32 = 32;
/// Hash cube for cascade 1. 28 cells at 1 m covers the 14 m keep window.
pub const HASH_DIM1: u32 = 28;
/// Integrated irradiance for cascade 1. Cascade 0 uses probe index 0.
pub const PIN_IRR1: u32 = PROBE_CAP0;

/// Texel where cascade 0 stores positions. The gather and the fragment sample read here.
pub const PIN_POS0: u32 = 98304;
/// Normals for cascade 0, one texel per reserved slot.
pub const PIN_NRM0: u32 = PIN_POS0 + PROBE_CAP0;
/// Positions for cascade 1.
pub const PIN_POS1: u32 = PIN_NRM0 + PROBE_CAP0;
/// Normals for cascade 1.
pub const PIN_NRM1: u32 = PIN_POS1 + PROBE_CAP1;
/// `xyz` is the cascade 0 hash origin. `w` is the cell size.
pub const HASH_ORIGIN0: u32 = PIN_NRM1 + PROBE_CAP1;
/// Cascade 0 hash. One texel holds two probe indices, or -1 when empty.
pub const HASH_BASE0: u32 = HASH_ORIGIN0 + 1;
/// Cells in the cascade 0 hash.
pub const HASH_CELLS0: u32 = HASH_DIM0 * HASH_DIM0 * HASH_DIM0;
/// `xyz` is the cascade 1 hash origin. `w` is the cell size.
pub const HASH_ORIGIN1: u32 = HASH_BASE0 + HASH_CELLS0;
/// Cascade 1 hash.
pub const HASH_BASE1: u32 = HASH_ORIGIN1 + 1;
/// Cells in the cascade 1 hash.
pub const HASH_CELLS1: u32 = HASH_DIM1 * HASH_DIM1 * HASH_DIM1;
/// First texel of the pin upload, and the number of texels it covers.
pub const PIN_SPAN_START: u32 = PIN_POS0;
pub const PIN_SPAN_LEN: u32 = HASH_BASE1 + HASH_CELLS1 - PIN_POS0;

/// The pin block ends before the world-probe irradiance at texel 176128.
const _: () = assert!(PIN_SPAN_START + PIN_SPAN_LEN < 176_128);
/// Cascade 0 radiance is stored at the probe index, below the screen-normal base.
const _: () = assert!(PROBE_CAP0 < 8192);
/// Cascade 1 irradiance sits after cascade 0 and still below the screen normals.
const _: () = assert!(PIN_IRR1 + PROBE_CAP1 < 8192);
/// Cascade 1 directions plus the farthest screen cascade end before the pin block.
const _: () =
    assert!(16_384 + PROBE_CAP1 * 32 + (SCREEN_MAX_W / 4) * (SCREEN_MAX_H / 4) * 64 < PIN_POS0);

/// Cascade 0 cells are created inside this window. The picture fades them out by 12 m.
const FINE_REACH: f32 = 8.0;
/// Cascade 1 cells cover the fade after the fine window.
const NEXT_REACH: f32 = 12.0;
/// Keep a cell until the fade into the world volume has finished.
const DROP: f32 = 14.0;
/// Two hits share a face when their normals agree by at least this much.
const FACE_DOT: f32 = 0.6;

/// One surface hit kept in the world.
#[derive(Clone, Copy, Debug)]
pub struct ScreenPin {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// A floor probe under a solid stays, and the sample skips it.
    pub hidden: bool,
    /// True when the probe belongs to a solid's face lattice.
    pub object: bool,
}

/// Cascade 0 and cascade 1. Cascade 2 is not stored.
#[derive(Clone, Debug, Default)]
pub struct ScreenPins {
    pub layers: [Vec<ScreenPin>; 2],
}

/// Radiance traced at the current pins. Positions are the pin set the gather used.
#[derive(Clone, Debug)]
pub struct PinnedGather {
    pub positions: [Vec<[f32; 3]>; 2],
    pub radiance: [Vec<[f32; 3]>; 2],
}

/// Camera basis the screen cast and the shader share.
#[derive(Clone, Copy, Debug)]
pub struct PinView {
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub tan_half: f32,
    pub aspect: f32,
    pub grid_w: u32,
    pub grid_h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SurfaceKind {
    Floor,
    Wall,
    Solid,
}

impl PinView {
    /// The same basis [`crate::pack::apply_view`] writes into the scene block.
    pub fn from_camera(camera: &Camera, aspect: f32, width: u32, height: u32) -> Self {
        let forward = genos_scene::look_direction(camera.yaw, camera.pitch);
        let mut right = forward.cross(genos_scene::Vec3::Y);
        let scale = right.length().max(1.0e-6);
        right = right / scale;
        let up = right.cross(forward);
        let (grid_w, grid_h) = screen_grid(width, height);
        Self {
            eye: [camera.position.x, camera.position.y, camera.position.z],
            forward: [forward.x, forward.y, forward.z],
            right: [right.x, right.y, right.z],
            up: [up.x, up.y, up.z],
            tan_half: 30.0_f32.to_radians().tan(),
            aspect: aspect.max(0.01),
            grid_w,
            grid_h,
        }
    }

    pub(crate) fn from_pack(pack: &crate::pack::Pack) -> Self {
        Self {
            eye: pack.eye,
            forward: pack.view_forward,
            right: pack.view_right,
            up: pack.view_up,
            tan_half: pack.view_tan,
            aspect: pack.view_aspect,
            grid_w: pack.grid_w,
            grid_h: pack.grid_h,
        }
    }
}

/// Columns and rows of one screen cascade. Cascade 0 is the 8-pixel grid.
pub fn layer_counts(grid_w: u32, grid_h: u32, cascade: u32) -> (u32, u32) {
    ((grid_w >> cascade).max(1), (grid_h >> cascade).max(1))
}

/// True when `world` is in front of the camera and inside the view.
pub fn on_screen(view: &PinView, world: [f32; 3]) -> bool {
    project(view, world).is_some()
}

/// Fill world cells for the two finest layers, and refresh each solid's lattice.
///
/// A floor or wall probe is inserted once, at the cell center. Its position
/// stays. A probe is removed when it leaves the drop window, or when the
/// safety cap drops the farthest probes. The same camera a second time does
/// not move a probe or grow the count.
pub fn update_screen_pins(pins: &mut ScreenPins, scene: &Scene, view: &PinView) {
    update_lattice(
        &mut pins.layers[0],
        scene,
        view.eye,
        FINE_SPACING,
        FINE_REACH,
        PROBE_CAP0 as usize,
        true,
    );
    update_lattice(
        &mut pins.layers[1],
        scene,
        view.eye,
        NEXT_SPACING,
        NEXT_REACH,
        PROBE_CAP1 as usize,
        false,
    );
}

/// Cascade 2 hits for this camera. The next call casts again. Nothing is kept.
pub fn far_screen_hits(scene: &Scene, view: &PinView) -> Vec<[f32; 3]> {
    let (nx, ny) = layer_counts(view.grid_w, view.grid_h, 2);
    let mut hits = Vec::new();
    for iy in 0..ny {
        for ix in 0..nx {
            if let Some(pin) = cast_cell(scene, view, nx, ny, ix, iy) {
                hits.push(pin.position);
            }
        }
    }
    hits
}

/// Unlit marks for one lattice layer. Probes outside the view are dimmer.
///
/// Each probe draws a short tick along its normal. Every eighth probe also
/// draws a few interval rays. Control+2 draws layer 0. Control+3 draws layer 1.
pub(crate) fn pinned_debug_lines(
    pins: &[ScreenPin],
    view: &PinView,
    cascade: u32,
    color: [f32; 3],
) -> Vec<crate::pack::GpuVertex> {
    let (t0, t1) = if cascade == 0 {
        (0.0, 0.55)
    } else {
        (0.55, 1.65)
    };
    let mut out = Vec::new();
    let mut drawn = 0u32;
    for pin in pins {
        if pin.hidden {
            continue;
        }
        let focused = project(view, pin.position).is_some();
        let ink = if focused {
            color
        } else {
            [color[0] * 0.35, color[1] * 0.35, color[2] * 0.35]
        };
        let origin = [
            pin.position[0] + pin.normal[0] * 0.04,
            pin.position[1] + pin.normal[1] * 0.04,
            pin.position[2] + pin.normal[2] * 0.04,
        ];
        let tip = [
            origin[0] + pin.normal[0] * 0.18,
            origin[1] + pin.normal[1] * 0.18,
            origin[2] + pin.normal[2] * 0.18,
        ];
        push_debug_line(&mut out, origin, tip, ink);
        if drawn % 8 == 0 {
            for dir in 0..4u32 {
                let ray = debug_hemisphere(pin.normal, dir, 4);
                let start = [
                    origin[0] + ray[0] * t0,
                    origin[1] + ray[1] * t0,
                    origin[2] + ray[2] * t0,
                ];
                let end = [
                    origin[0] + ray[0] * t1,
                    origin[1] + ray[1] * t1,
                    origin[2] + ray[2] * t1,
                ];
                push_debug_line(&mut out, start, end, ink);
            }
        }
        drawn += 1;
    }
    out
}

fn debug_hemisphere(normal: [f32; 3], dir: u32, dirs: u32) -> [f32; 3] {
    let nrm = dirs.max(1) as f32;
    let i = dir as f32 + 0.5;
    let cos_t = (1.0 - i / nrm).clamp(0.0, 1.0);
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let phi = i * 2.39996323;
    let up = if normal[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let tx = up[1] * normal[2] - up[2] * normal[1];
    let ty = up[2] * normal[0] - up[0] * normal[2];
    let tz = up[0] * normal[1] - up[1] * normal[0];
    let len = (tx * tx + ty * ty + tz * tz).sqrt().max(1.0e-6);
    let tx = tx / len;
    let ty = ty / len;
    let tz = tz / len;
    let bx = normal[1] * tz - normal[2] * ty;
    let by = normal[2] * tx - normal[0] * tz;
    let bz = normal[0] * ty - normal[1] * tx;
    let (s, c) = phi.sin_cos();
    [
        tx * c * sin_t + bx * s * sin_t + normal[0] * cos_t,
        ty * c * sin_t + by * s * sin_t + normal[1] * cos_t,
        tz * c * sin_t + bz * s * sin_t + normal[2] * cos_t,
    ]
}

fn push_debug_line(
    out: &mut Vec<crate::pack::GpuVertex>,
    a: [f32; 3],
    b: [f32; 3],
    color: [f32; 3],
) {
    out.push(crate::pack::overlay_vertex(a, color));
    out.push(crate::pack::overlay_vertex(b, color));
}

/// Trace radiance at the pinned positions. The previous frame is not an input.
pub fn gather_pinned(scene: &Scene, pins: &ScreenPins) -> PinnedGather {
    let mut positions = [Vec::new(), Vec::new()];
    let mut radiance = [Vec::new(), Vec::new()];
    for layer in 0..2 {
        for pin in &pins.layers[layer] {
            positions[layer].push(pin.position);
            let color = if pin.hidden {
                [0.0, 0.0, 0.0]
            } else {
                radiance_at(scene, pin)
            };
            radiance[layer].push(color);
        }
    }
    PinnedGather {
        positions,
        radiance,
    }
}

/// Field texels the gather reads for the two finest layers.
///
/// Index 0 of the returned buffer is texel [`PIN_SPAN_START`].
pub fn pin_texels(pins: &ScreenPins, view: &PinView) -> Vec<[f32; 4]> {
    let mut texels = vec![[0.0; 4]; PIN_SPAN_LEN as usize];
    write_layer(
        &mut texels,
        &pins.layers[0],
        view.eye,
        PIN_POS0,
        PIN_NRM0,
        HASH_ORIGIN0,
        HASH_BASE0,
        FINE_SPACING,
        HASH_DIM0,
    );
    write_layer(
        &mut texels,
        &pins.layers[1],
        view.eye,
        PIN_POS1,
        PIN_NRM1,
        HASH_ORIGIN1,
        HASH_BASE1,
        NEXT_SPACING,
        HASH_DIM1,
    );
    texels
}

fn write_layer(
    texels: &mut [[f32; 4]],
    pins: &[ScreenPin],
    eye: [f32; 3],
    pos_base: u32,
    nrm_base: u32,
    origin_base: u32,
    hash_base: u32,
    spacing: f32,
    dim: u32,
) {
    let origin_texel = PIN_SPAN_START;
    for (index, pin) in pins.iter().enumerate() {
        let pos_at = (pos_base - origin_texel) as usize + index;
        let nrm_at = (nrm_base - origin_texel) as usize + index;
        if pos_at >= texels.len() || nrm_at >= texels.len() {
            break;
        }
        let live = if pin.hidden { 0.0 } else { 1.0 };
        texels[pos_at] = [pin.position[0], pin.position[1], pin.position[2], live];
        texels[nrm_at] = [pin.normal[0], pin.normal[1], pin.normal[2], 0.0];
    }
    let hash_origin = hash_origin(eye, spacing, dim);
    texels[(origin_base - origin_texel) as usize] =
        [hash_origin[0], hash_origin[1], hash_origin[2], spacing];
    let cells = (dim * dim * dim) as usize;
    let hash_at = (hash_base - origin_texel) as usize;
    for cell in 0..cells {
        texels[hash_at + cell] = [-1.0, -1.0, 0.0, 0.0];
    }
    // Solid faces go in first. A parked floor probe is hidden and stays out.
    for object_pass in [true, false] {
        for (index, pin) in pins.iter().enumerate() {
            if pin.hidden || pin.object != object_pass {
                continue;
            }
            let Some(cell) = hash_cell(pin.position, hash_origin, spacing, dim) else {
                continue;
            };
            let slot = hash_at + hash_index(cell, dim);
            let texel = &mut texels[slot];
            if texel[0] < 0.0 {
                texel[0] = index as f32;
            } else if texel[1] < 0.0 {
                texel[1] = index as f32;
            }
        }
    }
}

fn hash_origin(eye: [f32; 3], spacing: f32, dim: u32) -> [f32; 3] {
    let half = (dim / 2) as f32 * spacing;
    [
        (eye[0] / spacing).floor() * spacing - half,
        (eye[1] / spacing).floor() * spacing - half,
        (eye[2] / spacing).floor() * spacing - half,
    ]
}

fn hash_cell(pos: [f32; 3], origin: [f32; 3], spacing: f32, dim: u32) -> Option<[i32; 3]> {
    let cell = [
        ((pos[0] - origin[0]) / spacing).floor() as i32,
        ((pos[1] - origin[1]) / spacing).floor() as i32,
        ((pos[2] - origin[2]) / spacing).floor() as i32,
    ];
    let dim = dim as i32;
    if cell[0] < 0
        || cell[1] < 0
        || cell[2] < 0
        || cell[0] >= dim
        || cell[1] >= dim
        || cell[2] >= dim
    {
        return None;
    }
    Some(cell)
}

fn hash_index(cell: [i32; 3], dim: u32) -> usize {
    let dim = dim as i32;
    ((cell[1] * dim + cell[2]) * dim + cell[0]) as usize
}

fn update_lattice(
    pins: &mut Vec<ScreenPin>,
    scene: &Scene,
    eye: [f32; 3],
    spacing: f32,
    reach: f32,
    cap: usize,
    with_objects: bool,
) {
    let hits = lattice_hits(scene, eye, spacing, reach);
    // Drop at the creation window. A fine pin kept past FINE_REACH sits outside the
    // fine hash: it is traced every build but no pixel can find it. A next-level pin
    // kept to 14 m made coverage depend on the path the camera took.
    pins.retain(|pin| pin.object || in_window(pin.position, eye, reach));
    if with_objects {
        pins.retain(|pin| !pin.object);
    }
    for pin in pins.iter_mut() {
        pin.hidden = !pin.object && covered_by_solid(scene, pin);
    }
    let mut taken: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    for (index, pin) in pins.iter().enumerate() {
        if pin.object {
            continue;
        }
        taken
            .entry(cell_key(pin.position, spacing))
            .or_default()
            .push(index);
    }
    for hit in &hits {
        if !in_window(hit.position, eye, reach) {
            continue;
        }
        let key = cell_key(hit.position, spacing);
        let same_face = taken.get(&key).is_some_and(|ids| {
            ids.iter()
                .any(|&index| dot(pins[index].normal, hit.normal) > FACE_DOT)
        });
        if same_face {
            continue;
        }
        let full = taken.get(&key).is_some_and(|ids| ids.len() >= 2);
        if full || pins.len() >= cap {
            continue;
        }
        let index = pins.len();
        pins.push(ScreenPin {
            position: hit.position,
            normal: hit.normal,
            hidden: covered_by_solid(scene, hit),
            object: false,
        });
        taken.entry(key).or_default().push(index);
    }
    if with_objects {
        let objects = object_probes(scene, eye);
        let extra = objects.len().saturating_sub(cap.saturating_sub(pins.len()));
        if extra > 0 {
            drop_farthest_world(pins, extra, eye);
        }
        for probe in objects {
            if pins.len() >= cap {
                break;
            }
            pins.push(probe);
        }
    }
    trim_to_cap(pins, cap, eye);
}

fn cell_key(position: [f32; 3], spacing: f32) -> [i32; 3] {
    [
        (position[0] / spacing).floor() as i32,
        (position[1] / spacing).floor() as i32,
        (position[2] / spacing).floor() as i32,
    ]
}

fn covered_by_solid(scene: &Scene, pin: &ScreenPin) -> bool {
    if pin.normal[1] <= 0.5 {
        return false;
    }
    scene
        .solids
        .iter()
        .any(|solid| solid.height > 0.05 && solid.contains_xz(pin.position[0], pin.position[2]))
}

fn farthest(pins: &[ScreenPin], eye: [f32; 3], object: bool) -> Option<usize> {
    let mut worst = None;
    let mut worst_distance = -1.0_f32;
    for (index, pin) in pins.iter().enumerate() {
        if pin.object != object {
            continue;
        }
        let away = distance(pin.position, eye);
        if away > worst_distance {
            worst_distance = away;
            worst = Some(index);
        }
    }
    worst
}

fn drop_farthest_world(pins: &mut Vec<ScreenPin>, count: usize, eye: [f32; 3]) {
    for _ in 0..count {
        let Some(index) = farthest(pins, eye, false) else {
            return;
        };
        pins.swap_remove(index);
    }
}

fn trim_to_cap(pins: &mut Vec<ScreenPin>, cap: usize, eye: [f32; 3]) {
    while pins.len() > cap {
        let index = farthest(pins, eye, false).or_else(|| farthest(pins, eye, true));
        let Some(index) = index else {
            return;
        };
        pins.swap_remove(index);
    }
}

fn object_probes(scene: &Scene, eye: [f32; 3]) -> Vec<ScreenPin> {
    let mut out = Vec::new();
    for solid in &scene.solids {
        let center = [solid.position.x, solid.height * 0.5, solid.position.z];
        if !in_window(center, eye, DROP + solid.size.max(solid.height)) {
            continue;
        }
        match solid.shape {
            Shape::Square => push_box(&mut out, solid),
            Shape::Circle => push_cylinder(&mut out, solid),
        }
    }
    out
}

fn push_box(out: &mut Vec<ScreenPin>, solid: &Solid) {
    let xs = axis_samples(solid.size);
    let ys = height_samples(solid.height);
    let zs = axis_samples(solid.size);
    let half = solid.size * 0.5;
    for y in &ys {
        for z in &zs {
            push_object(
                out,
                [solid.position.x + half, *y, solid.position.z + z],
                [1.0, 0.0, 0.0],
            );
            push_object(
                out,
                [solid.position.x - half, *y, solid.position.z + z],
                [-1.0, 0.0, 0.0],
            );
        }
        for x in &xs {
            push_object(
                out,
                [solid.position.x + x, *y, solid.position.z + half],
                [0.0, 0.0, 1.0],
            );
            push_object(
                out,
                [solid.position.x + x, *y, solid.position.z - half],
                [0.0, 0.0, -1.0],
            );
        }
    }
    for x in &xs {
        for z in &zs {
            push_object(
                out,
                [solid.position.x + x, solid.height, solid.position.z + z],
                [0.0, 1.0, 0.0],
            );
        }
    }
}

fn push_cylinder(out: &mut Vec<ScreenPin>, solid: &Solid) {
    let radius = solid.size * 0.5;
    let around = ((std::f32::consts::TAU * radius / FINE_SPACING).round() as i32).clamp(4, 48);
    for y in height_samples(solid.height) {
        for step in 0..around {
            let angle = step as f32 / around as f32 * std::f32::consts::TAU;
            let (s, c) = angle.sin_cos();
            push_object(
                out,
                [
                    solid.position.x + c * radius,
                    y,
                    solid.position.z + s * radius,
                ],
                [c, 0.0, s],
            );
        }
    }
    let radius_sq = radius * radius + 1.0e-4;
    for x in axis_samples(solid.size) {
        for z in axis_samples(solid.size) {
            if x * x + z * z <= radius_sq {
                push_object(
                    out,
                    [solid.position.x + x, solid.height, solid.position.z + z],
                    [0.0, 1.0, 0.0],
                );
            }
        }
    }
}

fn push_object(out: &mut Vec<ScreenPin>, position: [f32; 3], normal: [f32; 3]) {
    out.push(ScreenPin {
        position,
        normal,
        hidden: false,
        object: true,
    });
}

fn axis_samples(span: f32) -> Vec<f32> {
    let count = ((span / FINE_SPACING).round() as i32).clamp(1, 32);
    (0..count)
        .map(|index| {
            let t = (index as f32 + 0.5) / count as f32;
            -span * 0.5 + t * span
        })
        .collect()
}

fn height_samples(height: f32) -> Vec<f32> {
    let count = ((height / FINE_SPACING).round() as i32).clamp(1, 32);
    (0..count)
        .map(|index| {
            let t = (index as f32 + 0.5) / count as f32;
            t * height
        })
        .collect()
}

fn in_window(position: [f32; 3], eye: [f32; 3], reach: f32) -> bool {
    (position[0] - eye[0]).abs() <= reach
        && (position[1] - eye[1]).abs() <= reach
        && (position[2] - eye[2]).abs() <= reach
}

fn lattice_hits(scene: &Scene, eye: [f32; 3], spacing: f32, reach: f32) -> Vec<ScreenPin> {
    let mut hits = Vec::new();
    stamp_floor(&mut hits, scene, eye, spacing, reach);
    for wall in &scene.walls {
        stamp_wall(&mut hits, wall, eye, spacing, reach);
    }
    hits
}

fn stamp_floor(out: &mut Vec<ScreenPin>, scene: &Scene, eye: [f32; 3], spacing: f32, reach: f32) {
    let floor = &scene.floor;
    let x0 = ((eye[0] - reach) / spacing).floor() as i32;
    let x1 = ((eye[0] + reach) / spacing).floor() as i32;
    let z0 = ((eye[2] - reach) / spacing).floor() as i32;
    let z1 = ((eye[2] + reach) / spacing).floor() as i32;
    for ix in x0..=x1 {
        for iz in z0..=z1 {
            let point = [
                (ix as f32 + 0.5) * spacing,
                0.0,
                (iz as f32 + 0.5) * spacing,
            ];
            if !in_window(point, eye, reach) {
                continue;
            }
            let dx = point[0] - floor.position.x;
            let dz = point[2] - floor.position.z;
            if dx.abs() > floor.half_x + 0.05 || dz.abs() > floor.half_z + 0.05 {
                continue;
            }
            out.push(world_pin(point, [0.0, 1.0, 0.0]));
        }
    }
}

fn stamp_wall(
    out: &mut Vec<ScreenPin>,
    wall: &genos_scene::Wall,
    eye: [f32; 3],
    spacing: f32,
    reach: f32,
) {
    let x0 = wall.position.x - wall.half_x;
    let x1 = wall.position.x + wall.half_x;
    let z0 = wall.position.z - wall.half_z;
    let z1 = wall.position.z + wall.half_z;
    let y1 = wall.height.max(0.0);
    stamp_axis(
        out,
        eye,
        spacing,
        reach,
        0,
        x1,
        [1.0, 0.0, 0.0],
        1,
        0.0,
        y1,
        2,
        z0,
        z1,
    );
    stamp_axis(
        out,
        eye,
        spacing,
        reach,
        0,
        x0,
        [-1.0, 0.0, 0.0],
        1,
        0.0,
        y1,
        2,
        z0,
        z1,
    );
    stamp_axis(
        out,
        eye,
        spacing,
        reach,
        2,
        z1,
        [0.0, 0.0, 1.0],
        0,
        x0,
        x1,
        1,
        0.0,
        y1,
    );
    stamp_axis(
        out,
        eye,
        spacing,
        reach,
        2,
        z0,
        [0.0, 0.0, -1.0],
        0,
        x0,
        x1,
        1,
        0.0,
        y1,
    );
    stamp_axis(
        out,
        eye,
        spacing,
        reach,
        1,
        y1,
        [0.0, 1.0, 0.0],
        0,
        x0,
        x1,
        2,
        z0,
        z1,
    );
}

fn stamp_axis(
    out: &mut Vec<ScreenPin>,
    eye: [f32; 3],
    spacing: f32,
    reach: f32,
    fixed_axis: usize,
    fixed: f32,
    normal: [f32; 3],
    u_axis: usize,
    u0: f32,
    u1: f32,
    v_axis: usize,
    v0: f32,
    v1: f32,
) {
    let (u_lo, u_hi) = (u0.min(u1), u0.max(u1));
    let (v_lo, v_hi) = (v0.min(v1), v0.max(v1));
    let u_start = (u_lo / spacing).floor() as i32;
    let u_end = (u_hi / spacing).floor() as i32;
    let v_start = (v_lo / spacing).floor() as i32;
    let v_end = (v_hi / spacing).floor() as i32;
    for iu in u_start..=u_end {
        let u = (iu as f32 + 0.5) * spacing;
        if u < u_lo - 1.0e-4 || u > u_hi + 1.0e-4 {
            continue;
        }
        for iv in v_start..=v_end {
            let v = (iv as f32 + 0.5) * spacing;
            if v < v_lo - 1.0e-4 || v > v_hi + 1.0e-4 {
                continue;
            }
            let mut position = [0.0; 3];
            position[fixed_axis] = fixed;
            position[u_axis] = u;
            position[v_axis] = v;
            if in_window(position, eye, reach) {
                out.push(world_pin(position, normal));
            }
        }
    }
}

fn world_pin(position: [f32; 3], normal: [f32; 3]) -> ScreenPin {
    ScreenPin {
        position,
        normal,
        hidden: false,
        object: false,
    }
}

fn cast_cell(
    scene: &Scene,
    view: &PinView,
    nx: u32,
    ny: u32,
    ix: u32,
    iy: u32,
) -> Option<ScreenPin> {
    cast_uv(scene, view, cell_uv(nx, ny, ix, iy)).map(|(pin, _)| pin)
}

fn cell_uv(nx: u32, ny: u32, ix: u32, iy: u32) -> [f32; 2] {
    [(ix as f32 + 0.5) / nx as f32, (iy as f32 + 0.5) / ny as f32]
}

fn project(view: &PinView, world: [f32; 3]) -> Option<[f32; 2]> {
    let to = [
        world[0] - view.eye[0],
        world[1] - view.eye[1],
        world[2] - view.eye[2],
    ];
    let depth = dot(to, view.forward);
    if depth < 0.05 {
        return None;
    }
    let ndc_x = dot(to, view.right) / (depth * view.tan_half * view.aspect);
    let ndc_y = -dot(to, view.up) / (depth * view.tan_half);
    let uv = [(ndc_x + 1.0) * 0.5, (ndc_y + 1.0) * 0.5];
    if uv[0] < 0.0 || uv[1] < 0.0 || uv[0] > 1.0 || uv[1] > 1.0 {
        return None;
    }
    Some(uv)
}

fn cast_uv(scene: &Scene, view: &PinView, uv: [f32; 2]) -> Option<(ScreenPin, SurfaceKind)> {
    let mut dir = view_ray(view, uv);
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1.0e-8 {
        return None;
    }
    dir = [dir[0] / len, dir[1] / len, dir[2] / len];
    surface_hit(scene, view.eye, dir)
}

fn view_ray(view: &PinView, uv: [f32; 2]) -> [f32; 3] {
    let ndc_x = uv[0] * 2.0 - 1.0;
    let ndc_y = uv[1] * 2.0 - 1.0;
    [
        view.forward[0]
            + view.right[0] * (ndc_x * view.tan_half * view.aspect)
            + view.up[0] * (-ndc_y * view.tan_half),
        view.forward[1]
            + view.right[1] * (ndc_x * view.tan_half * view.aspect)
            + view.up[1] * (-ndc_y * view.tan_half),
        view.forward[2]
            + view.right[2] * (ndc_x * view.tan_half * view.aspect)
            + view.up[2] * (-ndc_y * view.tan_half),
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The same placement the gather uses. A miss returns nothing.
fn surface_hit(scene: &Scene, eye: [f32; 3], dir: [f32; 3]) -> Option<(ScreenPin, SurfaceKind)> {
    let mut best = f32::MAX;
    let mut found = false;
    let mut kind = SurfaceKind::Floor;
    let mut pos = eye;
    let mut normal = [0.0, 1.0, 0.0];
    for wall in &scene.walls {
        let half = [wall.half_x, wall.height * 0.5, wall.half_z];
        let center = [wall.position.x, wall.height * 0.5, wall.position.z];
        let min = [
            center[0] - half[0],
            center[1] - half[1],
            center[2] - half[2],
        ];
        let max = [
            center[0] + half[0],
            center[1] + half[1],
            center[2] + half[2],
        ];
        if let Some((t, n)) = hit_box(eye, dir, min, max, center, half) {
            if t > 0.002 && t < best {
                best = t;
                pos = [
                    eye[0] + dir[0] * t,
                    eye[1] + dir[1] * t,
                    eye[2] + dir[2] * t,
                ];
                normal = n;
                kind = SurfaceKind::Wall;
                found = true;
            }
        }
    }
    for solid in &scene.solids {
        let half = solid.size * 0.5;
        let center = [solid.position.x, solid.height * 0.5, solid.position.z];
        let hit = match solid.shape {
            Shape::Circle => hit_cylinder(eye, dir, center, half, 0.0, solid.height),
            Shape::Square => {
                let half_e = [half, solid.height * 0.5, half];
                let min = [
                    center[0] - half_e[0],
                    center[1] - half_e[1],
                    center[2] - half_e[2],
                ];
                let max = [
                    center[0] + half_e[0],
                    center[1] + half_e[1],
                    center[2] + half_e[2],
                ];
                hit_box(eye, dir, min, max, center, half_e)
            }
        };
        if let Some((t, n)) = hit {
            if t > 0.002 && t < best {
                best = t;
                pos = [
                    eye[0] + dir[0] * t,
                    eye[1] + dir[1] * t,
                    eye[2] + dir[2] * t,
                ];
                normal = n;
                kind = SurfaceKind::Solid;
                found = true;
            }
        }
    }
    if dir[1] < -1.0e-6 {
        let t = (0.0 - eye[1]) / dir[1];
        if t > 0.002 && t < best {
            let point = [
                eye[0] + dir[0] * t,
                eye[1] + dir[1] * t,
                eye[2] + dir[2] * t,
            ];
            let floor = &scene.floor;
            let dx = point[0] - floor.position.x;
            let dz = point[2] - floor.position.z;
            if dx.abs() <= floor.half_x + 0.05 && dz.abs() <= floor.half_z + 0.05 {
                pos = point;
                normal = [0.0, 1.0, 0.0];
                kind = SurfaceKind::Floor;
                found = true;
            }
        }
    }
    if !found {
        return None;
    }
    Some((
        ScreenPin {
            position: pos,
            normal,
            hidden: false,
            object: false,
        },
        kind,
    ))
}

fn hit_box(
    origin: [f32; 3],
    dir: [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
    center: [f32; 3],
    half: [f32; 3],
) -> Option<(f32, [f32; 3])> {
    let mut t_enter = 0.0_f32;
    let mut t_exit = 1.0e20_f32;
    for axis in 0..3 {
        if dir[axis].abs() < 1.0e-8 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[axis];
        let mut t1 = (min[axis] - origin[axis]) * inv;
        let mut t2 = (max[axis] - origin[axis]) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        t_enter = t_enter.max(t1);
        t_exit = t_exit.min(t2);
        if t_exit < t_enter {
            return None;
        }
    }
    if t_enter <= 0.0 {
        return None;
    }
    let point = [
        origin[0] + dir[0] * t_enter,
        origin[1] + dir[1] * t_enter,
        origin[2] + dir[2] * t_enter,
    ];
    let half = [
        half[0].max(1.0e-4),
        half[1].max(1.0e-4),
        half[2].max(1.0e-4),
    ];
    let q = [
        (point[0] - center[0]) / half[0],
        (point[1] - center[1]) / half[1],
        (point[2] - center[2]) / half[2],
    ];
    let aq = [q[0].abs(), q[1].abs(), q[2].abs()];
    let normal = if aq[0] >= aq[1] && aq[0] >= aq[2] {
        [q[0].signum(), 0.0, 0.0]
    } else if aq[1] >= aq[2] {
        [0.0, q[1].signum(), 0.0]
    } else {
        [0.0, 0.0, q[2].signum()]
    };
    let normal = if dot(normal, normal) < 0.5 {
        [0.0, 1.0, 0.0]
    } else {
        normal
    };
    Some((t_enter, normal))
}

fn hit_cylinder(
    origin: [f32; 3],
    dir: [f32; 3],
    center: [f32; 3],
    radius: f32,
    y0: f32,
    y1: f32,
) -> Option<(f32, [f32; 3])> {
    let ox = origin[0] - center[0];
    let oz = origin[2] - center[2];
    let a = dir[0] * dir[0] + dir[2] * dir[2];
    if a <= 1.0e-8 {
        return None;
    }
    let b = ox * dir[0] + oz * dir[2];
    let c = ox * ox + oz * oz - radius * radius;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    if t <= 0.0 {
        return None;
    }
    let y = origin[1] + dir[1] * t;
    if y < y0 || y > y1 {
        return None;
    }
    let point = [origin[0] + dir[0] * t, y, origin[2] + dir[2] * t];
    let side = [point[0] - center[0], 0.0, point[2] - center[2]];
    let len = (side[0] * side[0] + side[2] * side[2]).sqrt().max(1.0e-4);
    Some((t, [side[0] / len, 0.0, side[2] / len]))
}

fn radiance_at(scene: &Scene, pin: &ScreenPin) -> [f32; 3] {
    let origin = [
        pin.position[0] + pin.normal[0] * 0.03,
        pin.position[1] + pin.normal[1] * 0.03,
        pin.position[2] + pin.normal[2] * 0.03,
    ];
    let direct =
        crate::field::illuminate_facing(scene, origin[0], origin[1], origin[2], pin.normal);
    [direct, direct, direct]
}

#[cfg(test)]
mod debug_lines {
    use super::*;
    use genos_scene::{Floor, Vec3};

    #[test]
    fn control_layers_draw_one_tick_per_probe_and_dim_off_screen() {
        let scene = Scene {
            floor: Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 16.0,
                half_z: 16.0,
                color: [1.0, 1.0, 1.0],
            },
            walls: Vec::new(),
            solids: Vec::new(),
            lights: Vec::new(),
        };
        let mut camera = Camera::new(0.0, 2.0, 0.0);
        camera.pitch = -0.45;
        let view = PinView::from_camera(&camera, 16.0 / 9.0, 128, 72);
        let mut pins = ScreenPins::default();
        update_screen_pins(&mut pins, &scene, &view);
        let color = [0.25, 0.95, 1.0];
        let lines = pinned_debug_lines(&pins.layers[0], &view, 0, color);
        let ticks = pins.layers[0].iter().filter(|pin| !pin.hidden).count();
        let fans = ticks.div_ceil(8);
        assert_eq!(lines.len(), ticks * 2 + fans * 8);
        assert!(
            ticks as u32 <= PROBE_CAP0,
            "the checker drew {ticks} probes"
        );
        let dim = lines.iter().any(|vert| {
            (vert.albedo[0] - color[0] * 0.35).abs() < 1.0e-4
                && (vert.albedo[1] - color[1] * 0.35).abs() < 1.0e-4
        });
        let full = lines
            .iter()
            .any(|vert| (vert.albedo[0] - color[0]).abs() < 1.0e-4);
        assert!(dim, "a probe outside the view was not dimmed");
        assert!(full, "an in-view probe was not drawn");
    }
}
