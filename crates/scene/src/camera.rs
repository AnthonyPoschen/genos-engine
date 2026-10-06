use genos_math::{Mat4, Vec3};
use genos_physics::{step, Body, World, GRAVITY};
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::types::{Scene, Shape, Solid};

/// Eye height of the character controller. A resting capsule keeps the eye here.
pub const CAMERA_HEIGHT: f32 = 1.7;

/// Capsule radius around the view. The eye sits at the top of the capsule.
const VIEW_RADIUS: f32 = 0.3;

/// Half-length of the capsule segment. Total height is `2 * (radius + half_height)`.
const VIEW_HALF_HEIGHT: f32 = 0.55;

/// Eye position is this far above the capsule center.
const EYE_ABOVE_CENTER: f32 = VIEW_RADIUS + VIEW_HALF_HEIGHT;

/// Floor box extends this far below y = 0. The top stays on the ground.
const FLOOR_HALF_Y: f32 = 0.25;

/// Pitch stops here, short of straight up and straight down.
pub const PITCH_LIMIT: f32 = 1.45;

const MOVE_SPEED: f32 = 4.0;
const MOUSE_SENS: f32 = 0.003;
const STICK_LOOK_SPEED: f32 = 1.8;

/// Action values for one camera step.
#[derive(Clone, Debug, Default)]
pub struct Actions {
    pub forward: f32,
    pub strafe: f32,
    pub mouse_dx: f32,
    pub mouse_dy: f32,
    pub look_x: f32,
    pub look_y: f32,
    pub capture_click: bool,
    pub escape: bool,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub captured: bool,
    /// Static room colliders plus the one capsule that carries the eye.
    pub physics: World,
    /// Index of the view capsule in `physics`.
    pub view: usize,
    /// Body index of the first solid. `None` until `attach_scene`.
    solid_origin: Option<usize>,
}

impl Camera {
    /// Opening view for the shipped scene. Depth is `z = 6`, and yaw turns the corner onto screen-right.
    pub fn opening() -> Self {
        Self::new(-6.0, 6.0, std::f32::consts::FRAC_PI_4)
    }

    /// `x` is across the ground. `z` is depth. The eye starts at `CAMERA_HEIGHT`.
    pub fn new(x: f32, z: f32, yaw: f32) -> Self {
        let position = Vec3::new(x, CAMERA_HEIGHT, z);
        let mut physics = World::new(GRAVITY);
        physics.insert(view_capsule(position));
        Self {
            position,
            yaw,
            pitch: 0.0,
            captured: false,
            physics,
            view: 0,
            solid_origin: None,
        }
    }

    /// Replace the world with the room colliders and one capsule at the current eye.
    pub fn attach_scene(&mut self, scene: &Scene) {
        let mut physics = World::new(GRAVITY);
        let floor = &scene.floor;
        physics.insert(Body::cuboid(
            Vec3::new(floor.position.x, -FLOOR_HALF_Y, floor.position.z),
            0.0,
            Vec3::new(floor.half_x, FLOOR_HALF_Y, floor.half_z),
        ));
        for wall in &scene.walls {
            physics.insert(Body::cuboid(
                Vec3::new(wall.position.x, wall.height * 0.5, wall.position.z),
                0.0,
                Vec3::new(wall.half_x, wall.height * 0.5, wall.half_z),
            ));
        }
        for solid in &scene.solids {
            physics.insert(solid_collider(solid));
        }
        physics.insert(view_capsule(self.position));
        self.view = physics.count - 1;
        self.solid_origin = Some(1 + scene.walls.len());
        self.physics = physics;
    }

    /// Move the eye and the view capsule together. The next step starts here.
    pub fn set_pose(&mut self, position: Vec3, yaw: f32, pitch: f32) {
        self.position = position;
        self.yaw = yaw;
        self.pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        if let Some(body) = self.physics.bodies.get_mut(self.view) {
            body.position = position - Vec3::Y * EYE_ABOVE_CENTER;
            body.velocity = Vec3::ZERO;
            body.wish = Vec3::ZERO;
        }
    }

}

fn view_capsule(eye: Vec3) -> Body {
    let mut body = Body::capsule(
        eye - Vec3::Y * EYE_ABOVE_CENTER,
        1.0,
        VIEW_RADIUS,
        VIEW_HALF_HEIGHT,
    );
    body.motor = true;
    body
}

fn solid_collider(solid: &Solid) -> Body {
    match solid.shape {
        Shape::Square => {
            let half = solid.size * 0.5;
            Body::cuboid(
                solid.position + Vec3::Y * center_lift(solid),
                0.0,
                Vec3::new(half, solid.height * 0.5, half),
            )
        }
        Shape::Circle => {
            let radius = solid.size * 0.5;
            Body::mesh(solid.position, 0.0, &cylinder(radius, solid.height, 16))
        }
    }
}

fn cylinder(radius: f32, height: f32, sides: u32) -> Vec<[Vec3; 3]> {
    let mut triangles = Vec::new();
    for i in 0..sides {
        let a0 = i as f32 / sides as f32 * TAU;
        let a1 = (i + 1) as f32 / sides as f32 * TAU;
        let p0 = Vec3::new(a0.cos() * radius, 0.0, a0.sin() * radius);
        let p1 = Vec3::new(a1.cos() * radius, 0.0, a1.sin() * radius);
        let p2 = Vec3::new(p1.x, height, p1.z);
        let p3 = Vec3::new(p0.x, height, p0.z);
        triangles.push([p0, p1, p2]);
        triangles.push([p0, p2, p3]);
        triangles.push([Vec3::new(0.0, height, 0.0), p3, p2]);
        triangles.push([Vec3::ZERO, p1, p0]);
    }
    triangles
}

/// View direction. Yaw 0 looks along -Z. Positive yaw turns toward +X.
pub fn look_direction(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(sy * cp, sp, -cy * cp)
}

/// Vertical distance from a solid's scene position to its collider center.
fn center_lift(solid: &Solid) -> f32 {
    match solid.shape {
        Shape::Square => solid.height * 0.5,
        Shape::Circle => 0.0,
    }
}

/// Integrate look, then the physics step. Move input is a desired horizontal velocity.
pub fn update(camera: &mut Camera, actions: &Actions, dt: f32) {
    if actions.escape {
        camera.captured = false;
    } else if actions.capture_click {
        camera.captured = true;
    }

    if camera.captured {
        camera.yaw += actions.mouse_dx * MOUSE_SENS;
        camera.pitch -= actions.mouse_dy * MOUSE_SENS;
    }
    camera.yaw += actions.look_x * STICK_LOOK_SPEED * dt;
    camera.pitch += actions.look_y * STICK_LOOK_SPEED * dt;
    camera.pitch = camera.pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);

    // Ground travel ignores pitch. Right is the look direction a quarter turn toward +X.
    let forward = look_direction(camera.yaw, 0.0);
    let right = look_direction(camera.yaw + FRAC_PI_2, 0.0);
    let wish = forward * actions.forward.clamp(-1.0, 1.0) + right * actions.strafe.clamp(-1.0, 1.0);
    let speed = wish * MOVE_SPEED;
    if let Some(body) = camera.physics.bodies.get_mut(camera.view) {
        body.motor = true;
        body.wish = Vec3::new(speed.x, 0.0, speed.z);
    }

    let mut left = dt.max(0.0);
    let slice = 1.0 / 60.0;
    while left > 0.0 {
        let step_dt = left.min(slice);
        camera.physics = step(&camera.physics, step_dt);
        left -= step_dt;
    }
    if let Some(body) = camera.physics.bodies.get(camera.view) {
        camera.position = body.position + Vec3::Y * EYE_ABOVE_CENTER;
    }
}

/// Column-major view-projection for a Vulkan clip space with Y flipped.
pub fn view_proj(camera: &Camera, aspect: f32) -> [f32; 16] {
    let view = view_matrix(camera);
    let proj = perspective(60.0_f32.to_radians(), aspect.max(0.01), 0.05, 200.0);
    (proj * view).cols
}

fn view_matrix(camera: &Camera) -> Mat4 {
    let forward = look_direction(camera.yaw, camera.pitch);
    let mut right = forward.cross(Vec3::Y);
    let scale = right.length().max(1.0e-6);
    right = right / scale;
    let up = right.cross(forward);
    let eye = camera.position;
    Mat4::from_cols([
        right.x,
        up.x,
        -forward.x,
        0.0,
        right.y,
        up.y,
        -forward.y,
        0.0,
        right.z,
        up.z,
        -forward.z,
        0.0,
        -right.dot(eye),
        -up.dot(eye),
        forward.dot(eye),
        1.0,
    ])
}

fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let h = 1.0 / (fovy * 0.5).tan();
    let w = h / aspect;
    let mut cols = [0.0; 16];
    cols[0] = w;
    cols[5] = -h;
    cols[10] = far / (near - far);
    cols[11] = -1.0;
    cols[14] = (far * near) / (near - far);
    Mat4::from_cols(cols)
}

/// Screen position in camera space. Top-left is `(0, 0)`. Bottom-right is `(1, 1)`.
pub fn viewport_uv(camera: &Camera, aspect: f32, world: [f32; 3]) -> Option<[f32; 2]> {
    let clip = transform_point(&view_proj(camera, aspect), world);
    if clip[3] <= 0.0 {
        return None;
    }
    let ndc_x = clip[0] / clip[3];
    let ndc_y = clip[1] / clip[3];
    Some([(ndc_x + 1.0) * 0.5, (ndc_y + 1.0) * 0.5])
}

/// Transform a point by a column-major matrix. Returns clip x, y, z, w.
pub fn transform_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 4] {
    let clip = Mat4::from_cols(*m).transform_homogeneous(Vec3::new(p[0], p[1], p[2]));
    [clip.x, clip.y, clip.z, clip.w]
}
