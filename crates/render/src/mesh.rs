use genos_scene::{illuminate, sample, Field, Scene, Shape};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

pub fn scene_vertices(scene: &Scene, field: &Field) -> Vec<Vertex> {
    let mut out = Vec::new();
    push_floor(&mut out, scene, field);
    for wall in &scene.walls {
        let color = lit_surface(scene, field, wall.color, wall.x, wall.height * 0.5, wall.z);
        push_box(
            &mut out,
            wall.x,
            wall.height * 0.5,
            wall.z,
            wall.half_x,
            wall.height * 0.5,
            wall.half_z,
            color,
            false,
        );
    }
    for solid in &scene.solids {
        let color = lit_surface(scene, field, solid.color, solid.x, solid.height * 0.5, solid.z);
        match solid.shape {
            Shape::Square => push_box(
                &mut out,
                solid.x,
                solid.height * 0.5,
                solid.z,
                solid.size * 0.5,
                solid.height * 0.5,
                solid.size * 0.5,
                color,
                false,
            ),
            Shape::Circle => push_cylinder(&mut out, solid.x, solid.z, solid.size * 0.5, solid.height, color),
        }
    }
    out
}

fn lit_surface(scene: &Scene, field: &Field, albedo: [f32; 3], x: f32, y: f32, z: f32) -> [f32; 3] {
    let bounce = sample(field, x, z);
    let direct = illuminate(scene, x, y, z);
    [
        (albedo[0] * (direct + bounce[0])).min(1.0),
        (albedo[1] * (direct + bounce[1])).min(1.0),
        (albedo[2] * (direct + bounce[2])).min(1.0),
    ]
}

fn push_floor(out: &mut Vec<Vertex>, scene: &Scene, field: &Field) {
    let span_x = scene.floor.half_x * 2.0;
    let span_z = scene.floor.half_z * 2.0;
    let nx = grid_count(span_x, field.near.spacing);
    let nz = grid_count(span_z, field.near.spacing);
    let x0 = scene.floor.x - scene.floor.half_x;
    let z0 = scene.floor.z - scene.floor.half_z;
    let dx = span_x / nx as f32;
    let dz = span_z / nz as f32;
    for iz in 0..nz {
        for ix in 0..nx {
            let xa = x0 + ix as f32 * dx;
            let za = z0 + iz as f32 * dz;
            let c00 = lit_surface(scene, field, scene.floor.color, xa, 0.0, za);
            let c10 = lit_surface(scene, field, scene.floor.color, xa + dx, 0.0, za);
            let c11 = lit_surface(scene, field, scene.floor.color, xa + dx, 0.0, za + dz);
            let c01 = lit_surface(scene, field, scene.floor.color, xa, 0.0, za + dz);
            push_shaded_quad(
                out,
                [xa, 0.0, za],
                [xa + dx, 0.0, za],
                [xa + dx, 0.0, za + dz],
                [xa, 0.0, za + dz],
                c00,
                c10,
                c11,
                c01,
            );
        }
    }
}

fn grid_count(span: f32, spacing: f32) -> u32 {
    let cell = spacing.max(0.05) * 0.5;
    (span / cell).ceil().clamp(16.0, 96.0) as u32
}

fn push_box(
    out: &mut Vec<Vertex>,
    cx: f32,
    cy: f32,
    cz: f32,
    hx: f32,
    hy: f32,
    hz: f32,
    color: [f32; 3],
    bottom: bool,
) {
    let x0 = cx - hx;
    let x1 = cx + hx;
    let y0 = cy - hy;
    let y1 = cy + hy;
    let z0 = cz - hz;
    let z1 = cz + hz;
    push_quad(out, [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1], color);
    push_quad(out, [x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0], color);
    push_quad(out, [x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0], color);
    push_quad(out, [x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1], color);
    push_quad(out, [x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0], color);
    if bottom {
        push_quad(out, [x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1], color);
    }
}

fn push_cylinder(out: &mut Vec<Vertex>, x: f32, z: f32, radius: f32, height: f32, color: [f32; 3]) {
    let sides = 16;
    for i in 0..sides {
        let a0 = (i as f32) / sides as f32 * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32) / sides as f32 * std::f32::consts::TAU;
        let x0 = x + a0.cos() * radius;
        let z0 = z + a0.sin() * radius;
        let x1 = x + a1.cos() * radius;
        let z1 = z + a1.sin() * radius;
        push_quad(out, [x0, 0.0, z0], [x1, 0.0, z1], [x1, height, z1], [x0, height, z0], color);
        push_tri(out, [x, height, z], [x0, height, z0], [x1, height, z1], color);
    }
}

fn push_quad(out: &mut Vec<Vertex>, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], color: [f32; 3]) {
    push_shaded_quad(out, a, b, c, d, color, color, color, color);
}

fn push_shaded_quad(
    out: &mut Vec<Vertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    ca: [f32; 3],
    cb: [f32; 3],
    cc: [f32; 3],
    cd: [f32; 3],
) {
    push_tri_colored(out, a, b, c, ca, cb, cc);
    push_tri_colored(out, a, c, d, ca, cc, cd);
}

fn push_tri_colored(
    out: &mut Vec<Vertex>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    ca: [f32; 3],
    cb: [f32; 3],
    cc: [f32; 3],
) {
    out.push(Vertex { pos: a, color: ca });
    out.push(Vertex { pos: b, color: cb });
    out.push(Vertex { pos: c, color: cc });
}

fn push_tri(out: &mut Vec<Vertex>, a: [f32; 3], b: [f32; 3], c: [f32; 3], color: [f32; 3]) {
    out.push(Vertex { pos: a, color });
    out.push(Vertex { pos: b, color });
    out.push(Vertex { pos: c, color });
}
