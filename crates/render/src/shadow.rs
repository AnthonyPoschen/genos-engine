//! Straight floor shadows.
//!
//! Each occluder projects onto the floor through each lamp. The floor mesh is
//! cut along that polygon, so the light changes on the intersection instead of
//! stepping from cell to cell.

use genos_scene::{Light, Scene, Shape};

pub(crate) struct Poly {
    pub light: u32,
    verts: Vec<[f32; 2]>,
    min: [f32; 2],
    max: [f32; 2],
}

pub(crate) struct Shadows {
    pub polys: Vec<Poly>,
    /// Lights whose occlusion is the polygons above.
    pub resolved: u32,
}

pub(crate) fn shadows(scene: &Scene) -> Shadows {
    let mut polys = Vec::new();
    let mut resolved = 0u32;
    for (index, light) in scene.lights.iter().enumerate() {
        if index >= 32 || light.position.y <= 0.05 {
            continue;
        }
        let bit = 1u32 << index;
        let mut any = false;
        for wall in &scene.walls {
            if let Some(poly) = hull_box(
                light,
                wall.position.x,
                wall.position.z,
                wall.half_x,
                wall.half_z,
                wall.height,
                index as u32,
            ) {
                polys.push(poly);
                any = true;
            }
        }
        for solid in &scene.solids {
            let poly = match solid.shape {
                Shape::Square => hull_box(
                    light,
                    solid.position.x,
                    solid.position.z,
                    solid.size * 0.5,
                    solid.size * 0.5,
                    solid.height,
                    index as u32,
                ),
                Shape::Circle => hull_circle(
                    light,
                    solid.position.x,
                    solid.position.z,
                    solid.size * 0.5,
                    solid.height,
                    index as u32,
                ),
            };
            if let Some(poly) = poly {
                polys.push(poly);
                any = true;
            }
        }
        if any {
            resolved |= bit;
        }
    }
    Shadows { polys, resolved }
}

pub(crate) fn mask_at(polys: &[Poly], x: f32, z: f32) -> u32 {
    let mut mask = 0u32;
    for poly in polys {
        if x < poly.min[0] || x > poly.max[0] || z < poly.min[1] || z > poly.max[1] {
            continue;
        }
        if point_in_convex(x, z, &poly.verts) {
            mask |= 1u32 << poly.light;
        }
    }
    mask
}

/// Shadow edges binned onto one floor grid. A cell reads only the edges that meet it.
pub(crate) struct EdgeGrid {
    nx: u32,
    buckets: Vec<Vec<([f32; 2], [f32; 2])>>,
}

impl EdgeGrid {
    pub(crate) fn cuts(&self, ix: u32, iz: u32) -> &[([f32; 2], [f32; 2])] {
        &self.buckets[(iz * self.nx + ix) as usize]
    }
}

pub(crate) fn edge_grid(
    shadows: &Shadows,
    x0: f32,
    z0: f32,
    dx: f32,
    dz: f32,
    nx: u32,
    nz: u32,
) -> EdgeGrid {
    let mut buckets = vec![Vec::new(); (nx * nz) as usize];
    if nx == 0 || nz == 0 || dx <= 0.0 || dz <= 0.0 {
        return EdgeGrid { nx, buckets };
    }
    for poly in &shadows.polys {
        for index in 0..poly.verts.len() {
            let a = poly.verts[index];
            let b = poly.verts[(index + 1) % poly.verts.len()];
            let ix0 = cell_index(a[0].min(b[0]), x0, dx, nx);
            let ix1 = cell_index(a[0].max(b[0]), x0, dx, nx);
            let iz0 = cell_index(a[1].min(b[1]), z0, dz, nz);
            let iz1 = cell_index(a[1].max(b[1]), z0, dz, nz);
            for iz in iz0..=iz1 {
                for ix in ix0..=ix1 {
                    let xb = x0 + ix as f32 * dx;
                    let zb = z0 + iz as f32 * dz;
                    let quad = [[xb, zb], [xb + dx, zb], [xb + dx, zb + dz], [xb, zb + dz]];
                    if segment_meets_quad(a, b, quad) {
                        buckets[(iz * nx + ix) as usize].push((a, b));
                    }
                }
            }
        }
    }
    EdgeGrid { nx, buckets }
}

fn cell_index(value: f32, origin: f32, size: f32, count: u32) -> u32 {
    let index = ((value - origin) / size).floor() as i32;
    index.clamp(0, count as i32 - 1) as u32
}

fn segment_meets_quad(a: [f32; 2], b: [f32; 2], quad: [[f32; 2]; 4]) -> bool {
    let (min, max) = quad_bounds(quad);
    if a[0].max(b[0]) < min[0]
        || a[0].min(b[0]) > max[0]
        || a[1].max(b[1]) < min[1]
        || a[1].min(b[1]) > max[1]
    {
        return false;
    }
    if point_in_bounds(a, min, max) || point_in_bounds(b, min, max) {
        return true;
    }
    for index in 0..4 {
        if seg_intersect(a, b, quad[index], quad[(index + 1) % 4]).is_some() {
            return true;
        }
    }
    false
}

fn quad_bounds(quad: [[f32; 2]; 4]) -> ([f32; 2], [f32; 2]) {
    let mut min = quad[0];
    let mut max = quad[0];
    for corner in &quad[1..] {
        min[0] = min[0].min(corner[0]);
        min[1] = min[1].min(corner[1]);
        max[0] = max[0].max(corner[0]);
        max[1] = max[1].max(corner[1]);
    }
    (min, max)
}

fn point_in_bounds(point: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    point[0] >= min[0] && point[0] <= max[0] && point[1] >= min[1] && point[1] <= max[1]
}

/// Triangles covering one floor quad. A shadow edge splits the quad. The pieces cover the quad.
pub(crate) fn clip_cuts(
    quad: [[f32; 2]; 4],
    cuts: &[([f32; 2], [f32; 2])],
    shadows: &Shadows,
) -> Vec<([[f32; 2]; 3], u32)> {
    if cuts.is_empty() {
        return plain_quad(quad, shadows);
    }
    let mut parts: Vec<Vec<[f32; 2]>> = vec![quad.to_vec()];
    for (a, b) in cuts {
        let mut next = Vec::new();
        for part in parts {
            if !line_separates(&part, *a, *b) {
                next.push(part);
                continue;
            }
            let (left, right) = split_line(&part, *a, *b);
            if left.len() >= 3 {
                next.push(left);
            }
            if right.len() >= 3 {
                next.push(right);
            }
        }
        parts = next;
    }
    let mut out = Vec::new();
    for part in parts {
        for index in 1..part.len().saturating_sub(1) {
            let tri = [part[0], part[index], part[index + 1]];
            if area(tri) < 1.0e-8 {
                continue;
            }
            let center = centroid(tri);
            out.push((tri, mask_at(&shadows.polys, center[0], center[1])));
        }
    }
    if out.is_empty() {
        return plain_quad(quad, shadows);
    }
    out
}

fn plain_quad(quad: [[f32; 2]; 4], shadows: &Shadows) -> Vec<([[f32; 2]; 3], u32)> {
    let mid = [
        (quad[0][0] + quad[2][0]) * 0.5,
        (quad[0][1] + quad[2][1]) * 0.5,
    ];
    let mask = mask_at(&shadows.polys, mid[0], mid[1]);
    vec![
        ([quad[0], quad[1], quad[2]], mask),
        ([quad[0], quad[2], quad[3]], mask),
    ]
}

fn line_separates(poly: &[[f32; 2]], a: [f32; 2], b: [f32; 2]) -> bool {
    let mut pos = false;
    let mut neg = false;
    for point in poly {
        let turn = cross(a, b, *point);
        if turn > 1.0e-5 {
            pos = true;
        } else if turn < -1.0e-5 {
            neg = true;
        }
        if pos && neg {
            return true;
        }
    }
    false
}

fn split_line(poly: &[[f32; 2]], a: [f32; 2], b: [f32; 2]) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
    let eps = 1.0e-5;
    let mut left = Vec::new();
    let mut right = Vec::new();
    for index in 0..poly.len() {
        let cur = poly[index];
        let next = poly[(index + 1) % poly.len()];
        let sc = cross(a, b, cur);
        let sn = cross(a, b, next);
        if sc >= -eps {
            left.push(cur);
        }
        if sc <= eps {
            right.push(cur);
        }
        let crosses = (sc > eps && sn < -eps) || (sc < -eps && sn > eps);
        if crosses {
            let t = sc / (sc - sn);
            let hit = [
                cur[0] + (next[0] - cur[0]) * t,
                cur[1] + (next[1] - cur[1]) * t,
            ];
            left.push(hit);
            right.push(hit);
        }
    }
    (dedup_ring(left), dedup_ring(right))
}

fn dedup_ring(mut points: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    points.dedup_by(|a, b| (a[0] - b[0]).abs() < 1.0e-4 && (a[1] - b[1]).abs() < 1.0e-4);
    if points.len() >= 2 {
        let first = points[0];
        let last = *points.last().unwrap();
        if (first[0] - last[0]).abs() < 1.0e-4 && (first[1] - last[1]).abs() < 1.0e-4 {
            points.pop();
        }
    }
    points
}

fn hull_box(
    light: &Light,
    x: f32,
    z: f32,
    half_x: f32,
    half_z: f32,
    height: f32,
    light_index: u32,
) -> Option<Poly> {
    let mut points = Vec::new();
    for (cx, cz) in [
        (x - half_x, z - half_z),
        (x + half_x, z - half_z),
        (x + half_x, z + half_z),
        (x - half_x, z + half_z),
    ] {
        points.push([cx, cz]);
        if let Some(hit) = project(light, [cx, height, cz]) {
            points.push(hit);
        }
    }
    poly_from(points, light_index)
}

fn hull_circle(
    light: &Light,
    x: f32,
    z: f32,
    radius: f32,
    height: f32,
    light_index: u32,
) -> Option<Poly> {
    if radius <= 1.0e-4 {
        return None;
    }
    let scale = if light.position.y > height + 0.05 {
        light.position.y / (light.position.y - height)
    } else {
        1.0
    };
    let count = (std::f32::consts::TAU * radius * scale.max(1.0) / 0.02)
        .ceil()
        .clamp(32.0, 128.0) as u32;
    let mut points = Vec::with_capacity(count as usize * 2);
    for step in 0..count {
        let angle = step as f32 / count as f32 * std::f32::consts::TAU;
        let px = x + angle.cos() * radius;
        let pz = z + angle.sin() * radius;
        points.push([px, pz]);
        if let Some(hit) = project(light, [px, height, pz]) {
            points.push(hit);
        }
    }
    poly_from(points, light_index)
}

fn project(light: &Light, point: [f32; 3]) -> Option<[f32; 2]> {
    let dy = point[1] - light.position.y;
    if dy.abs() < 1.0e-5 {
        return None;
    }
    let t = -light.position.y / dy;
    if t < 0.0 {
        return None;
    }
    Some([
        light.position.x + (point[0] - light.position.x) * t,
        light.position.z + (point[2] - light.position.z) * t,
    ])
}

fn poly_from(points: Vec<[f32; 2]>, light: u32) -> Option<Poly> {
    let verts = convex_hull(points);
    if verts.len() < 3 {
        return None;
    }
    let mut min = verts[0];
    let mut max = verts[0];
    for point in &verts {
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    Some(Poly {
        light,
        verts,
        min,
        max,
    })
}

fn convex_hull(mut points: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    points.sort_by(|a, b| {
        a[0].partial_cmp(&b[0])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
    });
    points.dedup_by(|a, b| (a[0] - b[0]).abs() < 1.0e-5 && (a[1] - b[1]).abs() < 1.0e-5);
    if points.len() < 3 {
        return points;
    }
    let mut lower = Vec::new();
    for point in &points {
        while lower.len() >= 2
            && cross(lower[lower.len() - 2], lower[lower.len() - 1], *point) <= 0.0
        {
            lower.pop();
        }
        lower.push(*point);
    }
    let mut upper = Vec::new();
    for point in points.iter().rev() {
        while upper.len() >= 2
            && cross(upper[upper.len() - 2], upper[upper.len() - 1], *point) <= 0.0
        {
            upper.pop();
        }
        upper.push(*point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn cross(origin: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - origin[0]) * (b[1] - origin[1]) - (a[1] - origin[1]) * (b[0] - origin[0])
}

fn point_in_convex(x: f32, z: f32, verts: &[[f32; 2]]) -> bool {
    if verts.len() < 3 {
        return false;
    }
    let mut sign = 0.0;
    for index in 0..verts.len() {
        let a = verts[index];
        let b = verts[(index + 1) % verts.len()];
        let turn = cross(a, b, [x, z]);
        if turn.abs() <= 1.0e-5 {
            continue;
        }
        if sign == 0.0 {
            sign = turn;
        } else if turn * sign < 0.0 {
            return false;
        }
    }
    true
}

fn centroid(tri: [[f32; 2]; 3]) -> [f32; 2] {
    [
        (tri[0][0] + tri[1][0] + tri[2][0]) / 3.0,
        (tri[0][1] + tri[1][1] + tri[2][1]) / 3.0,
    ]
}

fn area(tri: [[f32; 2]; 3]) -> f32 {
    cross(tri[0], tri[1], tri[2]).abs() * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_scene::{Floor, Light, Scene, Vec3, Wall};

    fn wall_scene() -> Scene {
        Scene {
            floor: Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 8.0,
                half_z: 8.0,
                color: [1.0, 1.0, 1.0],
            },
            walls: vec![Wall {
                base: 0.0,
                position: Vec3::new(2.0, 0.0, 5.0),
                half_x: 4.0,
                half_z: 0.2,
                height: 2.6,
                color: [1.0, 1.0, 1.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            solids: Vec::new(),
            lights: vec![Light {
                position: Vec3::new(0.0, 7.0, 0.0),
                color: [1.0, 1.0, 1.0],

                direction: Vec3::ZERO,
            }],
            ceiling: None,
            sky: None,
        }
    }

    #[test]
    fn the_wall_shadow_edge_is_the_projected_silhouette() {
        let shadows = shadows(&wall_scene());
        let z = 6.0;
        let mut on_edge = false;
        for poly in &shadows.polys {
            for index in 0..poly.verts.len() {
                let a = poly.verts[index];
                let b = poly.verts[(index + 1) % poly.verts.len()];
                if (a[1] - z) * (b[1] - z) > 0.0 {
                    continue;
                }
                let dz = b[1] - a[1];
                if dz.abs() < 1.0e-6 {
                    continue;
                }
                let t = (z - a[1]) / dz;
                let x = a[0] + (b[0] - a[0]) * t;
                if (x + 2.5).abs() < 1.0e-3 {
                    on_edge = true;
                }
            }
        }
        assert!(on_edge, "projected edge is not at x=-2.5 when z=6");
        let inside = mask_at(&shadows.polys, -2.49, z);
        let outside = mask_at(&shadows.polys, -2.51, z);
        assert_ne!(inside, 0, "just inside the silhouette is lit");
        assert_eq!(outside, 0, "just outside the silhouette is shadowed");

        let dx = 16.0_f32 / (16.0_f32 / 0.03).ceil();
        let dz = dx;
        let ix = ((-2.5_f32 + 8.0) / dx).floor();
        let iz = ((6.0_f32 + 8.0) / dz).floor();
        let xb = -8.0 + ix * dx;
        let zb = -8.0 + iz * dz;
        let quad = [[xb, zb], [xb + dx, zb], [xb + dx, zb + dz], [xb, zb + dz]];
        let grid = edge_grid(&shadows, xb, zb, dx, dz, 1, 1);
        let pieces = clip_cuts(quad, grid.cuts(0, 0), &shadows);
        let mut nearest = f32::MAX;
        for (tri, _) in &pieces {
            for point in tri {
                let line = -2.0 + (-3.181818 - (-2.0)) * ((point[1] - 4.8) / (7.636363 - 4.8));
                nearest = nearest.min((point[0] - line).abs());
            }
        }
        assert!(
            nearest < 1.0e-3,
            "the cell cut missed the silhouette by {nearest}"
        );
    }
}

fn seg_intersect(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> Option<[f32; 2]> {
    let ray = [b[0] - a[0], b[1] - a[1]];
    let side = [d[0] - c[0], d[1] - c[1]];
    let denom = ray[0] * side[1] - ray[1] * side[0];
    if denom.abs() < 1.0e-8 {
        return None;
    }
    let offset = [c[0] - a[0], c[1] - a[1]];
    let t = (offset[0] * side[1] - offset[1] * side[0]) / denom;
    let u = (offset[0] * ray[1] - offset[1] * ray[0]) / denom;
    if !(-1.0e-4..=1.0 + 1.0e-4).contains(&t) || !(-1.0e-4..=1.0 + 1.0e-4).contains(&u) {
        return None;
    }
    Some([a[0] + t * ray[0], a[1] + t * ray[1]])
}
