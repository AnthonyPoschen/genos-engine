#![allow(dead_code)]
//! Probe placement and the surface poll the picture uses.
//!
//! The GPU field is one 128 by 128 grid on the floor. Spacing is finer beside
//! the player and coarser at the far side. `shaders/light.comp` and
//! `shaders/scene.frag` repeat `DENSITY_RATIO`, `DENSITY_FALLOFF`, and the
//! 16-step probe placement. An upward face reads probes outside its footprint.

use genos_scene::{Floor, Scene, Shape};

use crate::field::illuminate_facing;

/// Extra probe density at the player, divided by the density far away.
pub(crate) const DENSITY_RATIO: f32 = 4.0;
/// Distance, in meters, over which the extra density falls off.
pub(crate) const DENSITY_FALLOFF: f32 = 3.0;
const BISECT_STEPS: u32 = 16;
const OUTSIDE_MARGIN: f32 = 0.06;
/// A segment must cross this much of a wall before the probe is on the far side.
const WALL_OVERLAP: f32 = 0.05;

#[derive(Clone, Copy, Debug)]
pub struct AxisLayout {
    pub origin: f32,
    pub span: f32,
    pub player: f32,
    pub count: u32,
    total: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct FieldLayout {
    pub x: AxisLayout,
    pub z: AxisLayout,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceShade {
    pub bounce: [f32; 3],
    pub direct: f32,
    pub valid_probes: u32,
    pub sample_xz: [f32; 2],
}

pub(crate) fn layout_for(floor: &Floor, player_x: f32, player_z: f32) -> FieldLayout {
    FieldLayout {
        x: axis_layout(
            floor.position.x - floor.half_x,
            floor.half_x * 2.0,
            player_x,
        ),
        z: axis_layout(
            floor.position.z - floor.half_z,
            floor.half_z * 2.0,
            player_z,
        ),
    }
}

/// Cell size at `p` on one axis. This is the spacing the picture's grid uses.
pub(crate) fn axis_spacing(axis: &AxisLayout, p: f32) -> f32 {
    let end = axis.origin + axis.span.max(1.0e-3);
    let player = axis.player.clamp(axis.origin, end);
    let p = p.clamp(axis.origin, end);
    let shape = density_shape(p, player);
    axis.total / (shape * axis.count.max(1) as f32)
}

/// Neighbor accept distance uploaded with the field. It matches the fine cell.
pub(crate) fn near_spacing(layout: &FieldLayout, player_x: f32, player_z: f32) -> f32 {
    axis_spacing(&layout.x, player_x).max(axis_spacing(&layout.z, player_z))
}

pub(crate) fn shade_surface(
    scene: &Scene,
    player_x: f32,
    player_z: f32,
    pos: [f32; 3],
    normal: [f32; 3],
) -> SurfaceShade {
    let layout = layout_for(&scene.floor, player_x, player_z);
    let sample_xz = poll_xz(scene, pos, normal);
    let (bounce, valid_probes) = sample_bounce(scene, &layout, sample_xz);
    let origin = [
        pos[0] + normal[0] * 0.02,
        pos[1] + normal[1] * 0.02,
        pos[2] + normal[2] * 0.02,
    ];
    let direct = illuminate_facing(scene, origin[0], origin[1], origin[2], normal);
    let bounce = if normal[1] > 0.5 && inside_any(scene, pos[0], pos[2]) && direct <= 1.0e-4 {
        [0.0; 3]
    } else {
        bounce
    };
    SurfaceShade {
        bounce,
        direct,
        valid_probes,
        sample_xz,
    }
}

/// Light stored at one ground probe. A probe inside a wall or a solid is black.
pub(crate) fn stored_radiance(scene: &Scene, x: f32, z: f32) -> [f32; 3] {
    if inside_any(scene, x, z) {
        return [0.0; 3];
    }
    probe_radiance(scene, x, z)
}

fn axis_layout(origin: f32, span: f32, player: f32) -> AxisLayout {
    let span = span.max(0.0);
    let end = origin + span.max(1.0e-3);
    let player = player.clamp(origin, end);
    let count = if span < 1.0e-3 {
        1
    } else {
        crate::pack::FIELD_GRID
    };
    let total = density_integral(origin, end, player).max(1.0e-6);
    AxisLayout {
        origin,
        span: span.max(1.0e-3),
        player,
        count,
        total,
    }
}

fn density_shape(t: f32, player: f32) -> f32 {
    1.0 + (DENSITY_RATIO - 1.0) * (-(t - player).abs() / DENSITY_FALLOFF).exp()
}

fn density_integral(origin: f32, x: f32, player: f32) -> f32 {
    let extra = DENSITY_RATIO - 1.0;
    let radius = DENSITY_FALLOFF;
    let curve = if x <= player {
        radius * ((-(player - x) / radius).exp() - (-(player - origin) / radius).exp())
    } else {
        radius * (1.0 - (-(player - origin) / radius).exp())
            + radius * (1.0 - (-(x - player) / radius).exp())
    };
    (x - origin) + extra * curve
}

fn probe_coord(axis: &AxisLayout, index: u32) -> f32 {
    let end = axis.origin + axis.span;
    let player = axis.player.clamp(axis.origin, end);
    let target = (index as f32 + 0.5) / axis.count.max(1) as f32 * axis.total;
    let mut lo = axis.origin;
    let mut hi = end;
    for _ in 0..BISECT_STEPS {
        let mid = 0.5 * (lo + hi);
        if density_integral(axis.origin, mid, player) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

fn axis_index(axis: &AxisLayout, p: f32) -> f32 {
    let end = axis.origin + axis.span;
    let player = axis.player.clamp(axis.origin, end);
    let p = p.clamp(axis.origin, end);
    let acc = density_integral(axis.origin, p, player);
    let share = if axis.total > 1.0e-6 {
        acc / axis.total
    } else {
        0.0
    };
    share * axis.count as f32 - 0.5
}

fn poll_xz(scene: &Scene, pos: [f32; 3], normal: [f32; 3]) -> [f32; 2] {
    if normal[1] <= 0.5 {
        return [pos[0] + normal[0] * 0.04, pos[2] + normal[2] * 0.04];
    }
    let mut p = [pos[0], pos[2]];
    for _ in 0..4 {
        let Some(next) = push_outside(scene, p) else {
            return p;
        };
        p = next;
    }
    p
}

fn push_outside(scene: &Scene, p: [f32; 2]) -> Option<[f32; 2]> {
    let mut best_free: Option<(f32, [f32; 2])> = None;
    let mut best_any: Option<(f32, [f32; 2])> = None;
    let mut found = false;
    for (dist, dest) in exits_at(scene, p) {
        found = true;
        if best_any.map(|(old, _)| dist < old).unwrap_or(true) {
            best_any = Some((dist, dest));
        }
        if !inside_any(scene, dest[0], dest[1])
            && best_free.map(|(old, _)| dist < old).unwrap_or(true)
        {
            best_free = Some((dist, dest));
        }
    }
    if !found {
        return None;
    }
    Some(best_free.or(best_any).map(|(_, dest)| dest).unwrap_or(p))
}

fn exits_at(scene: &Scene, p: [f32; 2]) -> Vec<(f32, [f32; 2])> {
    let mut exits = Vec::new();
    for solid in &scene.solids {
        let half = solid.size * 0.5;
        match solid.shape {
            Shape::Circle => {
                if let Some(exit) = circle_exit(solid.position.x, solid.position.z, half, p) {
                    exits.push(exit);
                }
            }
            Shape::Square => {
                exits.extend(box_exits(solid.position.x, solid.position.z, half, half, p));
            }
        }
    }
    for wall in &scene.walls {
        exits.extend(box_exits(
            wall.position.x,
            wall.position.z,
            wall.half_x,
            wall.half_z,
            p,
        ));
    }
    exits
}

fn box_exits(cx: f32, cz: f32, half_x: f32, half_z: f32, p: [f32; 2]) -> Vec<(f32, [f32; 2])> {
    let dx = p[0] - cx;
    let dz = p[1] - cz;
    if dx.abs() > half_x || dz.abs() > half_z {
        return Vec::new();
    }
    vec![
        (
            half_x - dx + OUTSIDE_MARGIN,
            [cx + half_x + OUTSIDE_MARGIN, p[1]],
        ),
        (
            half_x + dx + OUTSIDE_MARGIN,
            [cx - half_x - OUTSIDE_MARGIN, p[1]],
        ),
        (
            half_z - dz + OUTSIDE_MARGIN,
            [p[0], cz + half_z + OUTSIDE_MARGIN],
        ),
        (
            half_z + dz + OUTSIDE_MARGIN,
            [p[0], cz - half_z - OUTSIDE_MARGIN],
        ),
    ]
}

fn circle_exit(cx: f32, cz: f32, radius: f32, p: [f32; 2]) -> Option<(f32, [f32; 2])> {
    let dx = p[0] - cx;
    let dz = p[1] - cz;
    let dist2 = dx * dx + dz * dz;
    if dist2 > radius * radius {
        return None;
    }
    let dist = dist2.sqrt();
    let (nx, nz) = if dist < 1.0e-6 {
        (1.0, 0.0)
    } else {
        (dx / dist, dz / dist)
    };
    let reach = radius + OUTSIDE_MARGIN;
    Some((reach - dist, [cx + nx * reach, cz + nz * reach]))
}

fn sample_bounce(scene: &Scene, layout: &FieldLayout, xz: [f32; 2]) -> ([f32; 3], u32) {
    if layout.x.count == 0 || layout.z.count == 0 {
        return ([0.0; 3], 0);
    }
    let fx = axis_index(&layout.x, xz[0]).clamp(0.0, (layout.x.count - 1) as f32);
    let fz = axis_index(&layout.z, xz[1]).clamp(0.0, (layout.z.count - 1) as f32);
    let x0 = fx.floor() as u32;
    let z0 = fz.floor() as u32;
    let x1 = (x0 + 1).min(layout.x.count - 1);
    let z1 = (z0 + 1).min(layout.z.count - 1);
    let tx = fx - x0 as f32;
    let tz = fz - z0 as f32;
    let corners = [(x0, z0), (x1, z0), (x0, z1), (x1, z1)];
    let weights = [
        (1.0 - tx) * (1.0 - tz),
        tx * (1.0 - tz),
        (1.0 - tx) * tz,
        tx * tz,
    ];
    let accept = near_spacing(layout, layout.x.player, layout.z.player) * 1.25;
    let mut sum_w = 0.0;
    let mut color = [0.0; 3];
    let mut valid = 0u32;
    for (corner, weight) in corners.into_iter().zip(weights) {
        let probe = [
            probe_coord(&layout.x, corner.0),
            probe_coord(&layout.z, corner.1),
        ];
        if probe_rejected(scene, xz, probe, accept) {
            continue;
        }
        if weight <= 1.0e-4 {
            continue;
        }
        valid += 1;
        let stored = stored_radiance(scene, probe[0], probe[1]);
        sum_w += weight;
        color[0] += stored[0] * weight;
        color[1] += stored[1] * weight;
        color[2] += stored[2] * weight;
    }
    if sum_w < 1.0e-4 {
        return ([0.0; 3], 0);
    }
    (
        [color[0] / sum_w, color[1] / sum_w, color[2] / sum_w],
        valid,
    )
}

fn probe_rejected(scene: &Scene, from: [f32; 2], probe: [f32; 2], accept: f32) -> bool {
    if inside_any(scene, probe[0], probe[1]) {
        return true;
    }
    let dx = probe[0] - from[0];
    let dz = probe[1] - from[1];
    if (dx * dx + dz * dz).sqrt() <= accept {
        return false;
    }
    segment_hidden(scene, from, probe)
}

fn probe_radiance(scene: &Scene, x: f32, z: f32) -> [f32; 3] {
    const DIRS: u32 = 32;
    let mut sum = [0.0; 3];
    for dir in 0..DIRS {
        let angle = (dir as f32 + 0.5) * std::f32::consts::TAU / DIRS as f32;
        let direction = [angle.cos(), angle.sin()];
        if let Some(hit) = first_hit(scene, [x, z], direction) {
            sum[0] += hit[0];
            sum[1] += hit[1];
            sum[2] += hit[2];
        }
    }
    let n = DIRS as f32;
    let floor_direct = illuminate_facing(scene, x, 0.05, z, [0.0, 1.0, 0.0]);
    let floor = scene.floor.color;
    [
        sum[0] / n + floor[0] * floor_direct * 0.1,
        sum[1] / n + floor[1] * floor_direct * 0.1,
        sum[2] / n + floor[2] * floor_direct * 0.1,
    ]
}

fn first_hit(scene: &Scene, origin: [f32; 2], dir: [f32; 2]) -> Option<[f32; 3]> {
    let limit = (scene.floor.half_x.max(scene.floor.half_z) * 4.0).max(8.0);
    let mut best_t = limit;
    let mut color = None;
    for solid in &scene.solids {
        let half = solid.size * 0.5;
        let hit = match solid.shape {
            Shape::Circle => hit_circle(origin, dir, [solid.position.x, solid.position.z], half),
            Shape::Square => hit_box(
                origin,
                dir,
                solid.position.x - half,
                solid.position.z - half,
                solid.position.x + half,
                solid.position.z + half,
            ),
        };
        if let Some((t, normal)) = hit {
            if t < best_t {
                best_t = t;
                color = Some(leaving(
                    scene,
                    solid.color,
                    [origin[0] + dir[0] * t, origin[1] + dir[1] * t],
                    normal,
                    [solid.position.x, solid.position.z],
                    half,
                    half,
                    matches!(solid.shape, Shape::Circle),
                    solid.height,
                ));
            }
        }
    }
    for wall in &scene.walls {
        if let Some((t, normal)) = hit_box(
            origin,
            dir,
            wall.position.x - wall.half_x,
            wall.position.z - wall.half_z,
            wall.position.x + wall.half_x,
            wall.position.z + wall.half_z,
        ) {
            if t < best_t {
                best_t = t;
                color = Some(leaving(
                    scene,
                    wall.color,
                    [origin[0] + dir[0] * t, origin[1] + dir[1] * t],
                    normal,
                    [wall.position.x, wall.position.z],
                    wall.half_x,
                    wall.half_z,
                    false,
                    wall.height,
                ));
            }
        }
    }
    color
}

fn leaving(
    scene: &Scene,
    albedo: [f32; 3],
    hit: [f32; 2],
    normal: [f32; 2],
    center: [f32; 2],
    half_x: f32,
    half_z: f32,
    circle: bool,
    height: f32,
) -> [f32; 3] {
    let mut direct = 0.0;
    for light in &scene.lights {
        let face =
            (light.position.x - hit[0]) * normal[0] + (light.position.z - hit[1]) * normal[1];
        let over = lamp_over(
            light.position.x,
            light.position.z,
            center,
            half_x,
            half_z,
            circle,
        );
        if face <= 0.0 && !over {
            continue;
        }
        direct += if face > 0.0 {
            illuminate_facing(
                scene,
                hit[0] + normal[0] * 0.02,
                height * 0.45,
                hit[1] + normal[1] * 0.02,
                [normal[0], 0.0, normal[1]],
            )
        } else {
            illuminate_facing(scene, center[0], height + 0.05, center[1], [0.0, 1.0, 0.0])
        };
    }
    [
        albedo[0] * direct * 0.55,
        albedo[1] * direct * 0.55,
        albedo[2] * direct * 0.55,
    ]
}

fn lamp_over(x: f32, z: f32, center: [f32; 2], half_x: f32, half_z: f32, circle: bool) -> bool {
    let dx = x - center[0];
    let dz = z - center[1];
    if circle {
        dx * dx + dz * dz <= half_x * half_x
    } else {
        dx.abs() <= half_x && dz.abs() <= half_z
    }
}

fn inside_any(scene: &Scene, x: f32, z: f32) -> bool {
    scene.solids.iter().any(|solid| solid.contains_xz(x, z))
        || scene.walls.iter().any(|wall| {
            (x - wall.position.x).abs() <= wall.half_x && (z - wall.position.z).abs() <= wall.half_z
        })
}

fn segment_hidden(scene: &Scene, from: [f32; 2], to: [f32; 2]) -> bool {
    for solid in &scene.solids {
        let half = solid.size * 0.5;
        let hit = match solid.shape {
            Shape::Circle => {
                segment_hits_circle(from, to, [solid.position.x, solid.position.z], half)
            }
            Shape::Square => segment_hits_box(
                from,
                to,
                solid.position.x - half,
                solid.position.z - half,
                solid.position.x + half,
                solid.position.z + half,
            ),
        };
        if hit {
            return true;
        }
    }
    scene.walls.iter().any(|wall| {
        segment_hits_box(
            from,
            to,
            wall.position.x - wall.half_x,
            wall.position.z - wall.half_z,
            wall.position.x + wall.half_x,
            wall.position.z + wall.half_z,
        )
    })
}

fn segment_hits_box(
    a: [f32; 2],
    b: [f32; 2],
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
) -> bool {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let dist = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    if dist < 1.0e-4 {
        return false;
    }
    let dir = [delta[0] / dist, delta[1] / dist];
    let min_p = [min_x, min_z];
    let max_p = [max_x, max_z];
    let mut t_enter: f32 = 0.0;
    let mut t_exit = dist;
    for axis in 0..2 {
        if dir[axis].abs() < 1.0e-8 {
            if a[axis] < min_p[axis] || a[axis] > max_p[axis] {
                return false;
            }
            continue;
        }
        let inv = 1.0 / dir[axis];
        let t1 = (min_p[axis] - a[axis]) * inv;
        let t2 = (max_p[axis] - a[axis]) * inv;
        t_enter = t_enter.max(t1.min(t2));
        t_exit = t_exit.min(t1.max(t2));
        if t_exit < t_enter {
            return false;
        }
    }
    let lo = t_enter.max(1.0e-3);
    let hi = t_exit.min(dist - 1.0e-3);
    hi > lo + WALL_OVERLAP
}

fn segment_hits_circle(a: [f32; 2], b: [f32; 2], center: [f32; 2], radius: f32) -> bool {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let dist = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    if dist < 1.0e-4 {
        return false;
    }
    let dir = [delta[0] / dist, delta[1] / dist];
    let o = [a[0] - center[0], a[1] - center[1]];
    let along = o[0] * dir[0] + o[1] * dir[1];
    let disc = along * along - (o[0] * o[0] + o[1] * o[1] - radius * radius);
    if disc < 0.0 {
        return false;
    }
    let root = disc.sqrt();
    let lo = (-along - root).max(1.0e-3);
    let hi = (-along + root).min(dist - 1.0e-3);
    hi > lo + WALL_OVERLAP
}

fn hit_box(
    origin: [f32; 2],
    dir: [f32; 2],
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
) -> Option<(f32, [f32; 2])> {
    let mut t_enter: f32 = 0.0;
    let mut t_exit: f32 = 1.0e20;
    let mut axis_n = 0;
    let mut sign = 1.0;
    let min_p = [min_x, min_z];
    let max_p = [max_x, max_z];
    for axis in 0..2 {
        if dir[axis].abs() < 1.0e-8 {
            if origin[axis] < min_p[axis] || origin[axis] > max_p[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[axis];
        let t1 = (min_p[axis] - origin[axis]) * inv;
        let t2 = (max_p[axis] - origin[axis]) * inv;
        let (lo, hi, face) = if t1 < t2 {
            (t1, t2, -1.0)
        } else {
            (t2, t1, 1.0)
        };
        if lo > t_enter {
            t_enter = lo;
            axis_n = axis;
            sign = face;
        }
        t_exit = t_exit.min(hi);
        if t_exit < t_enter {
            return None;
        }
    }
    if t_enter < 0.002 || t_enter >= t_exit {
        return None;
    }
    let mut normal = [0.0, 0.0];
    normal[axis_n] = sign;
    Some((t_enter, normal))
}

fn hit_circle(
    origin: [f32; 2],
    dir: [f32; 2],
    center: [f32; 2],
    radius: f32,
) -> Option<(f32, [f32; 2])> {
    let o = [origin[0] - center[0], origin[1] - center[1]];
    let b = o[0] * dir[0] + o[1] * dir[1];
    let c = o[0] * o[0] + o[1] * o[1] - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    if t < 0.002 {
        return None;
    }
    let hit = [origin[0] + dir[0] * t, origin[1] + dir[1] * t];
    let dx = hit[0] - center[0];
    let dz = hit[1] - center[1];
    let len = (dx * dx + dz * dz).sqrt().max(1.0e-6);
    Some((t, [dx / len, dz / len]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{pack_frame, scene_bytes};
    use crate::world::World;
    use genos_scene::{view_proj, Camera, Floor, Light, Scene, Shape, Solid, Vec3, Wall};

    fn shipped() -> Scene {
        genos_scene::load_path(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/camera/scene.rhai"
        )))
        .expect("shipped scene")
    }

    fn energy(color: [f32; 3]) -> f32 {
        color[0] + color[1] + color[2]
    }

    fn level(shade: &SurfaceShade) -> f32 {
        shade.direct + energy(shade.bounce)
    }

    #[test]
    fn the_shaders_share_the_density_curve() {
        let frag = include_str!("../shaders/scene.frag");
        let comp = include_str!("../shaders/light.comp");
        for source in [frag, comp] {
            assert!(
                source.contains("const uint FIELD_COPY = 524288u;"),
                "shader field copy left the Rust field"
            );
            assert!(
                source.contains("const float LAMBERT = 0.318309886;"),
                "shader reflectance left the Rust field"
            );
            assert!(
                source.contains("const float LAMP_UNIT = 72.0;"),
                "shader lamp unit left the Rust field"
            );
            assert!(
                source.contains(&format!(
                    "const uint SCREEN_DIRS = {}u;",
                    crate::field::SCREEN_DIRS
                )),
                "shader screen directions left the Rust field"
            );
            assert!(
                source.contains("vec4 view_grid;"),
                "shader view block left the scene uniform"
            );
            assert!(
                source.contains("Cascade cascades[3];"),
                "shader cascade table left the scene block"
            );
            assert!(
                source.contains(&format!(
                    "const uint PIN_POS0 = {}u;",
                    crate::pins::PIN_POS0
                )),
                "shader pin positions left the Rust field"
            );
            assert!(
                source.contains(&format!(
                    "const uint HASH_BASE0 = {}u;",
                    crate::pins::HASH_BASE0
                )),
                "shader lattice hash left the Rust field"
            );
            assert!(
                source.contains(&format!(
                    "const uint HASH_BASE1 = {}u;",
                    crate::pins::HASH_BASE1
                )),
                "shader lattice hash left the Rust field"
            );
            assert!(
                source.contains(&format!(
                    "const uint PIN_NRM0 = {}u;",
                    crate::pins::PIN_NRM0
                )),
                "shader pin normals left the Rust field"
            );
            assert!(
                source.contains(&format!(
                    "const uint HASH_DIM1 = {}u;",
                    crate::pins::HASH_DIM1
                )),
                "shader cascade 1 hash left the Rust field"
            );
            assert!(
                source.contains(&format!(
                    "const uint PIN_IRR1 = {}u;",
                    crate::pins::PIN_IRR1
                )),
                "shader cascade 1 irradiance left the Rust field"
            );
        }
        assert!(
            comp.contains("const float SCREEN_REACH = 4.0;"),
            "screen interval left the gather"
        );
    }

    #[test]
    fn probe_centers_follow_the_density_curve() {
        let scene = shipped();
        let layout = layout_for(&scene.floor, -6.0, 2.0);
        for index in [0, 1, 40, 80, 127] {
            let x = probe_coord(&layout.x, index);
            let found = axis_index(&layout.x, x);
            assert!(
                (found - index as f32).abs() < 0.05,
                "probe {index} landed at index {found}"
            );
        }
        let gap = probe_coord(&layout.x, 64) - probe_coord(&layout.x, 63);
        let mid = 0.5 * (probe_coord(&layout.x, 64) + probe_coord(&layout.x, 63));
        let predicted = axis_spacing(&layout.x, mid);
        assert!(
            (gap - predicted).abs() < predicted * 0.08,
            "cell gap {gap} left the spacing {predicted}"
        );
    }

    #[test]
    fn the_uploaded_field_is_finer_beside_the_player() {
        let scene = shipped();
        let world = World::from_scene(scene.clone());
        let west_x = scene.floor.position.x - scene.floor.half_x + 0.5;
        let east_x = scene.floor.position.x + scene.floor.half_x - 0.5;
        let z = scene.floor.position.z;
        let west = uploaded(&world, west_x, z);
        let east = uploaded(&world, east_x, z);
        assert!(
            (west.cascades[0].spacing - east.cascades[0].spacing).abs() < 1.0e-5,
            "the grid followed the camera: {:?} {:?}",
            west.cascades[0].spacing,
            east.cascades[0].spacing
        );
        assert!(
            west.cascades[1].spacing > west.cascades[0].spacing * 1.9,
            "the far cascade did not double the spacing"
        );
        assert!(
            west.cascades[1].dirs >= west.cascades[0].dirs * 2,
            "the far cascade did not double the directions"
        );
        assert!(west.cascades[2].origin_x < scene.floor.position.x - scene.floor.half_x);
        let west_bytes = scene_bytes(&west);
        let east_bytes = scene_bytes(&east);
        let west_spacing = f32::from_ne_bytes(west_bytes[20..24].try_into().unwrap());
        let east_eye = f32::from_ne_bytes(east_bytes[2224..2228].try_into().unwrap());
        assert!((west_spacing - west.spacing).abs() < 1.0e-6);
        assert!(
            (east_eye - east_x).abs() < 1.0e-4,
            "the draw did not upload the player"
        );
        assert_ne!(west_bytes[2224..2228], east_bytes[2224..2228]);
        assert!(
            west.cascades[2].offset
                + west.cascades[2].count_x * west.cascades[2].count_z * west.cascades[2].dirs
                <= crate::field::FIELD_COPY
        );
    }

    #[test]
    fn an_upward_face_reads_probes_outside_the_shape() {
        let mut scene = room();
        scene.solids[0].color = [1.0, 0.0, 0.0];
        scene.lights = vec![Light {
            position: Vec3::new(0.0, 5.0, 0.0),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }];
        let top = shade_surface(&scene, 0.0, 0.0, [0.0, 1.2, 0.0], [0.0, 1.0, 0.0]);
        let under = shade_surface(&scene, 0.0, 0.0, [0.0, 0.05, 0.0], [0.0, -1.0, 0.0]);
        let ground = shade_surface(&scene, 0.0, 0.0, [1.15, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let mut dark_scene = scene.clone();
        dark_scene.lights.clear();
        let dark = shade_surface(&dark_scene, 0.0, 0.0, [0.0, 1.2, 0.0], [0.0, 1.0, 0.0]);
        assert!(
            top.sample_xz[0].abs() > 0.75 || top.sample_xz[1].abs() > 0.75,
            "the top polled inside the solid: {:?}",
            top.sample_xz
        );
        assert!(energy(stored_radiance(&scene, 0.0, 0.0)) < 1.0e-4);
        assert!(
            level(&top) > level(&under) + 0.05,
            "top {:?} under {:?}",
            top,
            under
        );
        assert!(
            level(&top) > level(&dark) + 0.05,
            "top {:?} dark {:?}",
            top,
            dark
        );
        assert!(
            ground.bounce[0] > ground.bounce[1] + 0.01,
            "the ground lost the red bounce: {:?}",
            ground.bounce
        );
        assert!(
            energy(top.bounce) > 0.01,
            "the top stayed at the interior probe color: {:?}",
            top.bounce
        );

        scene.lights = vec![Light {
            position: Vec3::new(1.6, 0.3, 0.0),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }];
        let below = shade_surface(&scene, 0.0, 0.0, [0.0, 1.2, 0.0], [0.0, 1.0, 0.0]);
        assert!(
            level(&below) < level(&dark) + 0.02,
            "a lamp below the face lit the top: {:?}",
            below
        );

        scene.walls = vec![Wall {
            position: Vec3::new(-1.6, 0.0, 0.0),
            half_x: 0.2,
            half_z: 2.0,
            height: 3.0,
            color: [1.0, 1.0, 1.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }];
        scene.lights = vec![Light {
            position: Vec3::new(-4.0, 3.0, 0.0),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }];
        let blocked = shade_surface(&scene, 0.0, 0.0, [0.0, 1.2, 0.0], [0.0, 1.0, 0.0]);
        assert!(
            level(&blocked) < level(&dark) + 0.02,
            "a blocked lamp lit the top: {:?}",
            blocked
        );
    }

    #[test]
    fn a_corridor_leg_near_the_player_blends_more_than_one_probe() {
        let mut scene = shipped();
        scene.lights = vec![Light {
            position: Vec3::new(0.0, 3.0, 6.5),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }];
        let player = [0.0, 6.5];
        let layout = layout_for(&scene.floor, player[0], player[1]);
        let spacing = axis_spacing(&layout.z, player[1]);
        let first = shade_surface(
            &scene,
            player[0],
            player[1],
            [0.0, 0.0, 6.35],
            [0.0, 1.0, 0.0],
        );
        let second = shade_surface(
            &scene,
            player[0],
            player[1],
            [0.0, 0.0, 6.35 + spacing],
            [0.0, 1.0, 0.0],
        );
        let across = shade_surface(
            &scene,
            player[0],
            player[1],
            [-2.05, 0.0, 6.35],
            [0.0, 1.0, 0.0],
        );
        assert!(
            first.valid_probes > 1,
            "the leg used {} probes",
            first.valid_probes
        );
        assert!(
            second.valid_probes > 1,
            "the next cell used {} probes",
            second.valid_probes
        );
        assert!(
            energy(first.bounce) > 0.02,
            "the leg bounce is black: {:?}",
            first.bounce
        );
        assert!(
            energy(second.bounce) > 0.02,
            "the next cell bounce is black: {:?}",
            second.bounce
        );
        let brighter = energy(first.bounce).max(energy(second.bounce));
        let dimmer = energy(first.bounce).min(energy(second.bounce));
        assert!(
            dimmer * 4.0 > brighter,
            "nearby leg samples split: {first:?} {second:?}"
        );
        assert!(
            level(&across) + 0.02 < level(&first).min(level(&second)),
            "light crossed the corridor wall: across {:?} leg {:?}",
            across,
            first
        );
        assert!(energy(stored_radiance(&scene, -1.5, 6.5)) < 1.0e-4);
    }

    fn uploaded(world: &World, x: f32, z: f32) -> crate::pack::Pack {
        let camera = Camera::new(x, z, 0.0);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        let view = view_proj(&camera, 16.0 / 9.0);
        let mut memory = crate::budget::FrameMemory::default();
        pack_frame(world, &view, eye, &mut memory)
    }

    fn room() -> Scene {
        Scene {
            floor: Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 8.0,
                half_z: 8.0,
                color: [1.0, 1.0, 1.0],
            },
            walls: Vec::new(),
            solids: vec![Solid {
                shape: Shape::Square,
                position: Vec3::new(0.0, 0.0, 0.0),
                size: 1.5,
                height: 1.2,
                color: [1.0, 0.0, 0.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            lights: Vec::new(),
            ceiling: None,
        }
    }
}
