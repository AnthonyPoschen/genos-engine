//! One triangle with shading normals and a material slot, and axis-aligned boxes.

/// Axis-aligned box. An empty box has `min > max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };

    pub fn grow(&mut self, p: [f32; 3]) {
        for a in 0..3 {
            self.min[a] = self.min[a].min(p[a]);
            self.max[a] = self.max[a].max(p[a]);
        }
    }

    pub fn union(mut self, other: &Aabb) -> Aabb {
        self.grow(other.min);
        self.grow(other.max);
        self
    }

    pub fn centre(&self) -> [f32; 3] {
        std::array::from_fn(|a| 0.5 * (self.min[a] + self.max[a]))
    }

    pub fn is_empty(&self) -> bool {
        (0..3).any(|a| self.min[a] > self.max[a])
    }

    /// Half the surface area (the SAH only needs ratios).
    pub fn half_area(&self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let d: [f32; 3] = std::array::from_fn(|a| self.max[a] - self.min[a]);
        d[0] * d[1] + d[1] * d[2] + d[2] * d[0]
    }

    /// Squared distance from `p` to the box (0 inside).
    pub fn distance2(&self, p: [f32; 3]) -> f32 {
        (0..3)
            .map(|a| {
                let d = (self.min[a] - p[a]).max(p[a] - self.max[a]).max(0.0);
                d * d
            })
            .sum()
    }
}

/// A triangle in some space (mesh or world), with per-corner shading normals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshTriangle {
    pub positions: [[f32; 3]; 3],
    /// Shading normals per corner. Flat triangles repeat the face normal.
    pub normals: [[f32; 3]; 3],
    /// Texture coordinates per corner (zero when the mesh has none).
    pub uvs: [[f32; 2]; 3],
    /// Index into the owner's material table.
    pub material: u32,
    /// Both faces are hit. A one-sided triangle is only hit from its front
    /// (counter-clockwise) side, so closed solids are invisible from inside.
    pub two_sided: bool,
}

impl MeshTriangle {
    /// A flat triangle; the normal comes from the winding.
    pub fn flat(positions: [[f32; 3]; 3], material: u32, two_sided: bool) -> Self {
        let n = face_normal(&positions);
        Self {
            positions,
            normals: [n; 3],
            uvs: [[0.0; 2]; 3],
            material,
            two_sided,
        }
    }

    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for p in self.positions {
            b.grow(p);
        }
        b
    }

    /// Unit normal from the winding (counter-clockwise is the front).
    pub fn face_normal(&self) -> [f32; 3] {
        face_normal(&self.positions)
    }

    pub fn area(&self) -> f32 {
        let [a, b, c] = self.positions;
        0.5 * length(cross(sub(b, a), sub(c, a)))
    }

    /// Shading normal at barycentric (u, v) (weights of corners 1 and 2).
    pub fn normal_at(&self, u: f32, v: f32) -> [f32; 3] {
        let w = 1.0 - u - v;
        let n: [f32; 3] = std::array::from_fn(|a| {
            w * self.normals[0][a] + u * self.normals[1][a] + v * self.normals[2][a]
        });
        normalize(n)
    }

    pub fn uv_at(&self, u: f32, v: f32) -> [f32; 2] {
        let w = 1.0 - u - v;
        std::array::from_fn(|a| w * self.uvs[0][a] + u * self.uvs[1][a] + v * self.uvs[2][a])
    }

    /// Closest point on the triangle to `p` (Ericson, Real-Time Collision Detection 5.1.5).
    pub fn closest_point(&self, p: [f32; 3]) -> [f32; 3] {
        let [a, b, c] = self.positions;
        let ab = sub(b, a);
        let ac = sub(c, a);
        let ap = sub(p, a);
        let d1 = dot(ab, ap);
        let d2 = dot(ac, ap);
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = sub(p, b);
        let d3 = dot(ab, bp);
        let d4 = dot(ac, bp);
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            return add(a, scale(ab, v));
        }
        let cp = sub(p, c);
        let d5 = dot(ab, cp);
        let d6 = dot(ac, cp);
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let w = d2 / (d2 - d6);
            return add(a, scale(ac, w));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            return add(b, scale(sub(c, b), w));
        }
        let denom = 1.0 / (va + vb + vc);
        let v = vb * denom;
        let w = vc * denom;
        add(a, add(scale(ab, v), scale(ac, w)))
    }
}

fn face_normal(p: &[[f32; 3]; 3]) -> [f32; 3] {
    normalize(cross(sub(p[1], p[0]), sub(p[2], p[0])))
}

pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
pub(crate) fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        [0.0, 1.0, 0.0]
    }
}
