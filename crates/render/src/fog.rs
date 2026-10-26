#![cfg_attr(
    test,
    expect(
        clippy::expect_used,
        reason = "a test aborts when a fixture fails"
    )
)]

//! Froxel grid for height fog and local volumes.
//!
//! Slice edges follow Hillaire, SIGGRAPH 2015:
//! `z(i) = z_near * (z_far / z_near) ^ (i / n)` with `z_near = 0.10 m` and
//! `z_far = 64 m`. Extinction is the height-fog closed form averaged across
//! the slice, plus local boxes and spheres at the cell centre. History is
//! not read and not blended.
//!
//! Indirect in-scattering waits for lighting phase 3
//! (`cursor/lighting-p3-capped-refresh`). [`world_cache_inscatter`] does not
//! read a lighting schedule and does not touch the GI passes.

use genos_math::{
    height_fog_density_f32, height_optical_depth_f32, homogeneous_inscatter_f32, phase_hg_f32,
    transmittance_f32,
};
use genos_scene::{look_direction, FogSettings, FogShape, FogVolume, LightingPreset};

use crate::particles::{compose_fog, FogHit, SMOKE_ALBEDO, SMOKE_DENSITY, SMOKE_RADIUS};

/// Near slice edge, metres.
pub const Z_NEAR: f32 = 0.10;
/// Far slice edge, metres. The tail past this is closed-form height fog.
pub const Z_FAR: f32 = 64.0;

/// Preset grid. Potato, day-cycle, and gi-1bounce allocate nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    /// Columns.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// Exponential slices. Zero means no grid.
    pub slices: u32,
}

/// One camera for the frustum. Distances are view-Z along `forward`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogView {
    /// World eye, metres.
    pub origin: [f32; 3],
    /// Unit view direction.
    pub forward: [f32; 3],
    /// Unit camera right.
    pub right: [f32; 3],
    /// Unit camera up.
    pub up: [f32; 3],
    /// Tangent of half the horizontal field of view.
    pub tan_half_x: f32,
    /// Tangent of half the vertical field of view.
    pub tan_half_y: f32,
}

/// One column after the energy-conserving integral.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogColumn {
    /// In-scattering along the column, rgb.
    pub inscatter: [f32; 3],
    /// Transmittance of the column, including the tail.
    pub transmittance: f32,
}

/// Directional sun. `toward` points from the cell toward the sun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectSun {
    /// Unit direction toward the sun.
    pub toward: [f32; 3],
    /// Unattenuated sun colour.
    pub color: [f32; 3],
}

/// A column tap and the picture depth used to accept or reject it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthTap {
    /// Column x.
    pub x: u32,
    /// Column y.
    pub y: u32,
    /// View-Z at that pixel, metres.
    pub depth: f32,
}

/// One slice of a column integral.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliceSample {
    /// Average extinction, per metre.
    pub sigma: f32,
    /// In-scattering source, already including phase and shadow.
    pub source: [f32; 3],
    /// Ray length of the slice, metres.
    pub distance: f32,
}

/// Extinction per cell. The history slot is reserved in the byte count and
/// is not stored here: the accept weight is zero while history is off.
#[derive(Clone, Debug, PartialEq)]
pub struct FroxelGrid {
    /// Dimensions.
    pub size: GridSize,
    /// Average extinction, row-major `x + width * (y + height * z)`.
    pub extinction: Vec<f32>,
}

impl GridSize {
    /// Cell count. Zero when the preset allocates no volume.
    #[must_use]
    pub fn cells(self) -> usize {
        let width = usize::try_from(self.width).unwrap_or(0);
        let height = usize::try_from(self.height).unwrap_or(0);
        let slices = usize::try_from(self.slices).unwrap_or(0);
        width.saturating_mul(height).saturating_mul(slices)
    }
}

impl FogView {
    /// Yaw 0 looks along -Z. `tan_half_y` is the vertical half-angle tangent.
    #[must_use]
    pub fn look(origin: [f32; 3], yaw: f32, pitch: f32, tan_half_y: f32, aspect: f32) -> Self {
        let forward = look_direction(yaw, pitch);
        let forward = [forward.x, forward.y, forward.z];
        let mut right = normalize(cross(forward, [0.0, 1.0, 0.0]));
        if length_sq(right) < 1.0e-8 {
            right = normalize(cross(forward, [1.0, 0.0, 0.0]));
        }
        let up = cross(right, forward);
        Self {
            origin,
            forward,
            right,
            up,
            tan_half_x: tan_half_y * aspect,
            tan_half_y,
        }
    }
}

/// Grid for a lighting preset. Potato, day-cycle, and one-bounce GI allocate none.
#[must_use]
pub fn grid_size(preset: LightingPreset) -> GridSize {
    match preset {
        LightingPreset::Low => GridSize {
            width: 64,
            height: 36,
            slices: 24,
        },
        LightingPreset::Medium => GridSize {
            width: 96,
            height: 54,
            slices: 32,
        },
        LightingPreset::High => GridSize {
            width: 160,
            height: 90,
            slices: 48,
        },
        LightingPreset::Potato | LightingPreset::DayCycle | LightingPreset::Gi1Bounce => GridSize {
            width: 0,
            height: 0,
            slices: 0,
        },
    }
}

/// View-Z of slice edge `index` in `0..=slices`.
#[must_use]
pub fn slice_edge(index: u32, slices: u32) -> f32 {
    if slices == 0 {
        return Z_FAR;
    }
    let t = unit(index) / unit(slices);
    Z_NEAR * (Z_FAR / Z_NEAR).powf(t)
}

/// Fill extinction. Local volumes add. They do not replace the height field.
/// An empty size returns an empty grid.
#[must_use]
pub fn inject(settings: &FogSettings, view: &FogView, size: GridSize) -> FroxelGrid {
    let count = size.cells();
    let mut extinction = vec![0.0; count];
    if count == 0 {
        return FroxelGrid { size, extinction };
    }
    for z in 0..size.slices {
        for y in 0..size.height {
            for x in 0..size.width {
                let Some(index) = cell_index(size, x, y, z) else {
                    continue;
                };
                extinction[index] = cell_extinction(settings, view, size, x, y, z);
            }
        }
    }
    FroxelGrid { size, extinction }
}

/// A particle puff as a local sphere. The puff composite is skipped while
/// the froxel is on; this is the density it injects instead.
#[must_use]
pub fn puff_volume(center: [f32; 3]) -> FogVolume {
    FogVolume {
        shape: FogShape::Sphere,
        position: center,
        extents: [SMOKE_RADIUS, SMOKE_RADIUS, SMOKE_RADIUS],
        density: SMOKE_DENSITY,
        albedo: SMOKE_ALBEDO,
        phase_g: 0.0,
        falloff: 0.0,
    }
}

/// Peak extinction outside `volume` divided by the peak inside.
///
/// A grid that misses the volume returns 1. Zero outside returns 0.
#[must_use]
pub fn local_volume_leak(grid: &FroxelGrid, volume: &FogVolume, view: &FogView) -> f32 {
    let mut peak_in = 0.0_f32;
    let mut peak_out = 0.0_f32;
    let size = grid.size;
    for z in 0..size.slices {
        for y in 0..size.height {
            for x in 0..size.width {
                let Some(index) = cell_index(size, x, y, z) else {
                    continue;
                };
                let centre = cell_centre(view, size, x, y, z);
                let sigma = grid.extinction[index];
                if volume_contains(volume, centre) {
                    peak_in = peak_in.max(sigma);
                } else {
                    peak_out = peak_out.max(sigma);
                }
            }
        }
    }
    if peak_in <= 0.0 {
        return 1.0;
    }
    peak_out / peak_in
}

/// The leak fixture: height density zero, one box on the view axis.
#[must_use]
pub fn leak_fixture() -> f32 {
    let (grid, volume, view) = leak_scene();
    local_volume_leak(&grid, &volume, &view)
}

/// The same fixture with extinction written into a cell outside the box.
#[must_use]
pub fn leak_broken() -> f32 {
    let (mut grid, volume, view) = leak_scene();
    let size = grid.size;
    for z in 0..size.slices {
        for y in 0..size.height {
            for x in 0..size.width {
                let Some(index) = cell_index(size, x, y, z) else {
                    continue;
                };
                let centre = cell_centre(&view, size, x, y, z);
                if !volume_contains(&volume, centre) {
                    grid.extinction[index] = 1.0;
                    return local_volume_leak(&grid, &volume, &view);
                }
            }
        }
    }
    1.0
}

/// World-cache in-scattering. Always zero until lighting phase 3 lands.
///
/// The branch `cursor/lighting-p3-capped-refresh` owns that read. This
/// function does not consult a lighting schedule and does not run a GI pass.
/// A cache miss contributes zero, and so does this stub.
#[must_use]
pub fn world_cache_inscatter(_gi: bool) -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

/// Piecewise exact integral. `T` starts at 1.
///
/// `T_next = T * exp(-σ Δs)` and, when `σ > 0`,
/// `L += T * (S / σ) * (1 - exp(-σ Δs))`. The tail is height fog only.
#[must_use]
pub fn integrate_samples(samples: &[SliceSample], tail_tau: f32, tail_ratio: [f32; 3]) -> FogColumn {
    let mut transmittance = 1.0_f32;
    let mut inscatter = [0.0_f32; 3];
    for sample in samples {
        let (added, next) = step_scatter(
            transmittance,
            sample.sigma,
            sample.source,
            sample.distance,
        );
        for channel in 0..3 {
            inscatter[channel] += added[channel];
        }
        transmittance = next;
    }
    let (absorb, tail_t) = tail_gain(tail_tau);
    for channel in 0..3 {
        inscatter[channel] += transmittance * tail_ratio[channel] * absorb;
    }
    FogColumn {
        inscatter,
        transmittance: transmittance * tail_t,
    }
}

/// Column integral for every view column. History is not read.
/// Shadow is 1: an unsampled cell's stand-in is this unshadowed integral.
#[must_use]
pub fn integrate_unshadowed(
    settings: &FogSettings,
    view: &FogView,
    size: GridSize,
    sun: DirectSun,
) -> Vec<FogColumn> {
    let mut columns = Vec::new();
    if size.width == 0 || size.height == 0 {
        return columns;
    }
    for y in 0..size.height {
        for x in 0..size.width {
            columns.push(integrate_column(settings, view, size, sun, x, y));
        }
    }
    columns
}

/// `background * transmittance + in-scattering`.
#[must_use]
pub fn apply_fog(background: [f32; 3], column: FogColumn) -> [f32; 3] {
    compose_fog(
        background,
        FogHit {
            optical: 0.0,
            transmittance: column.transmittance,
            inscatter: column.inscatter,
        },
    )
}

/// Mean frame-to-frame relative luminance of the fog term, camera still.
#[must_use]
pub fn still_flicker(previous: &[f32], current: &[f32]) -> f32 {
    if previous.is_empty() || previous.len() != current.len() {
        return 1.0;
    }
    let mut sum = 0.0_f32;
    for (left, right) in previous.iter().zip(current.iter()) {
        sum += (left - right).abs() / left.max(*right).max(0.02);
    }
    sum / count_f32(previous.len())
}

fn count_f32(len: usize) -> f32 {
    let mut left = len;
    let mut sum = 0.0_f32;
    while left > 0 {
        let chunk = left.min(usize::from(u16::MAX));
        let Ok(narrow) = u16::try_from(chunk) else {
            break;
        };
        sum += f32::from(narrow);
        left -= chunk;
    }
    sum.max(1.0)
}

/// Two identical unshadowed frames. History is off, so the fog term matches.
#[must_use]
pub fn still_flicker_fixture() -> f32 {
    let luma = still_luma();
    still_flicker(&luma, &luma)
}

/// The second frame's fog luminance is scaled by 1.4.
#[must_use]
pub fn still_flicker_broken() -> f32 {
    let luma = still_luma();
    let scaled: Vec<f32> = luma.iter().map(|value| value * 1.4).collect();
    still_flicker(&luma, &scaled)
}

/// Depth-aware sample of the column target.
///
/// A neighbour column whose depth falls in a different slice gets weight zero.
/// The pixel's own column remains when every neighbour is rejected.
#[must_use]
pub fn sample_depth_aware(
    columns: &[FogColumn],
    size: GridSize,
    pixel: DepthTap,
    neighbor: DepthTap,
) -> FogColumn {
    let own = column_at(columns, size, pixel.x, pixel.y);
    if slice_of(pixel.depth, size.slices) != slice_of(neighbor.depth, size.slices) {
        return own;
    }
    let other = column_at(columns, size, neighbor.x, neighbor.y);
    FogColumn {
        inscatter: scale_color(add(own.inscatter, other.inscatter), 0.5),
        transmittance: f32::midpoint(own.transmittance, other.transmittance),
    }
}

fn integrate_column(
    settings: &FogSettings,
    view: &FogView,
    size: GridSize,
    sun: DirectSun,
    x: u32,
    y: u32,
) -> FogColumn {
    let dir = ray_dir(view, size, x, y);
    let mu = dot(dir, sun.toward).clamp(-1.0, 1.0);
    let mut samples = Vec::with_capacity(usize::try_from(size.slices).unwrap_or(0));
    for z in 0..size.slices {
        let z0 = slice_edge(z, size.slices);
        let z1 = slice_edge(z + 1, size.slices);
        let p0 = point_at(view, size, x, y, z0);
        let p1 = point_at(view, size, x, y, z1);
        let delta = sub(p1, p0);
        let distance = length(delta);
        let centre = point_at(view, size, x, y, f32::midpoint(z0, z1));
        let (sigma, source) = scatter_at(&ScatterQuery {
            settings,
            dir_y: dir[1],
            mu,
            p0,
            delta,
            distance,
            centre,
            sun,
            visibility: 1.0,
        });
        samples.push(SliceSample {
            sigma,
            source,
            distance,
        });
    }
    let far = point_at(view, size, x, y, Z_FAR);
    let tail_tau = tail_optical_depth(settings, far[1], dir[1]);
    let tail_phase = phase_hg_f32(settings.phase_g, mu);
    let albedo = settings.albedo.clamp(0.0, 1.0);
    let tail_ratio = scale_color(sun.color, albedo * tail_phase);
    integrate_samples(&samples, tail_tau, tail_ratio)
}

struct ScatterQuery<'a> {
    settings: &'a FogSettings,
    dir_y: f32,
    mu: f32,
    p0: [f32; 3],
    delta: [f32; 3],
    distance: f32,
    centre: [f32; 3],
    sun: DirectSun,
    visibility: f32,
}

fn scatter_at(query: &ScatterQuery<'_>) -> (f32, [f32; 3]) {
    let settings = query.settings;
    let sigma_height = if query.distance <= 1.0e-8 {
        0.0
    } else {
        let optical = height_optical_depth_f32(
            settings.density,
            settings.thickness,
            settings.height,
            query.p0[1],
            query.delta[1],
            query.dir_y,
            query.distance,
        );
        optical / query.distance
    };
    let mut sigma = sigma_height;
    let mut scatter = sigma_height
        * settings.albedo.clamp(0.0, 1.0)
        * phase_hg_f32(settings.phase_g, query.mu);
    for volume in &settings.volumes {
        let local = volume.density.max(0.0) * falloff(volume, query.centre);
        sigma += local;
        scatter += local * volume.albedo.clamp(0.0, 1.0) * phase_hg_f32(volume.phase_g, query.mu);
    }
    let source = scale_color(query.sun.color, scatter * query.visibility);
    let _ = world_cache_inscatter(settings.gi);
    (sigma, source)
}

fn tail_optical_depth(settings: &FogSettings, y0: f32, cos_theta: f32) -> f32 {
    if settings.density <= 0.0 {
        return 0.0;
    }
    let sigma = height_fog_density_f32(settings.density, settings.thickness, y0, settings.height);
    if sigma <= 0.0 {
        return 0.0;
    }
    if cos_theta.abs() < 1.0e-4 || settings.thickness <= 0.0 {
        return f32::INFINITY;
    }
    if cos_theta > 0.0 {
        sigma * settings.thickness / cos_theta
    } else {
        f32::INFINITY
    }
}

fn step_scatter(
    transmittance: f32,
    sigma: f32,
    source: [f32; 3],
    distance: f32,
) -> ([f32; 3], f32) {
    let mut added = [0.0_f32; 3];
    for channel in 0..3 {
        added[channel] = if sigma > 1.0e-8 {
            transmittance * homogeneous_inscatter_f32(sigma, source[channel], distance)
        } else {
            transmittance * source[channel] * distance
        };
    }
    let next = transmittance * transmittance_f32(sigma * distance);
    (added, next)
}

fn tail_gain(tau: f32) -> (f32, f32) {
    if !tau.is_finite() || tau >= 1.0e6 {
        (1.0, 0.0)
    } else {
        (1.0 - transmittance_f32(tau), transmittance_f32(tau))
    }
}

fn still_luma() -> Vec<f32> {
    let settings = FogSettings {
        enabled: true,
        density: 0.08,
        thickness: 40.0,
        albedo: 1.0,
        phase_g: 0.4,
        ..FogSettings::default()
    };
    let size = GridSize {
        width: 8,
        height: 4,
        slices: 8,
    };
    let view = FogView::look([0.0, 2.0, 0.0], 0.0, 0.0, 0.5, 1.0);
    let sun = DirectSun {
        toward: [0.0, 1.0, 0.0],
        color: [1.0, 1.0, 1.0],
    };
    integrate_unshadowed(&settings, &view, size, sun)
        .into_iter()
        .map(|column| luma(column.inscatter))
        .collect()
}

pub(crate) fn luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

fn slice_of(depth: f32, slices: u32) -> u32 {
    if slices == 0 {
        return 0;
    }
    let mut found = 0;
    for index in 0..slices {
        if depth >= slice_edge(index, slices) && depth < slice_edge(index + 1, slices) {
            found = index;
        }
    }
    found
}

fn column_at(columns: &[FogColumn], size: GridSize, x: u32, y: u32) -> FogColumn {
    let width = usize::try_from(size.width).unwrap_or(0);
    let x = usize::try_from(x.min(size.width.saturating_sub(1))).unwrap_or(0);
    let y = usize::try_from(y.min(size.height.saturating_sub(1))).unwrap_or(0);
    columns
        .get(y.saturating_mul(width).saturating_add(x))
        .copied()
        .unwrap_or(FogColumn {
            inscatter: [0.0; 3],
            transmittance: 1.0,
        })
}

pub(crate) fn ray_dir(view: &FogView, size: GridSize, x: u32, y: u32) -> [f32; 3] {
    let u = (unit(x) + 0.5) / unit(size.width.max(1));
    let v = (unit(y) + 0.5) / unit(size.height.max(1));
    let nx = (u * 2.0 - 1.0) * view.tan_half_x;
    let ny = (1.0 - v * 2.0) * view.tan_half_y;
    normalize(add(
        add(view.forward, scale(view.right, nx)),
        scale(view.up, ny),
    ))
}

pub(crate) fn scale_color(color: [f32; 3], factor: f32) -> [f32; 3] {
    [color[0] * factor, color[1] * factor, color[2] * factor]
}

fn leak_scene() -> (FroxelGrid, FogVolume, FogView) {
    let volume = FogVolume {
        shape: FogShape::Box,
        position: [0.0, 0.0, -5.0],
        extents: [1.5, 1.5, 1.5],
        density: 1.0,
        albedo: 1.0,
        phase_g: 0.0,
        falloff: 0.0,
    };
    let settings = FogSettings {
        volumes: vec![volume],
        ..FogSettings::default()
    };
    let size = GridSize {
        width: 8,
        height: 8,
        slices: 12,
    };
    let view = FogView::look([0.0, 0.0, 0.0], 0.0, 0.0, 1.0, 1.0);
    let grid = inject(&settings, &view, size);
    (grid, volume, view)
}

fn cell_extinction(settings: &FogSettings, view: &FogView, size: GridSize, x: u32, y: u32, z: u32) -> f32 {
    let z0 = slice_edge(z, size.slices);
    let z1 = slice_edge(z + 1, size.slices);
    let p0 = point_at(view, size, x, y, z0);
    let p1 = point_at(view, size, x, y, z1);
    let delta = sub(p1, p0);
    let distance = length(delta);
    let sigma_height = if distance <= 1.0e-8 {
        0.0
    } else {
        let cos_theta = delta[1] / distance;
        let optical = height_optical_depth_f32(
            settings.density,
            settings.thickness,
            settings.height,
            p0[1],
            delta[1],
            cos_theta,
            distance,
        );
        optical / distance
    };
    let centre = point_at(view, size, x, y, f32::midpoint(z0, z1));
    sigma_height + local_density(&settings.volumes, centre)
}

fn local_density(volumes: &[FogVolume], point: [f32; 3]) -> f32 {
    let mut sum = 0.0_f32;
    for volume in volumes {
        sum += volume.density.max(0.0) * falloff(volume, point);
    }
    sum
}

pub(crate) fn falloff(volume: &FogVolume, point: [f32; 3]) -> f32 {
    let dist_to_face = match volume.shape {
        FogShape::Box => {
            let dx = (point[0] - volume.position[0]).abs();
            let dy = (point[1] - volume.position[1]).abs();
            let dz = (point[2] - volume.position[2]).abs();
            if dx > volume.extents[0] || dy > volume.extents[1] || dz > volume.extents[2] {
                return 0.0;
            }
            (volume.extents[0] - dx)
                .min(volume.extents[1] - dy)
                .min(volume.extents[2] - dz)
        }
        FogShape::Sphere => {
            let offset = sub(point, volume.position);
            let dist = length(offset);
            let radius = volume.extents[0];
            if dist > radius {
                return 0.0;
            }
            radius - dist
        }
    };
    if volume.falloff <= 0.0 {
        1.0
    } else {
        (dist_to_face / volume.falloff).clamp(0.0, 1.0)
    }
}

fn volume_contains(volume: &FogVolume, point: [f32; 3]) -> bool {
    match volume.shape {
        FogShape::Box => {
            let dx = (point[0] - volume.position[0]).abs();
            let dy = (point[1] - volume.position[1]).abs();
            let dz = (point[2] - volume.position[2]).abs();
            dx <= volume.extents[0] && dy <= volume.extents[1] && dz <= volume.extents[2]
        }
        FogShape::Sphere => length_sq(sub(point, volume.position)) <= volume.extents[0] * volume.extents[0],
    }
}

pub(crate) fn point_at(view: &FogView, size: GridSize, x: u32, y: u32, view_z: f32) -> [f32; 3] {
    let u = (unit(x) + 0.5) / unit(size.width.max(1));
    let v = (unit(y) + 0.5) / unit(size.height.max(1));
    let nx = (u * 2.0 - 1.0) * view.tan_half_x;
    let ny = (1.0 - v * 2.0) * view.tan_half_y;
    add(
        view.origin,
        scale(
            add(
                add(view.forward, scale(view.right, nx)),
                scale(view.up, ny),
            ),
            view_z,
        ),
    )
}

fn cell_centre(view: &FogView, size: GridSize, x: u32, y: u32, z: u32) -> [f32; 3] {
    let z0 = slice_edge(z, size.slices);
    let z1 = slice_edge(z + 1, size.slices);
    point_at(view, size, x, y, f32::midpoint(z0, z1))
}

fn cell_index(size: GridSize, x: u32, y: u32, z: u32) -> Option<usize> {
    if x >= size.width || y >= size.height || z >= size.slices {
        return None;
    }
    let width = usize::try_from(size.width).ok()?;
    let height = usize::try_from(size.height).ok()?;
    let x = usize::try_from(x).ok()?;
    let y = usize::try_from(y).ok()?;
    let z = usize::try_from(z).ok()?;
    Some((z * height + y) * width + x)
}

/// Grid axes fit in `u16`, and every `u16` is an exact `f32`.
pub(crate) fn unit(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn length_sq(a: [f32; 3]) -> f32 {
    dot(a, a)
}

pub(crate) fn length(a: [f32; 3]) -> f32 {
    length_sq(a).sqrt()
}

pub(crate) fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = length(a);
    if len <= 1.0e-8 {
        [0.0, 0.0, -1.0]
    } else {
        scale(a, 1.0 / len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_math::{height_fog_density_f32, homogeneous_inscatter_f32};

    #[test]
    fn slice_edges_match_the_exponential() {
        let n = 24;
        let first = slice_edge(0, n);
        let last = slice_edge(n, n);
        assert!((first - Z_NEAR).abs() < 1.0e-5, "{first}");
        assert!((last - Z_FAR).abs() < 1.0e-4, "{last}");
        let mid = slice_edge(n / 2, n);
        assert!(mid > first && mid < last);
    }

    #[test]
    fn height_fog_matches_the_closed_form_on_a_level_ray() {
        let settings = FogSettings {
            density: 0.08,
            thickness: 40.0,
            height: 0.0,
            ..FogSettings::default()
        };
        let size = GridSize {
            width: 4,
            height: 4,
            slices: 4,
        };
        let view = FogView::look([0.0, 10.0, 0.0], 0.0, 0.0, 0.01, 1.0);
        let grid = inject(&settings, &view, size);
        let centre = cell_index(size, 1, 1, 0).expect("index");
        let expected = height_fog_density_f32(0.08, 40.0, 10.0, 0.0);
        let got = grid.extinction[centre];
        assert!((got - expected).abs() < 1.0e-4, "{got} vs {expected}");
    }

    #[test]
    fn a_box_does_not_leak_and_a_write_outside_does() {
        let leak = leak_fixture();
        assert!(leak <= 1.0e-4, "{leak}");
        let broken = leak_broken();
        assert!(broken > 1.0e-4, "{broken}");
    }

    #[test]
    fn potato_allocates_no_cells() {
        assert_eq!(grid_size(LightingPreset::Potato).cells(), 0);
        assert_eq!(grid_size(LightingPreset::DayCycle).cells(), 0);
        assert_eq!(grid_size(LightingPreset::Gi1Bounce).cells(), 0);
        assert_eq!(grid_size(LightingPreset::Low).cells(), 64 * 36 * 24);
    }

    #[test]
    fn the_world_cache_stub_is_black() {
        assert_eq!(world_cache_inscatter(true), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn one_slice_matches_the_homogeneous_integral() {
        let source = 0.15;
        let column = integrate_samples(
            &[SliceSample {
                sigma: 0.2,
                source: [source, source, source],
                distance: 2.0,
            }],
            0.0,
            [0.0; 3],
        );
        let expected = homogeneous_inscatter_f32(0.2, source, 2.0);
        assert!((column.inscatter[0] - expected).abs() < 1.0e-5);
        let background = [0.2, 0.4, 0.6];
        let picture = apply_fog(background, column);
        assert!(
            (picture[0] - (background[0] * column.transmittance + column.inscatter[0])).abs()
                < 1.0e-6
        );
    }

    #[test]
    fn a_still_camera_does_not_flicker_and_a_scaled_frame_does() {
        let flicker = still_flicker_fixture();
        assert!(flicker <= 0.02, "{flicker}");
        let broken = still_flicker_broken();
        assert!(broken > 0.02, "{broken}");
    }

    #[test]
    fn a_depth_discontinuity_drops_the_neighbour() {
        let red = FogColumn {
            inscatter: [1.0, 0.0, 0.0],
            transmittance: 0.5,
        };
        let blue = FogColumn {
            inscatter: [0.0, 0.0, 1.0],
            transmittance: 0.5,
        };
        let size = GridSize {
            width: 2,
            height: 1,
            slices: 8,
        };
        let columns = [red, blue];
        let near = slice_edge(0, size.slices) + 0.01;
        let far = slice_edge(6, size.slices);
        let kept = sample_depth_aware(
            &columns,
            size,
            DepthTap {
                x: 0,
                y: 0,
                depth: near,
            },
            DepthTap {
                x: 1,
                y: 0,
                depth: far,
            },
        );
        assert!((kept.inscatter[0] - 1.0).abs() < 1.0e-5);
        assert!(kept.inscatter[2].abs() < 1.0e-5);
    }
}
