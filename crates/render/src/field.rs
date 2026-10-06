use genos_scene::{Scene, Shape};

/// One distance range of the radiance field.
#[derive(Clone, Debug)]
pub struct CascadeLevel {
    pub spacing: f32,
    pub directions: u32,
    pub interval_start: f32,
    pub interval_end: f32,
    pub origin_x: f32,
    pub origin_z: f32,
    pub count_x: u32,
    pub count_z: u32,
    /// Radiance per probe, then per direction. Owned by this build only.
    pub radiance: Vec<[f32; 3]>,
    /// Transparency of that interval. `1` is a miss. `0` is a hit. Below `0` is an interior probe.
    pub beta: Vec<f32>,
    /// Mean merged radiance per probe. Empty until the field is sealed.
    pub merged: Vec<[f32; 3]>,
}

/// Near probe spacing, in meters. A larger floor keeps this cell until the buffer is full.
pub(crate) const TARGET_SPACING: f32 = 0.28;
/// Directions in the near cascade. Each coarser cascade doubles this count.
pub(crate) const FINE_DIRS: u32 = 16;
/// Texels reserved for one bounce inside the field buffer. The other bounce uses the next copy.
pub(crate) const FIELD_COPY: u32 = 524288;
/// Bounces after the direct pass. The picture runs the same count.
pub(crate) const BOUNCES: u32 = 4;
/// Coarse world-probe spacing. These probes update behind the screen field.
pub(crate) const WORLD_SPACING: f32 = 2.5;
/// Directions in the screen field. The pixel averages these.
#[allow(dead_code)]
pub(crate) const SCREEN_DIRS: u32 = 16;
/// One screen probe covers this many pixels.
pub(crate) const SCREEN_TILE: u32 = 16;
/// Caps so a 720p frame stays near an 80 by 45 gather.
/// `SCREEN_MAX_W * SCREEN_MAX_H * SCREEN_DIRS` stays below the world-cache texel.
pub(crate) const SCREEN_MAX_W: u32 = 80;
pub(crate) const SCREEN_MAX_H: u32 = 45;

/// Screen-probe counts for a viewport. The cost stays fixed as the world grows.
pub(crate) fn screen_grid(width: u32, height: u32) -> (u32, u32) {
    (
        (width / SCREEN_TILE).clamp(1, SCREEN_MAX_W),
        (height / SCREEN_TILE).clamp(1, SCREEN_MAX_H),
    )
}

/// Grid size of the screen field. A match keeps that field.
/// A camera move does not rebuild it. The fragment keeps the camera that built it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenKey {
    pub grid_w: u32,
    pub grid_h: u32,
}

/// Key for the screen gather. A camera move does not rebuild it.
pub(crate) fn screen_key(width: u32, height: u32) -> ScreenKey {
    let (grid_w, grid_h) = screen_grid(width, height);
    ScreenKey { grid_w, grid_h }
}

/// World-probe counts for a floor. The margin is two cells on each side.
pub(crate) fn world_counts(half_x: f32, half_z: f32) -> (u32, u32) {
    let span_x = (half_x * 2.0).max(1.0);
    let span_z = (half_z * 2.0).max(1.0);
    let margin = WORLD_SPACING * 2.0;
    let count_x = ((span_x + margin * 2.0) / WORLD_SPACING).ceil().max(1.0) as u32;
    let count_z = ((span_z + margin * 2.0) / WORLD_SPACING).ceil().max(1.0) as u32;
    (count_x, count_z)
}

/// One cascade in the paper's doubling sequence.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CascadePlan {
    pub spacing: f32,
    pub dirs: u32,
    pub t0: f32,
    pub t1: f32,
    pub origin_x: f32,
    pub origin_z: f32,
    pub count_x: u32,
    pub count_z: u32,
    pub offset: u32,
}

/// Near, far, and world plans for a floor. Spacing, directions, and interval length follow `2^i`.
/// The world plan starts four probes outside the floor on each side.
pub(crate) fn cascade_plans(floor: &genos_scene::Floor) -> [CascadePlan; 3] {
    let mut spacing0 = TARGET_SPACING;
    loop {
        let plans = plans_at(floor, spacing0);
        let used = plans[2].offset + plans[2].count_x * plans[2].count_z * plans[2].dirs;
        if used <= FIELD_COPY || spacing0 > 2.0 {
            debug_assert!(
                used <= FIELD_COPY,
                "cascade texels {used} exceed {FIELD_COPY}"
            );
            return plans;
        }
        spacing0 *= 1.25;
    }
}

fn plans_at(floor: &genos_scene::Floor, spacing0: f32) -> [CascadePlan; 3] {
    let span_x = (floor.half_x * 2.0).max(1.0);
    let span_z = (floor.half_z * 2.0).max(1.0);
    let span = span_x.max(span_z);
    // One near cell. The paper's near interval stays on the order of the probe spacing.
    let interval0 = spacing0;
    let min_x = floor.position.x - floor.half_x;
    let min_z = floor.position.z - floor.half_z;
    let mut offset = 0u32;
    let mut plans = [CascadePlan {
        spacing: spacing0,
        dirs: FINE_DIRS,
        t0: 0.0,
        t1: interval0,
        origin_x: min_x,
        origin_z: min_z,
        count_x: 1,
        count_z: 1,
        offset: 0,
    }; 3];
    for i in 0..3 {
        let scale = 1u32 << i;
        let spacing = spacing0 * scale as f32;
        let dirs = FINE_DIRS * scale;
        let t0 = if i == 0 {
            0.0
        } else {
            interval0 * (scale as f32 * 0.5)
        };
        // The last interval runs to the domain edge. Earlier intervals double.
        let doubled = interval0 * scale as f32;
        let t1 = if i + 1 == 3 {
            doubled.max(span * 4.0)
        } else {
            doubled
        };
        let (origin_x, origin_z, count_x, count_z) = if i < 2 {
            (
                min_x,
                min_z,
                (span_x / spacing).ceil().max(1.0) as u32,
                (span_z / spacing).ceil().max(1.0) as u32,
            )
        } else {
            let margin = spacing * 4.0;
            (
                min_x - margin,
                min_z - margin,
                ((span_x + margin * 2.0) / spacing).ceil().max(1.0) as u32,
                ((span_z + margin * 2.0) / spacing).ceil().max(1.0) as u32,
            )
        };
        plans[i] = CascadePlan {
            spacing,
            dirs,
            t0,
            t1,
            origin_x,
            origin_z,
            count_x,
            count_z,
            offset,
        };
        offset = offset.saturating_add(count_x.saturating_mul(count_z).saturating_mul(dirs));
    }
    plans
}

/// Near range, far range, and world probes. A build does not keep a previous frame.
///
/// World probes sit on a coarse grid in the world, past the floor, and carry
/// material-colored light that the nearer ranges do not reach.
#[derive(Clone, Debug)]
pub struct Field {
    pub near: CascadeLevel,
    pub far: CascadeLevel,
    pub world: CascadeLevel,
}

pub fn probe_counts(field: &Field) -> (u32, u32) {
    (
        field.near.count_x * field.near.count_z,
        field.far.count_x * field.far.count_z,
    )
}

/// Build the field from the scene. The previous frame is not an input.
///
/// The first pass records light that leaves each material under the lamps.
/// Each later pass bounces that light once and reads only the previous pass.
pub fn build(scene: &Scene) -> Field {
    let mut field = build_field(scene, None);
    for _ in 1..BOUNCES {
        field = build_field(scene, Some(&field));
    }
    field
}

pub fn ray_count(field: &Field) -> u64 {
    let level = |level: &CascadeLevel| {
        level.count_x as u64 * level.count_z as u64 * level.directions as u64
    };
    (level(&field.near) + level(&field.far) + level(&field.world)) * u64::from(BOUNCES)
}

/// Merged radiance at a ground position.
///
/// Each direction merges `L + β L_next`. A miss is 0 with β = 1, so the next interval replaces it.
/// The result is the mean of the near directions.
pub fn sample(field: &Field, x: f32, z: f32) -> [f32; 3] {
    merged_uniform(field, x, z)
}

/// Radiance stored on the world probes only, before the near and far ranges.
pub fn sample_world(field: &Field, x: f32, z: f32) -> [f32; 3] {
    bilinear(&field.world, x, z)
}

/// Scalar light reaching a point, from the scene lights only.
/// No lamp means zero. There is no ambient term.
/// A wall or solid between the point and a lamp stops that lamp.
pub fn illuminate(scene: &Scene, x: f32, y: f32, z: f32) -> f32 {
    illuminate_facing(scene, x, y, z, [0.0, 0.0, 0.0])
}

/// Like [`illuminate`], but a face with a normal receives a lamp only when it points toward that lamp.
pub fn illuminate_facing(scene: &Scene, x: f32, y: f32, z: f32, normal: [f32; 3]) -> f32 {
    let mut incoming = 0.0;
    for light in &scene.lights {
        incoming += one_lamp(scene, light, x, y, z, normal);
    }
    incoming
}

fn one_lamp(
    scene: &Scene,
    light: &genos_scene::Light,
    x: f32,
    y: f32,
    z: f32,
    normal: [f32; 3],
) -> f32 {
    let use_facing = normal[0] != 0.0 || normal[1] != 0.0 || normal[2] != 0.0;
    let (dx, dy, dz, dist2, reach) = lamp_ray(light, x, y, z);
    if use_facing && dx * normal[0] + dy * normal[1] + dz * normal[2] <= 0.0 {
        return 0.0;
    }
    if lamp_is_blocked(scene, x, y, z, [dx, dy, dz], reach) {
        return 0.0;
    }
    let strength = (light.color[0] + light.color[1] + light.color[2]) / 3.0;
    lamp_reach(dist2, strength, dx, dy, dz, normal)
}

/// Direction toward the source, the falloff distance squared, and the shadow-ray length.
/// A sun uses the same falloff as a point lamp 7 m away.
fn lamp_ray(light: &genos_scene::Light, x: f32, y: f32, z: f32) -> (f32, f32, f32, f32, f32) {
    let span = light.direction.x * light.direction.x
        + light.direction.y * light.direction.y
        + light.direction.z * light.direction.z;
    if span > 1.0e-8 {
        let len = span.sqrt();
        // Scale to length 7 so the cosine uses the same distance as the 7 m falloff.
        let scale = 7.0 / len;
        return (
            -light.direction.x * scale,
            -light.direction.y * scale,
            -light.direction.z * scale,
            49.0,
            80.0,
        );
    }
    let dx = light.position.x - x;
    let dy = light.position.y - y;
    let dz = light.position.z - z;
    let dist2 = dx * dx + dy * dy + dz * dz;
    (dx, dy, dz, dist2, dist2.sqrt())
}

/// Scene-lamp unit. The falloff is cosine over inverse-square.
/// A unit white lamp 7 m above a white floor stays near 0.46 after reflectance.
/// A lamp 1 m away stays under white.
const LAMP_UNIT: f32 = 72.0;

/// Cosine over inverse-square falloff. A face with no normal uses the distance term only.
fn lamp_reach(dist2: f32, strength: f32, dx: f32, dy: f32, dz: f32, normal: [f32; 3]) -> f32 {
    let facing = normal[0] != 0.0 || normal[1] != 0.0 || normal[2] != 0.0;
    let mut shade = 1.0;
    if facing {
        let dist = dist2.sqrt().max(1.0e-4);
        let nd = (dx * normal[0] + dy * normal[1] + dz * normal[2]) / dist;
        if nd <= 0.0 {
            return 0.0;
        }
        shade = nd;
    }
    strength * shade * LAMP_UNIT / (1.0 + dist2)
}

fn lamp_is_blocked(scene: &Scene, x: f32, y: f32, z: f32, toward: [f32; 3], reach: f32) -> bool {
    if covered_by_object(scene, x, y, z) {
        return true;
    }
    let dist = (toward[0] * toward[0] + toward[1] * toward[1] + toward[2] * toward[2]).sqrt();
    if dist < 1.0e-3 || reach < 1.0e-3 {
        return false;
    }
    let dir = [toward[0] / dist, toward[1] / dist, toward[2] / dist];
    let dist = reach;
    let origin = [x, y, z];
    for solid in &scene.solids {
        if point_in_solid(x, y, z, solid) {
            continue;
        }
        if let Some(t) = hit_solid_3d(origin, dir, solid) {
            if t > 1.0e-4 && t < dist - 1.0e-4 {
                return true;
            }
        }
    }
    for wall in &scene.walls {
        let min = [
            wall.position.x - wall.half_x,
            0.0,
            wall.position.z - wall.half_z,
        ];
        let max = [
            wall.position.x + wall.half_x,
            wall.height,
            wall.position.z + wall.half_z,
        ];
        if point_in_box(x, y, z, min, max) {
            continue;
        }
        if let Some(t) = hit_box(origin, dir, min, max) {
            if t > 1.0e-4 && t < dist - 1.0e-4 {
                return true;
            }
        }
    }
    false
}

/// A ground point inside an object's footprint gets no lamp. The lamp side, outside that footprint, stays lit.
fn covered_by_object(scene: &Scene, x: f32, y: f32, z: f32) -> bool {
    if y < -0.001 || y >= 0.02 {
        return false;
    }
    for solid in &scene.solids {
        if y > solid.height {
            continue;
        }
        if solid.contains_xz(x, z) {
            return true;
        }
    }
    for wall in &scene.walls {
        if y > wall.height {
            continue;
        }
        if (x - wall.position.x).abs() <= wall.half_x && (z - wall.position.z).abs() <= wall.half_z
        {
            return true;
        }
    }
    false
}

/// Direct light when `blocked` lights are already known to miss this point.
/// Lights marked in `resolved` do not get another ray test.
pub(crate) fn illuminate_resolved(
    scene: &Scene,
    x: f32,
    y: f32,
    z: f32,
    normal: [f32; 3],
    blocked: u32,
    resolved: u32,
) -> f32 {
    let use_facing = normal[0] != 0.0 || normal[1] != 0.0 || normal[2] != 0.0;
    let mut incoming = 0.0;
    for (index, light) in scene.lights.iter().enumerate() {
        let bit = if index < 32 { 1u32 << index } else { 0 };
        if bit != 0 && blocked & bit != 0 {
            continue;
        }
        let (dx, dy, dz, dist2, reach) = lamp_ray(light, x, y, z);
        if use_facing && dx * normal[0] + dy * normal[1] + dz * normal[2] <= 0.0 {
            continue;
        }
        let known = bit != 0 && resolved & bit != 0;
        if !known && lamp_is_blocked(scene, x, y, z, [dx, dy, dz], reach) {
            continue;
        }
        let strength = (light.color[0] + light.color[1] + light.color[2]) / 3.0;
        incoming += lamp_reach(dist2, strength, dx, dy, dz, normal);
    }
    incoming
}

fn point_in_solid(x: f32, y: f32, z: f32, solid: &genos_scene::Solid) -> bool {
    if y <= 0.001 || y >= solid.height {
        return false;
    }
    solid.contains_xz(x, z)
}

fn point_in_box(x: f32, y: f32, z: f32, min: [f32; 3], max: [f32; 3]) -> bool {
    x > min[0] && x < max[0] && y > min[1] + 0.001 && y < max[1] && z > min[2] && z < max[2]
}

fn hit_solid_3d(origin: [f32; 3], dir: [f32; 3], solid: &genos_scene::Solid) -> Option<f32> {
    let half = solid.size * 0.5;
    match solid.shape {
        Shape::Square => hit_box(
            origin,
            dir,
            [solid.position.x - half, 0.0, solid.position.z - half],
            [
                solid.position.x + half,
                solid.height,
                solid.position.z + half,
            ],
        ),
        Shape::Circle => hit_cylinder(
            origin,
            dir,
            solid.position.x,
            solid.position.z,
            half,
            0.0,
            solid.height,
        ),
    }
}

fn hit_box(origin: [f32; 3], dir: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Option<f32> {
    let mut t_enter = 0.0_f32;
    let mut t_exit = f32::INFINITY;
    for axis in 0..3 {
        if dir[axis].abs() < 1.0e-8 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[axis];
        let (mut t1, mut t2) = (
            (min[axis] - origin[axis]) * inv,
            (max[axis] - origin[axis]) * inv,
        );
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        t_enter = t_enter.max(t1);
        t_exit = t_exit.min(t2);
        if t_exit < t_enter {
            return None;
        }
    }
    if t_exit < 0.0 {
        return None;
    }
    if t_enter >= 0.0 {
        Some(t_enter)
    } else {
        None
    }
}

fn hit_cylinder(
    origin: [f32; 3],
    dir: [f32; 3],
    cx: f32,
    cz: f32,
    radius: f32,
    y0: f32,
    y1: f32,
) -> Option<f32> {
    let ox = origin[0] - cx;
    let oz = origin[2] - cz;
    let a = dir[0] * dir[0] + dir[2] * dir[2];
    let mut best: Option<f32> = None;
    if a > 1.0e-8 {
        let b = ox * dir[0] + oz * dir[2];
        let c = ox * ox + oz * oz - radius * radius;
        let disc = b * b - a * c;
        if disc >= 0.0 {
            let root = disc.sqrt();
            for t in [(-b - root) / a, (-b + root) / a] {
                if t < 0.0 {
                    continue;
                }
                let y = origin[1] + dir[1] * t;
                if y >= y0 && y <= y1 {
                    best = Some(best.map(|old| old.min(t)).unwrap_or(t));
                }
            }
        }
    }
    if dir[1].abs() > 1.0e-8 {
        for cap in [y0, y1] {
            let t = (cap - origin[1]) / dir[1];
            if t < 0.0 {
                continue;
            }
            let x = origin[0] + dir[0] * t;
            let z = origin[2] + dir[2] * t;
            let dx = x - cx;
            let dz = z - cz;
            if dx * dx + dz * dz <= radius * radius {
                best = Some(best.map(|old| old.min(t)).unwrap_or(t));
            }
        }
    }
    best
}

/// Floor mesh cell, in meters. The radiance field uses its own probe spacing.
pub(crate) const MESH_CELL: f32 = 0.16;

fn build_field(scene: &Scene, prev: Option<&Field>) -> Field {
    let plans = cascade_plans(&scene.floor);
    let near = build_level(scene, &plans[0], prev);
    let far = build_level(scene, &plans[1], prev);
    let world = build_level(scene, &plans[2], prev);
    let mut field = Field { near, far, world };
    seal(&mut field);
    field
}

fn build_level(scene: &Scene, plan: &CascadePlan, prev: Option<&Field>) -> CascadeLevel {
    let count = (plan.count_x * plan.count_z * plan.dirs) as usize;
    let mut radiance = Vec::with_capacity(count);
    let mut beta = Vec::with_capacity(count);
    for iz in 0..plan.count_z {
        for ix in 0..plan.count_x {
            let x = plan.origin_x + (ix as f32 + 0.5) * plan.spacing;
            let z = plan.origin_z + (iz as f32 + 0.5) * plan.spacing;
            for dir in 0..plan.dirs {
                let angle = (dir as f32 + 0.5) * std::f32::consts::TAU / plan.dirs as f32;
                let direction = [angle.cos(), angle.sin()];
                let (color, hit_beta) = gather(scene, prev, [x, z], direction, plan.t0, plan.t1);
                radiance.push(color);
                beta.push(hit_beta);
            }
        }
    }
    CascadeLevel {
        spacing: plan.spacing,
        directions: plan.dirs,
        interval_start: plan.t0,
        interval_end: plan.t1,
        origin_x: plan.origin_x,
        origin_z: plan.origin_z,
        count_x: plan.count_x,
        count_z: plan.count_z,
        radiance,
        beta,
        merged: Vec::new(),
    }
}

fn seal(field: &mut Field) {
    let mut near_merged = Vec::with_capacity((field.near.count_x * field.near.count_z) as usize);
    for iz in 0..field.near.count_z {
        for ix in 0..field.near.count_x {
            let x = field.near.origin_x + (ix as f32 + 0.5) * field.near.spacing;
            let z = field.near.origin_z + (iz as f32 + 0.5) * field.near.spacing;
            near_merged.push(merged_uniform(field, x, z));
        }
    }
    field.near.merged = near_merged;
    let world_probes = (field.world.count_x * field.world.count_z) as usize;
    field.world.merged = (0..world_probes)
        .map(|probe| average_probe(&field.world, probe))
        .collect();
}

fn bilinear(level: &CascadeLevel, x: f32, z: f32) -> [f32; 3] {
    if level.merged.is_empty() || level.count_x == 0 || level.count_z == 0 {
        return [0.0; 3];
    }
    let max_x = (level.count_x - 1) as f32;
    let max_z = (level.count_z - 1) as f32;
    let fx = ((x - level.origin_x) / level.spacing - 0.5).clamp(0.0, max_x);
    let fz = ((z - level.origin_z) / level.spacing - 0.5).clamp(0.0, max_z);
    let x0 = fx.floor() as u32;
    let z0 = fz.floor() as u32;
    let x1 = (x0 + 1).min(level.count_x - 1);
    let z1 = (z0 + 1).min(level.count_z - 1);
    let tx = fx - x0 as f32;
    let tz = fz - z0 as f32;
    let at = |ix: u32, iz: u32| level.merged[(iz * level.count_x + ix) as usize];
    let c00 = at(x0, z0);
    let c10 = at(x1, z0);
    let c01 = at(x0, z1);
    let c11 = at(x1, z1);
    let mut out = [0.0; 3];
    for channel in 0..3 {
        let a = c00[channel] + (c10[channel] - c00[channel]) * tx;
        let b = c01[channel] + (c11[channel] - c01[channel]) * tx;
        out[channel] = a + (b - a) * tz;
    }
    out
}

/// Mean of the merged directions. A floor point uses every direction.
fn merged_uniform(field: &Field, x: f32, z: f32) -> [f32; 3] {
    let directions = field.near.directions.max(1);
    let mut sum = [0.0; 3];
    let mut weight = 0.0;
    for dir in 0..directions {
        let angle = (dir as f32 + 0.5) * std::f32::consts::TAU / directions as f32;
        let color = merged_direction(field, x, z, angle);
        sum[0] += color[0];
        sum[1] += color[1];
        sum[2] += color[2];
        weight += 1.0;
    }
    if weight <= 0.0 {
        return [0.0; 3];
    }
    [sum[0] / weight, sum[1] / weight, sum[2] / weight]
}

/// Light arriving on a vertical face. Directions behind the normal contribute nothing.
fn merged_facing(field: &Field, x: f32, z: f32, normal: [f32; 2]) -> [f32; 3] {
    let directions = field.near.directions.max(1);
    let mut sum = [0.0; 3];
    let mut weight = 0.0;
    for dir in 0..directions {
        let angle = (dir as f32 + 0.5) * std::f32::consts::TAU / directions as f32;
        let direction = [angle.cos(), angle.sin()];
        let facing = direction[0] * normal[0] + direction[1] * normal[1];
        if facing <= 0.0 {
            continue;
        }
        let color = merged_direction(field, x, z, angle);
        sum[0] += color[0] * facing;
        sum[1] += color[1] * facing;
        sum[2] += color[2] * facing;
        weight += facing;
    }
    if weight <= 1.0e-4 {
        return [0.0; 3];
    }
    [sum[0] / weight, sum[1] / weight, sum[2] / weight]
}

/// `L_near + β_near (L_far + β_far L_world)` at one angle.
fn merged_direction(field: &Field, x: f32, z: f32, angle: f32) -> [f32; 3] {
    let (near, near_beta) = sample_interval(&field.near, x, z, angle);
    let (far, far_beta) = sample_interval(&field.far, x, z, angle);
    let (world, _) = sample_interval(&field.world, x, z, angle);
    [
        near[0] + near_beta * (far[0] + far_beta * world[0]),
        near[1] + near_beta * (far[1] + far_beta * world[1]),
        near[2] + near_beta * (far[2] + far_beta * world[2]),
    ]
}

fn sample_interval(level: &CascadeLevel, x: f32, z: f32, angle: f32) -> ([f32; 3], f32) {
    if level.count_x == 0 || level.count_z == 0 || level.beta.is_empty() {
        return ([0.0; 3], 1.0);
    }
    let max_x = (level.count_x - 1) as f32;
    let max_z = (level.count_z - 1) as f32;
    let fx = ((x - level.origin_x) / level.spacing - 0.5).clamp(0.0, max_x);
    let fz = ((z - level.origin_z) / level.spacing - 0.5).clamp(0.0, max_z);
    let x0 = fx.floor() as u32;
    let z0 = fz.floor() as u32;
    let x1 = (x0 + 1).min(level.count_x - 1);
    let z1 = (z0 + 1).min(level.count_z - 1);
    let tx = fx - x0 as f32;
    let tz = fz - z0 as f32;
    let corners = [
        (x0, z0, (1.0 - tx) * (1.0 - tz)),
        (x1, z0, tx * (1.0 - tz)),
        (x0, z1, (1.0 - tx) * tz),
        (x1, z1, tx * tz),
    ];
    let mut color = [0.0; 3];
    let mut beta = 0.0;
    let mut weight = 0.0;
    for (ix, iz, corner_weight) in corners {
        if corner_weight <= 1.0e-6 {
            continue;
        }
        let probe = (iz * level.count_x + ix) as usize;
        let (sample, sample_beta) = probe_angle(level, probe, angle);
        if sample_beta < 0.0 {
            continue;
        }
        color[0] += sample[0] * corner_weight;
        color[1] += sample[1] * corner_weight;
        color[2] += sample[2] * corner_weight;
        beta += sample_beta * corner_weight;
        weight += corner_weight;
    }
    if weight <= 1.0e-6 {
        return ([0.0; 3], 1.0);
    }
    (
        [color[0] / weight, color[1] / weight, color[2] / weight],
        beta / weight,
    )
}

fn probe_angle(level: &CascadeLevel, probe: usize, angle: f32) -> ([f32; 3], f32) {
    let n = level.directions.max(1);
    let f = angle / std::f32::consts::TAU * n as f32 - 0.5;
    let i0 = f.floor();
    let t = (f - i0).clamp(0.0, 1.0);
    let a = wrap_index(i0, n);
    let b = wrap_index(i0 + 1.0, n);
    let (c0, b0) = probe_slot(level, probe, a);
    let (c1, b1) = probe_slot(level, probe, b);
    if b0 < 0.0 {
        return (c1, b1);
    }
    if b1 < 0.0 {
        return (c0, b0);
    }
    (
        [
            c0[0] + (c1[0] - c0[0]) * t,
            c0[1] + (c1[1] - c0[1]) * t,
            c0[2] + (c1[2] - c0[2]) * t,
        ],
        b0 + (b1 - b0) * t,
    )
}

fn wrap_index(index: f32, count: u32) -> u32 {
    let count = count.max(1) as i32;
    (index.floor() as i32).rem_euclid(count) as u32
}

fn probe_slot(level: &CascadeLevel, probe: usize, dir: u32) -> ([f32; 3], f32) {
    let slot = probe * level.directions as usize + dir as usize;
    (level.radiance[slot], level.beta[slot])
}

fn average_probe(level: &CascadeLevel, probe: usize) -> [f32; 3] {
    let start = probe * level.directions as usize;
    let mut sum = [0.0; 3];
    for sample in &level.radiance[start..start + level.directions as usize] {
        sum[0] += sample[0];
        sum[1] += sample[1];
        sum[2] += sample[2];
    }
    let n = level.directions as f32;
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

fn gather(
    scene: &Scene,
    prev: Option<&Field>,
    origin: [f32; 2],
    dir: [f32; 2],
    t0: f32,
    t1: f32,
) -> ([f32; 3], f32) {
    if xz_inside(scene, origin[0], origin[1]) {
        return ([0.0; 3], -1.0);
    }
    let mut best_t = t1;
    let mut color = [0.0; 3];
    let mut found = false;
    for solid in &scene.solids {
        if let Some((t, normal)) = hit_solid(origin, dir, solid) {
            if t >= t0 && t < best_t {
                best_t = t;
                let half = solid.size * 0.5;
                let footprint = match solid.shape {
                    Shape::Square => Footprint::Box {
                        half_x: half,
                        half_z: half,
                    },
                    Shape::Circle => Footprint::Circle { radius: half },
                };
                color = leaving(
                    scene,
                    prev,
                    solid.color,
                    solid.reflectance,
                    solid.color_mix,
                    [origin[0] + dir[0] * t, origin[1] + dir[1] * t],
                    normal,
                    [solid.position.x, solid.position.z],
                    footprint,
                    solid.height,
                );
                found = true;
            }
        }
    }
    for wall in &scene.walls {
        if let Some((t, normal)) = hit_aabb(
            origin,
            dir,
            wall.position.x - wall.half_x,
            wall.position.z - wall.half_z,
            wall.position.x + wall.half_x,
            wall.position.z + wall.half_z,
        ) {
            if t >= t0 && t < best_t {
                best_t = t;
                color = leaving(
                    scene,
                    prev,
                    wall.color,
                    wall.reflectance,
                    wall.color_mix,
                    [origin[0] + dir[0] * t, origin[1] + dir[1] * t],
                    normal,
                    [wall.position.x, wall.position.z],
                    Footprint::Box {
                        half_x: wall.half_x,
                        half_z: wall.half_z,
                    },
                    wall.height,
                );
                found = true;
            }
        }
    }
    if found {
        (color, 0.0)
    } else {
        ([0.0; 3], 1.0)
    }
}

fn xz_inside(scene: &Scene, x: f32, z: f32) -> bool {
    scene.solids.iter().any(|solid| solid.contains_xz(x, z))
        || scene.walls.iter().any(|wall| {
            (x - wall.position.x).abs() <= wall.half_x && (z - wall.position.z).abs() <= wall.half_z
        })
}

#[derive(Clone, Copy)]
enum Footprint {
    Box { half_x: f32, half_z: f32 },
    Circle { radius: f32 },
}

fn lamp_over(light: &genos_scene::Light, center: [f32; 2], footprint: Footprint) -> bool {
    let dx = light.position.x - center[0];
    let dz = light.position.z - center[1];
    match footprint {
        Footprint::Box { half_x, half_z } => dx.abs() <= half_x && dz.abs() <= half_z,
        Footprint::Circle { radius } => dx * dx + dz * dz <= radius * radius,
    }
}

/// Light leaving a hit face.
///
/// A face that points toward a lamp stores that lamp. A lamp above the object stores light on every side, so the floor around the object keeps the color. A lamp on one side does not store light on the far face.
fn leaving(
    scene: &Scene,
    prev: Option<&Field>,
    albedo: [f32; 3],
    reflectance: f32,
    color_mix: f32,
    hit: [f32; 2],
    normal: [f32; 2],
    center: [f32; 2],
    footprint: Footprint,
    height: f32,
) -> [f32; 3] {
    let mut direct = 0.0;
    for light in &scene.lights {
        let face =
            (light.position.x - hit[0]) * normal[0] + (light.position.z - hit[1]) * normal[1];
        let over = lamp_over(light, center, footprint);
        if face <= 0.0 && !over {
            continue;
        }
        direct += if face > 0.0 {
            let gap = 1.0e-3;
            one_lamp(
                scene,
                light,
                hit[0] + normal[0] * gap,
                height * 0.5,
                hit[1] + normal[1] * gap,
                [normal[0], 0.0, normal[1]],
            )
        } else {
            // A capped share of the top. The side does not become the lamp.
            one_lamp(
                scene,
                light,
                center[0],
                height + 0.05,
                center[1],
                [0.0, 1.0, 0.0],
            )
            .min(2.0)
                * 0.2
        };
    }
    let incoming = prev
        .map(|field| merged_facing(field, hit[0], hit[1], normal))
        .unwrap_or([0.0; 3]);
    genos_scene::bounce_radiance(albedo, reflectance, color_mix, direct, incoming)
}

fn hit_solid(
    origin: [f32; 2],
    dir: [f32; 2],
    solid: &genos_scene::Solid,
) -> Option<(f32, [f32; 2])> {
    let half = solid.size * 0.5;
    match solid.shape {
        Shape::Square => hit_aabb(
            origin,
            dir,
            solid.position.x - half,
            solid.position.z - half,
            solid.position.x + half,
            solid.position.z + half,
        ),
        Shape::Circle => hit_circle(origin, dir, solid.position.x, solid.position.z, half),
    }
}

fn hit_aabb(
    origin: [f32; 2],
    dir: [f32; 2],
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
) -> Option<(f32, [f32; 2])> {
    let inv_x = if dir[0].abs() < 1.0e-8 {
        f32::INFINITY
    } else {
        1.0 / dir[0]
    };
    let inv_z = if dir[1].abs() < 1.0e-8 {
        f32::INFINITY
    } else {
        1.0 / dir[1]
    };
    let (tx1, tx2, nx) = slab(origin[0], inv_x, min_x, max_x);
    let (tz1, tz2, nz) = slab(origin[1], inv_z, min_z, max_z);
    let t_enter = tx1.max(tz1);
    let t_exit = tx2.min(tz2);
    if t_exit < t_enter || t_exit < 0.0 || t_enter < 0.0 {
        return None;
    }
    let normal = if tx1 >= tz1 { [nx, 0.0] } else { [0.0, nz] };
    Some((t_enter, normal))
}

fn slab(origin: f32, inv_dir: f32, min_v: f32, max_v: f32) -> (f32, f32, f32) {
    let a = (min_v - origin) * inv_dir;
    let b = (max_v - origin) * inv_dir;
    if a < b {
        (a, b, -1.0)
    } else {
        (b, a, 1.0)
    }
}

fn hit_circle(
    origin: [f32; 2],
    dir: [f32; 2],
    cx: f32,
    cz: f32,
    radius: f32,
) -> Option<(f32, [f32; 2])> {
    let ox = origin[0] - cx;
    let oz = origin[1] - cz;
    let b = ox * dir[0] + oz * dir[1];
    let c = ox * ox + oz * oz - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t0 = -b - root;
    if t0 < 0.0 {
        return None;
    }
    let px = origin[0] + dir[0] * t0;
    let pz = origin[1] + dir[1] * t0;
    let nx = px - cx;
    let nz = pz - cz;
    let len = (nx * nx + nz * nz).sqrt().max(1.0e-8);
    Some((t0, [nx / len, nz / len]))
}

#[cfg(test)]
mod tests {
    use super::illuminate_facing;
    use crate::mesh::compose;
    use genos_scene::{Floor, Light, Scene, Vec3};

    fn one_lamp(y: f32) -> Scene {
        Scene {
            floor: Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 8.0,
                half_z: 8.0,
                color: [1.0, 1.0, 1.0],
            },
            walls: Vec::new(),
            solids: Vec::new(),
            lights: vec![Light {
                position: Vec3::new(0.0, y, 0.0),
                color: [1.0, 1.0, 1.0],

                direction: Vec3::ZERO,
            }],
        }
    }

    fn shaded_floor(y: f32) -> f32 {
        let direct = illuminate_facing(&one_lamp(y), 0.0, 0.0, 0.0, [0.0, 1.0, 0.0]);
        compose([1.0, 1.0, 1.0], direct, [0.0, 0.0, 0.0], true)[0]
    }

    #[test]
    fn a_unit_lamp_at_the_shipped_height_lights_a_white_floor() {
        let room = shaded_floor(7.0);
        let close = shaded_floor(1.0);
        assert!(
            (0.42..0.52).contains(&room),
            "a unit lamp 7 m up left the white floor dull or clipped: {room}"
        );
        assert!(
            close > room && close < 0.85,
            "a lamp 1 m away is not brighter, or it is flat white: close {close} room {room}"
        );
    }
}
