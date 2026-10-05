use crate::types::{Scene, Shape};

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
    /// One merged color per probe. Empty until the field is sealed.
    pub merged: Vec<[f32; 3]>,
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
/// The first pass records light that leaves each material. The second pass
/// bounces that colored light one more time. Both passes are discarded after
/// this call returns.
pub fn build(scene: &Scene) -> Field {
    let direct = build_field(scene, None);
    build_field(scene, Some(&direct))
}

/// Merged radiance at a ground position.
///
/// A near hit wins. An empty near interval uses the far interval. An empty far
/// interval uses the world probe. The result keeps the color of the material
/// that was hit.
pub fn sample(field: &Field, x: f32, z: f32) -> [f32; 3] {
    let merged = bilinear(&field.near, x, z);
    [
        merged[0].clamp(0.0, 1.5),
        merged[1].clamp(0.0, 1.5),
        merged[2].clamp(0.0, 1.5),
    ]
}

/// Radiance stored on the world probes only, before the near and far ranges.
pub fn sample_world(field: &Field, x: f32, z: f32) -> [f32; 3] {
    bilinear(&field.world, x, z)
}

/// Scalar light reaching a point, from the scene lights only.
/// No lamp means zero. There is no ambient term.
pub fn illuminate(scene: &Scene, x: f32, y: f32, z: f32) -> f32 {
    let mut incoming = 0.0;
    for light in &scene.lights {
        let dx = x - light.x;
        let dy = y - light.y;
        let dz = z - light.z;
        let dist2 = dx * dx + dy * dy + dz * dz;
        let strength = (light.color[0] + light.color[1] + light.color[2]) / 3.0;
        incoming += strength * 2.4 / (1.0 + dist2 * 0.08);
    }
    incoming
}

/// Distances and probe counts for any floor. Spacing follows the floor size.
/// Direction counts stay in the penumbra order: near < far < world.
struct RangePlan {
    spacing: f32,
    directions: u32,
    start: f32,
    end: f32,
}

fn plans_for(span: f32) -> (RangePlan, RangePlan, RangePlan, f32) {
    let span = span.max(1.0);
    // About 48 probes across the floor. Finer than that mostly repeats the
    // bilinear blend. Coarser than that reads as tiles.
    let near_spacing = (span / 48.0).clamp(0.12, 0.45);
    let near_end = (span * 0.12).max(near_spacing * 4.0);
    let far_end = (span * 0.85).max(near_end * 2.0);
    let near = RangePlan {
        spacing: near_spacing,
        directions: 12,
        start: 0.0,
        end: near_end,
    };
    let far = RangePlan {
        spacing: near_spacing * 3.0,
        directions: 36,
        start: near.end,
        end: far_end,
    };
    let world = RangePlan {
        spacing: near_spacing * 8.0,
        directions: 64,
        start: far.end,
        end: (span * 4.0).max(far.end * 2.0),
    };
    let margin = span * 0.5;
    (near, far, world, margin)
}

fn build_field(scene: &Scene, prev: Option<&Field>) -> Field {
    let min_x = scene.floor.x - scene.floor.half_x;
    let min_z = scene.floor.z - scene.floor.half_z;
    let span_x = scene.floor.half_x * 2.0;
    let span_z = scene.floor.half_z * 2.0;
    let (near_plan, far_plan, world_plan, margin) = plans_for(span_x.max(span_z));
    let near = build_level(
        scene,
        &near_plan,
        min_x,
        min_z,
        span_x,
        span_z,
        prev,
    );
    let far = build_level(scene, &far_plan, min_x, min_z, span_x, span_z, prev);
    let world = build_level(
        scene,
        &world_plan,
        min_x - margin,
        min_z - margin,
        span_x + margin * 2.0,
        span_z + margin * 2.0,
        prev,
    );
    let mut field = Field { near, far, world };
    seal(&mut field);
    field
}

fn build_level(
    scene: &Scene,
    plan: &RangePlan,
    min_x: f32,
    min_z: f32,
    span_x: f32,
    span_z: f32,
    prev: Option<&Field>,
) -> CascadeLevel {
    let count_x = (span_x / plan.spacing).floor().max(1.0) as u32;
    let count_z = (span_z / plan.spacing).floor().max(1.0) as u32;
    let mut radiance = Vec::with_capacity((count_x * count_z * plan.directions) as usize);
    for iz in 0..count_z {
        for ix in 0..count_x {
            let x = min_x + (ix as f32 + 0.5) * plan.spacing;
            let z = min_z + (iz as f32 + 0.5) * plan.spacing;
            for dir in 0..plan.directions {
                let angle = (dir as f32 + 0.5) * std::f32::consts::TAU / plan.directions as f32;
                let direction = [angle.cos(), angle.sin()];
                radiance.push(gather(scene, prev, [x, z], direction, plan.start, plan.end));
            }
        }
    }
    CascadeLevel {
        spacing: plan.spacing,
        directions: plan.directions,
        interval_start: plan.start,
        interval_end: plan.end,
        origin_x: min_x,
        origin_z: min_z,
        count_x,
        count_z,
        radiance,
        merged: Vec::new(),
    }
}

fn seal(field: &mut Field) {
    let mut merged = Vec::with_capacity((field.near.count_x * field.near.count_z) as usize);
    for iz in 0..field.near.count_z {
        for ix in 0..field.near.count_x {
            let x = field.near.origin_x + (ix as f32 + 0.5) * field.near.spacing;
            let z = field.near.origin_z + (iz as f32 + 0.5) * field.near.spacing;
            merged.push(merge_at(field, x, z));
        }
    }
    field.near.merged = merged;
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

fn merge_at(field: &Field, x: f32, z: f32) -> [f32; 3] {
    let near_i = nearest_probe(&field.near, x, z);
    let far_i = nearest_probe(&field.far, x, z);
    let world_i = nearest_probe(&field.world, x, z);
    let mut sum = [0.0; 3];
    let directions = field.near.directions.max(1);
    for dir in 0..directions {
        let angle = (dir as f32 + 0.5) * std::f32::consts::TAU / directions as f32;
        let near = probe_dir(&field.near, near_i, dir);
        let chosen = if radiance_of(near) > 1.0e-4 {
            near
        } else {
            let far = probe_dir_at_angle(&field.far, far_i, angle);
            if radiance_of(far) > 1.0e-4 {
                scale_rgb(far, 0.45)
            } else {
                scale_rgb(probe_dir_at_angle(&field.world, world_i, angle), 0.2)
            }
        };
        sum[0] += chosen[0];
        sum[1] += chosen[1];
        sum[2] += chosen[2];
    }
    let n = directions as f32;
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

fn probe_dir(level: &CascadeLevel, probe: usize, dir: u32) -> [f32; 3] {
    level.radiance[probe * level.directions as usize + dir as usize]
}

fn probe_dir_at_angle(level: &CascadeLevel, probe: usize, angle: f32) -> [f32; 3] {
    let turns = angle / std::f32::consts::TAU;
    let dir = (turns * level.directions as f32).floor() as u32 % level.directions;
    probe_dir(level, probe, dir)
}

fn radiance_of(color: [f32; 3]) -> f32 {
    color[0] + color[1] + color[2]
}

fn scale_rgb(color: [f32; 3], scale: f32) -> [f32; 3] {
    [color[0] * scale, color[1] * scale, color[2] * scale]
}

fn nearest_probe(level: &CascadeLevel, x: f32, z: f32) -> usize {
    let ix = ((x - level.origin_x) / level.spacing - 0.5)
        .round()
        .clamp(0.0, (level.count_x - 1) as f32) as u32;
    let iz = ((z - level.origin_z) / level.spacing - 0.5)
        .round()
        .clamp(0.0, (level.count_z - 1) as f32) as u32;
    (iz * level.count_x + ix) as usize
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
) -> [f32; 3] {
    let mut best_t = t1;
    let mut color = [0.0; 3];
    for solid in &scene.solids {
        if let Some(t) = hit_solid(origin, dir, solid) {
            if t >= t0 && t < best_t {
                best_t = t;
                color = outgoing(scene, prev, solid.color, origin[0] + dir[0] * t, solid.height * 0.5, origin[1] + dir[1] * t);
            }
        }
    }
    for wall in &scene.walls {
        if let Some(t) = hit_aabb(
            origin,
            dir,
            wall.x - wall.half_x,
            wall.z - wall.half_z,
            wall.x + wall.half_x,
            wall.z + wall.half_z,
        ) {
            if t >= t0 && t < best_t {
                best_t = t;
                color = outgoing(scene, prev, wall.color, origin[0] + dir[0] * t, wall.height * 0.5, origin[1] + dir[1] * t);
            }
        }
    }
    color
}

/// Light leaving a surface. The material color multiplies the lamp and the light that arrived from the previous pass.
fn outgoing(
    scene: &Scene,
    prev: Option<&Field>,
    albedo: [f32; 3],
    x: f32,
    y: f32,
    z: f32,
) -> [f32; 3] {
    let direct = illuminate(scene, x, y, z);
    let incoming = prev.map(|field| sample(field, x, z)).unwrap_or([0.0; 3]);
    // A diffuse surface returns a bit over half of the light that hits it.
    // That is enough to tint a neighbor, and it does not flood the scene white.
    let reflected = direct * 0.55;
    [
        albedo[0] * (reflected + incoming[0] * 0.5),
        albedo[1] * (reflected + incoming[1] * 0.5),
        albedo[2] * (reflected + incoming[2] * 0.5),
    ]
}

fn hit_solid(origin: [f32; 2], dir: [f32; 2], solid: &crate::types::Solid) -> Option<f32> {
    let half = solid.size * 0.5;
    match solid.shape {
        Shape::Square => hit_aabb(
            origin,
            dir,
            solid.x - half,
            solid.z - half,
            solid.x + half,
            solid.z + half,
        ),
        Shape::Circle => hit_circle(origin, dir, solid.x, solid.z, half),
    }
}

fn hit_aabb(origin: [f32; 2], dir: [f32; 2], min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> Option<f32> {
    let inv_x = if dir[0].abs() < 1.0e-8 { f32::INFINITY } else { 1.0 / dir[0] };
    let inv_z = if dir[1].abs() < 1.0e-8 { f32::INFINITY } else { 1.0 / dir[1] };
    let (tx1, tx2) = slab(origin[0], inv_x, min_x, max_x);
    let (tz1, tz2) = slab(origin[1], inv_z, min_z, max_z);
    let t_enter = tx1.max(tz1);
    let t_exit = tx2.min(tz2);
    if t_exit < t_enter || t_exit < 0.0 {
        return None;
    }
    if t_enter >= 0.0 {
        Some(t_enter)
    } else {
        Some(t_exit)
    }
}

fn slab(origin: f32, inv_dir: f32, min_v: f32, max_v: f32) -> (f32, f32) {
    let a = (min_v - origin) * inv_dir;
    let b = (max_v - origin) * inv_dir;
    if a < b { (a, b) } else { (b, a) }
}

fn hit_circle(origin: [f32; 2], dir: [f32; 2], cx: f32, cz: f32, radius: f32) -> Option<f32> {
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
    let t1 = -b + root;
    if t0 >= 0.0 {
        Some(t0)
    } else if t1 >= 0.0 {
        Some(t1)
    } else {
        None
    }
}
