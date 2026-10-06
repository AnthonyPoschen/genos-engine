use genos_math::{Mat4, Vec3};
use std::f32::consts::FRAC_PI_2;

/// Eye height of the character controller. Movement keeps this value.
pub const CAMERA_HEIGHT: f32 = 1.7;

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
}

impl Camera {
    /// Opening view for the shipped scene. Depth is `z = 6`, and yaw turns the corner onto screen-right.
    pub fn opening() -> Self {
        Self::new(-6.0, 6.0, std::f32::consts::FRAC_PI_4)
    }

    /// `x` is across the ground. `z` is depth. Height stays on Y.
    pub fn new(x: f32, z: f32, yaw: f32) -> Self {
        Self {
            position: Vec3::new(x, CAMERA_HEIGHT, z),
            yaw,
            pitch: 0.0,
            captured: false,
        }
    }
}

/// View direction. Yaw 0 looks along -Z. Positive yaw turns toward +X.
pub fn look_direction(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(sy * cp, sp, -cy * cp)
}

/// Integrate the character controller. Walls are not tested. Height stays fixed.
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
    camera.position += wish * (MOVE_SPEED * dt);
    camera.position.y = CAMERA_HEIGHT;
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
