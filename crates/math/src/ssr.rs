//! Hierarchical-Z screen-space hit finding (ADR 0029).
//!
//! The walk is Uludag, GPU Pro 5, §4.4.3: descend into a cell whose depth
//! range the ray meets, and step to the boundary of a cell it misses.
//! The printed listing tests the minimum only. This march tests the minimum
//! and the maximum, which Listing 4.4 stores and which the chapter says the
//! demo traces. Depth is linear view-space metres. The camera is the origin,
//! looking down −Z, Y up, pixel y down.
//!
//! The march does not shade, fade, or read the surface light cache.

use crate::optics::ReflectionPreset;

/// World thickness of a depth sample, in metres along the ray (ADR 0019).
pub const THICKNESS_M: f64 = 0.25;

/// Metres the ray travels before the first cell test, so it leaves the shading point.
pub const START_BIAS_M: f64 = 0.01;

/// View-depth floor when the ray is parallel to the screen. `|D_z|` is then 0.
const THICKNESS_FLOOR_M: f64 = 1.0e-3;

/// Hierarchy iterations for one preset. Ultra uses [`ReflectionPreset::High`].
#[must_use]
pub fn step_cap(preset: ReflectionPreset) -> u32 {
    match preset {
        ReflectionPreset::Potato => 0,
        ReflectionPreset::Low => 16,
        ReflectionPreset::Medium => 32,
        ReflectionPreset::High => 48,
    }
}

/// `I − 2 dot(N, I) N`, with `I` pointing at the surface. Same formula as `optics::mirror_dir`.
#[must_use]
pub fn reflection_dir(incident: [f64; 3], normal: [f64; 3]) -> [f64; 3] {
    let Some(incident) = unit(incident) else {
        return [0.0, 0.0, 0.0];
    };
    let Some(normal) = unit(normal) else {
        return [0.0, 0.0, 0.0];
    };
    let along = dot(normal, incident);
    sub(incident, scale(normal, 2.0 * along))
}

/// Pinhole at the origin. `fovy` is the vertical field in radians.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub width: u32,
    pub height: u32,
    pub fovy: f64,
    pub near: f64,
}

/// A projected view-space point. `depth` is metres in front of the camera.
#[derive(Clone, Copy, Debug)]
pub struct Projected {
    pub px: f64,
    pub py: f64,
    pub depth: f64,
}

impl View {
    /// Project a view-space point. `None` when it is not in front of `near`.
    #[must_use]
    pub fn project(&self, point: [f64; 3]) -> Option<Projected> {
        if self.width == 0 || self.height == 0 || point[2] >= -self.near {
            return None;
        }
        let focal = 1.0 / (self.fovy * 0.5).tan();
        let aspect = f64::from(self.width) / f64::from(self.height);
        let ndc_x = (focal / aspect) * point[0] / -point[2];
        let ndc_y = focal * point[1] / point[2];
        Some(Projected {
            px: f64::midpoint(ndc_x, 1.0) * f64::from(self.width),
            py: f64::midpoint(ndc_y, 1.0) * f64::from(self.height),
            depth: -point[2],
        })
    }

    /// View direction through the centre of a pixel. Y is up, the camera looks down −Z.
    #[must_use]
    pub fn pixel_dir(&self, x: u32, y: u32) -> Option<[f64; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let ndc_x = (f64::from(x) + 0.5) / f64::from(self.width) * 2.0 - 1.0;
        let ndc_y = (f64::from(y) + 0.5) / f64::from(self.height) * 2.0 - 1.0;
        let focal = 1.0 / (self.fovy * 0.5).tan();
        let aspect = f64::from(self.width) / f64::from(self.height);
        let vx = ndc_x * aspect / focal;
        let vy = -ndc_y / focal;
        unit([vx, vy, -1.0])
    }
}

/// One depth sample. `surface == 0` is empty. Depth is positive metres, or non-finite when empty.
#[derive(Clone, Debug)]
pub struct DepthImage {
    width: u32,
    height: u32,
    depth: Vec<f64>,
    surface: Vec<u32>,
}

impl DepthImage {
    /// An empty buffer. Every sample is a miss.
    #[must_use]
    pub fn empty(width: u32, height: u32) -> Self {
        let n = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
            .unwrap_or(0);
        Self {
            width,
            height,
            depth: vec![f64::INFINITY; n],
            surface: vec![0; n],
        }
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Write one sample. `false` when `(x, y)` is outside the buffer.
    pub fn set(&mut self, x: u32, y: u32, depth: f64, surface: u32) -> bool {
        let Some(index) = self.index(x, y) else {
            return false;
        };
        if let (Some(z), Some(id)) = (self.depth.get_mut(index), self.surface.get_mut(index)) {
            *z = depth;
            *id = surface;
            return true;
        }
        false
    }

    #[must_use]
    pub fn sample(&self, x: u32, y: u32) -> Option<(f64, u32)> {
        let index = self.index(x, y)?;
        Some((
            self.depth.get(index).copied().unwrap_or(f64::INFINITY),
            self.surface.get(index).copied().unwrap_or(0),
        ))
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let row = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.width).ok()?)?;
        row.checked_add(usize::try_from(x).ok()?)
    }
}

/// Min and max depth at one pyramid level.
#[derive(Clone, Debug)]
struct Level {
    width: u32,
    height: u32,
    min_z: Vec<f64>,
    max_z: Vec<f64>,
}

impl Level {
    fn get(&self, x: u32, y: u32) -> Option<(f64, f64)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.width).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?;
        Some((
            self.min_z.get(index).copied().unwrap_or(f64::INFINITY),
            self.max_z.get(index).copied().unwrap_or(f64::NEG_INFINITY),
        ))
    }
}

/// The depth pyramid the march walks.
#[derive(Clone, Debug)]
pub struct HierarchicalZ {
    width: u32,
    height: u32,
    levels: Vec<Level>,
}

impl HierarchicalZ {
    /// Build the pyramid. Level 0 is the buffer. Each coarser level is the 2×2 min and max.
    #[must_use]
    pub fn build(image: &DepthImage) -> Self {
        let mut levels = Vec::new();
        let mut width = image.width;
        let mut height = image.height;
        if width == 0 || height == 0 {
            return Self {
                width,
                height,
                levels,
            };
        }
        levels.push(level_from_image(image));
        let mut guard = 0u32;
        while (width > 1 || height > 1) && guard < 16 {
            let prev = levels.last();
            let Some(prev) = prev else {
                break;
            };
            let next_w = width.div_ceil(2);
            let next_h = height.div_ceil(2);
            levels.push(coarsen(prev, next_w, next_h));
            width = next_w;
            height = next_h;
            guard += 1;
        }
        Self {
            width: image.width,
            height: image.height,
            levels,
        }
    }

    /// Walk `query` through this pyramid. `image` supplies the surface id at level 0.
    #[must_use]
    pub fn march(&self, image: &DepthImage, view: &View, query: &Query) -> Trace {
        if self.levels.is_empty()
            || image.width != self.width
            || image.height != self.height
            || view.width != self.width
            || view.height != self.height
        {
            return miss(MissKind::BadImage, 0);
        }
        let Some(dir) = unit(query.direction) else {
            return miss(MissKind::NoScreenMotion, 0);
        };
        let Some(ray) = screen_ray(view, query.origin, dir) else {
            return miss(MissKind::BehindCamera, 0);
        };
        if ray.dpx.abs() < 1e-8 && ray.dpy.abs() < 1e-8 {
            return miss(MissKind::NoScreenMotion, 0);
        }
        let max_level = u32::try_from(self.levels.len().saturating_sub(1)).unwrap_or(0);
        let thickness = (THICKNESS_M * dir[2].abs()).max(THICKNESS_FLOOR_M);
        let mut t = 0.0;
        let mut level = 0u32;
        let mut steps = 0u32;
        while steps < query.max_steps {
            let px = ray.px0 + ray.dpx * t;
            let py = ray.py0 + ray.dpy * t;
            if px < 0.0 || py < 0.0 || px >= f64::from(self.width) || py >= f64::from(self.height) {
                return miss(MissKind::LeftScreen, steps);
            }
            if ray.inv0 + ray.dinv * t <= 1.0e-12 {
                return miss(MissKind::BehindCamera, steps);
            }
            let Some(t_exit) = cell_exit(&ray, t, level) else {
                return miss(MissKind::NoScreenMotion, steps);
            };
            let Some((z_lo, z_hi)) = depth_span(&ray, t, t_exit) else {
                return miss(MissKind::BehindCamera, steps);
            };
            let (cell_min, cell_max) = self.range(level, px, py);
            let overlaps = cell_overlaps(
                query.coarse,
                level,
                z_lo,
                z_hi,
                cell_min,
                cell_max,
                thickness,
            );
            if overlaps && level == 0 {
                let (Some(x), Some(y)) = (trunc_u32(px), trunc_u32(py)) else {
                    return miss(MissKind::LeftScreen, steps);
                };
                let surface = image.sample(x, y).map_or(0, |sample| sample.1);
                if surface != 0 && surface != query.skip_surface {
                    return Trace::Hit(Hit {
                        x,
                        y,
                        depth: cell_min,
                        surface,
                        steps: steps.saturating_add(1),
                    });
                }
                t = t_exit;
                steps = steps.saturating_add(1);
                continue;
            }
            if overlaps {
                level = level.saturating_sub(1);
                steps = steps.saturating_add(1);
                continue;
            }
            t = t_exit;
            steps = steps.saturating_add(1);
            if level < max_level {
                level = level.saturating_add(1);
            }
        }
        miss(MissKind::StepCap, steps)
    }

    fn range(&self, level: u32, px: f64, py: f64) -> (f64, f64) {
        let empty = (f64::INFINITY, f64::NEG_INFINITY);
        let Some(mip) = self.levels.get(usize::try_from(level).unwrap_or(0)) else {
            return empty;
        };
        let cell = cell_size(level);
        let Some(ix) = trunc_u32(px / cell) else {
            return empty;
        };
        let Some(iy) = trunc_u32(py / cell) else {
            return empty;
        };
        mip.get(ix, iy).unwrap_or(empty)
    }
}

fn cell_overlaps(
    coarse: CoarseTest,
    level: u32,
    z_lo: f64,
    z_hi: f64,
    cell_min: f64,
    cell_max: f64,
    thickness: f64,
) -> bool {
    if cell_max < cell_min {
        return false;
    }
    // Listing 4.5 descends whenever the ray has reached the minimum, at every coarser cell.
    if level > 0 && coarse == CoarseTest::MinPlane {
        return z_hi >= cell_min;
    }
    z_hi >= cell_min && z_lo <= cell_max + thickness
}

/// Which coarse test the walk uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoarseTest {
    /// Overlap against `[min, max + thickness]`. This is the march ADR 0029 runs.
    MinMax,
    /// Overlap against `[min, +∞)`. Uludag's printed Listing 4.5.
    MinPlane,
}

/// One march.
#[derive(Clone, Copy, Debug)]
pub struct Query {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
    pub skip_surface: u32,
    pub max_steps: u32,
    pub coarse: CoarseTest,
}

/// A finest-level hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub x: u32,
    pub y: u32,
    pub depth: f64,
    pub surface: u32,
    pub steps: u32,
}

/// Why the walk stopped without a hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissKind {
    LeftScreen,
    BehindCamera,
    NoScreenMotion,
    StepCap,
    BadImage,
}

/// The march result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trace {
    Hit(Hit),
    Miss { kind: MissKind, steps: u32 },
}

fn miss(kind: MissKind, steps: u32) -> Trace {
    Trace::Miss { kind, steps }
}

struct ScreenRay {
    px0: f64,
    py0: f64,
    inv0: f64,
    dpx: f64,
    dpy: f64,
    dinv: f64,
}

fn screen_ray(view: &View, origin: [f64; 3], dir: [f64; 3]) -> Option<ScreenRay> {
    let p0 = add(origin, scale(dir, START_BIAS_M));
    let p1 = second_point(view, origin, dir, p0)?;
    let a = view.project(p0)?;
    let b = view.project(p1)?;
    if a.depth <= 0.0 || b.depth <= 0.0 {
        return None;
    }
    Some(ScreenRay {
        px0: a.px,
        py0: a.py,
        inv0: 1.0 / a.depth,
        dpx: b.px - a.px,
        dpy: b.py - a.py,
        dinv: 1.0 / b.depth - 1.0 / a.depth,
    })
}

fn second_point(view: &View, origin: [f64; 3], dir: [f64; 3], p0: [f64; 3]) -> Option<[f64; 3]> {
    if p0[2] >= -view.near {
        return None;
    }
    let p1 = add(origin, scale(dir, START_BIAS_M + 1.0));
    if p1[2] < -view.near {
        return Some(p1);
    }
    if dir[2].abs() < 1.0e-12 {
        return None;
    }
    let t_front = (-view.near * 2.0 - origin[2]) / dir[2];
    if t_front <= START_BIAS_M + 1.0e-4 {
        return None;
    }
    Some(add(
        origin,
        scale(dir, START_BIAS_M + (t_front - START_BIAS_M) * 0.5),
    ))
}

fn cell_exit(ray: &ScreenRay, t: f64, level: u32) -> Option<f64> {
    let cell = cell_size(level);
    let tx = axis_exit(ray.px0, ray.dpx, t, cell);
    let ty = axis_exit(ray.py0, ray.dpy, t, cell);
    let boundary = tx.min(ty);
    if !boundary.is_finite() {
        return None;
    }
    let speed = ray.dpx.abs().max(ray.dpy.abs()).max(1.0e-12);
    let eps = 1.0e-4 / speed;
    let next = if boundary > t {
        boundary + eps
    } else {
        t + eps
    };
    Some(next)
}

fn axis_exit(origin: f64, delta: f64, t: f64, cell: f64) -> f64 {
    if delta.abs() < 1.0e-15 || cell <= 0.0 {
        return f64::INFINITY;
    }
    let pos = origin + delta * t;
    let index = (pos / cell).floor();
    let bound = if delta > 0.0 {
        let edge = (index + 1.0) * cell;
        if pos >= edge - 1.0e-8 {
            edge + cell
        } else {
            edge
        }
    } else {
        let edge = index * cell;
        if pos <= edge + 1.0e-8 {
            edge - cell
        } else {
            edge
        }
    };
    (bound - origin) / delta
}

fn depth_span(ray: &ScreenRay, t0: f64, t1: f64) -> Option<(f64, f64)> {
    let i0 = ray.inv0 + ray.dinv * t0;
    if i0 <= 1.0e-12 {
        return None;
    }
    let mut far = ray.inv0 + ray.dinv * t1;
    if far <= 1.0e-12 {
        if ray.dinv.abs() < 1.0e-18 {
            return None;
        }
        let clipped = (1.0e-8 - ray.inv0) / ray.dinv;
        if clipped <= t0 {
            return None;
        }
        far = 1.0e-8;
    }
    let z0 = 1.0 / i0;
    let z1 = 1.0 / far;
    Some((z0.min(z1), z0.max(z1)))
}

fn cell_size(level: u32) -> f64 {
    f64::from(1u32 << level.min(30))
}

fn level_from_image(image: &DepthImage) -> Level {
    let n = image.depth.len();
    let mut min_z = Vec::with_capacity(n);
    let mut max_z = Vec::with_capacity(n);
    for z in &image.depth {
        if z.is_finite() && *z > 0.0 {
            min_z.push(*z);
            max_z.push(*z);
        } else {
            min_z.push(f64::INFINITY);
            max_z.push(f64::NEG_INFINITY);
        }
    }
    Level {
        width: image.width,
        height: image.height,
        min_z,
        max_z,
    }
}

fn coarsen(prev: &Level, width: u32, height: u32) -> Level {
    let n = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .unwrap_or(0);
    let mut min_z = vec![f64::INFINITY; n];
    let mut max_z = vec![f64::NEG_INFINITY; n];
    for y in 0..height {
        for x in 0..width {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    let Some((a, b)) = prev.get(
                        x.saturating_mul(2).saturating_add(dx),
                        y.saturating_mul(2).saturating_add(dy),
                    ) else {
                        continue;
                    };
                    if b >= a {
                        lo = lo.min(a);
                        hi = hi.max(b);
                    }
                }
            }
            let Some(index) = usize::try_from(y)
                .ok()
                .and_then(|row| usize::try_from(width).ok().and_then(|w| row.checked_mul(w)))
                .and_then(|row| row.checked_add(usize::try_from(x).unwrap_or(0)))
            else {
                continue;
            };
            if let Some(slot) = min_z.get_mut(index) {
                *slot = lo;
            }
            if let Some(slot) = max_z.get_mut(index) {
                *slot = hi;
            }
        }
    }
    Level {
        width,
        height,
        min_z,
        max_z,
    }
}

/// A bounded plane. `normal` need not be unit. Bounds are inclusive.
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub surface: u32,
}

/// A sphere.
#[derive(Clone, Copy, Debug)]
pub struct Sphere {
    pub center: [f64; 3],
    pub radius: f64,
    pub surface: u32,
}

/// One analytic surface.
#[derive(Clone, Copy, Debug)]
pub enum Primitive {
    Plane(Plane),
    Sphere(Sphere),
}

/// A 3D hit against [`Primitive`].
#[derive(Clone, Copy, Debug)]
pub struct AnalyticHit {
    pub point: [f64; 3],
    pub depth: f64,
    pub surface: u32,
    pub travel: f64,
}

/// Closest hit with `travel > t_min`. The camera is the origin, so depth is `−z`.
#[must_use]
pub fn trace_analytic(
    origin: [f64; 3],
    direction: [f64; 3],
    primitives: &[Primitive],
    t_min: f64,
) -> Option<AnalyticHit> {
    let dir = unit(direction)?;
    let mut best: Option<AnalyticHit> = None;
    for primitive in primitives {
        let Some(hit) = intersect(origin, dir, primitive, t_min) else {
            continue;
        };
        if best.as_ref().is_none_or(|kept| hit.travel < kept.travel) {
            best = Some(hit);
        }
    }
    best
}

fn intersect(
    origin: [f64; 3],
    dir: [f64; 3],
    primitive: &Primitive,
    t_min: f64,
) -> Option<AnalyticHit> {
    match primitive {
        Primitive::Plane(plane) => intersect_plane(origin, dir, plane, t_min),
        Primitive::Sphere(sphere) => intersect_sphere(origin, dir, sphere, t_min),
    }
}

fn intersect_plane(
    origin: [f64; 3],
    dir: [f64; 3],
    plane: &Plane,
    t_min: f64,
) -> Option<AnalyticHit> {
    let normal = unit(plane.normal)?;
    let denom = dot(dir, normal);
    if denom.abs() < 1.0e-8 {
        return None;
    }
    let travel = dot(sub(plane.point, origin), normal) / denom;
    if travel <= t_min {
        return None;
    }
    let point = add(origin, scale(dir, travel));
    if !inside(point, plane.min, plane.max) {
        return None;
    }
    if point[2] >= 0.0 {
        return None;
    }
    Some(AnalyticHit {
        point,
        depth: -point[2],
        surface: plane.surface,
        travel,
    })
}

fn intersect_sphere(
    origin: [f64; 3],
    dir: [f64; 3],
    sphere: &Sphere,
    t_min: f64,
) -> Option<AnalyticHit> {
    if sphere.radius <= 0.0 {
        return None;
    }
    let offset = sub(origin, sphere.center);
    let b = dot(offset, dir);
    let c = dot(offset, offset) - sphere.radius * sphere.radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let near = -b - root;
    let far = -b + root;
    let travel = if near > t_min {
        near
    } else if far > t_min {
        far
    } else {
        return None;
    };
    let point = add(origin, scale(dir, travel));
    if point[2] >= 0.0 {
        return None;
    }
    Some(AnalyticHit {
        point,
        depth: -point[2],
        surface: sphere.surface,
        travel,
    })
}

fn inside(point: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    point
        .iter()
        .zip(min.iter().zip(max.iter()))
        .all(|(p, (lo, hi))| *p >= lo - 1.0e-4 && *p <= hi + 1.0e-4)
}

/// Rasterise `primitives` into a depth buffer from the origin.
#[must_use]
pub fn rasterize(view: &View, primitives: &[Primitive]) -> DepthImage {
    let mut image = DepthImage::empty(view.width, view.height);
    for y in 0..view.height {
        for x in 0..view.width {
            let Some(dir) = view.pixel_dir(x, y) else {
                continue;
            };
            let Some(hit) = trace_analytic([0.0, 0.0, 0.0], dir, primitives, view.near) else {
                continue;
            };
            let _ = image.set(x, y, hit.depth, hit.surface);
        }
    }
    image
}

/// One reference scene scored against the analytic ray.
#[derive(Clone, Debug)]
pub struct SceneGrade {
    pub name: &'static str,
    pub depth_rel: f64,
    pub steps: u32,
    pub min_plane_steps: u32,
    pub analytic_depth: Option<f64>,
    pub march_depth: Option<f64>,
    pub pixel_delta: Option<u32>,
}

impl SceneGrade {
    /// `ssr_depth_rel` ≤ 0.02, the ADR 0019 gate.
    #[must_use]
    pub fn within_gate(&self) -> bool {
        self.depth_rel <= 0.02
    }
}

/// Grade the built-in scenes. `max_steps` is the hierarchy budget for the min-max march.
#[must_use]
pub fn grade_reference(max_steps: u32) -> Vec<SceneGrade> {
    reference_scenes()
        .iter()
        .map(|scene| grade_scene(scene, max_steps))
        .collect()
}

/// March one built-in scene. `None` when `name` is not one of those scenes.
#[must_use]
pub fn trace_reference(name: &str, max_steps: u32) -> Option<Trace> {
    let scene = reference_scenes()
        .into_iter()
        .find(|scene| scene.name == name)?;
    let direction = scene
        .direction
        .unwrap_or_else(|| reflection_dir(scene.receiver, scene.normal));
    let image = rasterize(&scene.view, &scene.primitives);
    let pyramid = HierarchicalZ::build(&image);
    Some(pyramid.march(
        &image,
        &scene.view,
        &Query {
            origin: scene.receiver,
            direction,
            skip_surface: scene.skip,
            max_steps,
            coarse: CoarseTest::MinMax,
        },
    ))
}

struct Scene {
    name: &'static str,
    view: View,
    primitives: Vec<Primitive>,
    receiver: [f64; 3],
    normal: [f64; 3],
    direction: Option<[f64; 3]>,
    skip: u32,
}

fn reference_view() -> View {
    View {
        width: 128,
        height: 128,
        fovy: 60.0_f64.to_radians(),
        near: 0.05,
    }
}

fn reference_scenes() -> Vec<Scene> {
    let view = reference_view();
    vec![
        Scene {
            name: "floor_to_wall",
            view,
            primitives: vec![
                floor_primitive(),
                wall_primitive(-4.0, -2.0, 2.0, -1.5, 2.5, 2),
            ],
            receiver: [0.2, -1.0, -2.0],
            normal: [0.0, 1.0, 0.0],
            direction: None,
            skip: 1,
        },
        Scene {
            name: "floor_to_sphere",
            view: View {
                width: 256,
                height: 256,
                ..view
            },
            primitives: vec![
                floor_primitive(),
                Primitive::Sphere(Sphere {
                    center: [0.0, -0.35, -3.4],
                    radius: 0.55,
                    surface: 3,
                }),
            ],
            receiver: [0.0, -1.0, -2.2],
            normal: [0.0, 1.0, 0.0],
            direction: None,
            skip: 1,
        },
        Scene {
            name: "near_contact",
            view,
            primitives: vec![
                floor_primitive(),
                wall_primitive(-2.7, -1.5, 1.5, -1.2, 1.5, 2),
            ],
            receiver: [0.0, -1.0, -2.5],
            normal: [0.0, 1.0, 0.0],
            direction: None,
            skip: 1,
        },
        Scene {
            name: "behind_post",
            view,
            primitives: vec![
                floor_primitive(),
                Primitive::Plane(Plane {
                    point: [0.0, 0.0, -2.0],
                    normal: [0.0, 0.0, 1.0],
                    min: [-0.3, -0.6, -2.05],
                    max: [0.3, -0.1, -1.95],
                    surface: 5,
                }),
                wall_primitive(-5.0, -2.0, 2.0, -1.5, 2.5, 2),
            ],
            receiver: [0.0, -1.0, -2.4],
            normal: [0.0, 1.0, 0.0],
            direction: None,
            skip: 1,
        },
        Scene {
            name: "near_occludes_far",
            view,
            primitives: vec![
                floor_primitive(),
                wall_primitive(-3.2, -1.5, 1.5, -1.5, 2.0, 2),
                wall_primitive(-5.0, -1.5, 1.5, -1.5, 2.0, 4),
            ],
            receiver: [0.15, -1.0, -2.0],
            normal: [0.0, 1.0, 0.0],
            direction: None,
            skip: 1,
        },
        Scene {
            name: "leaves_screen",
            view,
            primitives: vec![floor_primitive()],
            receiver: [0.0, -0.2, -2.0],
            normal: [0.0, 1.0, 0.0],
            direction: Some([0.0, 1.0, -0.05]),
            skip: 1,
        },
        Scene {
            name: "beside_wall",
            view,
            primitives: vec![
                floor_primitive(),
                wall_primitive(-3.5, -2.0, -0.5, -1.5, 2.0, 2),
            ],
            receiver: [0.9, -1.0, -2.0],
            normal: [0.0, 1.0, 0.0],
            direction: None,
            skip: 1,
        },
    ]
}

fn floor_primitive() -> Primitive {
    Primitive::Plane(Plane {
        point: [0.0, -1.0, 0.0],
        normal: [0.0, 1.0, 0.0],
        min: [-8.0, -1.05, -12.0],
        max: [8.0, -0.95, -0.3],
        surface: 1,
    })
}

fn wall_primitive(z: f64, x0: f64, x1: f64, y0: f64, y1: f64, surface: u32) -> Primitive {
    Primitive::Plane(Plane {
        point: [0.0, 0.0, z],
        normal: [0.0, 0.0, 1.0],
        min: [x0, y0, z - 0.05],
        max: [x1, y1, z + 0.05],
        surface,
    })
}

fn grade_scene(scene: &Scene, max_steps: u32) -> SceneGrade {
    let direction = scene
        .direction
        .unwrap_or_else(|| reflection_dir(scene.receiver, scene.normal));
    let image = rasterize(&scene.view, &scene.primitives);
    let pyramid = HierarchicalZ::build(&image);
    let query = |coarse| Query {
        origin: scene.receiver,
        direction,
        skip_surface: scene.skip,
        max_steps,
        coarse,
    };
    let marched = pyramid.march(&image, &scene.view, &query(CoarseTest::MinMax));
    let min_plane = pyramid.march(&image, &scene.view, &query(CoarseTest::MinPlane));
    let analytic = trace_analytic(scene.receiver, direction, &scene.primitives, 1.0e-3)
        .and_then(|hit| visible_hit(&scene.view, hit));
    score(scene.name, analytic, marched, min_plane)
}

fn visible_hit(view: &View, hit: AnalyticHit) -> Option<Visible> {
    let projected = view.project(hit.point)?;
    if projected.px < 0.0
        || projected.py < 0.0
        || projected.px >= f64::from(view.width)
        || projected.py >= f64::from(view.height)
    {
        return None;
    }
    Some(Visible {
        depth: hit.depth,
        surface: hit.surface,
        px: projected.px,
        py: projected.py,
    })
}

struct Visible {
    depth: f64,
    surface: u32,
    px: f64,
    py: f64,
}

fn score(
    name: &'static str,
    analytic: Option<Visible>,
    marched: Trace,
    min_plane: Trace,
) -> SceneGrade {
    let min_plane_steps = match min_plane {
        Trace::Hit(hit) => hit.steps,
        Trace::Miss { steps, .. } => steps,
    };
    let (steps, march_depth, pixel_delta, depth_rel, analytic_depth) = match (analytic, marched) {
        (None, Trace::Miss { steps, .. }) => (steps, None, None, 0.0, None),
        (Some(want), Trace::Hit(hit)) => {
            let delta = pixel_delta(want.px, want.py, hit.x, hit.y);
            let rel = (hit.depth - want.depth).abs() / want.depth.max(1.0e-3);
            let rel = if delta > 1 || hit.surface != want.surface {
                1.0
            } else {
                rel
            };
            (
                hit.steps,
                Some(hit.depth),
                Some(delta),
                rel,
                Some(want.depth),
            )
        }
        (Some(want), Trace::Miss { steps, .. }) => (steps, None, None, 1.0, Some(want.depth)),
        (None, Trace::Hit(hit)) => (hit.steps, Some(hit.depth), None, 1.0, None),
    };
    SceneGrade {
        name,
        depth_rel,
        steps,
        min_plane_steps,
        analytic_depth,
        march_depth,
        pixel_delta,
    }
}

fn pixel_delta(px: f64, py: f64, x: u32, y: u32) -> u32 {
    let Some(ax) = trunc_u32(px) else {
        return u32::MAX;
    };
    let Some(ay) = trunc_u32(py) else {
        return u32::MAX;
    };
    ax.abs_diff(x).max(ay.abs_diff(y))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let len = dot(v, v).sqrt();
    if len > 1.0e-12 {
        Some(scale(v, 1.0 / len))
    } else {
        None
    }
}

/// Truncate a non-negative finite `value` toward zero. `None` when it is not in `0 .. 2^32`.
#[must_use]
pub fn trunc_u32(value: f64) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value >= 4_294_967_296.0 {
        return None;
    }
    let bits = value.to_bits();
    let biased = (bits >> 52) & 0x7ff;
    if biased == 0 {
        return Some(0);
    }
    let exp = i32::try_from(biased).ok()? - 1023;
    if exp < 0 {
        return Some(0);
    }
    let frac = (bits & 0x000f_ffff_ffff_ffff) | (1u64 << 52);
    let shift = u32::try_from(52i32.checked_sub(exp)?).ok()?;
    u32::try_from(frac >> shift).ok()
}

#[cfg(test)]
mod tests {
    use super::{trunc_u32, DepthImage, HierarchicalZ};

    #[test]
    fn trunc_matches_known_values() {
        let samples = [0.0, 0.1, 1.0, 1.9, 2.0, 3.9, 127.9, 255.0, 1024.0, 4095.5];
        let expect = [0, 0, 1, 1, 2, 3, 127, 255, 1024, 4095];
        for (value, want) in samples.into_iter().zip(expect) {
            assert_eq!(trunc_u32(value), Some(want), "{value}");
        }
        assert_eq!(trunc_u32(-0.1), None);
        assert_eq!(trunc_u32(f64::NAN), None);
    }

    #[test]
    fn pyramid_min_and_max_cover_the_block() {
        let mut image = DepthImage::empty(2, 2);
        let _ = image.set(0, 0, 2.0, 1);
        let _ = image.set(1, 0, 4.0, 1);
        let _ = image.set(0, 1, 6.0, 1);
        let _ = image.set(1, 1, 8.0, 1);
        let pyramid = HierarchicalZ::build(&image);
        let (lo, hi) = pyramid.range(1, 0.5, 0.5);
        assert!((lo - 2.0).abs() < 1.0e-12, "{lo}");
        assert!((hi - 8.0).abs() < 1.0e-12, "{hi}");
    }

    #[test]
    fn an_empty_texel_does_not_move_the_max() {
        let mut image = DepthImage::empty(2, 2);
        let _ = image.set(0, 0, 2.0, 1);
        let pyramid = HierarchicalZ::build(&image);
        let (lo, hi) = pyramid.range(1, 0.5, 0.5);
        assert!(
            (lo - 2.0).abs() < 1.0e-12 && (hi - 2.0).abs() < 1.0e-12,
            "{lo} {hi}"
        );
    }
}
