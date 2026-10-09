use genos_scene::{Scene, Shape};

use crate::field::Field;

use crate::field::{illuminate_facing, illuminate_resolved};
use crate::shadow::{self, Shadows};
use crate::world::{transform_pose, Bounds, Displacement, DrawKind, FixedPart, ShaderSpace, World};

pub(crate) struct ShadedObject {
    pub hidden: bool,
    pub bounds: Bounds,
    pub vertices: Vec<LitVertex>,
}

/// A shaded corner before the cached field supplies the bounce.
#[derive(Clone, Copy)]
pub(crate) struct LitVertex {
    pub pos: [f32; 3],
    pub albedo: [f32; 3],
    pub direct: f32,
    /// A level face. Its shadow keeps enough bounced light for a lit wall to show.
    pub level: bool,
}

pub(crate) fn compose(albedo: [f32; 3], direct: f32, bounce: [f32; 3], _level: bool) -> [f32; 3] {
    let reflect = genos_scene::DEFAULT_BRDF;
    [
        tone(albedo[0] * reflect * (direct + bounce[0])),
        tone(albedo[1] * reflect * (direct + bounce[1])),
        tone(albedo[2] * reflect * (direct + bounce[2])),
    ]
}

/// A shadow stays linear. A hot corridor wall bends instead of clipping.
pub(crate) fn tone(channel: f32) -> f32 {
    if channel <= 0.64 {
        return channel.max(0.0);
    }
    let extra = channel - 0.64;
    0.64 + 0.14 * (extra / (extra + 1.1))
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

/// Shaded triangles for every object. Hidden objects keep their flag and no triangles.
/// Occlusion uses `light`, which has already dropped objects that do not affect light.
/// Fixed-part indices still read `world.scene`, because `light` reindexes those lists.
pub(crate) fn shade_world(
    world: &World,
    light: &genos_scene::Scene,
    field: &Field,
) -> Vec<ShadedObject> {
    let floor_shadows = shadow::shadows(light);
    let mut shaded = Vec::new();
    for object in &world.objects {
        if object.hidden {
            shaded.push(ShadedObject {
                hidden: true,
                bounds: object.bounds,
                vertices: Vec::new(),
            });
            continue;
        }
        let mut vertices = Vec::new();
        match &object.kind {
            DrawKind::Fixed(part) => push_fixed(
                &mut vertices,
                &world.scene,
                light,
                field,
                &floor_shadows,
                part,
            ),
            DrawKind::Mesh {
                vertices: mesh,
                color,
                pose,
                ..
            } => push_mesh(&mut vertices, light, field, mesh, *color, pose),
            DrawKind::Particles {
                points,
                color,
                size,
                density,
                ..
            } => {
                if *density <= 0.0 {
                    push_particles(&mut vertices, light, field, points, *color, *size);
                }
            }
            DrawKind::Shader {
                space: ShaderSpace::Screen,
                ..
            } => {}
            DrawKind::Shader {
                space: ShaderSpace::Mesh,
                displacement: Some(map),
                color,
            } => push_displacement(&mut vertices, light, field, map, *color),
            DrawKind::Shader {
                space: ShaderSpace::Mesh,
                displacement: None,
                ..
            } => {}
        }
        shaded.push(ShadedObject {
            hidden: false,
            bounds: object.bounds,
            vertices,
        });
    }
    shaded
}

fn push_fixed(
    out: &mut Vec<LitVertex>,
    geom: &Scene,
    light: &Scene,
    field: &Field,
    floor_shadows: &Shadows,
    part: &FixedPart,
) {
    match part {
        FixedPart::Floor => push_floor(out, geom, light, field, floor_shadows),
        // The CPU-lit mesh has no roof term yet. The GPU path draws and lights it.
        FixedPart::Ceiling => {}
        FixedPart::Wall(index) => {
            let Some(wall) = geom.walls.get(*index) else {
                return;
            };
            push_lit_box(
                out,
                light,
                field,
                wall.color,
                wall.position.x,
                wall.base + wall.height * 0.5,
                wall.position.z,
                wall.half_x,
                wall.height * 0.5,
                wall.half_z,
                wall.base > 0.0,
            );
        }
        FixedPart::Solid(index) => {
            let Some(solid) = geom.solids.get(*index) else {
                return;
            };
            match solid.shape {
                Shape::Square => push_lit_box(
                    out,
                    light,
                    field,
                    solid.color,
                    solid.position.x,
                    solid.height * 0.5,
                    solid.position.z,
                    solid.size * 0.5,
                    solid.height * 0.5,
                    solid.size * 0.5,
                    false,
                ),
                Shape::Circle => push_lit_cylinder(
                    out,
                    light,
                    field,
                    solid.color,
                    solid.position.x,
                    solid.position.z,
                    solid.size * 0.5,
                    solid.height,
                ),
            }
        }
    }
}

fn push_mesh(
    out: &mut Vec<LitVertex>,
    scene: &Scene,
    field: &Field,
    vertices: &[[f32; 3]],
    albedo: [f32; 3],
    pose: &[f32; 16],
) {
    let _ = field;
    if vertices.len() < 3 {
        return;
    }
    for tri in vertices.chunks(3) {
        if tri.len() < 3 {
            break;
        }
        let a = transform_pose(pose, tri[0]);
        let b = transform_pose(pose, tri[1]);
        let c = transform_pose(pose, tri[2]);
        let normal = face_normal(a, b, c);
        push_tri_colored(
            out,
            a,
            b,
            c,
            albedo,
            direct_at(scene, a[0], a[1], a[2], normal),
            direct_at(scene, b[0], b[1], b[2], normal),
            direct_at(scene, c[0], c[1], c[2], normal),
        );
    }
}

fn push_particles(
    out: &mut Vec<LitVertex>,
    scene: &Scene,
    field: &Field,
    points: &[[f32; 3]],
    albedo: [f32; 3],
    size: f32,
) {
    let half = size.max(0.02) * 0.5;
    for point in points {
        let direct = direct_at(scene, point[0], point[1], point[2], [0.0, 1.0, 0.0]);
        let _ = field;
        push_quad(
            out,
            [point[0] - half, point[1], point[2]],
            [point[0] + half, point[1], point[2]],
            [point[0] + half, point[1] + size, point[2]],
            [point[0] - half, point[1] + size, point[2]],
            albedo,
            direct,
        );
    }
}

fn push_displacement(
    out: &mut Vec<LitVertex>,
    scene: &Scene,
    field: &Field,
    map: &Displacement,
    albedo: [f32; 3],
) {
    if map.width < 2 || map.height < 2 || map.heights.len() < (map.width * map.height) as usize {
        return;
    }
    let sample_h = |ix: u32, iz: u32| {
        let byte = map.heights[(iz * map.width + ix) as usize];
        map.origin[1] + (byte as f32 / 255.0) * map.scale[1]
    };
    for iz in 0..map.height - 1 {
        for ix in 0..map.width - 1 {
            let x0 = map.origin[0] + ix as f32 * map.scale[0];
            let x1 = map.origin[0] + (ix + 1) as f32 * map.scale[0];
            let z0 = map.origin[2] + iz as f32 * map.scale[2];
            let z1 = map.origin[2] + (iz + 1) as f32 * map.scale[2];
            let y00 = sample_h(ix, iz);
            let y10 = sample_h(ix + 1, iz);
            let y11 = sample_h(ix + 1, iz + 1);
            let y01 = sample_h(ix, iz + 1);
            let up = [0.0, 1.0, 0.0];
            let _ = field;
            push_shaded_quad(
                out,
                [x0, y00, z0],
                [x1, y10, z0],
                [x1, y11, z1],
                [x0, y01, z1],
                albedo,
                direct_at(scene, x0, y00, z0, up),
                direct_at(scene, x1, y10, z0, up),
                direct_at(scene, x1, y11, z1, up),
                direct_at(scene, x0, y01, z1, up),
            );
        }
    }
}

fn surface_cell(_field: &Field) -> f32 {
    crate::field::MESH_CELL
}

fn direct_at(scene: &Scene, x: f32, y: f32, z: f32, normal: [f32; 3]) -> f32 {
    illuminate_facing(scene, x, y, z, normal)
}

fn push_floor(
    out: &mut Vec<LitVertex>,
    geom: &Scene,
    light: &Scene,
    field: &Field,
    shadows: &Shadows,
) {
    let span_x = geom.floor.half_x * 2.0;
    let span_z = geom.floor.half_z * 2.0;
    let cell = surface_cell(field);
    let nx = (span_x / cell).ceil().max(1.0) as u32;
    let nz = (span_z / cell).ceil().max(1.0) as u32;
    let x0 = geom.floor.position.x - geom.floor.half_x;
    let z0 = geom.floor.position.z - geom.floor.half_z;
    let dx = span_x / nx as f32;
    let dz = span_z / nz as f32;
    let albedo = geom.floor.color;
    let edges = shadow::edge_grid(shadows, x0, z0, dx, dz, nx, nz);
    for iz in 0..nz {
        for ix in 0..nx {
            let xb = x0 + ix as f32 * dx;
            let zb = z0 + iz as f32 * dz;
            let quad = [[xb, zb], [xb + dx, zb], [xb + dx, zb + dz], [xb, zb + dz]];
            for (tri, mask) in shadow::clip_cuts(quad, edges.cuts(ix, iz), shadows) {
                push_tri_colored(
                    out,
                    [tri[0][0], 0.0, tri[0][1]],
                    [tri[1][0], 0.0, tri[1][1]],
                    [tri[2][0], 0.0, tri[2][1]],
                    albedo,
                    floor_direct(light, tri[0][0], tri[0][1], mask, shadows.resolved),
                    floor_direct(light, tri[1][0], tri[1][1], mask, shadows.resolved),
                    floor_direct(light, tri[2][0], tri[2][1], mask, shadows.resolved),
                );
            }
        }
    }
}

fn floor_direct(light: &Scene, x: f32, z: f32, blocked: u32, resolved: u32) -> f32 {
    illuminate_resolved(light, x, 0.0, z, [0.0, 1.0, 0.0], blocked, resolved)
}

fn push_lit_box(
    out: &mut Vec<LitVertex>,
    scene: &Scene,
    field: &Field,
    albedo: [f32; 3],
    cx: f32,
    cy: f32,
    cz: f32,
    hx: f32,
    hy: f32,
    hz: f32,
    bottom: bool,
) {
    let x0 = cx - hx;
    let x1 = cx + hx;
    let y0 = cy - hy;
    let y1 = cy + hy;
    let z0 = cz - hz;
    let z1 = cz + hz;
    shade_face(
        out,
        scene,
        field,
        albedo,
        [x0, y0, z1],
        [x1 - x0, 0.0, 0.0],
        [0.0, y1 - y0, 0.0],
        [0.0, 0.0, 1.0],
    );
    shade_face(
        out,
        scene,
        field,
        albedo,
        [x1, y0, z0],
        [x0 - x1, 0.0, 0.0],
        [0.0, y1 - y0, 0.0],
        [0.0, 0.0, -1.0],
    );
    shade_face(
        out,
        scene,
        field,
        albedo,
        [x0, y0, z0],
        [0.0, 0.0, z1 - z0],
        [0.0, y1 - y0, 0.0],
        [-1.0, 0.0, 0.0],
    );
    shade_face(
        out,
        scene,
        field,
        albedo,
        [x1, y0, z1],
        [0.0, 0.0, z0 - z1],
        [0.0, y1 - y0, 0.0],
        [1.0, 0.0, 0.0],
    );
    shade_face(
        out,
        scene,
        field,
        albedo,
        [x0, y1, z1],
        [x1 - x0, 0.0, 0.0],
        [0.0, 0.0, z0 - z1],
        [0.0, 1.0, 0.0],
    );
    if bottom {
        shade_face(
            out,
            scene,
            field,
            albedo,
            [x0, y0, z0],
            [x1 - x0, 0.0, 0.0],
            [0.0, 0.0, z1 - z0],
            [0.0, -1.0, 0.0],
        );
    }
}

fn shade_face(
    out: &mut Vec<LitVertex>,
    scene: &Scene,
    field: &Field,
    albedo: [f32; 3],
    origin: [f32; 3],
    axis_u: [f32; 3],
    axis_v: [f32; 3],
    normal: [f32; 3],
) {
    let cell = surface_cell(field);
    let len_u = (axis_u[0] * axis_u[0] + axis_u[1] * axis_u[1] + axis_u[2] * axis_u[2]).sqrt();
    let len_v = (axis_v[0] * axis_v[0] + axis_v[1] * axis_v[1] + axis_v[2] * axis_v[2]).sqrt();
    let nu = (len_u / cell).ceil().max(1.0) as u32;
    let nv = (len_v / cell).ceil().max(1.0) as u32;
    let eps = 0.03;
    let at = |i: u32, j: u32| {
        let u = i as f32 / nu as f32;
        let v = j as f32 / nv as f32;
        [
            origin[0] + axis_u[0] * u + axis_v[0] * v,
            origin[1] + axis_u[1] * u + axis_v[1] * v,
            origin[2] + axis_u[2] * u + axis_v[2] * v,
        ]
    };
    let color_at = |i: u32, j: u32| {
        let mut u = i as f32 / nu as f32;
        let mut v = j as f32 / nv as f32;
        let margin_u = 0.35 / nu as f32;
        let margin_v = 0.35 / nv as f32;
        u = u.clamp(margin_u, 1.0 - margin_u);
        v = v.clamp(margin_v, 1.0 - margin_v);
        let point = [
            origin[0] + axis_u[0] * u + axis_v[0] * v + normal[0] * eps,
            origin[1] + axis_u[1] * u + axis_v[1] * v + normal[1] * eps,
            origin[2] + axis_u[2] * u + axis_v[2] * v + normal[2] * eps,
        ];
        direct_at(scene, point[0], point[1], point[2], normal)
    };
    for j in 0..nv {
        for i in 0..nu {
            let p00 = at(i, j);
            let p10 = at(i + 1, j);
            let p11 = at(i + 1, j + 1);
            let p01 = at(i, j + 1);
            push_shaded_quad(
                out,
                p00,
                p10,
                p11,
                p01,
                albedo,
                color_at(i, j),
                color_at(i + 1, j),
                color_at(i + 1, j + 1),
                color_at(i, j + 1),
            );
        }
    }
}

fn push_lit_cylinder(
    out: &mut Vec<LitVertex>,
    scene: &Scene,
    field: &Field,
    albedo: [f32; 3],
    x: f32,
    z: f32,
    radius: f32,
    height: f32,
) {
    let cell = surface_cell(field);
    let sides = (std::f32::consts::TAU * radius / cell).ceil().max(12.0) as u32;
    let bands = (height / cell).ceil().max(1.0) as u32;
    for i in 0..sides {
        let a0 = (i as f32) / sides as f32 * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32) / sides as f32 * std::f32::consts::TAU;
        let mid = (a0 + a1) * 0.5;
        let ox = mid.cos() * 0.04;
        let oz = mid.sin() * 0.04;
        for band in 0..bands {
            let y0 = height * band as f32 / bands as f32;
            let y1 = height * (band + 1) as f32 / bands as f32;
            let x0 = x + a0.cos() * radius;
            let z0 = z + a0.sin() * radius;
            let x1 = x + a1.cos() * radius;
            let z1 = z + a1.sin() * radius;
            let side = [mid.cos(), 0.0, mid.sin()];
            let color = |px: f32, py: f32, pz: f32| direct_at(scene, px + ox, py, pz + oz, side);
            push_shaded_quad(
                out,
                [x0, y0, z0],
                [x1, y0, z1],
                [x1, y1, z1],
                [x0, y1, z0],
                albedo,
                color(x0, y0, z0),
                color(x1, y0, z1),
                color(x1, y1, z1),
                color(x0, y1, z0),
            );
        }
        let x0 = x + a0.cos() * radius;
        let z0 = z + a0.sin() * radius;
        let x1 = x + a1.cos() * radius;
        let z1 = z + a1.sin() * radius;
        let top = direct_at(scene, x, height + 0.04, z, [0.0, 1.0, 0.0]);
        push_tri(
            out,
            [x, height, z],
            [x0, height, z0],
            [x1, height, z1],
            albedo,
            top,
        );
    }
}

fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ux = b[0] - a[0];
    let uy = b[1] - a[1];
    let uz = b[2] - a[2];
    let vx = c[0] - a[0];
    let vy = c[1] - a[1];
    let vz = c[2] - a[2];
    let nx = uy * vz - uz * vy;
    let ny = uz * vx - ux * vz;
    let nz = ux * vy - uy * vx;
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    if len < 1.0e-8 {
        [0.0, 1.0, 0.0]
    } else {
        [nx / len, ny / len, nz / len]
    }
}

fn push_quad(
    out: &mut Vec<LitVertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    albedo: [f32; 3],
    direct: f32,
) {
    push_shaded_quad(out, a, b, c, d, albedo, direct, direct, direct, direct);
}

fn push_shaded_quad(
    out: &mut Vec<LitVertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    albedo: [f32; 3],
    da: f32,
    db: f32,
    dc: f32,
    dd: f32,
) {
    push_tri_colored(out, a, b, c, albedo, da, db, dc);
    push_tri_colored(out, a, c, d, albedo, da, dc, dd);
}

fn push_tri_colored(
    out: &mut Vec<LitVertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    albedo: [f32; 3],
    da: f32,
    db: f32,
    dc: f32,
) {
    let level = face_normal(a, b, c)[1].abs() > 0.5;
    out.push(LitVertex {
        pos: a,
        albedo,
        direct: da,
        level,
    });
    out.push(LitVertex {
        pos: b,
        albedo,
        direct: db,
        level,
    });
    out.push(LitVertex {
        pos: c,
        albedo,
        direct: dc,
        level,
    });
}

fn push_tri(
    out: &mut Vec<LitVertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    albedo: [f32; 3],
    direct: f32,
) {
    push_tri_colored(out, a, b, c, albedo, direct, direct, direct);
}
