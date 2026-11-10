#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![cfg_attr(
    test,
    expect(
        clippy::float_cmp,
        reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
    )
)]
#![expect(
    clippy::many_single_char_names,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! The analytic scene as triangles: floor, roof, walls, square and round solids.
//!
//! GI traces meshes only, so the authored shapes become meshes with the same
//! surfaces the analytic tracer in `trace.rs` hits: the floor and the roof are
//! two-sided planes at y = 0 and at the roof height (the footprint grown by 5 cm,
//! as `on_floor` does); walls and solids are closed, one-sided boxes and cylinders
//! (invisible from inside, like the analytic slab test). A round solid is a
//! 64-sided prism with smooth side normals, so its shading matches the true
//! cylinder and only its silhouette moves (by at most 0.12 % of the radius).

use genos_mesh::{Bvh, MeshTriangle};
use genos_scene::{Scene, Shape, Solid, Wall};

use crate::trace::Hit;

/// Sides of a round solid.
pub const CIRCLE_SEGMENTS: usize = 64;
/// The analytic floor test accepts points this far outside the footprint.
const FLOOR_MARGIN: f32 = 0.05;

/// Which authored shape a triangle came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeRef {
    Floor,
    Ceiling,
    Wall(usize),
    Solid(usize),
}

/// Surface properties of one shape (one material per shape).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceMaterial {
    pub albedo: [f32; 3],
    /// Emitted radiance. Authored shapes emit nothing; glTF materials can.
    pub emissive: [f32; 3],
    pub source: ShapeRef,
}

/// Triangles for a whole scene, with a BVH for the CPU tracers.
#[derive(Clone, Debug)]
pub struct SceneMesh {
    pub triangles: Vec<MeshTriangle>,
    pub materials: Vec<SurfaceMaterial>,
    bvh: Bvh,
}

impl SceneMesh {
    /// Triangulate every surface the analytic tracer sees.
    pub fn from_scene(scene: &Scene) -> SceneMesh {
        let mut triangles = Vec::new();
        let mut materials = Vec::new();
        let mut material = |source: ShapeRef, albedo: [f32; 3]| {
            materials.push(SurfaceMaterial {
                albedo,
                emissive: [0.0; 3],
                source,
            });
            (materials.len() - 1) as u32
        };
        let position = scene.floor.position;
        let half_x = scene.floor.half_x;
        let half_z = scene.floor.half_z;
        let floor_color = scene.floor.color;
        let place = scene.floor_translation();
        let (x, z, y) = if same_point(place, position) {
            (position.x, position.z, 0.0)
        } else {
            (place.x, place.z, place.y - position.y)
        };
        let (x0, x1) = (x - half_x - FLOOR_MARGIN, x + half_x + FLOOR_MARGIN);
        let (z0, z1) = (z - half_z - FLOOR_MARGIN, z + half_z + FLOOR_MARGIN);
        let m = material(ShapeRef::Floor, floor_color);
        plane(&mut triangles, y, [x0, z0, x1, z1], m);
        if let Some(ceiling) = scene.ceiling.clone() {
            let m = material(ShapeRef::Ceiling, ceiling.color);
            plane(&mut triangles, ceiling.height + y, [x0, z0, x1, z1], m);
        }
        for i in 0..scene.wall_len() {
            let color = scene.wall(i).color;
            let m = material(ShapeRef::Wall(i), color);
            let anchor = scene.wall_translation(i);
            let yaw = scene.wall_yaw(i);
            let wall = scene.wall(i).clone();
            if yaw == 0.0 && same_point(anchor, wall.position) {
                wall_box(&mut triangles, &wall, m);
            } else {
                wall_box_world(&mut triangles, &wall, anchor, yaw, m);
            }
        }
        for i in 0..scene.solid_len() {
            let color = scene.solid(i).color;
            let m = material(ShapeRef::Solid(i), color);
            let anchor = scene.solid_translation(i);
            let yaw = scene.solid_yaw(i);
            let solid = scene.solid(i).clone();
            let rooted = same_point(anchor, solid.position)
                && (solid.shape == Shape::Circle || yaw.to_bits() == solid.yaw.to_bits());
            if rooted {
                match solid.shape {
                    Shape::Square => square_solid(&mut triangles, &solid, m),
                    Shape::Circle => round_solid(&mut triangles, &solid, m),
                }
            } else {
                let mut posed = solid.clone();
                posed.position = anchor;
                if solid.shape == Shape::Square {
                    posed.yaw = yaw;
                }
                let y_shift = anchor.y - solid.position.y;
                match posed.shape {
                    Shape::Square => square_solid_at(&mut triangles, &posed, y_shift, m),
                    Shape::Circle => round_solid_at(&mut triangles, &posed, y_shift, m),
                }
            }
        }
        let bvh = Bvh::build(&triangles);
        SceneMesh {
            triangles,
            materials,
            bvh,
        }
    }

    /// The first surface along `dir` within `reach`, as `trace::trace` reports it:
    /// the normal faces the ray.
    pub fn trace(&self, origin: [f32; 3], dir: [f32; 3], reach: f32) -> Option<Hit> {
        let hit = self.bvh.intersect(&self.triangles, origin, dir, reach)?;
        let tri = &self.triangles[hit.triangle as usize];
        let mut normal = tri.normal_at(hit.u, hit.v);
        if hit.back {
            normal = normal.map(|c| -c);
        }
        Some(Hit {
            point: std::array::from_fn(|a| origin[a] + dir[a] * hit.t),
            normal,
            albedo: self.materials[tri.material as usize].albedo,
        })
    }

    /// True when something lies between `from` and `to`.
    pub fn occluded(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        let d: [f32; 3] = std::array::from_fn(|a| to[a] - from[a]);
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist < 1.0e-3 {
            return false;
        }
        let dir = d.map(|c| c / dist);
        self.bvh.occluded(&self.triangles, from, dir, dist - 1.0e-3)
    }
}

/// A horizontal two-sided rectangle; its front faces up.
fn plane(out: &mut Vec<MeshTriangle>, y: f32, [x0, z0, x1, z1]: [f32; 4], m: u32) {
    let (a, b, c, d) = ([x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]);
    // Counter-clockwise seen from +y: a, d, c and a, c, b.
    out.push(MeshTriangle::flat([a, d, c], m, true));
    out.push(MeshTriangle::flat([a, c, b], m, true));
}

/// A closed box from 8 corners (bit 0: +x, bit 1: +y, bit 2: +z), faces outward.
fn closed_box(out: &mut Vec<MeshTriangle>, corner: impl Fn(usize) -> [f32; 3], m: u32) {
    // Each face lists its corners counter-clockwise seen from outside.
    const FACES: [[usize; 4]; 6] = [
        [1, 3, 7, 5], // +x
        [0, 4, 6, 2], // -x
        [2, 6, 7, 3], // +y
        [0, 1, 5, 4], // -y
        [4, 5, 7, 6], // +z
        [0, 2, 3, 1], // -z
    ];
    for f in FACES {
        let [a, b, c, d] = f.map(&corner);
        out.push(MeshTriangle::flat([a, b, c], m, false));
        out.push(MeshTriangle::flat([a, c, d], m, false));
    }
}

fn same_point(world: genos_scene::Vec3, local: genos_scene::Vec3) -> bool {
    world.x.to_bits() == local.x.to_bits()
        && world.y.to_bits() == local.y.to_bits()
        && world.z.to_bits() == local.z.to_bits()
}

fn wall_box_world(
    out: &mut Vec<MeshTriangle>,
    w: &Wall,
    anchor: genos_scene::Vec3,
    yaw: f32,
    m: u32,
) {
    let y_shift = anchor.y - w.position.y;
    let (sin, cos) = yaw.sin_cos();
    closed_box(
        out,
        |i| {
            let lx = if i & 1 == 1 { w.half_x } else { -w.half_x };
            let y = if i & 2 == 2 {
                w.base + w.height + y_shift
            } else {
                w.base + y_shift
            };
            let lz = if i & 4 == 4 { w.half_z } else { -w.half_z };
            [
                anchor.x + lx * cos + lz * sin,
                y,
                anchor.z - lx * sin + lz * cos,
            ]
        },
        m,
    );
}

fn square_solid_at(out: &mut Vec<MeshTriangle>, s: &Solid, y_shift: f32, m: u32) {
    let h = s.size * 0.5;
    closed_box(
        out,
        |i| {
            let lx = if i & 1 == 1 { h } else { -h };
            let y = if i & 2 == 2 { s.height } else { 0.0 };
            let lz = if i & 4 == 4 { h } else { -h };
            let mut point = solid_point(s, lx, y, lz);
            point[1] += y_shift;
            point
        },
        m,
    );
}

fn round_solid_at(out: &mut Vec<MeshTriangle>, s: &Solid, y_shift: f32, m: u32) {
    let first = out.len();
    round_solid(out, s, m);
    if y_shift != 0.0 {
        for tri in &mut out[first..] {
            for vert in &mut tri.positions {
                vert[1] += y_shift;
            }
        }
    }
}

fn wall_box(out: &mut Vec<MeshTriangle>, w: &Wall, m: u32) {
    let lo = [w.position.x - w.half_x, w.base, w.position.z - w.half_z];
    let hi = [
        w.position.x + w.half_x,
        w.base + w.height,
        w.position.z + w.half_z,
    ];
    closed_box(
        out,
        |i| std::array::from_fn(|a| if i >> a & 1 == 1 { hi[a] } else { lo[a] }),
        m,
    );
}

/// Local (x, z) about the solid's centre to world, as `trace::world_dir` turns it.
fn solid_point(s: &Solid, lx: f32, y: f32, lz: f32) -> [f32; 3] {
    let (sn, c) = s.yaw.sin_cos();
    [
        s.position.x + lx * c + lz * sn,
        y,
        s.position.z - lx * sn + lz * c,
    ]
}

fn square_solid(out: &mut Vec<MeshTriangle>, s: &Solid, m: u32) {
    let h = s.size * 0.5;
    closed_box(
        out,
        |i| {
            let lx = if i & 1 == 1 { h } else { -h };
            let y = if i & 2 == 2 { s.height } else { 0.0 };
            let lz = if i & 4 == 4 { h } else { -h };
            solid_point(s, lx, y, lz)
        },
        m,
    );
}

fn round_solid(out: &mut Vec<MeshTriangle>, s: &Solid, m: u32) {
    let r = s.size * 0.5;
    let n = CIRCLE_SEGMENTS;
    let ring = |k: usize| {
        let a = std::f32::consts::TAU * (k % n) as f32 / n as f32;
        (a.cos(), a.sin())
    };
    let centre_lo = [s.position.x, 0.0, s.position.z];
    let centre_hi = [s.position.x, s.height, s.position.z];
    for k in 0..n {
        let (c0, s0) = ring(k);
        let (c1, s1) = ring(k + 1);
        let p = |c: f32, sn: f32, y: f32| [s.position.x + r * c, y, s.position.z + r * sn];
        let (a, b) = (p(c0, s0, 0.0), p(c1, s1, 0.0));
        let (a2, b2) = (p(c0, s0, s.height), p(c1, s1, s.height));
        let (na, nb) = ([c0, 0.0, s0], [c1, 0.0, s1]);
        // Side quad, counter-clockwise from outside: a, a2, b2 and a, b2, b.
        for (pos, nrm) in [([a, a2, b2], [na, na, nb]), ([a, b2, b], [na, nb, nb])] {
            out.push(MeshTriangle {
                normals: nrm,
                ..MeshTriangle::flat(pos, m, false)
            });
        }
        out.push(MeshTriangle::flat([centre_hi, b2, a2], m, false));
        out.push(MeshTriangle::flat([centre_lo, a, b], m, false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::trace;
    use genos_scene::{Ceiling, Floor, Vec3};

    fn scene() -> Scene {
        let wall = |x: f32, z: f32, hx: f32, hz: f32, base: f32| Wall {
            position: Vec3::new(x, 0.0, z),
            half_x: hx,
            half_z: hz,
            height: 2.0,
            base,
            color: [0.9, 0.5, 0.2],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        };
        let solid = |shape: Shape, x: f32, z: f32, yaw: f32| Solid {
            shape,
            position: Vec3::new(x, 0.0, z),
            size: 1.2,
            height: 1.5,
            yaw,
            color: [0.2, 0.6, 0.9],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        };
        {
            let mut __scene = Scene::from_lists(
                Floor {
                    position: Vec3::new(1.0, 0.0, -2.0),
                    half_x: 9.0,
                    half_z: 7.0,
                    color: [0.8, 0.8, 0.8],
                },
                vec![
                    wall(3.0, 2.0, 0.1, 4.0, 0.0),
                    wall(-4.0, -1.0, 3.0, 0.15, 1.0),
                ],
                vec![
                    solid(Shape::Square, -1.0, 2.0, 0.6),
                    solid(Shape::Circle, 1.5, -3.0, 0.0),
                ],
                Vec::new(),
            );
            __scene.ceiling = Some(Ceiling {
                height: 3.0,
                color: [0.7, 0.7, 0.7],
            });
            __scene.sky = None;
            __scene.lighting = genos_scene::LightingSettings::default();
            __scene.debug = genos_scene::DebugSettings::default();
            __scene.fps_counter = genos_scene::FpsCounterSettings::default();
            __scene.event_loop = genos_scene::EventLoopSettings::default();
            __scene.reflections = genos_scene::ReflectionSettings::default();
            __scene.hdr = genos_scene::HdrSettings::legacy_bytes();
            __scene.fog = genos_scene::FogSettings::default();
            __scene.menu = genos_scene::MenuSettings::default();
            __scene
        }
    }

    /// Rays from many points in many directions meet the same surface in both forms.
    #[test]
    fn triangles_hit_what_the_analytic_shapes_hit() {
        let scene = scene();
        let mesh = SceneMesh::from_scene(&scene);
        let (mut agree, mut total, mut normal_off) = (0, 0, 0);
        let mut seed = 12345u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        for _ in 0..20000 {
            let o = [rnd() * 20.0 - 9.0, rnd() * 2.9 + 0.05, rnd() * 16.0 - 10.0];
            let d = [rnd() - 0.5, rnd() - 0.5, rnd() - 0.5];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let d = d.map(|c| c / l);
            let a = trace(&scene, o, d, 40.0);
            let b = mesh.trace(o, d, 40.0);
            total += 1;
            match (a, b) {
                (None, None) => agree += 1,
                (Some(a), Some(b)) => {
                    let dp: f32 = (0..3)
                        .map(|i| (a.point[i] - b.point[i]).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    if dp < 0.01 && a.albedo == b.albedo {
                        agree += 1;
                        let dn: f32 = (0..3).map(|i| a.normal[i] * b.normal[i]).sum();
                        if dn < 0.999 {
                            normal_off += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        // Disagreement comes from rays starting inside a solid (the analytic cylinder
        // is seen from inside, the closed mesh is not) and the 64-gon silhouette.
        let rate = agree as f32 / total as f32;
        assert!(rate > 0.995, "agree {agree}/{total}");
        assert!(normal_off * 200 < total, "normals off {normal_off}");
    }

    #[test]
    fn counts_and_sources() {
        let mesh = SceneMesh::from_scene(&scene());
        // floor 2 + roof 2 + 2 walls x 12 + square 12 + round 64 x 4.
        assert_eq!(mesh.triangles.len(), 2 + 2 + 24 + 12 + CIRCLE_SEGMENTS * 4);
        assert_eq!(mesh.materials[0].source, ShapeRef::Floor);
        assert_eq!(mesh.materials[5].source, ShapeRef::Solid(1));
        assert!(mesh.triangles.iter().all(|t| t.area() > 0.0));
    }
}
