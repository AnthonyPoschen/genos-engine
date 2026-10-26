#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::items_after_statements, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::neg_cmp_op_on_partial_ord, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

use genos_math::{Mat4, Quat, Vec3};
use genos_physics::{step, Body, World, GRAVITY};
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::codimation::Codimation;
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
    /// Looping paths bound to solids. The frame step drives these bodies.
    codimations: Vec<BoundCodimation>,
    /// Seconds saved until the next fixed physics step.
    step_left: f32,
}

#[derive(Clone, Debug)]
struct BoundCodimation {
    solid: usize,
    motion: Codimation,
    /// False holds the solid at its current point on the path.
    running: bool,
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
            codimations: Vec::new(),
            step_left: 0.0,
        }
    }

    /// Bind a looping path to one solid. The solid should already sit on the first position.
    /// Returns false when `solid` is not in `scene`.
    pub fn codimate(&mut self, scene: &Scene, solid: usize, motion: Codimation) -> bool {
        if solid >= scene.solids.len() {
            return false;
        }
        if let Some(slot) = self.codimations.iter_mut().find(|slot| slot.solid == solid) {
            slot.motion = motion;
            slot.running = true;
        } else {
            self.codimations.push(BoundCodimation {
                solid,
                motion,
                running: true,
            });
        }
        self.arm_solid(solid);
        true
    }

    /// Hold or resume the path on `solid`. A hold leaves the solid where it is.
    pub fn set_codimation_running(&mut self, solid: usize, running: bool) -> bool {
        let Some(slot) = self.codimations.iter_mut().find(|slot| slot.solid == solid) else {
            return false;
        };
        slot.running = running;
        if running {
            self.arm_solid(solid);
        }
        true
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
                Vec3::new(wall.position.x, wall.base + wall.height * 0.5, wall.position.z),
                0.0,
                Vec3::new(wall.half_x, wall.height * 0.5, wall.half_z),
            ));
        }
        for solid in &scene.solids {
            physics.insert(solid_collider(solid));
        }
        self.view = physics.insert(view_capsule(self.position));
        self.solid_origin = Some(1 + scene.walls.len());
        self.physics = physics;
        let count = self.codimations.len();
        for index in 0..count {
            let solid = self.codimations[index].solid;
            self.arm_solid(solid);
        }
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

    /// Stop the looping path on this solid so a later step does not move it.
    pub fn release_codimation(&mut self, solid: usize) {
        self.codimations.retain(|slot| slot.solid != solid);
    }

    /// Drop the path on `solid` and keep later paths pointed at the solids that remain.
    pub fn solid_removed(&mut self, solid: usize) {
        self.codimations.retain(|slot| slot.solid != solid);
        for slot in &mut self.codimations {
            if slot.solid > solid {
                slot.solid -= 1;
            }
        }
    }

    fn arm_solid(&mut self, solid: usize) {
        let Some(origin) = self.solid_origin else {
            return;
        };
        let index = origin + solid;
        if index >= self.physics.bodies.len() || index == self.view {
            return;
        }
        let body = &mut self.physics.bodies[index];
        if body.inverse_mass == 0.0 {
            body.inverse_mass = 1.0;
        }
        body.motor = true;
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
            let mut body = Body::cuboid(
                solid.position + Vec3::Y * center_lift(solid),
                0.0,
                Vec3::new(half, solid.height * 0.5, half),
            );
            body.orientation = Quat::from_axis_angle(Vec3::Y, solid.yaw);
            body
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

/// Submit each codimation displacement for this slice. Contact still runs inside `step`.
fn drive_codimations(camera: &mut Camera, scene: &Scene, dt: f32) {
    if !(dt > 0.0) {
        return;
    }
    let gravity_y = camera.physics.gravity.y;
    let Some(origin) = camera.solid_origin else {
        return;
    };
    let count = camera.codimations.len();
    for index in 0..count {
        let solid_index = camera.codimations[index].solid;
        let lift = {
            let Some(solid) = scene.solids.get(solid_index) else {
                continue;
            };
            center_lift(solid)
        };
        let body_index = origin + solid_index;
        if body_index >= camera.physics.bodies.len() || body_index == camera.view {
            continue;
        }
        if !camera.codimations[index].running {
            let body = &mut camera.physics.bodies[body_index];
            body.inverse_mass = 0.0;
            body.motor = false;
            body.wish = Vec3::ZERO;
            body.velocity = Vec3::ZERO;
            continue;
        }
        let before = camera.codimations[index].motion.sample();
        camera.codimations[index].motion.advance(dt);
        let after = camera.codimations[index].motion.sample();
        let body = &mut camera.physics.bodies[body_index];
        if body.inverse_mass == 0.0 {
            body.inverse_mass = 1.0;
        }
        body.motor = true;
        let current = body.position - Vec3::Y * lift;
        let mut delta = after - current;
        let path_step = (after - before).length();
        let distance = delta.length();
        if distance > path_step && distance > 1.0e-8 {
            delta = delta * (path_step / distance);
        }
        let velocity = delta * (1.0 / dt);
        body.wish = Vec3::new(velocity.x, 0.0, velocity.z);
        // Gravity is applied inside the step. This preset lands on the path's vertical speed.
        body.velocity.y = velocity.y - gravity_y * dt;
    }
}

fn write_codimations(camera: &Camera, scene: &mut Scene) {
    let Some(origin) = camera.solid_origin else {
        return;
    };
    for slot in &camera.codimations {
        let body_index = origin + slot.solid;
        if body_index == camera.view {
            continue;
        }
        let Some(body) = camera.physics.bodies.get(body_index) else {
            continue;
        };
        let position = body.position;
        let Some(solid) = scene.solids.get_mut(slot.solid) else {
            continue;
        };
        solid.position = position - Vec3::Y * center_lift(solid);
    }
}

/// Integrate look by `dt`, then run fixed physics steps.
/// Move input is a desired horizontal velocity in units per second.
/// The solver runs at 1/60 s once the saved time reaches that step.
/// A shorter remainder waits for the next frame.
/// A codimated solid moves in those same steps. The resolved body position is written back.
pub fn update(camera: &mut Camera, scene: &mut Scene, actions: &Actions, dt: f32) {
    if actions.escape {
        camera.captured = false;
    } else if actions.capture_click {
        camera.captured = true;
    }

    if camera.captured {
        camera.yaw += actions.mouse_dx * MOUSE_SENS;
        camera.pitch -= actions.mouse_dy * MOUSE_SENS;
    }
    // Mouse look is a pointer delta. Stick look, movement, and codimation use seconds.
    let stepping = dt.is_finite() && dt > 0.0;
    if stepping {
        camera.yaw += actions.look_x * STICK_LOOK_SPEED * dt;
        camera.pitch += actions.look_y * STICK_LOOK_SPEED * dt;
    }
    camera.pitch = camera.pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
    if !stepping {
        return;
    }

    // Ground travel ignores pitch. Right is the look direction a quarter turn toward +X.
    let forward = look_direction(camera.yaw, 0.0);
    let right = look_direction(camera.yaw + FRAC_PI_2, 0.0);
    let wish = forward * actions.forward.clamp(-1.0, 1.0) + right * actions.strafe.clamp(-1.0, 1.0);
    let speed = wish * MOVE_SPEED;
    if let Some(body) = camera.physics.bodies.get_mut(camera.view) {
        body.motor = true;
        body.wish = Vec3::new(speed.x, 0.0, speed.z);
    }

    const STEP: f32 = 1.0 / 60.0;
    camera.step_left += dt;
    while camera.step_left >= STEP {
        drive_codimations(camera, scene, STEP);
        step(&mut camera.physics, STEP);
        write_codimations(camera, scene);
        camera.step_left -= STEP;
    }
    if let Some(body) = camera.physics.bodies.get(camera.view) {
        camera.position = body.position + Vec3::Y * EYE_ABOVE_CENTER;
    }
}

/// Column-major view-projection for a Vulkan clip space with Y flipped.
pub fn view_proj(camera: &Camera, aspect: f32) -> [f32; 16] {
    view_proj_guard(
        camera,
        aspect,
        GuardFrustum {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        },
    )
}

/// Extra frustum on each side of the displayed frame, as a fraction of that frame.
/// The eye stays put. A pulled-back camera is not this frustum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuardFrustum {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// View-projection for a guard band. `aspect` is the displayed frame, not the lit image.
pub fn view_proj_guard(camera: &Camera, aspect: f32, guard: GuardFrustum) -> [f32; 16] {
    let view = view_matrix(camera);
    let proj = perspective_guard(
        60.0_f32.to_radians(),
        aspect.max(0.01),
        CAMERA_NEAR,
        CAMERA_FAR,
        guard,
    );
    (proj * view).cols
}

/// World direction of one lit-image pixel. `+Y` in camera space is the top of the picture.
#[must_use]
pub fn project_guard(
    camera: &Camera,
    aspect: f32,
    guard: GuardFrustum,
    lit_w: u32,
    lit_h: u32,
    px: f32,
    py: f32,
) -> Option<[f32; 2]> {
    let dir = lit_pixel_dir(camera, aspect, guard, lit_w, lit_h, px, py)?;
    let point = [
        camera.position.x + dir[0],
        camera.position.y + dir[1],
        camera.position.z + dir[2],
    ];
    let clip = transform_point(&view_proj_guard(camera, aspect, guard), point);
    if clip[3] <= 0.0 {
        return None;
    }
    let lit_w = (lit_w as f32).max(1.0);
    let lit_h = (lit_h as f32).max(1.0);
    let ndc_x = clip[0] / clip[3];
    let ndc_y = clip[1] / clip[3];
    Some([(ndc_x + 1.0) * 0.5 * lit_w, (ndc_y + 1.0) * 0.5 * lit_h])
}

fn lit_pixel_dir(
    camera: &Camera,
    aspect: f32,
    guard: GuardFrustum,
    lit_w: u32,
    lit_h: u32,
    px: f32,
    py: f32,
) -> Option<[f32; 3]> {
    if lit_w == 0 || lit_h == 0 {
        return None;
    }
    let (ax, bx, ay, by) = guard_axes(60.0_f32.to_radians(), aspect.max(0.01), guard);
    let nx = (px + 0.5) / (lit_w as f32) * 2.0 - 1.0;
    let ny = (py + 0.5) / (lit_h as f32) * 2.0 - 1.0;
    if ax.abs() < 1.0e-8 || ay.abs() < 1.0e-8 {
        return None;
    }
    let x_cam = (nx + bx) / ax;
    let y_cam = (ny + by) / ay;
    let forward = look_direction(camera.yaw, camera.pitch);
    let mut right = forward.cross(Vec3::Y);
    let scale = right.length().max(1.0e-6);
    right = right / scale;
    let up = right.cross(forward);
    let dir = right * x_cam + up * y_cam + forward;
    let len = dir.length().max(1.0e-6);
    Some([dir.x / len, dir.y / len, dir.z / len])
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

/// Near and far clip of the picture, in metres. The occluder box reads [`CAMERA_FAR`].
pub const CAMERA_NEAR: f32 = 0.05;
pub const CAMERA_FAR: f32 = 200.0;

fn perspective_guard(fovy: f32, aspect: f32, near: f32, far: f32, guard: GuardFrustum) -> Mat4 {
    let (ax, bx, ay, by) = guard_axes(fovy, aspect, guard);
    let mut cols = [0.0; 16];
    cols[0] = ax;
    cols[5] = ay;
    cols[8] = bx;
    cols[9] = by;
    cols[10] = far / (near - far);
    cols[11] = -1.0;
    cols[14] = (far * near) / (near - far);
    Mat4::from_cols(cols)
}

fn guard_axes(fovy: f32, aspect: f32, guard: GuardFrustum) -> (f32, f32, f32, f32) {
    let tan_y = (fovy * 0.5).tan();
    let tan_x = tan_y * aspect.max(0.01);
    let left = -tan_x * (1.0 + 2.0 * guard.left);
    let right = tan_x * (1.0 + 2.0 * guard.right);
    let top_y = tan_y * (1.0 + 2.0 * guard.top);
    let bottom_y = -tan_y * (1.0 + 2.0 * guard.bottom);
    let ax = 2.0 / (right - left);
    let bx = (right + left) / (right - left);
    let ay = 2.0 / (bottom_y - top_y);
    let by = 1.0 + ay * top_y;
    (ax, bx, ay, by)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_guard_matches_the_displayed_projection() {
        let mut camera = Camera::new(1.0, 3.0, 0.4);
        camera.pitch = 0.1;
        let aspect = 16.0 / 9.0;
        let plain = view_proj(&camera, aspect);
        let guarded = view_proj_guard(
            &camera,
            aspect,
            GuardFrustum {
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            },
        );
        for (a, b) in plain.iter().zip(guarded.iter()) {
            assert!((a - b).abs() < 1.0e-5);
        }
    }

    #[test]
    fn the_look_direction_stays_on_the_display_center() {
        let mut camera = Camera::new(0.0, 4.0, 0.2);
        camera.pitch = -0.05;
        let aspect = 320.0 / 180.0;
        let guard = GuardFrustum {
            left: 0.12,
            right: 0.18,
            top: 0.14,
            bottom: 0.16,
        };
        let display_w = 320.0;
        let display_h = 180.0;
        let lit_w = display_w * (1.0 + guard.left + guard.right);
        let lit_h = display_h * (1.0 + guard.top + guard.bottom);
        let look = look_direction(camera.yaw, camera.pitch);
        let world = [
            camera.position.x + look.x * 6.0,
            camera.position.y + look.y * 6.0,
            camera.position.z + look.z * 6.0,
        ];
        let clip = transform_point(&view_proj_guard(&camera, aspect, guard), world);
        assert!(clip[3] > 0.0);
        let u = (clip[0] / clip[3] + 1.0) * 0.5 * lit_w;
        let v = (clip[1] / clip[3] + 1.0) * 0.5 * lit_h;
        let center_x = guard.left * display_w + display_w * 0.5;
        let center_y = guard.top * display_h + display_h * 0.5;
        assert!((u - center_x).abs() < 0.75);
        assert!((v - center_y).abs() < 0.75);
    }

    #[test]
    fn a_right_margin_pixel_enters_the_display_after_a_right_yaw() {
        let mut camera = Camera::new(0.0, 4.0, 0.0);
        let aspect = 16.0 / 9.0;
        let guard = GuardFrustum {
            left: 0.05,
            right: 0.25,
            top: 0.15,
            bottom: 0.15,
        };
        let display_w = 320.0_f32;
        let display_h = 180.0_f32;
        // 320 * 1.3 and 180 * 1.3. Both products are exact.
        let lit_w = 416u32;
        let lit_h = 234u32;
        assert!((display_w * (1.0 + guard.left + guard.right) - 416.0).abs() < 1.0e-3);
        assert!((display_h * (1.0 + guard.top + guard.bottom) - 234.0).abs() < 1.0e-3);
        let px = guard.left * display_w + display_w + 4.0;
        let py = guard.top * display_h + display_h * 0.5;
        let dir = lit_pixel_dir(&camera, aspect, guard, lit_w, lit_h, px, py).unwrap_or([0.0, 0.0, -1.0]);
        let world = [
            camera.position.x + dir[0] * 5.0,
            camera.position.y + dir[1] * 5.0,
            camera.position.z + dir[2] * 5.0,
        ];
        let before = viewport_uv(&camera, aspect, world).unwrap_or([-1.0, -1.0]);
        assert!(before[0] > 1.0);
        camera.yaw += 0.18;
        let after = viewport_uv(&camera, aspect, world).unwrap_or([-1.0, -1.0]);
        assert!(after[0] > 0.0 && after[0] < 1.0);
        assert!(after[1] > 0.0 && after[1] < 1.0);
    }
}
