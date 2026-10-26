//! Shadowed shafts, and the yaw evidence while history stays off.
//!
//! Sun and point lamps take `V` from [`crate::trace::sees`]. The world-cache
//! term stays the zero stub in `fog::world_cache_inscatter`.

use genos_math::{height_optical_depth_f32, phase_hg_f32};
use genos_scene::{
    Camera, Floor, FogSettings, FogShape, FogVolume, LightingPreset, LightingSettings, Scene, Vec3,
    Wall,
};

use crate::fog::{
    add, aligned_volume_point, apply_fog, dot, falloff, grid_size, height_closed_column,
    height_segment_column,
    integrate_samples,
    integrate_unshadowed, length, length_sq, luma, near_columns, normalize, point_at, ray_dir,
    scale, scale_color, slice_edge, sub, unit, world_cache_inscatter, DirectSun, FogColumn,
    FogView, GridSize, SliceSample, Z_FAR, Z_NEAR,
};
use crate::trace::sees;

/// Matches `crates/render/src/trace.rs` `LAMP_UNIT`.
const LAMP_UNIT: f32 = 72.0;
/// Yaw samples in the smear and pop fixtures. Frame 0 is −45°, the last is +45°.
pub const YAW_FRAMES: u32 = 320;
const PICTURE_WIDTH: f64 = 2560.0;

/// Planted-bug bits. The integrate shader reads the same values.
pub const FOG_SKIP_EXTINCTION: u32 = 1;
/// Visibility is 1. The shadow ray is not traced.
pub const FOG_SKIP_SHADOW: u32 = 2;
/// A local volume's bound is ignored, so its density is written in every cell.
/// Sphere and box both use this bit.
pub const FOG_IGNORE_SPHERE: u32 = 4;
/// Slice edges are linear in the index instead of exponential.
pub const FOG_LINEAR_SLICE: u32 = 8;
/// Henyey-Greenstein `g` is negated.
pub const FOG_FLIP_G: u32 = 16;

/// One GPU froxel dispatch. `scene` None forces [`FOG_SKIP_SHADOW`].
#[derive(Clone, Debug)]
pub struct FogDispatch {
    /// Height fog and local volumes.
    pub settings: FogSettings,
    /// Frustum the grid is built in.
    pub view: FogView,
    /// Column and slice counts.
    pub size: GridSize,
    /// Directional sun.
    pub sun: DirectSun,
    /// Point lamps. The pass keeps the first eight.
    pub lamps: Vec<FogPointLamp>,
    /// Packed into the resident scene when shadows are on.
    pub scene: Option<Scene>,
    /// [`FOG_SKIP_EXTINCTION`] and the other planted-bug bits.
    pub flags: u32,
    /// Near slices only, with no tail. Zero keeps every slice and the tail.
    pub stop_slices: u32,
    /// Copy the extinction image back. The leak rows need it.
    pub read_extinction: bool,
}

/// Columns, and the extinction image when it was requested.
#[derive(Clone, Debug, PartialEq)]
pub struct FogGpu {
    /// One entry per column, row-major.
    pub columns: Vec<FogColumn>,
    /// Extinction, or empty when the dispatch did not read it back.
    pub extinction: Vec<f32>,
    /// Device timestamp span of inject plus integrate, milliseconds.
    pub pass_ms: f64,
    /// Extinction image, column buffer, and the history reserve.
    pub vram_bytes: u64,
}

/// A lit point lamp. Colour is multiplied by `LAMP_UNIT / distance²`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogPointLamp {
    /// World position, metres.
    pub position: [f32; 3],
    /// Colour.
    pub color: [f32; 3],
}

/// Linear RGB of one fog picture.
#[derive(Clone, Debug, PartialEq)]
pub struct FogImage {
    /// Columns.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// Row-major rgb.
    pub linear: Vec<f32>,
}

/// Froxel picture and the closed-form height ray over the same view.
#[must_use]
pub fn media_images() -> (FogImage, FogImage) {
    let (settings, view, size, sun) = media_scene();
    let froxel = integrate_unshadowed(&settings, &view, size, sun);
    let mut closed = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            closed.push(height_closed_column(&settings, &view, size, sun, x, y));
        }
    }
    (columns_image(size, &froxel), columns_image(size, &closed))
}

/// Nearest quarter of the slices, froxel against the closed-form segment.
#[must_use]
pub fn near_images() -> (FogImage, FogImage) {
    let (settings, view, size, sun) = media_scene();
    let froxel = near_columns(&settings, &view, size, sun);
    let stop = (size.slices / 4).max(1);
    let z1 = slice_edge(stop, size.slices);
    let mut closed = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            closed.push(height_segment_column(
                &settings,
                &view,
                size,
                sun,
                x,
                y,
                [Z_NEAR, z1],
            ));
        }
    }
    (columns_image(size, &froxel), columns_image(size, &closed))
}

fn media_scene() -> (FogSettings, FogView, GridSize, DirectSun) {
    let settings = FogSettings {
        enabled: true,
        density: 0.08,
        height: 0.0,
        thickness: 40.0,
        albedo: 1.0,
        phase_g: 0.4,
        ..FogSettings::default()
    };
    let size = GridSize {
        width: 16,
        height: 8,
        slices: 12,
    };
    let view = FogView::look([0.0, 1.6, 2.0], 0.15, -0.05, 0.55, 1.4);
    let sun = DirectSun {
        toward: normalize([0.3, 0.85, 0.2]),
        color: [1.0, 0.95, 0.85],
    };
    (settings, view, size, sun)
}

/// Shadowed sun and one point lamp, or the same integral with `V` forced to 1.
#[must_use]
pub fn shaft_image(traced_shadows: bool) -> FogImage {
    let (settings, view, size, sun, lamp, scene) = shaft_scene();
    let columns = integrate_lit(&settings, &view, size, sun, &[lamp], &scene, traced_shadows);
    columns_image(size, &columns)
}

/// Centroid lag, in 1440p picture pixels, of a static fog blob across a 90° yaw.
/// The worse of every frame and a stride of 16. History is off.
#[must_use]
pub fn smear_fixture() -> f64 {
    smear_steps(false)
}

/// The rendered centroid is held on the first frame, which is what a history
/// blend does to a moving shaft.
#[must_use]
pub fn smear_broken() -> f64 {
    smear_steps(true)
}

/// Max frame-to-frame relative luminance after frame 8.
/// `energy` is one fog-term sum per yaw frame, frame 0 first.
/// An empty series returns 1.
#[must_use]
pub fn yaw_pop_rel(energy: &[f32]) -> f32 {
    let mut max_rel = 0.0_f32;
    let Some((first, rest)) = energy.split_first() else {
        return 1.0;
    };
    let mut previous = *first;
    for (offset, current) in rest.iter().copied().enumerate() {
        let frame = u32::try_from(offset).unwrap_or(0).saturating_add(1);
        let rel = (current - previous).abs() / previous.max(current).max(0.02);
        if frame > 8 {
            max_rel = max_rel.max(rel);
        }
        previous = current;
    }
    max_rel
}

/// Max frame-to-frame relative luminance of the CPU fog term after frame 8.
#[must_use]
pub fn yaw_pop_fixture() -> f32 {
    let energy: Vec<f32> = (0..YAW_FRAMES).map(yaw_energy).collect();
    yaw_pop_rel(&energy)
}

/// The CPU fog term scaled by `1 + 0.5 sin(yaw * 48)`.
/// That scale follows the yaw, so a smooth shaft still pops.
#[must_use]
pub fn yaw_pop_broken() -> f32 {
    let energy: Vec<f32> = (0..YAW_FRAMES)
        .map(|frame| yaw_energy(frame) * yaw_jitter(frame))
        .collect();
    yaw_pop_rel(&energy)
}

fn smear_steps(hold: bool) -> f64 {
    let every = smear_at(1, hold);
    let stride = smear_at(16, hold);
    every.max(stride)
}

fn smear_at(step: u32, hold: bool) -> f64 {
    let step = step.max(1);
    let mut max_lag = 0.0_f64;
    let mut prev_rendered = picture_x(centroid(0));
    let mut prev_geometric = picture_x(project_blob(0));
    let frozen = prev_rendered;
    let mut frame = step;
    while frame < YAW_FRAMES {
        let rendered = if hold {
            frozen
        } else {
            picture_x(centroid(frame))
        };
        let geometric = picture_x(project_blob(frame));
        let lag = ((rendered - prev_rendered) - (geometric - prev_geometric)).abs();
        max_lag = max_lag.max(lag);
        prev_rendered = rendered;
        prev_geometric = geometric;
        frame = frame.saturating_add(step);
    }
    max_lag
}

fn yaw_energy(frame: u32) -> f32 {
    blob_luma(frame).into_iter().sum()
}

fn yaw_jitter(frame: u32) -> f32 {
    1.0 + 0.5 * (blob_yaw(frame) * 48.0).sin()
}

fn blob_yaw(frame: u32) -> f32 {
    let t = unit(frame) / unit(YAW_FRAMES - 1);
    std::f32::consts::FRAC_PI_2 * (t - 0.5)
}

/// Centroid lag of caller-supplied per-frame luma. `step` 1 walks every yaw frame.
/// The geometric centre does not use the fog integral.
#[must_use]
pub fn smear_tracking(step: u32, mut luma_at: impl FnMut(u32) -> Vec<f32>) -> f64 {
    let step = step.max(1);
    let mut max_lag = 0.0_f64;
    let mut prev_rendered = picture_x(centroid_from(0, &luma_at(0)));
    let mut prev_geometric = picture_x(project_blob(0));
    let mut frame = step;
    while frame < YAW_FRAMES {
        let rendered = picture_x(centroid_from(frame, &luma_at(frame)));
        let geometric = picture_x(project_blob(frame));
        let lag = ((rendered - prev_rendered) - (geometric - prev_geometric)).abs();
        max_lag = max_lag.max(lag);
        prev_rendered = rendered;
        prev_geometric = geometric;
        frame = frame.saturating_add(step);
    }
    max_lag
}

fn centroid(frame: u32) -> f32 {
    centroid_from(frame, &blob_luma(frame))
}

fn centroid_from(frame: u32, luma: &[f32]) -> f32 {
    let size = blob_size();
    let view = blob_view(frame);
    let mut weight = 0.0_f32;
    let mut sum = 0.0_f32;
    for y in 0..size.height {
        for x in 0..size.width {
            let index = usize::try_from(y).unwrap_or(0) * usize::try_from(size.width).unwrap_or(1)
                + usize::try_from(x).unwrap_or(0);
            let sample = luma.get(index).copied().unwrap_or(0.0) / ray_scale(&view, size, x, y);
            sum += (unit(x) + 0.5) * sample;
            weight += sample;
        }
    }
    if weight <= 1.0e-6 {
        unit(size.width) * 0.5
    } else {
        sum / weight
    }
}

/// Metres travelled per view-Z metre. Dividing it out keeps an off-axis ray's
/// extra path length from moving the shaft centroid.
fn ray_scale(view: &FogView, size: GridSize, x: u32, y: u32) -> f32 {
    let u = (unit(x) + 0.5) / unit(size.width.max(1));
    let v = (unit(y) + 0.5) / unit(size.height.max(1));
    let nx = (u * 2.0 - 1.0) * view.tan_half_x;
    let ny = (1.0 - v * 2.0) * view.tan_half_y;
    (1.0 + nx * nx + ny * ny).sqrt()
}

fn project_blob(frame: u32) -> f32 {
    let view = blob_view(frame);
    let size = blob_size();
    let rel = sub(blob_centre(), view.origin);
    let depth = dot(rel, view.forward);
    if depth <= 0.05 {
        return unit(size.width) * 0.5;
    }
    let x = dot(rel, view.right) / depth;
    let u = x / view.tan_half_x * 0.5 + 0.5;
    u.clamp(0.0, 1.0) * unit(size.width)
}

fn picture_x(column_x: f32) -> f64 {
    f64::from(column_x) * PICTURE_WIDTH / f64::from(unit(blob_size().width))
}

fn blob_luma(frame: u32) -> Vec<f32> {
    let settings = FogSettings {
        enabled: true,
        density: 0.0,
        volumes: vec![blob_volume()],
        ..FogSettings::default()
    };
    let size = blob_size();
    let view = blob_view(frame);
    let sun = DirectSun {
        toward: [0.0, 1.0, 0.0],
        color: [1.0, 1.0, 1.0],
    };
    let scene = empty_scene();
    let width = usize::try_from(size.width).unwrap_or(0);
    let mut samples = vec![0.0_f32; size.cells().saturating_div(usize::try_from(size.slices.max(1)).unwrap_or(1))];
    for y in 0..size.height {
        for x in 0..size.width {
            if !ray_hits_blob(&view, size, x, y) {
                continue;
            }
            let column = lit_column(
                &ColumnJob {
                    settings: &settings,
                    view: &view,
                    size,
                    sun,
                    lamps: &[],
                    scene: &scene,
                    traced_shadows: false,
                },
                x,
                y,
            );
            let index = usize::try_from(y).unwrap_or(0) * width + usize::try_from(x).unwrap_or(0);
            if let Some(slot) = samples.get_mut(index) {
                *slot = luma(column.inscatter);
            }
        }
    }
    samples
}

fn ray_hits_blob(view: &FogView, size: GridSize, x: u32, y: u32) -> bool {
    let dir = ray_dir(view, size, x, y);
    let rel = sub(blob_centre(), view.origin);
    let along = dot(rel, dir).max(0.0);
    let closest = add(view.origin, scale(dir, along));
    length(sub(closest, blob_centre())) <= blob_volume().extents[0] + 1.5
}

fn blob_volume() -> FogVolume {
    FogVolume {
        shape: FogShape::Sphere,
        position: blob_centre(),
        extents: [0.9, 0.9, 0.9],
        density: 0.04,
        albedo: 1.0,
        phase_g: 0.0,
        falloff: 0.9,
    }
}

fn blob_centre() -> [f32; 3] {
    [0.0, 1.4, -10.0]
}

fn blob_size() -> GridSize {
    grid_size(LightingPreset::High)
}

fn blob_view(frame: u32) -> FogView {
    FogView::look([0.0, 1.4, 0.0], blob_yaw(frame), 0.0, 1.5, 2.0)
}

fn shaft_scene() -> (
    FogSettings,
    FogView,
    GridSize,
    DirectSun,
    FogPointLamp,
    Scene,
) {
    let settings = FogSettings {
        enabled: true,
        density: 0.2,
        height: 0.0,
        thickness: 30.0,
        albedo: 1.0,
        phase_g: 0.2,
        ..FogSettings::default()
    };
    let size = GridSize {
        width: 16,
        height: 8,
        slices: 10,
    };
    let view = FogView::look([0.0, 1.6, 3.0], 0.0, 0.35, 0.55, 1.0);
    let sun = DirectSun {
        toward: normalize([1.0, 0.15, 0.0]),
        color: [1.0, 0.95, 0.8],
    };
    let lamp = FogPointLamp {
        position: [2.5, 2.2, -4.0],
        color: [1.0, 0.7, 0.45],
    };
    let scene = Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 30.0,
            half_z: 30.0,
            color: [0.5, 0.5, 0.5],
        },
        walls: vec![Wall {
            position: Vec3::new(0.0, 0.0, -4.0),
            half_x: 0.2,
            half_z: 4.0,
            height: 6.0,
            base: 0.0,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: genos_scene::LightingSettings::default(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        fog: FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    };
    (settings, view, size, sun, lamp, scene)
}

fn empty_scene() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 1.0,
            half_z: 1.0,
            color: [0.0; 3],
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
        fog: FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    }
}

/// Shadowed column integral. Sun and lamp visibility use `trace::sees`.
#[must_use]
pub fn integrate_traced(
    settings: &FogSettings,
    view: &FogView,
    size: GridSize,
    sun: DirectSun,
    lamps: &[FogPointLamp],
    scene: &Scene,
) -> Vec<FogColumn> {
    integrate_lit(settings, view, size, sun, lamps, scene, true)
}

fn integrate_lit(
    settings: &FogSettings,
    view: &FogView,
    size: GridSize,
    sun: DirectSun,
    lamps: &[FogPointLamp],
    scene: &Scene,
    traced_shadows: bool,
) -> Vec<FogColumn> {
    let mut columns = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            columns.push(lit_column(
                &ColumnJob {
                    settings,
                    view,
                    size,
                    sun,
                    lamps,
                    scene,
                    traced_shadows,
                },
                x,
                y,
            ));
        }
    }
    columns
}

#[derive(Clone, Copy)]
struct ColumnJob<'a> {
    settings: &'a FogSettings,
    view: &'a FogView,
    size: GridSize,
    sun: DirectSun,
    lamps: &'a [FogPointLamp],
    scene: &'a Scene,
    traced_shadows: bool,
}

fn lit_column(job: &ColumnJob<'_>, x: u32, y: u32) -> FogColumn {
    let ColumnJob {
        settings,
        view,
        size,
        sun,
        lamps,
        scene,
        traced_shadows,
    } = *job;
    let dir = ray_dir(view, size, x, y);
    let mut samples = Vec::new();
    for z in 0..size.slices {
        let z0 = slice_edge(z, size.slices);
        let z1 = slice_edge(z + 1, size.slices);
        let p0 = point_at(view, size, x, y, z0);
        let p1 = point_at(view, size, x, y, z1);
        let delta = sub(p1, p0);
        let distance = length(delta);
        let centre = point_at(view, size, x, y, f32::midpoint(z0, z1));
        let sigma = extinction_at(settings, p0, delta, dir[1], distance, centre);
        let sigma_height = sigma
            - settings.volumes.iter().fold(0.0, |sum, volume| {
                sum + volume.density.max(0.0) * falloff(volume, centre)
            });
        let mut source = light_source(&LightQuery {
            settings,
            view,
            sigma_height: sigma_height.max(0.0),
            p0,
            centre,
            p1,
            dir,
            sun,
            lamps,
            scene,
            traced_shadows,
        });
        let _ = world_cache_inscatter(settings.gi);
        if distance <= 1.0e-8 {
            source = [0.0; 3];
        }
        samples.push(SliceSample {
            sigma,
            source,
            distance,
        });
    }
    let far = point_at(view, size, x, y, Z_FAR);
    let mu = dot(dir, sun.toward).clamp(-1.0, 1.0);
    let tail_tau = if settings.density <= 0.0 {
        0.0
    } else {
        tail_tau(settings, far[1], dir[1])
    };
    let visible = if traced_shadows {
        visibility(scene, far, add(far, scale(sun.toward, 48.0)))
    } else {
        1.0
    };
    let tail_ratio = scale_color(
        sun.color,
        settings.albedo.clamp(0.0, 1.0) * phase_hg_f32(settings.phase_g, mu) * visible,
    );
    integrate_samples(&samples, tail_tau, tail_ratio)
}

struct LightQuery<'a> {
    settings: &'a FogSettings,
    view: &'a FogView,
    sigma_height: f32,
    p0: [f32; 3],
    centre: [f32; 3],
    p1: [f32; 3],
    dir: [f32; 3],
    sun: DirectSun,
    lamps: &'a [FogPointLamp],
    scene: &'a Scene,
    traced_shadows: bool,
}

fn light_source(query: &LightQuery<'_>) -> [f32; 3] {
    let mut source = [0.0_f32; 3];
    let sun_mu = dot(query.dir, query.sun.toward).clamp(-1.0, 1.0);
    let sun_v = if query.traced_shadows {
        visibility(
            query.scene,
            query.centre,
            add(query.centre, scale(query.sun.toward, 48.0)),
        )
    } else {
        1.0
    };
    let sun_coeff = phase_coeff(
        query.view,
        query.settings,
        query.sigma_height,
        query.p0,
        query.centre,
        query.p1,
        sun_mu,
    );
    for (channel, slot) in source.iter_mut().enumerate() {
        *slot += sun_coeff * query.sun.color[channel] * sun_v;
    }
    for lamp in query.lamps.iter().take(8) {
        let offset = sub(lamp.position, query.centre);
        let dist2 = length_sq(offset).max(0.25);
        let toward = normalize(offset);
        let mu = dot(query.dir, toward).clamp(-1.0, 1.0);
        let lamp_v = if query.traced_shadows {
            visibility(query.scene, query.centre, lamp.position)
        } else {
            1.0
        };
        let falloff_l = LAMP_UNIT / dist2;
        let coeff = phase_coeff(
            query.view,
            query.settings,
            query.sigma_height,
            query.p0,
            query.centre,
            query.p1,
            mu,
        ) * falloff_l
            * lamp_v;
        for (channel, slot) in source.iter_mut().enumerate() {
            *slot += coeff * lamp.color[channel];
        }
    }
    source
}

fn phase_coeff(
    view: &FogView,
    settings: &FogSettings,
    sigma_height: f32,
    p0: [f32; 3],
    _centre: [f32; 3],
    p1: [f32; 3],
    mu: f32,
) -> f32 {
    let mut coeff = sigma_height * settings.albedo.clamp(0.0, 1.0) * phase_hg_f32(settings.phase_g, mu);
    for volume in &settings.volumes {
        let mut local = 0.0_f32;
        for tap in 0..8 {
            let t = (unit(tap) + 0.5) / 8.0;
            let point = add(p0, scale(sub(p1, p0), t));
            local += falloff(volume, aligned_volume_point(view, volume, point));
        }
        local = volume.density.max(0.0) * local / 8.0;
        coeff += local * volume.albedo.clamp(0.0, 1.0) * phase_hg_f32(volume.phase_g, mu);
    }
    coeff
}

fn extinction_at(
    settings: &FogSettings,
    p0: [f32; 3],
    delta: [f32; 3],
    dir_y: f32,
    distance: f32,
    centre: [f32; 3],
) -> f32 {
    let sigma_height = if distance <= 1.0e-8 || settings.density <= 0.0 {
        0.0
    } else {
        height_optical_depth_f32(
            settings.density,
            settings.thickness,
            settings.height,
            p0[1],
            delta[1],
            dir_y,
            distance,
        ) / distance
    };
    let mut sigma = sigma_height;
    for volume in &settings.volumes {
        sigma += volume.density.max(0.0) * falloff(volume, centre);
    }
    sigma
}

fn tail_tau(settings: &FogSettings, y0: f32, cos_theta: f32) -> f32 {
    if cos_theta.abs() < 1.0e-4 || settings.thickness <= 0.0 {
        return f32::INFINITY;
    }
    if cos_theta > 0.0 {
        let sigma = genos_math::height_fog_density_f32(
            settings.density,
            settings.thickness,
            y0,
            settings.height,
        );
        if sigma <= 0.0 {
            0.0
        } else {
            sigma * settings.thickness / cos_theta
        }
    } else {
        f32::INFINITY
    }
}

fn visibility(scene: &Scene, from: [f32; 3], to: [f32; 3]) -> f32 {
    if sees(scene, from, to) {
        1.0
    } else {
        0.0
    }
}

fn columns_image(size: GridSize, columns: &[FogColumn]) -> FogImage {
    let mut linear = Vec::new();
    for column in columns {
        let rgb = apply_fog([0.05, 0.05, 0.06], *column);
        linear.extend_from_slice(&rgb);
    }
    FogImage {
        width: size.width,
        height: size.height,
        linear,
    }
}

/// Height-fog dispatch. `scene` is absent, so shadows are skipped.
#[must_use]
pub fn media_dispatch(flags: u32, stop_slices: u32) -> FogDispatch {
    let (settings, view, size, sun) = media_scene();
    FogDispatch {
        settings,
        view,
        size,
        sun,
        lamps: Vec::new(),
        scene: None,
        flags,
        stop_slices,
        read_extinction: false,
    }
}

/// Nearest quarter of the slices, no tail.
#[must_use]
pub fn near_dispatch(flags: u32) -> FogDispatch {
    let stop = media_scene().2.slices / 4;
    media_dispatch(flags, stop.max(1))
}

/// Shadowed shaft. `flags` can skip the shadow lookup.
#[must_use]
pub fn shaft_dispatch(flags: u32) -> FogDispatch {
    let (settings, view, size, sun, lamp, scene) = shaft_scene();
    FogDispatch {
        settings,
        view,
        size,
        sun,
        lamps: vec![lamp],
        scene: Some(scene),
        flags,
        stop_slices: 0,
        read_extinction: false,
    }
}

/// One sphere, height density zero. The extinction image comes back.
#[must_use]
pub fn sphere_dispatch(flags: u32) -> FogDispatch {
    let mut job = media_dispatch(flags, 0);
    job.settings.density = 0.0;
    job.settings.volumes = vec![sphere_volume()];
    job.size = GridSize {
        width: 8,
        height: 8,
        slices: 12,
    };
    job.view = FogView::look([0.0, 0.0, 0.0], 0.0, 0.0, 1.0, 1.0);
    job.read_extinction = true;
    job
}

/// One box, height density zero. The extinction image comes back.
#[must_use]
pub fn box_dispatch(flags: u32) -> FogDispatch {
    let mut job = sphere_dispatch(flags);
    job.settings.volumes = vec![box_volume()];
    job
}

/// The box the leak row injects. Same bound as the CPU leak fixture.
#[must_use]
pub fn box_volume() -> FogVolume {
    FogVolume {
        shape: FogShape::Box,
        position: [0.0, 0.0, -5.0],
        extents: [1.5, 1.5, 1.5],
        density: 1.0,
        albedo: 1.0,
        phase_g: 0.0,
        falloff: 0.0,
    }
}

/// The sphere the leak row injects.
#[must_use]
pub fn sphere_volume() -> FogVolume {
    FogVolume {
        shape: FogShape::Sphere,
        position: [0.0, 0.0, -6.0],
        extents: [1.2, 1.2, 1.2],
        density: 1.0,
        albedo: 1.0,
        phase_g: 0.0,
        falloff: 0.0,
    }
}

/// Still camera, two identical dispatches.
#[must_use]
pub fn flicker_dispatch() -> FogDispatch {
    media_dispatch(0, 0)
}

/// One yaw frame of the High-grid blob. History is not read.
#[must_use]
pub fn smear_dispatch(frame: u32) -> FogDispatch {
    let mut job = media_dispatch(0, 0);
    job.settings.density = 0.0;
    job.settings.volumes = vec![blob_volume()];
    job.view = blob_view(frame);
    job.size = blob_size();
    job.sun = DirectSun {
        toward: [0.0, 1.0, 0.0],
        color: [1.0, 1.0, 1.0],
    };
    job
}

/// In-scatter luminance, one value per column.
#[must_use]
pub fn column_luma(columns: &[FogColumn]) -> Vec<f32> {
    columns.iter().map(|column| luma(column.inscatter)).collect()
}

/// `apply_fog` of `[0.05, 0.05, 0.06]` over the columns.
#[must_use]
pub fn picture_from(size: GridSize, columns: &[FogColumn]) -> FogImage {
    columns_image(size, columns)
}

/// Low-preset floor the frame grade draws. `enabled` is the fog switch.
#[must_use]
pub fn composed_frame_scene(enabled: bool) -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 4.0,
            color: [0.6, 0.6, 0.6],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::Low),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        fog: FogSettings {
            enabled,
            density: if enabled { 0.08 } else { 0.0 },
            thickness: 40.0,
            albedo: 1.0,
            phase_g: 0.4,
            history: false,
            gi: false,
            ..FogSettings::default()
        },
        menu: genos_scene::MenuSettings::default(),
    }
}

/// Eye for [`composed_frame_scene`]. Yaw 0 looks along -Z, across the floor.
#[must_use]
pub fn composed_frame_camera() -> Camera {
    Camera::new(0.0, 2.0, 0.0)
}

/// Peak channel error. An empty or mismatched buffer scores 1.
#[must_use]
pub fn column_peak(gpu: &[FogColumn], cpu: &[FogColumn]) -> f64 {
    if gpu.is_empty() || gpu.len() != cpu.len() {
        return 1.0;
    }
    let mut peak = 0.0_f32;
    for (left, right) in gpu.iter().zip(cpu) {
        for channel in 0..3 {
            peak = peak.max((left.inscatter[channel] - right.inscatter[channel]).abs());
        }
        peak = peak.max((left.transmittance - right.transmittance).abs());
    }
    f64::from(peak)
}

/// 0 when any byte differs, 1 when the pictures match or one is missing.
#[must_use]
pub fn picture_unchanged(on: &[u8], off: &[u8]) -> f64 {
    if on.is_empty() || on.len() != off.len() {
        return 1.0;
    }
    if on.iter().zip(off).any(|(left, right)| left != right) {
        0.0
    } else {
        1.0
    }
}

/// Mean absolute byte delta. Printed beside [`picture_unchanged`].
#[must_use]
pub fn picture_mean_abs(on: &[u8], off: &[u8]) -> f64 {
    if on.is_empty() || on.len() != off.len() {
        return 0.0;
    }
    let mut sum = 0_u64;
    for (left, right) in on.iter().zip(off) {
        sum += u64::from(left.abs_diff(*right));
    }
    let count = u32::try_from(on.len()).unwrap_or(1).max(1);
    let total = u32::try_from(sum).unwrap_or(u32::MAX);
    f64::from(total) / f64::from(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_evidence_stays_inside_the_gates() {
        let every = smear_tracking(1, blob_luma);
        let stride = smear_tracking(16, blob_luma);
        let pop = yaw_pop_fixture();
        assert!(every <= 0.50, "smear {every} pop {pop}");
        assert!(stride <= 0.50, "stride16 {stride}");
        assert!(pop <= 0.05, "pop {pop}");
        assert!(smear_broken() > 0.50, "broken smear");
        assert!(yaw_pop_broken() > 0.05, "broken pop");
    }

    #[test]
    fn a_matching_picture_scores_unchanged() {
        assert_eq!(picture_unchanged(&[1, 2, 3, 4], &[1, 2, 3, 4]), 1.0);
        assert_eq!(picture_unchanged(&[0, 0, 0, 0], &[9, 0, 0, 0]), 0.0);
        assert_eq!(picture_unchanged(&[], &[1]), 1.0);
        assert!(picture_mean_abs(&[0, 10], &[0, 0]) > 0.0);
    }
}

