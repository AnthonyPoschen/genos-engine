//! Where cells sit: the cubic lattice the engine uses today, or the
//! body-centred cubic (BCC) lattice.
//!
//! Both are described by integer points `k`. Cubic: cell `k` is the cube
//! `[k h, (k + 1) h)` with its centre at `(k + 0.5) h`. BCC: points are
//! `k a / 2` with all three of `k` even (cube corners) or all odd (cube
//! centres), `a` the cube edge. At equal point density `a = h 2^(1/3)`.
//!
//! A cell owns the surface points nearest to it (its Voronoi region: a cube, or
//! a truncated octahedron for BCC). A read interpolates over the points around
//! it: 8 (trilinear) for cubic, 4 (the containing tetrahedron of the BCC
//! Delaunay tetrahedralisation) for BCC.

use crate::geom::{floor_i32, round_i32, V3};

/// Lattice kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Cubic,
    Bcc,
}

impl Layout {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cubic" => Some(Self::Cubic),
            "bcc" => Some(Self::Bcc),
            _ => None,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cubic => "cubic",
            Self::Bcc => "bcc",
        }
    }
}

/// Points fetched and arithmetic counted for one interpolated read: a cost
/// proxy, not a timing. ALU counts are hand counts of the code below (adds,
/// multiplies, compares, floors), excluding the hash probe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LookupCost {
    pub fetches: u32,
    pub alu: u32,
}

/// A lattice at one spacing.
#[derive(Clone, Copy, Debug)]
pub struct Lattice {
    pub layout: Layout,
    /// Cubic: cell edge. BCC: cube edge `a`.
    pub step: f64,
}

/// Up to 8 points and weights.
pub type Stencil = ([([i32; 3], f64); 8], usize);

impl Lattice {
    /// Lattice with the point density of a cubic grid of edge `h`.
    #[must_use]
    pub fn with_density_of_cubic(layout: Layout, h: f64) -> Self {
        let step = match layout {
            Layout::Cubic => h,
            Layout::Bcc => h * 2f64.powf(1.0 / 3.0),
        };
        Self { layout, step }
    }

    /// Same layout, half the cubic-equivalent spacing (one subdivision level).
    #[must_use]
    pub fn child(self) -> Self {
        Self {
            layout: self.layout,
            step: self.step * 0.5,
        }
    }

    /// Cubic-equivalent spacing: edge of a cube holding one point.
    #[must_use]
    pub fn cubic_equivalent(self) -> f64 {
        match self.layout {
            Layout::Cubic => self.step,
            Layout::Bcc => self.step / 2f64.powf(1.0 / 3.0),
        }
    }

    /// World position of point `k`.
    #[must_use]
    pub fn position(self, k: [i32; 3]) -> V3 {
        let f = |i: i32| match self.layout {
            Layout::Cubic => (f64::from(i) + 0.5) * self.step,
            Layout::Bcc => f64::from(i) * 0.5 * self.step,
        };
        V3::new(f(k[0]), f(k[1]), f(k[2]))
    }

    /// The point whose Voronoi cell contains `p`.
    #[must_use]
    pub fn nearest(self, p: V3) -> [i32; 3] {
        let u = p.scale(1.0 / self.step);
        match self.layout {
            Layout::Cubic => [floor_i32(u.x), floor_i32(u.y), floor_i32(u.z)],
            Layout::Bcc => {
                let even = [round_i32(u.x), round_i32(u.y), round_i32(u.z)];
                let odd = [floor_i32(u.x), floor_i32(u.y), floor_i32(u.z)];
                let de: f64 = (0..3)
                    .map(|i| (u.axis(i) - f64::from(even[i])).powi(2))
                    .sum();
                let dodd: f64 = (0..3)
                    .map(|i| (u.axis(i) - f64::from(odd[i]) - 0.5).powi(2))
                    .sum();
                if de <= dodd {
                    [2 * even[0], 2 * even[1], 2 * even[2]]
                } else {
                    [2 * odd[0] + 1, 2 * odd[1] + 1, 2 * odd[2] + 1]
                }
            }
        }
    }

    /// Interpolation points and weights at `p` (weights sum to one).
    #[must_use]
    pub fn stencil(self, p: V3) -> Stencil {
        let mut out = [([0i32; 3], 0.0); 8];
        let u = p.scale(1.0 / self.step);
        match self.layout {
            Layout::Cubic => {
                let c = u.sub(V3::splat(0.5));
                let i0 = [floor_i32(c.x), floor_i32(c.y), floor_i32(c.z)];
                let f = [
                    c.x - f64::from(i0[0]),
                    c.y - f64::from(i0[1]),
                    c.z - f64::from(i0[2]),
                ];
                let mut n = 0;
                for dz in 0..2 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let w = (if dx == 1 { f[0] } else { 1.0 - f[0] })
                                * (if dy == 1 { f[1] } else { 1.0 - f[1] })
                                * (if dz == 1 { f[2] } else { 1.0 - f[2] });
                            out[n] = ([i0[0] + dx, i0[1] + dy, i0[2] + dz], w);
                            n += 1;
                        }
                    }
                }
                (out, 8)
            }
            Layout::Bcc => {
                let c = [floor_i32(u.x), floor_i32(u.y), floor_i32(u.z)];
                let f = V3::new(
                    u.x - f64::from(c[0]) - 0.5,
                    u.y - f64::from(c[1]) - 0.5,
                    u.z - f64::from(c[2]) - 0.5,
                );
                // Dominant axis k picks the neighbouring cube centre across
                // the nearer face; the larger of the other two picks the edge
                // of that face.
                let k = f.dominant_axis();
                let (j, l) = ((k + 1) % 3, (k + 2) % 3);
                let (m, o) = if f.axis(j).abs() >= f.axis(l).abs() {
                    (j, l)
                } else {
                    (l, j)
                };
                let s = if f.axis(k) >= 0.0 { 1 } else { -1 };
                let t = if f.axis(m) >= 0.0 { 1 } else { -1 };
                let b1 = [2 * c[0] + 1, 2 * c[1] + 1, 2 * c[2] + 1];
                let mut b2 = b1;
                b2[k] += 2 * s;
                let mut a1 = b1;
                a1[k] += s;
                a1[m] += t;
                let mut a2 = a1;
                a1[o] += 1;
                a2[o] -= 1;
                let verts = [b1, b2, a1, a2];
                let pos = verts.map(|v| self.position(v));
                let w = barycentric(p, pos);
                for (i, v) in verts.iter().enumerate() {
                    out[i] = (*v, w[i]);
                }
                (out, 4)
            }
        }
    }

    /// Cost proxy for one [`Lattice::stencil`] read.
    #[must_use]
    pub const fn lookup_cost(self) -> LookupCost {
        match self.layout {
            // 3 scale, 3 sub, 3 floor, 3 frac, 8 x (2 mul) weights, 8 x 3 index adds.
            Layout::Cubic => LookupCost {
                fetches: 8,
                alu: 3 + 3 + 3 + 3 + 16 + 24,
            },
            // 3 scale, 3 floor, 6 frac, 2+1 compares for axes, 2 sign compares,
            // ~12 index adds, 4 x 3 positions, 3x3 solve ~ 45.
            Layout::Bcc => LookupCost {
                fetches: 4,
                alu: 3 + 3 + 6 + 3 + 2 + 12 + 12 + 45,
            },
        }
    }
}

/// Barycentric weights of `p` in the tetrahedron `v`.
#[must_use]
pub fn barycentric(p: V3, v: [V3; 4]) -> [f64; 4] {
    let (e1, e2, e3) = (v[1].sub(v[0]), v[2].sub(v[0]), v[3].sub(v[0]));
    let d = p.sub(v[0]);
    let det = e1.dot(e2.cross(e3));
    if det.abs() < 1e-18 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    let b1 = d.dot(e2.cross(e3)) / det;
    let b2 = e1.dot(d.cross(e3)) / det;
    let b3 = e1.dot(e2.cross(d)) / det;
    [1.0 - b1 - b2 - b3, b1, b2, b3]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn bcc_stencil_is_a_containing_tetrahedron() {
        let lat = Lattice::with_density_of_cubic(Layout::Bcc, 0.25);
        let mut rng = Rng::new(3);
        for _ in 0..20_000 {
            let p = V3::new(
                rng.next_f64() * 3.0 - 1.0,
                rng.next_f64() * 3.0,
                rng.next_f64() * 2.0,
            );
            let (pts, n) = lat.stencil(p);
            assert_eq!(n, 4);
            let mut sum = 0.0;
            let mut back = V3::default();
            for (k, w) in &pts[..n] {
                assert!(*w > -1e-9, "negative weight {w} at {p:?}");
                assert_eq!(k[0].rem_euclid(2), k[1].rem_euclid(2));
                assert_eq!(k[1].rem_euclid(2), k[2].rem_euclid(2));
                sum += w;
                back = back.add(lat.position(*k).scale(*w));
            }
            assert!((sum - 1.0).abs() < 1e-9);
            assert!(back.sub(p).length() < 1e-9);
        }
    }

    #[test]
    fn nearest_is_nearest() {
        for layout in [Layout::Cubic, Layout::Bcc] {
            let lat = Lattice::with_density_of_cubic(layout, 0.25);
            let mut rng = Rng::new(5);
            for _ in 0..5_000 {
                let p = V3::new(
                    rng.next_f64() * 2.0,
                    rng.next_f64() * 2.0,
                    rng.next_f64() * 2.0,
                );
                let k = lat.nearest(p);
                let d = lat.position(k).sub(p).length();
                let (pts, n) = lat.stencil(p);
                for (q, _) in &pts[..n] {
                    assert!(lat.position(*q).sub(p).length() >= d - 1e-9);
                }
            }
        }
    }
}
