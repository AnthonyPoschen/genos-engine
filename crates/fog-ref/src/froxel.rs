//! CPU froxel model for stages F1 and F2 of ADR 0026.
//!
//! The same grid the GPU will run (layout and resolution per preset), computed
//! on the CPU so it can be graded against the F0 ray march before any shader
//! exists. F1 picks lights and casts the shadow rays. F2 is the volumetric
//! integration reference, in Hillaire 2015 order:
//!
//! 1. Inject extinction and the in-scattering source into the froxel grid.
//! 2. Temporal reprojection of that scattering. Disabled: the stage is the
//!    identity (ADR 0026 decision 5). Hillaire blends a reprojected sample.
//! 3. Front-to-back integration, the energy-conserving slice integral.
//!
//! Departures from Hillaire 2015, named here and in
//! `docs/systems/rendering/fog/plan.md`:
//!
//! - Extinction is the closed-form optical depth of the slice, not a density
//!   sample at the cell centre (ADR 0018 departure 6).
//! - In-scattering is single-scatter direct light. Probe indirect is stage F4.
//! - Shadows are rays against the boxes, not shadow maps (ADR 0018 departure 5).
//! - Compose stays the F1 trilinear read of the integrated column. This stage
//!   does not march the picture.
//!
//! No state survives between frames.

use crate::{hg, Ray, Scene, V3};

/// A graphics preset from ADR 0023 with its ADR 0026 fog grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// 64×36×32, 2 shadow rays.
    Low,
    /// 96×54×48, 1 shadow ray.
    Medium,
    /// 160×90×64, 2 shadow rays.
    High,
    /// 240×135×96, 2 shadow rays.
    Ultra,
}

impl Preset {
    /// All presets, cheapest first.
    pub const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Ultra];

    /// The grid of this preset.
    #[must_use]
    pub const fn grid(self) -> Grid {
        let (w, h, d, rays, taps) = match self {
            Self::Low => (64, 36, 32, 2, 4),
            Self::Medium => (96, 54, 48, 1, 4),
            Self::High => (160, 90, 64, 2, 4),
            Self::Ultra => (240, 135, 96, 2, 4),
        };
        Grid {
            w,
            h,
            d,
            shadow_rays: rays,
            reuse_taps: taps,
        }
    }

    /// Name for reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }
}

/// Froxel grid layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    /// Columns across the screen.
    pub w: u32,
    /// Rows down the screen.
    pub h: u32,
    /// Depth slices, exponentially distributed between near and far.
    pub d: u32,
    /// Shadow rays per froxel.
    pub shadow_rays: u32,
    /// Neighbours in the same slice whose shaded picks each froxel reuses.
    pub reuse_taps: u32,
}

/// A pinhole camera; slices are in view depth (along `forward`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Position.
    pub pos: V3,
    /// Unit view direction.
    pub forward: V3,
    /// Vertical field of view in radians.
    pub fov_y: f64,
    /// Width over height.
    pub aspect: f64,
    /// First slice boundary in view depth.
    pub near: f64,
    /// Last slice boundary in view depth.
    pub far: f64,
}

fn cross(a: V3, b: V3) -> V3 {
    V3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

impl Camera {
    fn basis(&self) -> (V3, V3) {
        let up = if self.forward.y.abs() > 0.99 {
            V3::new(0.0, 0.0, 1.0)
        } else {
            V3::new(0.0, 1.0, 0.0)
        };
        let right = cross(self.forward, up).norm();
        (right, cross(right, self.forward))
    }

    /// Unit ray through screen coordinates in [0, 1]² (v down).
    #[must_use]
    pub fn dir(&self, u: f64, v: f64) -> V3 {
        let (right, up) = self.basis();
        let ty = (self.fov_y * 0.5).tan();
        let x = (2.0 * u - 1.0) * ty * self.aspect;
        let y = (1.0 - 2.0 * v) * ty;
        (self.forward + right * x + up * y).norm()
    }

    /// Screen coordinates of a direction in front of the camera.
    #[must_use]
    pub fn project(&self, d: V3) -> (f64, f64) {
        let (right, up) = self.basis();
        let ty = (self.fov_y * 0.5).tan();
        let z = d.dot(self.forward);
        let x = d.dot(right) / z / (ty * self.aspect);
        let y = d.dot(up) / z / ty;
        (f64::midpoint(x, 1.0), f64::midpoint(-y, 1.0))
    }

    /// View depth of slice boundary `k` (0 is the camera).
    fn boundary(&self, k: u32, d: u32) -> f64 {
        if k == 0 {
            0.0
        } else {
            let f = f64::from(k - 1) / f64::from(d - 1);
            self.near * (self.far / self.near).powf(f)
        }
    }

    /// Continuous slice-boundary coordinate of view depth `z`.
    fn slice_coord(&self, z: f64, d: u32) -> f64 {
        if z <= self.near {
            (z / self.near).max(0.0)
        } else {
            1.0 + (z / self.near).ln() / (self.far / self.near).ln() * f64::from(d - 1)
        }
    }
}

/// Index from a grid coordinate.
fn ix(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Grid cell of a coordinate, clamped to `[0, max]`. Grids are at most a few
/// hundred cells on a side, so a count beats a lossy float cast.
fn cell(v: f64, max: u32) -> usize {
    (0..max).take_while(|&i| f64::from(i + 1) <= v).count()
}

/// Fixed per-froxel jitter in [0, 1): the same every frame for a still camera.
///
/// Blue-noise-like: a 4×4 Bayer stratum across neighbouring columns, rotated per
/// slice and per sample by the golden ratio, so the 2×2 columns the compose
/// filter blends always cover different strata of the slice.
fn jitter(col: u32, row: u32, slice: u32, sample: u32) -> f64 {
    const BAYER: [u32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    const PHI: f64 = 0.618_033_988_749_894_8;
    let stratum = f64::from(BAYER[ix((row % 4) * 4 + col % 4)]);
    let v = (stratum + 0.5) / 16.0 + f64::from(slice) * PHI + f64::from(sample) * PHI * PHI;
    v - v.floor()
}

/// Neighbour offsets for spatial reuse within a slice: the 4-neighbour cross
/// of ADR 0026, then the diagonals for a wider preset if one ever needs it.
const REUSE: [(isize, isize); 8] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// Extinction and in-scattering injected into one froxel grid.
///
/// This is the volume Hillaire integrates. It has no history slot: fog history
/// stays off, so a previous frame is not stored here.
#[derive(Clone, Debug, PartialEq)]
pub struct Injected {
    grid: Grid,
    camera: Camera,
    /// In-scattering source per metre, after spatial reuse of the light picks.
    /// Phase, albedo, and shadow are already in this value.
    pub scattering: Vec<f64>,
    /// Closed-form optical depth of each froxel. Mean extinction per metre is
    /// [`Injected::mean_extinction`].
    pub optical_depth: Vec<f64>,
    /// Froxel length along the view ray, in metres.
    pub length: Vec<f64>,
    /// Shadow rays cast while injecting this volume.
    pub shadow_rays: u64,
}

impl Injected {
    /// Mean extinction per metre, `optical_depth / length`.
    ///
    /// `None` when `index` is outside the grid. Zero when the froxel has no length.
    #[must_use]
    pub fn mean_extinction(&self, index: usize) -> Option<f64> {
        let length = self.length.get(index)?;
        let tau = self.optical_depth.get(index)?;
        Some(if *length > 0.0 { tau / length } else { 0.0 })
    }
}

/// Output of the temporal stage.
///
/// `history_accepts` and `history_rejects` are the probe for fog history. Both
/// stay 0 while reprojection is disabled.
#[derive(Clone, Debug, PartialEq)]
pub struct Temporal {
    /// Volume handed to the front-to-back integral.
    pub volume: Injected,
    /// Froxels that blended a reprojected sample. Always 0 in this reference.
    pub history_accepts: u64,
    /// Froxels whose history sample was tested and rejected. Always 0: the
    /// previous volume is not sampled.
    pub history_rejects: u64,
}

/// One integrated fog frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    grid: Grid,
    camera: Camera,
    /// Accumulated in-scatter at each slice boundary, `(d + 1)` per column.
    scatter: Vec<f64>,
    /// Transmittance at each slice boundary.
    trans: Vec<f64>,
    /// Shadow rays cast this frame.
    pub shadow_rays: u64,
}

impl Scene {
    /// Direct in-scatter per unit scattering at `p`, from light picks: resampled
    /// importance sampling over the lights by unshadowed contribution, then one
    /// shadow ray to the picked light.
    fn froxel_direct(&self, p: V3, view: V3, u: f64, rays: &mut u64) -> f64 {
        let legs: Vec<_> = self
            .lights
            .iter()
            .filter_map(|l| self.leg(p, l))
            .map(|leg| {
                let w = hg(self.g, leg.dir.dot(view))
                    * leg.radiance
                    * self.medium.transmittance(p, leg.dir, leg.medium);
                (leg, w)
            })
            .collect();
        let total: f64 = legs.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return 0.0;
        }
        let mut pick = u * total;
        let last = legs.len() - 1;
        for (i, (leg, w)) in legs.iter().enumerate() {
            if pick < *w || i == last {
                *rays += 1;
                return if self.visible(p, leg.dir, leg.occlude) {
                    total
                } else {
                    0.0
                };
            }
            pick -= w;
        }
        0.0
    }

    /// Injects extinction and direct in-scattering into the froxel grid.
    ///
    /// Extinction is the closed-form optical depth of the slice. Scattering is
    /// the F1 light pick: stratified shadow rays, then the 4-neighbour reuse
    /// inside the slice. Nothing is read from a previous frame.
    #[must_use]
    pub fn inject_froxels(&self, grid: Grid, camera: Camera) -> Injected {
        let (gw, gh, gd) = (ix(grid.w), ix(grid.h), ix(grid.d));
        let at = |col: usize, row: usize, slice: usize| (slice * gh + row) * gw + col;
        let mut length = vec![0.0; gw * gh * gd];
        let mut optical_depth = vec![0.0; gw * gh * gd];
        let mut lit = vec![0.0; gw * gh * gd];
        let mut rays = 0_u64;
        let albedo = self.medium.albedo();
        let rays_per = f64::from(grid.shadow_rays);
        for row in 0..grid.h {
            for col in 0..grid.w {
                let dir = camera.dir(
                    (f64::from(col) + 0.5) / f64::from(grid.w),
                    (f64::from(row) + 0.5) / f64::from(grid.h),
                );
                let cos = dir.dot(camera.forward);
                for slice in 0..grid.d {
                    let i = at(ix(col), ix(row), ix(slice));
                    let t0 = camera.boundary(slice, grid.d) / cos;
                    let froxel_len = (camera.boundary(slice + 1, grid.d) / cos - t0).max(0.0);
                    let o0 = camera.pos + dir * t0;
                    length[i] = froxel_len;
                    optical_depth[i] = self.medium.optical_depth(o0, dir, froxel_len);
                    // Stratified jittered points along the froxel, one shadow ray each.
                    let mut sum = 0.0;
                    for r in 0..grid.shadow_rays {
                        let f = (f64::from(r) + jitter(col, row, slice, 2 * r)) / rays_per;
                        let p = o0 + dir * (froxel_len * f);
                        sum += albedo
                            * self.medium.sigma_t(p)
                            * self.froxel_direct(
                                p,
                                dir,
                                jitter(col, row, slice, 2 * r + 1),
                                &mut rays,
                            );
                    }
                    lit[i] = sum / rays_per;
                }
            }
        }
        // Spatial reuse stays inside the slice. It is not a temporal blend.
        let mut scattering = vec![0.0; gw * gh * gd];
        for slice in 0..gd {
            for row in 0..gh {
                for col in 0..gw {
                    let mut acc = lit[at(col, row, slice)];
                    let mut n = 1.0;
                    for (dx, dy) in REUSE[..ix(grid.reuse_taps)].iter().copied() {
                        let (nx, ny) = (col.wrapping_add_signed(dx), row.wrapping_add_signed(dy));
                        if nx < gw && ny < gh {
                            acc += lit[at(nx, ny, slice)];
                            n += 1.0;
                        }
                    }
                    scattering[at(col, row, slice)] = acc / n;
                }
            }
        }
        Injected {
            grid,
            camera,
            scattering,
            optical_depth,
            length,
            shadow_rays: rays,
        }
    }

    /// Builds one fog frame: inject, skip temporal reprojection, integrate.
    #[must_use]
    pub fn froxel_frame(&self, grid: Grid, camera: Camera) -> Frame {
        let injected = self.inject_froxels(grid, camera);
        let temporal = reproject(injected, None);
        integrate_front_to_back(&temporal.volume)
    }
}

/// Temporal reprojection of the scattering volume.
///
/// Hillaire 2015 projects each froxel into the previous view and blends that
/// sample into the current scattering. ADR 0026 decision 5 leaves fog history
/// off, so this returns `current` and does not read `previous`. Accepts and
/// rejects stay 0, which is the probe that the blend did not run.
#[must_use]
pub fn reproject(current: Injected, previous: Option<&Injected>) -> Temporal {
    let _ = previous;
    Temporal {
        volume: current,
        history_accepts: 0,
        history_rejects: 0,
    }
}

/// Front-to-back integration of an injected volume.
///
/// Per slice, with transmittance `T` starting at 1 (Hillaire 2015):
/// `L += T * (S / σ) * (1 - exp(-σ Δs))` and `T *= exp(-σ Δs)`.
/// `σ Δs` is the stored optical depth, so a height-fog slice keeps the
/// closed form. The `σ → 0` limit is `L += T * S * Δs`.
#[must_use]
pub fn integrate_front_to_back(volume: &Injected) -> Frame {
    let g = volume.grid;
    let (gw, gh, gd) = (ix(g.w), ix(g.h), ix(g.d));
    let at = |col: usize, row: usize, slice: usize| (slice * gh + row) * gw + col;
    let stride = gd + 1;
    let mut scatter = vec![0.0; gw * gh * stride];
    let mut trans = vec![1.0; gw * gh * stride];
    for row in 0..gh {
        for col in 0..gw {
            let base = (row * gw + col) * stride;
            let (mut s_acc, mut t_acc) = (0.0, 1.0);
            for slice in 0..gd {
                let i = at(col, row, slice);
                let tau = volume.optical_depth[i];
                let source = volume.scattering[i];
                let froxel_len = volume.length[i];
                let piece = if tau > 1e-9 {
                    source * froxel_len * (1.0 - (-tau).exp()) / tau
                } else {
                    source * froxel_len
                };
                s_acc += t_acc * piece;
                t_acc *= (-tau).exp();
                scatter[base + slice + 1] = s_acc;
                trans[base + slice + 1] = t_acc;
            }
        }
    }
    Frame {
        grid: g,
        camera: volume.camera,
        scatter,
        trans,
        shadow_rays: volume.shadow_rays,
    }
}

impl Frame {
    /// Composed radiance for a camera ray from the frame's camera, ending on a
    /// surface of radiance `background` at `ray.tmax`: trilinear read of the
    /// integrated grid at the pixel's depth, `background * T + in_scatter`.
    #[must_use]
    pub fn compose(&self, ray: Ray, background: f64) -> f64 {
        let g = self.grid;
        let (u, v) = self.camera.project(ray.d);
        let fx = (u * f64::from(g.w) - 0.5).clamp(0.0, f64::from(g.w - 1));
        let fy = (v * f64::from(g.h) - 0.5).clamp(0.0, f64::from(g.h - 1));
        let fz = self
            .camera
            .slice_coord(ray.tmax * ray.d.dot(self.camera.forward), g.d)
            .clamp(0.0, f64::from(g.d));
        let stride = ix(g.d) + 1;
        let read = |buf: &[f64]| {
            let mut acc = 0.0;
            let (x0, y0, z0) = (fx.floor(), fy.floor(), fz.floor());
            for (xi, wx) in [(x0, 1.0 - (fx - x0)), (x0 + 1.0, fx - x0)] {
                for (yi, wy) in [(y0, 1.0 - (fy - y0)), (y0 + 1.0, fy - y0)] {
                    for (zi, wz) in [(z0, 1.0 - (fz - z0)), (z0 + 1.0, fz - z0)] {
                        let w = wx * wy * wz;
                        if w == 0.0 {
                            continue;
                        }
                        let cx = cell(xi, g.w - 1);
                        let cy = cell(yi, g.h - 1);
                        let cz = cell(zi, g.d);
                        acc += w * buf[(cy * ix(g.w) + cx) * stride + cz];
                    }
                }
            }
            acc
        };
        background * read(&self.trans) + read(&self.scatter)
    }
}

#[cfg(test)]
mod tests {
    use super::{integrate_front_to_back, reproject, Camera, Grid, Injected};
    use crate::{Light, Medium, Scene, V3};

    fn camera() -> Camera {
        Camera {
            pos: V3::new(0.0, 1.0, 0.0),
            forward: V3::new(1.0, 0.0, 0.0),
            fov_y: 60_f64.to_radians(),
            aspect: 1.0,
            near: 0.1,
            far: 8.0,
        }
    }

    fn one_cell(sigma: f64, source: f64, len: f64) -> Injected {
        Injected {
            grid: Grid {
                w: 1,
                h: 1,
                d: 1,
                shadow_rays: 0,
                reuse_taps: 0,
            },
            camera: camera(),
            scattering: vec![source],
            optical_depth: vec![sigma * len],
            length: vec![len],
            shadow_rays: 0,
        }
    }

    fn sun_scene() -> Scene {
        Scene {
            medium: Medium::Homogeneous {
                sigma_t: 0.2,
                albedo: 0.8,
                top: 5.0,
            },
            g: 0.0,
            lights: vec![Light::Sun {
                to_light: V3::new(0.0, 1.0, 0.0),
                irradiance: 3.0,
            }],
            occluders: vec![],
            background: 0.5,
        }
    }

    #[test]
    fn one_slice_matches_the_energy_conserving_integral() {
        let frame = integrate_front_to_back(&one_cell(0.2, 0.05, 4.0));
        let tau = 0.8_f64;
        let want = 0.05 / 0.2 * (1.0 - (-tau).exp());
        assert!((frame.scatter[1] - want).abs() < 1e-12);
        assert!((frame.trans[1] - (-tau).exp()).abs() < 1e-12);
        assert!(frame.scatter[0].abs() < 1e-15);
        assert!((frame.trans[0] - 1.0).abs() < 1e-15);
    }

    #[test]
    fn zero_extinction_slice_is_the_linear_limit() {
        let frame = integrate_front_to_back(&one_cell(0.0, 0.05, 4.0));
        assert!((frame.scatter[1] - 0.2).abs() < 1e-12);
        assert!((frame.trans[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn injection_extinction_matches_sigma_and_history_is_ignored() {
        let scene = sun_scene();
        let grid = Grid {
            w: 4,
            h: 4,
            d: 8,
            shadow_rays: 2,
            reuse_taps: 4,
        };
        let cam = camera();
        let injected = scene.inject_froxels(grid, cam);
        let (gw, gh, col, row) = (4_usize, 4_usize, 2_usize, 2_usize);
        for slice in 0..8_usize {
            let index = (slice * gh + row) * gw + col;
            let mean = injected.mean_extinction(index).unwrap_or(-1.0);
            assert!(
                (mean - 0.2).abs() < 1e-9,
                "slice {slice} mean extinction {mean}"
            );
            assert!(injected.scattering[index] > 0.0);
        }
        let staged = integrate_front_to_back(&reproject(injected.clone(), None).volume);
        assert_eq!(staged, scene.froxel_frame(grid, cam));

        let mut poisoned = injected.clone();
        poisoned.scattering.fill(9.0);
        poisoned.optical_depth.fill(4.0);
        let temporal = reproject(injected.clone(), Some(&poisoned));
        assert_eq!(temporal.history_accepts, 0);
        assert_eq!(temporal.history_rejects, 0);
        assert_eq!(temporal.volume, injected);
    }
}
