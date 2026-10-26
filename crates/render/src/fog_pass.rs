//! Shadowed shafts, and the yaw evidence while history stays off.
//!
//! Sun and point lamps take `V` from [`crate::trace::sees`]. The world-cache
//! term stays the zero stub in `fog::world_cache_inscatter`.

use genos_math::{height_optical_depth_f32, phase_hg_f32};
use genos_scene::{Floor, FogSettings, FogShape, FogVolume, Scene, Vec3, Wall};

use crate::fog::{
    add, apply_fog, dot, falloff, integrate_samples, length, length_sq, luma, normalize, point_at,
    ray_dir, scale, scale_color, slice_edge, sub, unit, world_cache_inscatter,
    DirectSun, FogColumn, FogView, GridSize, SliceSample, Z_FAR,
};
use crate::trace::sees;

/// Matches `crates/render/src/trace.rs` `LAMP_UNIT`.
const LAMP_UNIT: f32 = 72.0;
const YAW_FRAMES: u32 = 320;
const PICTURE_WIDTH: f64 = 2560.0;

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

/// Shadowed sun and one point lamp, or the same integral with `V` forced to 1.
#[must_use]
pub fn shaft_image(traced_shadows: bool) -> FogImage {
    let (settings, view, size, sun, lamp, scene) = shaft_scene();
    let columns = integrate_lit(&settings, &view, size, sun, &[lamp], &scene, traced_shadows);
    columns_image(size, &columns)
}

/// Centroid lag, in 1440p picture pixels, of a static fog blob across a 90° yaw.
/// History is off, so the rendered centroid follows the geometric one.
#[must_use]
pub fn smear_fixture() -> f64 {
    smear(false)
}

/// The rendered centroid is held on the first frame, which is what a history
/// blend does to a moving shaft.
#[must_use]
pub fn smear_broken() -> f64 {
    smear(true)
}

/// Max frame-to-frame relative luminance of the fog term after frame 8.
#[must_use]
pub fn yaw_pop_fixture() -> f32 {
    yaw_pop(false)
}

/// Odd frames scale the fog luminance by 1.4.
#[must_use]
pub fn yaw_pop_broken() -> f32 {
    yaw_pop(true)
}

fn smear(hold: bool) -> f64 {
    let mut max_lag = 0.0_f64;
    let mut prev_rendered = picture_x(centroid(0));
    let mut prev_geometric = picture_x(project_blob(0));
    let frozen = prev_rendered;
    for frame in 1..YAW_FRAMES {
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
    }
    max_lag
}

fn yaw_pop(scale_odd: bool) -> f32 {
    let mut max_rel = 0.0_f32;
    let mut previous = yaw_energy(0, scale_odd);
    for frame in 1..YAW_FRAMES {
        let current = yaw_energy(frame, scale_odd);
        let rel = (current - previous).abs() / previous.max(current).max(0.02);
        if frame > 8 {
            max_rel = max_rel.max(rel);
        }
        previous = current;
    }
    max_rel
}

fn yaw_energy(frame: u32, scale_odd: bool) -> f32 {
    let mut sum = blob_luma(frame).into_iter().sum::<f32>();
    if scale_odd && frame % 2 == 1 {
        sum *= 1.4;
    }
    sum
}

fn centroid(frame: u32) -> f32 {
    let luma = blob_luma(frame);
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
    GridSize {
        width: 384,
        height: 48,
        slices: 96,
    }
}

fn blob_view(frame: u32) -> FogView {
    let t = unit(frame) / unit(YAW_FRAMES - 1);
    let yaw = std::f32::consts::FRAC_PI_2 * (t - 0.5);
    FogView::look([0.0, 1.4, 0.0], yaw, 0.0, 1.5, 2.0)
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
            sigma_height: sigma_height.max(0.0),
            centre,
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
    sigma_height: f32,
    centre: [f32; 3],
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
    let sun_coeff = phase_coeff(query.settings, query.sigma_height, query.centre, sun_mu);
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
        let coeff =
            phase_coeff(query.settings, query.sigma_height, query.centre, mu) * falloff_l * lamp_v;
        for (channel, slot) in source.iter_mut().enumerate() {
            *slot += coeff * lamp.color[channel];
        }
    }
    source
}

fn phase_coeff(settings: &FogSettings, sigma_height: f32, centre: [f32; 3], mu: f32) -> f32 {
    let mut coeff = sigma_height * settings.albedo.clamp(0.0, 1.0) * phase_hg_f32(settings.phase_g, mu);
    for volume in &settings.volumes {
        let local = volume.density.max(0.0) * falloff(volume, centre);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_evidence_stays_inside_the_gates() {
        let smear = smear_fixture();
        let pop = yaw_pop_fixture();
        assert!(smear <= 0.50, "smear {smear} pop {pop}");
        assert!(pop <= 0.05, "pop {pop}");
        assert!(smear_broken() > 0.50, "broken smear");
        assert!(yaw_pop_broken() > 0.05, "broken pop");
    }
}

