//! Independent f64 trace of the presented grade frame.
//!
//! The scene literals are copied here. This file does not call `genos_math::optics`,
//! does not read that module's constants, and does not call the renderer.
//! The camera matches the grade window: 160×120, eye `(0, 1.5, 4)`, yaw and pitch 0.
//! A pixel's ray uses `hy = -ndc_y * tan(30°)`, the same reconstruction the
//! water shader applies to `gl_FragCoord`.
//!
//! Water shading is the dielectric shade: facing normal, Fresnel, the mirrored
//! direction, and transmission along the refracted ray. Beer–Lambert uses the
//! view-ray thickness divided by the refracted cosine.

/// Grade window, in pixels.
pub const WIDTH: u32 = 160;
/// Grade window, in pixels.
pub const HEIGHT: u32 = 120;

const RED: [f64; 3] = [0.84, 0.32, 0.28];
const BLUE: [f64; 3] = [0.22, 0.48, 0.90];
const LEAK: [f64; 3] = [0.95, 0.25, 0.85];
const REFL: [f64; 3] = [0.96, 0.55, 0.30];
const TRANS: [f64; 3] = [0.25, 0.72, 0.40];
const PINK: [f64; 3] = [0.90, 0.42, 0.62];
const ALBEDO: [f64; 3] = [0.28, 0.30, 0.34];
const MISS: [f64; 3] = [0.5, 0.5, 0.55];
const SKY: [f64; 3] = [0.45, 0.58, 0.72];
const OCC: [f64; 3] = [0.92, 0.12, 0.08];
const IOR: f64 = 1.333;
const SIGMA: f64 = 0.55;
const TAN_HALF: f64 = 0.577_350_269_189_625_7;

/// Linear colour and a positive depth weight for one presented frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub color: Vec<[f64; 3]>,
    pub depth: Vec<f64>,
}

struct Plane {
    origin: [f64; 3],
    normal: [f64; 3],
    tangent: [f64; 3],
    bitangent: [f64; 3],
    half_u: f64,
    half_v: f64,
    color: [f64; 3],
}

/// One still frame of the grade scene, before 8-bit quantisation.
#[must_use]
pub fn trace() -> Reference {
    let count = idx(WIDTH * HEIGHT);
    let mut color = vec![[0.0; 3]; count];
    let mut depth = vec![1.0; count];
    let eye = [0.0, 1.5, 4.0];
    let forward = [0.0, 0.0, -1.0];
    let right = [1.0, 0.0, 0.0];
    let up = [0.0, 1.0, 0.0];
    let aspect = f64::from(WIDTH) / f64::from(HEIGHT);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (rgb, dist) = pixel(x, y, eye, forward, right, up, aspect);
            let at = idx(y * WIDTH + x);
            color[at] = rgb;
            depth[at] = dist;
        }
    }
    Reference { color, depth }
}

/// Projected GGX integral with a fine step count. A normalised NDF lands on 1.
#[cfg(test)]
#[must_use]
pub fn ggx_energy() -> f64 {
    ggx_integral(0.35, 4096)
}

/// Rust optics samples, in the order `optics_parity` reads them.
///
/// Mirror direction, Fresnel from air at cos 1 and cos 0.5, Fresnel from water
/// at cos 0.2 (total internal reflection), the refracted direction, Beer–Lambert,
/// the GGX normal distribution, flat-water transmittance, the grade portal map
/// of the eye, the Fresnel view budget, and the Low ray cap.
pub const OPTICS_SAMPLES: usize = 17;

/// f64 values for [`OPTICS_SAMPLES`]. This does not call `genos_math::optics`.
#[must_use]
pub fn optics_reference() -> [f64; OPTICS_SAMPLES] {
    let incident = [0.6, -0.8, 0.0];
    let normal = [0.0, 1.0, 0.0];
    let reflected = reflect(incident, normal);
    let (_bent, transmitted) = refract(incident, normal, 1.0 / IOR);
    let eye = portal_map([0.0, 1.5, 4.0]);
    [
        reflected[0],
        reflected[1],
        reflected[2],
        fresnel_dielectric(IOR, 1.0),
        fresnel_dielectric(IOR, 0.5),
        fresnel_dielectric(1.0 / IOR, 0.2),
        transmitted[0],
        transmitted[1],
        transmitted[2],
        (-0.55_f64).exp(),
        ggx_ndf(1.0, 0.35),
        1.0 - fresnel_dielectric(IOR, 1.0),
        eye[0],
        eye[1],
        eye[2],
        view_budget(&[0.020_373_188, 0.06], 3, 0.02),
        0.0,
    ]
}

/// Fresnel product against a view cap. The design cutoff is 0.02.
fn view_budget(fresnel: &[f64], cap: u32, cutoff: f64) -> f64 {
    let mut weight = 1.0;
    let mut emitted = 0.0;
    for reflectance in fresnel {
        if emitted >= f64::from(cap) {
            break;
        }
        weight *= *reflectance;
        if weight < cutoff {
            break;
        }
        emitted += 1.0;
    }
    emitted
}

/// Breaking-limited wave height, and the crest in deep water.
///
/// The crest is the Finch displacement at `(2, 0)`, time 0. The solitary-wave
/// breaking index is `H / d = 0.78` (Philosophical Magazine 32, 1891, pp. 45–58).
/// Returns `(max of min(crest, 0.78 × depth) / depth, crest / 4 m)`. The limit
/// is that index.
#[must_use]
pub fn shore_oracle() -> (f64, f64) {
    let crest = gerstner_point([2.0, 0.0])[1].abs();
    // McCowan 1891 breaking index.
    let breaking = 0.78;
    let depths = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0];
    let mut max_ratio = 0.0_f64;
    for depth in depths {
        let height = crest.min(breaking * depth);
        max_ratio = max_ratio.max(height / depth);
    }
    (max_ratio, crest / 4.0)
}

fn pixel(
    x: u32,
    y: u32,
    eye: [f64; 3],
    forward: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    aspect: f64,
) -> ([f64; 3], f64) {
    if x < 8 && y < 8 {
        let energy = ggx_integral(0.35, 64);
        return ([energy, energy, energy], 1.0);
    }
    let dir = view_ray(x, y, forward, right, up, aspect);
    if y < 60 {
        let plane = if x < 80 {
            if (52..60).contains(&x) && y >= 46 {
                snell_plane()
            } else if y >= 46 && x < 16 {
                f0_plane()
            } else {
                air_plane()
            }
        } else {
            below_plane()
        };
        return shade_water(eye, dir, &plane);
    }
    if x < 80 {
        return shade_camera(x, y, &mirror_camera());
    }
    if x < 120 {
        return shade_camera(x, y, &portal_camera());
    }
    (ALBEDO, 1.0)
}

struct Camera {
    eye: [f64; 3],
    forward: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    plane_origin: [f64; 3],
    plane_normal: [f64; 3],
}

fn mirror_camera() -> Camera {
    let eye = [0.0, 1.5, 4.0];
    let forward = [0.0, 0.0, -1.0];
    let right = [1.0, 0.0, 0.0];
    let up = [0.0, 1.0, 0.0];
    let origin = [0.0, 1.5, 1.0];
    let normal = [0.0, 0.0, 1.0];
    Camera {
        eye: mirror_point(eye, origin, normal),
        forward: reflect(forward, normal),
        right: reflect(right, normal),
        up: reflect(up, normal),
        plane_origin: origin,
        plane_normal: normal,
    }
}

fn portal_camera() -> Camera {
    let eye = [0.0, 1.5, 4.0];
    let forward = [0.0, 0.0, -1.0];
    let right = [1.0, 0.0, 0.0];
    let up = [0.0, 1.0, 0.0];
    let origin = [1.15, 1.5, 2.4];
    let normal = [0.0, 0.0, 1.0];
    let looked = portal_map(eye);
    Camera {
        eye: looked,
        forward: normalize(sub(portal_map(add(eye, forward)), looked)),
        right: normalize(sub(portal_map(add(eye, right)), looked)),
        up: normalize(sub(portal_map(add(eye, up)), looked)),
        plane_origin: portal_map(origin),
        plane_normal: normalize(sub(portal_map(add(origin, normal)), portal_map(origin))),
    }
}

/// `to + to_basis * flip_z * transpose(from_basis) * (point - from)`.
fn portal_map(point: [f64; 3]) -> [f64; 3] {
    let origin = [1.15, 1.5, 2.4];
    let destination = [10.0, 1.5, 0.0];
    let from = [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
    let to = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
    let delta = sub(point, origin);
    let local = [
        dot(from[0], delta),
        dot(from[1], delta),
        dot(from[2], delta),
    ];
    let flipped = [local[0], local[1], -local[2]];
    add(
        destination,
        add(
            scale(to[0], flipped[0]),
            add(scale(to[1], flipped[1]), scale(to[2], flipped[2])),
        ),
    )
}

fn mirror_point(point: [f64; 3], origin: [f64; 3], normal: [f64; 3]) -> [f64; 3] {
    add(origin, reflect(sub(point, origin), normal))
}

fn shade_camera(x: u32, y: u32, camera: &Camera) -> ([f64; 3], f64) {
    let aspect = f64::from(WIDTH) / f64::from(HEIGHT);
    let dir = view_ray(x, y, camera.forward, camera.right, camera.up, aspect);
    let mut best: Option<(f64, [f64; 3])> = None;
    for plane in shade_planes() {
        let Some(travel) = hit_from(camera.eye, dir, &plane, 0.01) else {
            continue;
        };
        let at = add(camera.eye, scale(dir, travel));
        if !oblique_keeps(camera, at, aspect) {
            continue;
        }
        if best.is_none_or(|(best_t, _)| travel < best_t) {
            best = Some((travel, plane.color));
        }
    }
    best.map_or((MISS, 1.0), |(travel, color)| (color, travel.max(0.25)))
}

fn oblique_keeps(camera: &Camera, point: [f64; 3], aspect: f64) -> bool {
    let clip = mul_mat(
        oblique_proj(camera, aspect),
        [point[0], point[1], point[2], 1.0],
    );
    if clip[3] <= 1.0e-6 {
        return false;
    }
    let z = clip[2] / clip[3];
    (0.0..=1.0).contains(&z)
}

fn oblique_proj(camera: &Camera, aspect: f64) -> [f64; 16] {
    let mut proj = perspective(aspect);
    let rel = sub(camera.plane_origin, camera.eye);
    let origin_cam = [
        dot(camera.right, rel),
        dot(camera.up, rel),
        -dot(camera.forward, rel),
    ];
    let normal_cam = [
        dot(camera.right, camera.plane_normal),
        dot(camera.up, camera.plane_normal),
        -dot(camera.forward, camera.plane_normal),
    ];
    let plane = [
        normal_cam[0],
        normal_cam[1],
        normal_cam[2],
        -dot(normal_cam, origin_cam),
    ];
    let base = perspective(aspect);
    let corner = [sgn(plane[0]), sgn(plane[1]), 1.0, 1.0];
    let q = mul_mat(invert(base), corner);
    let denom = dot4(plane, q);
    let scale = if denom.abs() < 1.0e-12 {
        0.0
    } else {
        1.0 / denom
    };
    let row = scale4(plane, scale);
    proj[2] = row[0];
    proj[6] = row[1];
    proj[10] = row[2];
    proj[14] = row[3];
    let view = view_matrix(camera);
    mul_mats(proj, view)
}

fn perspective(aspect: f64) -> [f64; 16] {
    let near = 0.05;
    let far = 200.0;
    let h = 1.0 / (30.0_f64.to_radians() * 0.5).tan();
    let w = h / aspect.max(0.01);
    let mut proj = [0.0; 16];
    proj[0] = w;
    proj[5] = -h;
    proj[10] = far / (near - far);
    proj[11] = -1.0;
    proj[14] = (far * near) / (near - far);
    proj
}

fn view_matrix(camera: &Camera) -> [f64; 16] {
    let mut view = [0.0; 16];
    view[0] = camera.right[0];
    view[1] = camera.up[0];
    view[2] = -camera.forward[0];
    view[4] = camera.right[1];
    view[5] = camera.up[1];
    view[6] = -camera.forward[1];
    view[8] = camera.right[2];
    view[9] = camera.up[2];
    view[10] = -camera.forward[2];
    view[12] = -dot(camera.right, camera.eye);
    view[13] = -dot(camera.up, camera.eye);
    view[14] = dot(camera.forward, camera.eye);
    view[15] = 1.0;
    view
}

fn sgn(value: f64) -> f64 {
    if value < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn mul_mat(m: [f64; 16], v: [f64; 4]) -> [f64; 4] {
    let mut out = [0.0; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[row] += m[col * 4 + row] * v[col];
        }
    }
    out
}

fn mul_mats(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        let column = mul_mat(
            a,
            [b[col * 4], b[col * 4 + 1], b[col * 4 + 2], b[col * 4 + 3]],
        );
        for row in 0..4 {
            out[col * 4 + row] = column[row];
        }
    }
    out
}

#[expect(
    clippy::needless_range_loop,
    reason = "Gauss-Jordan reads one augmented row while it writes another"
)]
fn invert(m: [f64; 16]) -> [f64; 16] {
    let mut aug = [[0.0; 8]; 4];
    for row in 0..4 {
        for col in 0..4 {
            aug[row][col] = m[col * 4 + row];
            aug[row][4 + col] = if col == row { 1.0 } else { 0.0 };
        }
    }
    for col in 0..4 {
        let mut pivot = col;
        let mut best = aug[col][col].abs();
        for row in (col + 1)..4 {
            let value = aug[row][col].abs();
            if value > best {
                best = value;
                pivot = row;
            }
        }
        if best < 1.0e-12 {
            let mut identity = [0.0; 16];
            identity[0] = 1.0;
            identity[5] = 1.0;
            identity[10] = 1.0;
            identity[15] = 1.0;
            return identity;
        }
        if pivot != col {
            aug.swap(col, pivot);
        }
        let div = aug[col][col];
        for slot in 0..8 {
            aug[col][slot] /= div;
        }
        for row in 0..4 {
            if row == col {
                continue;
            }
            let factor = aug[row][col];
            for slot in 0..8 {
                aug[row][slot] -= factor * aug[col][slot];
            }
        }
    }
    let mut out = [0.0; 16];
    for row in 0..4 {
        for col in 0..4 {
            out[col * 4 + row] = aug[row][4 + col];
        }
    }
    out
}

fn dot4(a: [f64; 4], b: [f64; 4]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn scale4(a: [f64; 4], k: f64) -> [f64; 4] {
    [a[0] * k, a[1] * k, a[2] * k, a[3] * k]
}

fn shade_water(eye: [f64; 3], dir: [f64; 3], plane: &Plane) -> ([f64; 3], f64) {
    let Some(t_surf) = hit(eye, dir, plane) else {
        return (SKY, 8.0);
    };
    let surf = add(eye, scale(dir, t_surf));
    let geom = plane.normal;
    let below = dot(geom, sub(eye, surf)) < 0.0;
    let facing = if below { scale(geom, -1.0) } else { geom };
    let eta_ratio = if below { 1.0 / IOR } else { IOR };
    let incident = dir;
    let cos_i = dot(facing, scale(incident, -1.0)).clamp(0.0, 1.0);
    let fresnel = fresnel_dielectric(eta_ratio, cos_i);
    let reflected_dir = reflect(incident, facing);
    let eta_refract = 1.0 / eta_ratio;
    let (bent, transmitted_dir) = refract(incident, facing, eta_refract);
    let reflected = nearest(surf, reflected_dir).unwrap_or(SKY);
    // The transmitted sample is the opaque colour along the refracted ray.
    // Total internal reflection leaves no direction; the weight is then 1.
    let transmitted = if bent {
        nearest(surf, transmitted_dir).unwrap_or(plane.color)
    } else {
        plane.color
    };
    let mut beer_dist = 0.0;
    if let Some(t_hit) = nearest_t(eye, dir) {
        if t_hit > t_surf + 1.0e-4 {
            let hit_at = add(eye, scale(dir, t_hit));
            let thickness = dot(sub(hit_at, surf), facing).abs();
            let cos_t = if bent {
                dot(normalize(transmitted_dir), facing).abs().max(0.05)
            } else {
                1.0
            };
            beer_dist = thickness / cos_t;
        }
    }
    let absorb = (-SIGMA * beer_dist).exp();
    let weight = fresnel.clamp(0.0, 1.0);
    let rgb = add(
        scale(reflected, weight),
        scale(transmitted, (1.0 - weight) * absorb),
    );
    (rgb, t_surf.max(0.25))
}

fn view_ray(
    x: u32,
    y: u32,
    forward: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    aspect: f64,
) -> [f64; 3] {
    let ndc_x = (f64::from(x) + 0.5) / f64::from(WIDTH) * 2.0 - 1.0;
    let ndc_y = (f64::from(y) + 0.5) / f64::from(HEIGHT) * 2.0 - 1.0;
    let hx = ndc_x * aspect * TAN_HALF;
    let hy = -ndc_y * TAN_HALF;
    normalize(add(forward, add(scale(right, hx), scale(up, hy))))
}

fn fresnel_dielectric(eta: f64, cos_i: f64) -> f64 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let sin2_t = (1.0 - cos_i * cos_i).max(0.0) / (eta * eta);
    if sin2_t >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    let rs = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    let rp = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    f64::midpoint(rs * rs, rp * rp)
}

fn reflect(incident: [f64; 3], normal: [f64; 3]) -> [f64; 3] {
    let along = dot(normal, incident);
    sub(incident, scale(normal, 2.0 * along))
}

fn refract(incident: [f64; 3], normal: [f64; 3], eta: f64) -> (bool, [f64; 3]) {
    let along = dot(normal, incident);
    let k = 1.0 - eta * eta * (1.0 - along * along);
    if k < 0.0 {
        return (false, [0.0; 3]);
    }
    let scale_n = eta * along + k.sqrt();
    (true, sub(scale(incident, eta), scale(normal, scale_n)))
}

fn ggx_integral(alpha: f64, steps: u32) -> f64 {
    let mut sum = 0.0;
    let n = f64::from(steps);
    for i in 0..steps {
        let mu = (f64::from(i) + 0.5) / n;
        sum += ggx_ndf(mu, alpha) * mu / n;
    }
    sum * 2.0 * std::f64::consts::PI
}

fn ggx_ndf(mu: f64, alpha: f64) -> f64 {
    let mu = mu.clamp(0.0, 1.0);
    let a2 = alpha * alpha;
    let den = mu * mu * (a2 - 1.0) + 1.0;
    a2 / (std::f64::consts::PI * den * den)
}

/// Finch, GPU Gems, chapter 1. One component, time 0, speed 0.
fn gerstner_point(xz: [f64; 2]) -> [f64; 3] {
    let amplitude = 0.4;
    let wavelength = 8.0;
    let steepness = 0.5;
    let axis = [1.0, 0.0];
    let wave_number = std::f64::consts::TAU / wavelength;
    let phase = wave_number * (axis[0] * xz[0] + axis[1] * xz[1]);
    let sine = phase.sin();
    let cosine = phase.cos();
    [
        xz[0] + steepness * amplitude * axis[0] * cosine,
        amplitude * sine,
        xz[1] + steepness * amplitude * axis[1] * cosine,
    ]
}

fn nearest(origin: [f64; 3], dir: [f64; 3]) -> Option<[f64; 3]> {
    let mut best: Option<(f64, [f64; 3])> = None;
    for plane in shade_planes() {
        if let Some(t) = hit_from(origin, dir, &plane, 0.01) {
            if best.is_none_or(|(best_t, _)| t < best_t) {
                best = Some((t, plane.color));
            }
        }
    }
    best.map(|(_, color)| color)
}

fn nearest_t(origin: [f64; 3], dir: [f64; 3]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for plane in shade_planes() {
        if let Some(t) = hit_from(origin, dir, &plane, 0.05) {
            if best.is_none_or(|best_t| t < best_t) {
                best = Some(t);
            }
        }
    }
    best
}

fn hit(origin: [f64; 3], dir: [f64; 3], plane: &Plane) -> Option<f64> {
    hit_from(origin, dir, plane, 0.01)
}

fn hit_from(origin: [f64; 3], dir: [f64; 3], plane: &Plane, min_t: f64) -> Option<f64> {
    let denom = dot(plane.normal, dir);
    if denom.abs() < 1.0e-8 {
        return None;
    }
    let t = dot(plane.normal, sub(plane.origin, origin)) / denom;
    if t < min_t {
        return None;
    }
    let at = add(origin, scale(dir, t));
    let rel = sub(at, plane.origin);
    if dot(rel, plane.tangent).abs() > plane.half_u
        || dot(rel, plane.bitangent).abs() > plane.half_v
    {
        return None;
    }
    Some(t)
}

fn air_plane() -> Plane {
    Plane {
        origin: [-0.549_450_977, 1.910_349_464, 2.554_414_672],
        normal: [0.343_406_860, -0.256_468_415, 0.903_490_830],
        tangent: [-0.934_755_976, 0.0, 0.355_290_396],
        bitangent: [0.091_120_765, 0.966_552_612, 0.239_735_383],
        half_u: 12.0,
        half_v: 12.0,
        color: TRANS,
    }
}

fn below_plane() -> Plane {
    Plane {
        origin: [0.596_769_694, 1.934_684_098, 2.468_688_252],
        normal: [-0.630_530_365, 0.255_696_528, -0.732_837_461],
        tangent: [0.758_036_805, 0.0, -0.652_211_778],
        bitangent: [0.166_768_287, 0.966_757_097, 0.193_827_379],
        half_u: 12.0,
        half_v: 12.0,
        color: TRANS,
    }
}

fn snell_plane() -> Plane {
    Plane {
        origin: [-0.269_148_23, 1.589_716_1, 2.834_019_7],
        normal: [0.926_810_74, -0.042_882_52, 0.373_072_33],
        tangent: [0.373_415_83, 0.0, -0.927_664_1],
        bitangent: [0.039_780_576, 0.999_080_12, 0.016_013_011],
        half_u: 12.0,
        half_v: 12.0,
        color: TRANS,
    }
}

fn f0_plane() -> Plane {
    Plane {
        origin: [-1.023_523, 1.599_509_2, 2.522_671_8],
        normal: [0.568_623_9, -0.055_282_88, 0.820_737_9],
        tangent: [0.821_994_94, 0.0, -0.569_494_8],
        bitangent: [0.031_483_31, 0.998_470_7, 0.045_442_25],
        half_u: 12.0,
        half_v: 12.0,
        color: [0.0, 0.0, 0.0],
    }
}

fn shade_planes() -> [Plane; 9] {
    [
        Plane {
            origin: [-0.77, 0.92, 0.0],
            normal: [0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0],
            bitangent: [0.0, 1.0, 0.0],
            half_u: 1.05,
            half_v: 0.85,
            color: LEAK,
        },
        Plane {
            origin: [-2.75, -0.52, 5.0],
            normal: [0.0, 0.0, -1.0],
            tangent: [1.0, 0.0, 0.0],
            bitangent: [0.0, 1.0, 0.0],
            half_u: 2.85,
            half_v: 2.15,
            color: RED,
        },
        Plane {
            origin: [1.0, 1.5, 8.0],
            normal: [0.0, 0.0, -1.0],
            tangent: [1.0, 0.0, 0.0],
            bitangent: [0.0, 1.0, 0.0],
            half_u: 6.0,
            half_v: 4.0,
            color: REFL,
        },
        Plane {
            origin: [2.469_792_454, 2.861_310_052, -0.184_100_106],
            normal: [-0.630_530_365, 0.255_696_528, -0.732_837_461],
            tangent: [0.758_036_805, 0.0, -0.652_211_778],
            bitangent: [0.166_768_287, 0.966_757_097, 0.193_827_379],
            half_u: 4.55,
            half_v: 1.406,
            color: TRANS,
        },
        Plane {
            origin: [3.063_806_48, 1.929_368_31, 2.064_088_16],
            normal: [-0.986_814_714, 0.002_126_320, 0.161_840_041],
            tangent: [0.161_840_407, 0.0, 0.986_816_945],
            bitangent: [0.002_098_289, 0.999_997_739, -0.000_344_124],
            half_u: 1.4,
            half_v: 1.4,
            color: PINK,
        },
        Plane {
            origin: [12.0, 1.5, 0.0],
            normal: [-1.0, 0.0, 0.0],
            tangent: [0.0, 0.0, 1.0],
            bitangent: [0.0, 1.0, 0.0],
            half_u: 6.0,
            half_v: 4.0,
            color: BLUE,
        },
        Plane {
            origin: [-0.966_147_65, 1.593_955, 2.604_326_5],
            normal: [-0.568_908_6, 0.055_297_96, -0.820_539_5],
            tangent: [-0.821_797, 0.0, 0.569_780_4],
            bitangent: [0.031_507_7, 0.998_469_9, 0.045_443_7],
            half_u: 0.077_033_48,
            half_v: 0.079_672_84,
            color: [1.0, 1.0, 1.0],
        },
        Plane {
            origin: [-1.045_739_5, 1.601_692_2, 2.489_488_1],
            normal: [0.568_410_3, -0.055_271_56, 0.820_886_6],
            tangent: [0.822_143_3, 0.0, -0.569_280_5],
            bitangent: [0.031_465_02, 0.998_471_4, 0.045_441_15],
            half_u: 0.074_491_31,
            half_v: 0.077_033_76,
            color: [0.0, 0.0, 0.0],
        },
        Plane {
            origin: [-0.380_753_7, 1.627_173_8, 2.347_205_4],
            normal: [0.224_290_19, -0.074_763_395, 0.971_650_3],
            tangent: [0.974_377_33, 0.0, -0.224_919_68],
            bitangent: [0.016_815_759, 0.997_201_3, 0.072_847_754],
            half_u: 0.061_653_294,
            half_v: 0.064_625_71,
            color: OCC,
        },
    ]
}

fn idx(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
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

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let len = length(a);
    if len < 1.0e-12 {
        [0.0, 0.0, 1.0]
    } else {
        scale(a, 1.0 / len)
    }
}

#[cfg(test)]
mod tests {
    use super::{ggx_energy, optics_reference, shore_oracle, trace, HEIGHT, IOR, WIDTH};

    #[test]
    fn the_trace_paints_the_grade_regions() {
        let frame = trace();
        assert_eq!(
            frame.color.len(),
            usize::try_from(WIDTH * HEIGHT).unwrap_or(0)
        );
        let at = |x: u32, y: u32| frame.color[usize::try_from(y * WIDTH + x).unwrap_or(0)];
        let mirror = at(40, 90);
        assert!(mirror[0] > 0.8, "{mirror:?}");
        let portal = at(100, 90);
        assert!(portal[2] > portal[0], "{portal:?}");
        let extra = at(140, 90);
        assert!((extra[0] - 0.28).abs() < 1.0e-9, "{extra:?}");
        let below = at(120, 30);
        assert!(below[0] > 0.8, "below-water total reflection {below:?}");
        // Transmission hits the green backstop, so this pixel stays green.
        let snell = at(55, 51);
        assert!(
            snell[1] > snell[0] + 0.2,
            "refracted transmission {snell:?}"
        );
        let (crest, deep) = shore_oracle();
        assert!((crest - 0.78).abs() < 1.0e-12, "{crest}");
        assert!((deep - 0.1).abs() < 1.0e-12, "{deep}");
        let optics = optics_reference();
        let f0 = ((IOR - 1.0) / (IOR + 1.0)).powi(2);
        assert!((optics[3] - f0).abs() < 1.0e-12, "{}", optics[3]);
        assert!((optics[5] - 1.0).abs() < 1.0e-12, "{}", optics[5]);
        assert!(optics[16].abs() < 1.0e-12, "{}", optics[16]);
        let energy = ggx_energy();
        assert!((energy - 1.0).abs() < 1.0e-3, "{energy}");
    }
}
