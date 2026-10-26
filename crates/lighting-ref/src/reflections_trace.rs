//! Independent Whitted oracle for the planar mirror, the portal, and flat water.
//!
//! Fresnel, Snell, Beer–Lambert, the portal map, and the view count are f64
//! closed forms in this file. It does not call `genos_math::optics` and it
//! does not read that module's constants. It does not call `shade_picture`
//! or `water_shade`.

/// Pixels on a side of one tile. The engine picture uses the same count.
const TILE: u32 = 8;
const FOV: f64 = 1.047_197_551_196_597_6; // 60 degrees
const SIGMA: f64 = 0.35;
const WATER_ETA: f64 = 1.333;
const VIEW_CUTOFF: f64 = 0.02;
const LOW_VIEWS: u32 = 1;
const WALL_POS: [f64; 3] = [0.85, 0.12, 0.08];
const WALL_NEG: [f64; 3] = [0.10, 0.55, 0.20];
const PORTAL_HIGH: [f64; 3] = [0.15, 0.35, 0.90];
const PORTAL_LOW: [f64; 3] = [0.90, 0.70, 0.20];
const FLOOR: [f64; 3] = [0.08, 0.22, 0.72];
const CEILING: [f64; 3] = [0.72, 0.68, 0.55];

/// Linear colour and primary-hit distance, three tiles stacked.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub color: Vec<[f64; 3]>,
    pub depth: Vec<f64>,
}

/// One still frame. The camera does not move, so every frame is this image.
#[must_use]
pub fn trace() -> Reference {
    let count = idx(TILE * TILE * 3);
    let mut color = vec![[0.0; 3]; count];
    let mut depth = vec![0.0; count];
    for py in 0..TILE {
        for px in 0..TILE {
            store(&mut color, &mut depth, 0, px, py, mirror_pixel(px, py));
            store(&mut color, &mut depth, 1, px, py, portal_pixel(px, py));
            store(&mut color, &mut depth, 2, px, py, water_pixel(px, py));
        }
    }
    Reference { color, depth }
}

/// Mean channel of one lit pixel. The reference is still, so the series is flat.
#[must_use]
pub fn still_pixel(index: usize) -> f64 {
    let image = trace();
    let pixel = image.color.get(index).copied().unwrap_or([0.0; 3]);
    (pixel[0] + pixel[1] + pixel[2]) / 3.0
}

fn mirror_pixel(px: u32, py: u32) -> ([f64; 3], f64) {
    let (eye, ray) = primary(
        px,
        py,
        v3(0.0, 1.0, 4.0),
        v3(0.0, 0.0, -1.0),
        v3(1.0, 0.0, 0.0),
        v3(0.0, 1.0, 0.0),
    );
    let Some((hit, distance)) = hit_plane(eye, ray, 0.0, 2) else {
        return ([0.0; 3], 0.0);
    };
    (mirror_color(hit, ray), distance)
}

fn portal_pixel(px: u32, py: u32) -> ([f64; 3], f64) {
    let (eye, ray) = primary(
        px,
        py,
        v3(0.0, 1.0, 4.0),
        v3(0.0, 0.0, -1.0),
        v3(1.0, 0.0, 0.0),
        v3(0.0, 1.0, 0.0),
    );
    let Some((hit, distance)) = hit_plane(eye, ray, 0.0, 2) else {
        return ([0.0; 3], 0.0);
    };
    (portal_color(hit, ray), distance)
}

fn water_pixel(px: u32, py: u32) -> ([f64; 3], f64) {
    let (eye, ray) = primary(
        px,
        py,
        v3(0.0, 3.0, 0.5),
        v3(0.0, -1.0, 0.0),
        v3(1.0, 0.0, 0.0),
        v3(0.0, 0.0, -1.0),
    );
    let Some((hit, distance)) = hit_plane(eye, ray, 1.0, 1) else {
        return ([0.0; 3], 0.0);
    };
    let opaque = if hit_plane(eye, ray, 0.0, 1).is_some() {
        FLOOR
    } else {
        [0.0; 3]
    };
    (water_color(ray, hit, opaque), distance)
}

fn mirror_color(hit: V3, ray: V3) -> [f64; 3] {
    if view_count(&[1.0], LOW_VIEWS, VIEW_CUTOFF) == 0 {
        return [0.0; 3];
    }
    let bounce = reflect(ray, v3(0.0, 0.0, 1.0));
    let Some((place, _)) = hit_plane(hit, bounce, 2.0, 2) else {
        return [0.0; 3];
    };
    if place.x >= 0.0 {
        WALL_POS
    } else {
        WALL_NEG
    }
}

fn portal_color(hit: V3, ray: V3) -> [f64; 3] {
    let bounce = portal_dir(ray);
    let origin = add(portal_point(hit), scale(bounce, 1.0e-3));
    let Some((place, _)) = hit_plane(origin, bounce, 2.0, 2) else {
        return [0.0; 3];
    };
    if place.y >= 1.0 {
        PORTAL_HIGH
    } else {
        PORTAL_LOW
    }
}

/// `F * reflected + (1 - F) * transmitted * exp(-sigma * distance)`.
fn water_color(ray: V3, hit: V3, opaque: [f64; 3]) -> [f64; 3] {
    let cos_i = dot(v3(0.0, 1.0, 0.0), scale(ray, -1.0)).clamp(0.0, 1.0);
    let fresnel = fresnel_dielectric(WATER_ETA, cos_i);
    let bounce = reflect(ray, v3(0.0, 1.0, 0.0));
    let reflected = if hit_plane(hit, bounce, 4.0, 1).is_some() {
        CEILING
    } else {
        opaque
    };
    let Some(bent) = refract(ray, v3(0.0, 1.0, 0.0), 1.0 / WATER_ETA) else {
        return CEILING;
    };
    let (sample, travel) = match hit_plane(hit, bent, 0.0, 1) {
        Some((place, _)) => (FLOOR, length(sub(place, hit))),
        None => (opaque, hit.y.max(0.0)),
    };
    let absorb = beer(SIGMA, travel);
    mix(reflected, sample, fresnel, (1.0 - fresnel) * absorb)
}

/// Unpolarized dielectric Fresnel. `eta` is `n_transmitted / n_incident`.
#[expect(
    clippy::manual_midpoint,
    reason = "keep the same product the GLSL Fresnel mirror uses"
)]
fn fresnel_dielectric(eta: f64, cos_i: f64) -> f64 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let sin2_t = (1.0 - cos_i * cos_i).max(0.0) / (eta * eta);
    if sin2_t >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    let rs = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    let rp = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    0.5 * (rs * rs + rp * rp)
}

/// Snell direction. `eta` is `n_incident / n_transmitted`. `None` is total internal reflection.
fn refract(incident: V3, normal: V3, eta: f64) -> Option<V3> {
    let along = dot(normal, incident);
    let k = 1.0 - eta * eta * (1.0 - along * along);
    if k < 0.0 {
        return None;
    }
    let scale_n = eta * along + k.sqrt();
    Some(sub(scale(incident, eta), scale(normal, scale_n)))
}

fn beer(sigma: f64, distance: f64) -> f64 {
    (-sigma * distance).exp()
}

fn reflect(incident: V3, normal: V3) -> V3 {
    let along = dot(normal, incident);
    sub(incident, scale(normal, 2.0 * along))
}

/// `to + to_basis * flip_z * transpose(from_basis) * (point - from)`.
/// Both bases are identity, so the inverse is identity and the map is `(x, y, -z) + (10, 0, 0)`.
fn portal_point(point: V3) -> V3 {
    let local = sub(point, v3(0.0, 0.0, 0.0));
    let flipped = v3(local.x, local.y, -local.z);
    add(v3(10.0, 0.0, 0.0), flipped)
}

fn portal_dir(direction: V3) -> V3 {
    sub(portal_point(direction), portal_point(v3(0.0, 0.0, 0.0)))
}

fn view_count(fresnel: &[f64], cap: u32, cutoff: f64) -> u32 {
    let mut weight = 1.0;
    let mut emitted = 0u32;
    for reflectance in fresnel {
        if emitted >= cap {
            break;
        }
        weight *= *reflectance;
        if weight < cutoff {
            break;
        }
        emitted += 1;
    }
    emitted
}

fn primary(px: u32, py: u32, eye: V3, forward: V3, right: V3, up: V3) -> (V3, V3) {
    let spread = (FOV * 0.5).tan();
    let u = (unit(px) + 0.5) / unit(TILE) * 2.0 - 1.0;
    let v = 1.0 - (unit(py) + 0.5) / unit(TILE) * 2.0;
    let ray = norm(add(
        forward,
        add(scale(right, u * spread), scale(up, v * spread)),
    ));
    (eye, ray)
}

fn hit_plane(origin: V3, ray: V3, place: f64, axis: usize) -> Option<(V3, f64)> {
    let (start, slope) = match axis {
        0 => (origin.x, ray.x),
        1 => (origin.y, ray.y),
        _ => (origin.z, ray.z),
    };
    if slope.abs() < 1.0e-8 {
        return None;
    }
    let travel = (place - start) / slope;
    if travel <= 1.0e-4 {
        return None;
    }
    Some((add(origin, scale(ray, travel)), travel))
}

fn store(
    color: &mut [[f64; 3]],
    depth: &mut [f64],
    tile: u32,
    px: u32,
    py: u32,
    pixel: ([f64; 3], f64),
) {
    let index = idx(tile * TILE + py) * idx(TILE) + idx(px);
    color[index] = pixel.0;
    depth[index] = pixel.1;
}

fn mix(a: [f64; 3], b: [f64; 3], wa: f64, wb: f64) -> [f64; 3] {
    [
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
    ]
}

#[derive(Clone, Copy)]
struct V3 {
    x: f64,
    y: f64,
    z: f64,
}

fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

fn add(a: V3, b: V3) -> V3 {
    v3(a.x + b.x, a.y + b.y, a.z + b.z)
}

fn sub(a: V3, b: V3) -> V3 {
    v3(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn scale(a: V3, k: f64) -> V3 {
    v3(a.x * k, a.y * k, a.z * k)
}

fn dot(a: V3, b: V3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}

fn norm(a: V3) -> V3 {
    let len = length(a);
    if len <= 1.0e-12 {
        v3(0.0, 0.0, 1.0)
    } else {
        scale(a, 1.0 / len)
    }
}

fn idx(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

fn unit(value: u32) -> f64 {
    f64::from(u16::try_from(value).unwrap_or(0))
}
