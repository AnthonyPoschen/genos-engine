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

use genos_math::height_optical_depth_f32;
use genos_scene::{look_direction, FogSettings, FogShape, FogVolume, LightingPreset};

use crate::particles::{SMOKE_ALBEDO, SMOKE_DENSITY, SMOKE_RADIUS};

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

fn falloff(volume: &FogVolume, point: [f32; 3]) -> f32 {
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

fn point_at(view: &FogView, size: GridSize, x: u32, y: u32, view_z: f32) -> [f32; 3] {
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
fn unit(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length_sq(a: [f32; 3]) -> f32 {
    dot(a, a)
}

fn length(a: [f32; 3]) -> f32 {
    length_sq(a).sqrt()
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
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
    use genos_math::height_fog_density_f32;

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
}
