//! A binned-SAH bounding volume hierarchy over triangles.
//!
//! Build: 16 bins per axis on the triangle centroids, leaves of at most 4 triangles,
//! split only when the SAH says it pays. Nodes are flattened depth-first, so the
//! left child follows its parent. Queries: first hit along a ray (with the
//! one-sided rule of [`MeshTriangle`]), any hit (shadow rays), and the closest
//! triangle to a point (distance field baking).

use crate::triangle::{cross, dot, sub, Aabb, MeshTriangle};

const BINS: usize = 16;
const LEAF: usize = 4;
/// Hits closer than this are ignored, as in the analytic tracer.
pub const T_MIN: f32 = 1.0e-3;

#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Aabb,
    /// Leaf: first triangle in `order`. Inner: index of the right child.
    first: u32,
    /// Triangles in a leaf; 0 for an inner node.
    count: u32,
}

/// The hierarchy. It stores indices, not triangles; queries take the slice it was built on.
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

/// The first triangle a ray meets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub triangle: u32,
    pub t: f32,
    /// Barycentric weights of corners 1 and 2.
    pub u: f32,
    pub v: f32,
    /// True when the ray met the back of a two-sided triangle.
    pub back: bool,
}

impl Bvh {
    pub fn build(triangles: &[MeshTriangle]) -> Bvh {
        let bounds: Vec<Aabb> = triangles.iter().map(MeshTriangle::bounds).collect();
        let centres: Vec<[f32; 3]> = bounds.iter().map(Aabb::centre).collect();
        let mut order: Vec<u32> = (0..triangles.len() as u32).collect();
        let mut nodes = Vec::with_capacity(triangles.len().max(1) * 2);
        if !triangles.is_empty() {
            build_node(
                &mut nodes,
                &mut order,
                0,
                triangles.len(),
                &bounds,
                &centres,
            );
        }
        Bvh { nodes, order }
    }

    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map_or(Aabb::EMPTY, |n| n.bounds)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The nearest hit with `T_MIN < t < t_max`.
    pub fn intersect(
        &self,
        tris: &[MeshTriangle],
        origin: [f32; 3],
        dir: [f32; 3],
        t_max: f32,
    ) -> Option<RayHit> {
        let mut best: Option<RayHit> = None;
        let mut limit = t_max;
        self.walk(origin, dir, &mut limit, &mut |i, limit| {
            if let Some(hit) = hit_triangle(&tris[i as usize], i, origin, dir, *limit) {
                *limit = hit.t;
                best = Some(hit);
            }
            false
        });
        best
    }

    /// True when anything is hit with `T_MIN < t < t_max`.
    pub fn occluded(
        &self,
        tris: &[MeshTriangle],
        origin: [f32; 3],
        dir: [f32; 3],
        t_max: f32,
    ) -> bool {
        let mut limit = t_max;
        let mut found = false;
        self.walk(origin, dir, &mut limit, &mut |i, limit| {
            found = hit_triangle(&tris[i as usize], i, origin, dir, *limit).is_some();
            found
        });
        found
    }

    /// How many triangles a ray crosses beyond `T_MIN`, both sides counted. An odd
    /// count from a point means it is inside a closed mesh.
    pub fn crossings(&self, tris: &[MeshTriangle], origin: [f32; 3], dir: [f32; 3]) -> u32 {
        let mut count = 0;
        let mut limit = f32::INFINITY;
        self.walk(origin, dir, &mut limit, &mut |i, limit| {
            let tri = MeshTriangle {
                two_sided: true,
                ..tris[i as usize]
            };
            if hit_triangle(&tri, i, origin, dir, *limit).is_some() {
                count += 1;
            }
            false
        });
        count
    }

    /// Closest triangle to `p` within `max_dist`: (triangle, closest point, distance).
    pub fn closest(
        &self,
        tris: &[MeshTriangle],
        p: [f32; 3],
        max_dist: f32,
    ) -> Option<(u32, [f32; 3], f32)> {
        let mut best: Option<(u32, [f32; 3], f32)> = None;
        let mut best_d2 = max_dist * max_dist;
        if self.nodes.is_empty() {
            return None;
        }
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if node.bounds.distance2(p) >= best_d2 {
                continue;
            }
            if node.count > 0 {
                for &i in &self.order[node.first as usize..(node.first + node.count) as usize] {
                    let q = tris[i as usize].closest_point(p);
                    let d = sub(q, p);
                    let d2 = dot(d, d);
                    if d2 < best_d2 {
                        best_d2 = d2;
                        best = Some((i, q, d2.sqrt()));
                    }
                }
            } else {
                let (l, r) = (n + 1, node.first);
                let dl = self.nodes[l as usize].bounds.distance2(p);
                let dr = self.nodes[r as usize].bounds.distance2(p);
                // Nearer child last, so it is popped first.
                if dl < dr {
                    stack.push(r);
                    stack.push(l);
                } else {
                    stack.push(l);
                    stack.push(r);
                }
            }
        }
        best
    }

    /// Visit leaf triangles whose boxes the ray enters before `limit`, near child first.
    /// `visit` returns true to stop.
    fn walk(
        &self,
        origin: [f32; 3],
        dir: [f32; 3],
        limit: &mut f32,
        visit: &mut dyn FnMut(u32, &mut f32) -> bool,
    ) {
        if self.nodes.is_empty() {
            return;
        }
        let inv: [f32; 3] = std::array::from_fn(|a| 1.0 / dir[a]);
        let mut stack = [0u32; 64];
        let mut top = 1usize;
        while top > 0 {
            top -= 1;
            let n = stack[top];
            let node = &self.nodes[n as usize];
            if slab(&node.bounds, origin, inv, *limit).is_none() {
                continue;
            }
            if node.count > 0 {
                for &i in &self.order[node.first as usize..(node.first + node.count) as usize] {
                    if visit(i, limit) {
                        return;
                    }
                }
                continue;
            }
            let (l, r) = (n + 1, node.first);
            let tl = slab(&self.nodes[l as usize].bounds, origin, inv, *limit);
            let tr = slab(&self.nodes[r as usize].bounds, origin, inv, *limit);
            let (near, far) = match (tl, tr) {
                (Some(a), Some(b)) if b < a => (Some(r), Some(l)),
                (Some(_), Some(_)) => (Some(l), Some(r)),
                (Some(_), None) => (Some(l), None),
                (None, Some(_)) => (Some(r), None),
                (None, None) => (None, None),
            };
            for child in [far, near].into_iter().flatten() {
                if top < stack.len() {
                    stack[top] = child;
                    top += 1;
                }
            }
        }
    }
}

fn slab(b: &Aabb, origin: [f32; 3], inv: [f32; 3], limit: f32) -> Option<f32> {
    let mut t0 = 0.0f32;
    let mut t1 = limit;
    for a in 0..3 {
        let mut ta = (b.min[a] - origin[a]) * inv[a];
        let mut tb = (b.max[a] - origin[a]) * inv[a];
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        // NaN (0 x inf on a flat box edge) must not reject the box.
        if ta.is_nan() || tb.is_nan() {
            continue;
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// Möller-Trumbore with the one-sided rule.
fn hit_triangle(
    tri: &MeshTriangle,
    index: u32,
    origin: [f32; 3],
    dir: [f32; 3],
    t_max: f32,
) -> Option<RayHit> {
    let [a, b, c] = tri.positions;
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(dir, e2);
    let det = dot(e1, p);
    // det > 0: the ray meets the front (counter-clockwise) face.
    let back = det < 0.0;
    if det.abs() < 1.0e-12 || (back && !tri.two_sided) {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(origin, a);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(dir, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    if t <= T_MIN || t >= t_max {
        return None;
    }
    Some(RayHit {
        triangle: index,
        t,
        u,
        v,
        back,
    })
}

fn build_node(
    nodes: &mut Vec<Node>,
    order: &mut [u32],
    start: usize,
    end: usize,
    bounds: &[Aabb],
    centres: &[[f32; 3]],
) -> u32 {
    let me = nodes.len() as u32;
    let mut box_all = Aabb::EMPTY;
    let mut box_c = Aabb::EMPTY;
    for &i in &order[start..end] {
        box_all = box_all.union(&bounds[i as usize]);
        box_c.grow(centres[i as usize]);
    }
    nodes.push(Node {
        bounds: box_all,
        first: start as u32,
        count: (end - start) as u32,
    });
    let count = end - start;
    if count <= LEAF {
        return me;
    }
    // Binned SAH over the widest useful axes.
    let mut best: Option<(f32, usize, f32)> = None;
    for axis in 0..3 {
        let lo = box_c.min[axis];
        let extent = box_c.max[axis] - lo;
        if extent <= 0.0 {
            continue;
        }
        let mut bin_box = [Aabb::EMPTY; BINS];
        let mut bin_n = [0usize; BINS];
        let k = BINS as f32 / extent;
        for &i in &order[start..end] {
            let b = (((centres[i as usize][axis] - lo) * k) as usize).min(BINS - 1);
            bin_box[b] = bin_box[b].union(&bounds[i as usize]);
            bin_n[b] += 1;
        }
        let mut right_area = [0.0f32; BINS];
        let mut right_n = [0usize; BINS];
        let (mut acc, mut n) = (Aabb::EMPTY, 0);
        for b in (1..BINS).rev() {
            acc = acc.union(&bin_box[b]);
            n += bin_n[b];
            right_area[b] = acc.half_area();
            right_n[b] = n;
        }
        let (mut acc, mut n) = (Aabb::EMPTY, 0);
        for b in 0..BINS - 1 {
            acc = acc.union(&bin_box[b]);
            n += bin_n[b];
            if n == 0 || right_n[b + 1] == 0 {
                continue;
            }
            let cost = acc.half_area() * n as f32 + right_area[b + 1] * right_n[b + 1] as f32;
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, axis, lo + (b + 1) as f32 / k));
            }
        }
    }
    let leaf_cost = box_all.half_area() * count as f32;
    let split = match best {
        Some((cost, axis, at)) if cost < leaf_cost || count > 16 => {
            let slice = &mut order[start..end];
            let mut left = 0;
            for j in 0..slice.len() {
                if centres[slice[j] as usize][axis] < at {
                    slice.swap(left, j);
                    left += 1;
                }
            }
            if left == 0 || left == slice.len() {
                count / 2
            } else {
                left
            }
        }
        // All centroids equal, or the SAH prefers a leaf: split in half only if big.
        _ if count > 16 => count / 2,
        _ => return me,
    };
    nodes[me as usize].count = 0;
    build_node(nodes, order, start, start + split, bounds, centres);
    let right = build_node(nodes, order, start + split, end, bounds, centres);
    nodes[me as usize].first = right;
    me
}
