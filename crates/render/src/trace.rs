//! Geometric light at a surface. No probe field and no gather shader.
//!
//! Direct light uses the same lamp unit as the picture: cosine over inverse square,
//! a sun as a lamp 7 m away, and a ray that stops on the first wall, solid, floor or
//! roof. Bounce is a cosine-weighted hemisphere. A hit returns that surface's direct
//! light. A miss returns the sky.

use genos_scene::{Scene, Shape, Solid, Wall};

const LAMBERT: f32 = 0.254647909;
const LAMP_UNIT: f32 = 72.0;
const LAMP_RADIUS: f32 = 0.1;
const PI: f32 = std::f32::consts::PI;

/// Rec.709 luminance.
pub fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Undo [`crate::mesh::tone`] on one display channel in 0..1.
pub fn linear_from_display(displayed: f32) -> f32 {
    if displayed <= 0.64 {
        return displayed.max(0.0);
    }
    let k = ((displayed - 0.64) / 0.14).clamp(0.0, 0.999);
    0.64 + 1.1 * k / (1.0 - k)
}

/// `1 - min(1, |engine - reference| / max(reference, 0.02))`. A dark reference
/// scores 0 when the picture is visibly lit.
pub fn agreement(engine_y: f32, ref_y: f32) -> f32 {
    if ref_y < 0.02 && engine_y > 0.06 {
        return 0.0;
    }
    let err = (engine_y - ref_y).abs() / ref_y.max(0.02);
    1.0 - err.min(1.0)
}

/// How many surface hits a bounce path may take. The picture's probes feed themselves,
/// so the reference keeps diffuse hops until a further hop stops moving the result.
const BOUNCE_HITS: u32 = 6;
/// Cosine samples in the first hemisphere. Later hops take one sample each.
/// A lamp against the ceiling lights a small patch, so the set has to be fine
/// enough to land in that patch or the floor under it comes out too dark.
const HEMISPHERE_SAMPLES: u32 = 1024;

/// Linear shaded radiance at `point` on a face with `normal` and `albedo`.
/// This is the value before the tone curve: albedo × BRDF × (direct + π × bounce).
pub fn radiance(scene: &Scene, point: [f32; 3], normal: [f32; 3], albedo: [f32; 3]) -> [f32; 3] {
    radiance_hits(scene, point, normal, albedo, BOUNCE_HITS)
}

/// Same as [`radiance`], with an explicit hop count. `hits` of 1 is the light that
/// leaves the first surface a ray meets. Each extra hit lets that surface see one more hop.
pub fn radiance_hits(
    scene: &Scene,
    point: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
    hits: u32,
) -> [f32; 3] {
    let origin = add(point, scale(normal, 0.02));
    let direct = direct_light(scene, origin, normal);
    let bounce = scale(hemisphere(scene, origin, normal, hits), PI);
    mul(albedo, scale(add(direct, bounce), LAMBERT))
}

fn direct_light(scene: &Scene, origin: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    if point_inside(scene, origin) {
        return [0.0; 3];
    }
    let mut sum = [0.0; 3];
    for light in &scene.lights {
        let (toward, dist2, target) = lamp_toward(light, origin);
        let nd = dot(toward, normal);
        if nd <= 0.0 {
            continue;
        }
        if occluded(scene, origin, target) {
            continue;
        }
        let scale_l = nd * LAMP_UNIT / dist2.max(LAMP_RADIUS * LAMP_RADIUS);
        sum = add(sum, scale(light.color, scale_l));
    }
    sum
}

fn lamp_toward(light: &genos_scene::Light, origin: [f32; 3]) -> ([f32; 3], f32, [f32; 3]) {
    let span = light.direction.length_squared();
    if span > 1.0e-8 {
        let toward = scale(xyz(light.direction), -1.0 / span.sqrt());
        return (toward, 49.0, add(origin, scale(toward, 80.0)));
    }
    let toward = sub(xyz(light.position), origin);
    let dist = length(toward).max(1.0e-4);
    let dir = scale(toward, 1.0 / dist);
    (dir, dist * dist, xyz(light.position))
}

/// Mean incoming radiance over a cosine hemisphere.
/// `hits` is how many surfaces a path may shade. After the first hop each path
/// follows one cosine sample, so the cost stays linear in the hop count.
fn hemisphere(scene: &Scene, origin: [f32; 3], normal: [f32; 3], hits: u32) -> [f32; 3] {
    let n = HEMISPHERE_SAMPLES;
    let mut sum = [0.0; 3];
    for i in 0..n {
        let dir = cosine_dir(i, n, normal);
        sum = add(sum, path_in(scene, origin, dir, hits, i));
    }
    scale(sum, 1.0 / n as f32)
}

/// How many cosine samples from `point` land within 8 degrees of `target`, and the
/// brightest direct outgoing among those samples. Zero samples means the set misses it.
pub fn samples_toward(
    scene: &Scene,
    point: [f32; 3],
    normal: [f32; 3],
    target: [f32; 3],
) -> (u32, f32) {
    let origin = add(point, scale(normal, 0.02));
    let aim = normalize(sub(target, origin));
    let n = HEMISPHERE_SAMPLES;
    let mut count = 0u32;
    let mut peak = 0.0_f32;
    for i in 0..n {
        let dir = cosine_dir(i, n, normal);
        if dot(dir, aim) < 0.990 { // about 8 degrees
            continue;
        }
        count += 1;
        let Some(hit) = trace(scene, origin, dir, 80.0) else {
            continue;
        };
        let at = add(hit.point, scale(hit.normal, 0.02));
        let y = luminance(mul(
            hit.albedo,
            scale(direct_light(scene, at, hit.normal), LAMBERT),
        ));
        peak = peak.max(y);
    }
    (count, peak)
}

/// Brightest and mean direct-only outgoing luminance of the first surfaces seen
/// from `point`. Used to see whether a small hot spot is in the sample set.
pub fn first_outgoing(scene: &Scene, point: [f32; 3], normal: [f32; 3]) -> (f32, f32, u32) {
    let origin = add(point, scale(normal, 0.02));
    let n = HEMISPHERE_SAMPLES;
    let mut sum = 0.0;
    let mut peak = 0.0_f32;
    let mut hits = 0u32;
    for i in 0..n {
        let dir = cosine_dir(i, n, normal);
        let Some(hit) = trace(scene, origin, dir, 80.0) else {
            continue;
        };
        hits += 1;
        let at = add(hit.point, scale(hit.normal, 0.02));
        let y = luminance(mul(
            hit.albedo,
            scale(direct_light(scene, at, hit.normal), LAMBERT),
        ));
        sum += y;
        peak = peak.max(y);
    }
    (peak, sum / n as f32, hits)
}

/// Radiance arriving along `dir`. A hit returns the light leaving that surface.
fn path_in(scene: &Scene, origin: [f32; 3], dir: [f32; 3], hits: u32, seed: u32) -> [f32; 3] {
    if hits == 0 {
        return [0.0; 3];
    }
    let Some(hit) = trace(scene, origin, dir, 80.0) else {
        return scene.sky.as_ref().map(|sky| sky.color).unwrap_or([0.0; 3]);
    };
    let origin = add(hit.point, scale(hit.normal, 0.02));
    let direct = direct_light(scene, origin, hit.normal);
    let next = if hits <= 1 {
        [0.0; 3]
    } else {
        let dir = cosine_dir(
            seed.wrapping_add(hits.wrapping_mul(17)),
            HEMISPHERE_SAMPLES,
            hit.normal,
        );
        scale(
            path_in(scene, origin, dir, hits - 1, seed ^ hits.wrapping_mul(0x9E37)),
            PI,
        )
    };
    mul(hit.albedo, scale(add(direct, next), LAMBERT))
}

struct Hit {
    point: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
}

fn trace(scene: &Scene, origin: [f32; 3], dir: [f32; 3], reach: f32) -> Option<Hit> {
    let mut best_t = reach;
    let mut best: Option<Hit> = None;
    let consider = |t: f32, normal: [f32; 3], albedo: [f32; 3], best_t: &mut f32, best: &mut Option<Hit>| {
        if t > 1.0e-3 && t < *best_t {
            *best_t = t;
            *best = Some(Hit {
                point: add(origin, scale(dir, t)),
                normal,
                albedo,
            });
        }
    };
    if dir[1].abs() > 1.0e-8 {
        let t = (0.0 - origin[1]) / dir[1];
        let p = add(origin, scale(dir, t));
        if on_floor(scene, p[0], p[2]) {
            let n = if origin[1] >= 0.0 { [0.0, 1.0, 0.0] } else { [0.0, -1.0, 0.0] };
            consider(t, n, scene.floor.color, &mut best_t, &mut best);
        }
    }
    if let Some(ceiling) = &scene.ceiling {
        if dir[1].abs() > 1.0e-8 {
            let t = (ceiling.height - origin[1]) / dir[1];
            let p = add(origin, scale(dir, t));
            if on_floor(scene, p[0], p[2]) {
                let n = if origin[1] >= ceiling.height {
                    [0.0, 1.0, 0.0]
                } else {
                    [0.0, -1.0, 0.0]
                };
                consider(t, n, ceiling.color, &mut best_t, &mut best);
            }
        }
    }
    for wall in &scene.walls {
        if let Some((t, n)) = hit_wall(origin, dir, wall, best_t) {
            consider(t, n, wall.color, &mut best_t, &mut best);
        }
    }
    for solid in &scene.solids {
        if let Some((t, n)) = hit_solid(origin, dir, solid, best_t) {
            consider(t, n, solid.color, &mut best_t, &mut best);
        }
    }
    best
}

/// Where a ray from `from` toward `point` first lands, and that surface's direct luminance.
pub fn ray_land(scene: &Scene, from: [f32; 3], point: [f32; 3]) -> Option<([f32; 3], f32)> {
    let delta = sub(point, from);
    let dist = length(delta);
    let hit = trace(scene, from, scale(delta, 1.0 / dist.max(1.0e-6)), dist + 1.0)?;
    let at = add(hit.point, scale(hit.normal, 0.02));
    let y = luminance(mul(
        hit.albedo,
        scale(direct_light(scene, at, hit.normal), LAMBERT),
    ));
    Some((hit.point, y))
}

/// True when a ray from `from` meets `point` before any other surface.
pub fn sees(scene: &Scene, from: [f32; 3], point: [f32; 3]) -> bool {
    let delta = sub(point, from);
    let dist = length(delta);
    if dist < 1.0e-3 {
        return true;
    }
    match trace(scene, from, scale(delta, 1.0 / dist), dist + 0.05) {
        None => true,
        Some(hit) => length(sub(hit.point, point)) < 0.25,
    }
}

fn occluded(scene: &Scene, from: [f32; 3], to: [f32; 3]) -> bool {
    let delta = sub(to, from);
    let dist = length(delta);
    if dist < 1.0e-3 {
        return false;
    }
    trace(scene, from, scale(delta, 1.0 / dist), dist - 1.0e-3).is_some()
}

fn on_floor(scene: &Scene, x: f32, z: f32) -> bool {
    (x - scene.floor.position.x).abs() <= scene.floor.half_x + 0.05
        && (z - scene.floor.position.z).abs() <= scene.floor.half_z + 0.05
}

fn point_inside(scene: &Scene, p: [f32; 3]) -> bool {
    if p[1] < -0.02 {
        return false;
    }
    for wall in &scene.walls {
        if inside_wall(p, wall) {
            return true;
        }
    }
    for solid in &scene.solids {
        if inside_solid(p, solid) {
            return true;
        }
    }
    false
}

fn inside_wall(p: [f32; 3], wall: &Wall) -> bool {
    // Strictly inside. A sample lifted 0.02 m off a face must stay outside, or the
    // underside of a roof reads as inside the slab and loses its lamp.
    p[1] > wall.base + 1.0e-4
        && p[1] < wall.base + wall.height - 1.0e-4
        && (p[0] - wall.position.x).abs() < wall.half_x - 1.0e-4
        && (p[2] - wall.position.z).abs() < wall.half_z - 1.0e-4
}

fn inside_solid(p: [f32; 3], solid: &Solid) -> bool {
    if p[1] <= 1.0e-4 || p[1] >= solid.height - 1.0e-4 {
        return false;
    }
    let (x, z) = local_xz(p[0], p[2], solid);
    let half = solid.size * 0.5 - 1.0e-4;
    match solid.shape {
        Shape::Square => x.abs() < half && z.abs() < half,
        Shape::Circle => x * x + z * z < half * half,
    }
}

fn hit_wall(origin: [f32; 3], dir: [f32; 3], wall: &Wall, t_max: f32) -> Option<(f32, [f32; 3])> {
    let min = [
        wall.position.x - wall.half_x,
        wall.base,
        wall.position.z - wall.half_z,
    ];
    let max = [
        wall.position.x + wall.half_x,
        wall.base + wall.height,
        wall.position.z + wall.half_z,
    ];
    hit_box(origin, dir, min, max, t_max)
}

fn hit_solid(origin: [f32; 3], dir: [f32; 3], solid: &Solid, t_max: f32) -> Option<(f32, [f32; 3])> {
    match solid.shape {
        Shape::Circle => hit_cylinder(origin, dir, solid, t_max),
        Shape::Square => {
            let (ox, oz) = local_xz(origin[0], origin[2], solid);
            let (dx, dz) = local_dir(dir[0], dir[2], solid);
            let half = solid.size * 0.5;
            let local_o = [ox, origin[1], oz];
            let local_d = [dx, dir[1], dz];
            let (t, mut n) = hit_box(
                local_o,
                local_d,
                [-half, 0.0, -half],
                [half, solid.height, half],
                t_max,
            )?;
            n = world_dir(n[0], n[2], solid, n[1]);
            Some((t, n))
        }
    }
}

fn hit_cylinder(origin: [f32; 3], dir: [f32; 3], solid: &Solid, t_max: f32) -> Option<(f32, [f32; 3])> {
    let (ox, oz) = local_xz(origin[0], origin[2], solid);
    let (dx, dz) = local_dir(dir[0], dir[2], solid);
    let radius = solid.size * 0.5;
    let a = dx * dx + dz * dz;
    let b = 2.0 * (ox * dx + oz * dz);
    let c = ox * ox + oz * oz - radius * radius;
    let mut best: Option<(f32, [f32; 3])> = None;
    if a > 1.0e-8 {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let root = disc.sqrt();
            for sign in [-1.0, 1.0] {
                let t = (-b + sign * root) / (2.0 * a);
                if t > 1.0e-3 && t < t_max {
                    let y = origin[1] + dir[1] * t;
                    if (0.0..=solid.height).contains(&y) {
                        let px = ox + dx * t;
                        let pz = oz + dz * t;
                        let len = (px * px + pz * pz).sqrt().max(1.0e-4);
                        let n = world_dir(px / len, pz / len, solid, 0.0);
                        if best.map(|(bt, _)| t < bt).unwrap_or(true) {
                            best = Some((t, n));
                        }
                    }
                }
            }
        }
    }
    if dir[1].abs() > 1.0e-8 {
        for (height, ny) in [(0.0, -1.0), (solid.height, 1.0)] {
            let t = (height - origin[1]) / dir[1];
            if t > 1.0e-3 && t < t_max && best.map(|(bt, _)| t < bt).unwrap_or(true) {
                let x = ox + dx * t;
                let z = oz + dz * t;
                if x * x + z * z <= radius * radius {
                    best = Some((t, [0.0, ny, 0.0]));
                }
            }
        }
    }
    best
}

fn hit_box(
    origin: [f32; 3],
    dir: [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
    t_max: f32,
) -> Option<(f32, [f32; 3])> {
    let mut t0 = 0.0;
    let mut t1 = t_max;
    let mut normal = [0.0, 1.0, 0.0];
    for axis in 0..3 {
        let inv = if dir[axis].abs() < 1.0e-8 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        } else {
            1.0 / dir[axis]
        };
        let mut ta = (min[axis] - origin[axis]) * inv;
        let mut tb = (max[axis] - origin[axis]) * inv;
        let mut n = [0.0; 3];
        n[axis] = -dir[axis].signum();
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
            n[axis] = -n[axis];
        }
        if ta > t0 {
            t0 = ta;
            normal = n;
        }
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    if t0 <= 1.0e-3 {
        return None;
    }
    Some((t0, normal))
}

fn local_xz(x: f32, z: f32, solid: &Solid) -> (f32, f32) {
    let dx = x - solid.position.x;
    let dz = z - solid.position.z;
    let (s, c) = solid.yaw.sin_cos();
    (dx * c - dz * s, dx * s + dz * c)
}

fn local_dir(dx: f32, dz: f32, solid: &Solid) -> (f32, f32) {
    let (s, c) = solid.yaw.sin_cos();
    (dx * c - dz * s, dx * s + dz * c)
}

fn world_dir(lx: f32, lz: f32, solid: &Solid, y: f32) -> [f32; 3] {
    let (s, c) = solid.yaw.sin_cos();
    [lx * c + lz * s, y, -lx * s + lz * c]
}

fn cosine_dir(i: u32, n: u32, normal: [f32; 3]) -> [f32; 3] {
    let u = ((i % n.max(1)) as f32 + 0.5) / n.max(1) as f32;
    let v = (i as f32 * 0.754877666).fract();
    let r = u.sqrt();
    let phi = 2.0 * PI * v;
    let local = [r * phi.cos(), (1.0 - u).sqrt(), r * phi.sin()];
    let up = if normal[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let tangent = normalize(cross(up, normal));
    let bitangent = cross(normal, tangent);
    normalize(add(
        add(scale(tangent, local[0]), scale(normal, local[1])),
        scale(bitangent, local[2]),
    ))
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
fn mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = length(a).max(1.0e-8);
    scale(a, 1.0 / len)
}

fn xyz(v: genos_scene::Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_scene::{Floor, Light, Scene, Solid, Vec3, Wall};

    fn room(lights: Vec<Light>, walls: Vec<Wall>, solids: Vec<Solid>) -> Scene {
        Scene {
            floor: Floor {
                position: Vec3::ZERO,
                half_x: 8.0,
                half_z: 8.0,
                color: [1.0, 1.0, 1.0],
            },
            walls,
            solids,
            lights,
            ceiling: None,
            sky: None,
        }
    }

    #[test]
    fn a_lamp_lights_the_floor_and_a_wall_blocks_it() {
        let lamp = Light {
            position: Vec3::new(0.0, 2.0, 0.0),
            color: [1.0, 1.0, 1.0],
            direction: Vec3::ZERO,
        };
        let open = room(vec![lamp.clone()], Vec::new(), Vec::new());
        let lit = radiance(&open, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0]);
        assert!(luminance(lit) > 0.2, "{lit:?}");
        let wall = Wall {
            position: Vec3::new(0.0, 0.0, 1.0),
            half_x: 2.0,
            half_z: 0.1,
            height: 3.0,
            base: 0.0,
            color: [1.0, 1.0, 1.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        };
        let blocked = room(vec![lamp], vec![wall], Vec::new());
        let shadow = radiance(&blocked, [0.0, 0.0, 2.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0]);
        assert!(luminance(shadow) < luminance(lit) * 0.25, "shadow {shadow:?} lit {lit:?}");
    }

    #[test]
    fn a_lit_wall_bounces_onto_a_floor_the_lamp_cannot_see() {
        let lamp = Light {
            position: Vec3::new(0.0, 1.5, -1.0),
            color: [1.0, 1.0, 1.0],
            direction: Vec3::ZERO,
        };
        let wall = Wall {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 2.0,
            half_z: 0.1,
            height: 3.0,
            base: 0.0,
            color: [1.0, 1.0, 1.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        };
        let scene = room(vec![lamp], vec![wall], Vec::new());
        let floor = radiance(&scene, [0.0, 0.0, 1.5], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0]);
        let direct = direct_light(&scene, [0.0, 0.02, 1.5], [0.0, 1.0, 0.0]);
        assert!(luminance(direct) < 1.0e-4, "direct leaks {direct:?}");
        assert!(luminance(floor) > 0.01, "no bounce {floor:?}");
    }

    #[test]
    fn a_dark_reference_rejects_a_lit_picture() {
        assert_eq!(agreement(0.1, 0.01), 0.0);
        assert!(agreement(0.10, 0.10) > 0.99);
    }

    #[test]
    fn the_tone_curve_undoes_below_the_bend() {
        assert!((linear_from_display(0.25) - 0.25).abs() < 1.0e-5);
    }
}
