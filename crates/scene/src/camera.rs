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
    pub position: [f32; 3],
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
            position: [x, CAMERA_HEIGHT, z],
            yaw,
            pitch: 0.0,
            captured: false,
        }
    }
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

    let (sy, cy) = camera.yaw.sin_cos();
    // Yaw 0 looks along -Z. Positive yaw turns toward +X. Right is +X at yaw 0.
    let forward_x = sy;
    let forward_z = -cy;
    let right_x = cy;
    let right_z = sy;
    let forward = actions.forward.clamp(-1.0, 1.0);
    let strafe = actions.strafe.clamp(-1.0, 1.0);
    camera.position[0] += (forward_x * forward + right_x * strafe) * MOVE_SPEED * dt;
    camera.position[2] += (forward_z * forward + right_z * strafe) * MOVE_SPEED * dt;
    camera.position[1] = CAMERA_HEIGHT;
}

/// Column-major view-projection for a Vulkan clip space with Y flipped.
pub fn view_proj(camera: &Camera, aspect: f32) -> [f32; 16] {
    let view = view_matrix(camera);
    let proj = perspective(60.0_f32.to_radians(), aspect.max(0.01), 0.05, 200.0);
    mul(&proj, &view)
}

fn view_matrix(camera: &Camera) -> [f32; 16] {
    let cp = camera.pitch.cos();
    let sp = camera.pitch.sin();
    let (sy, cy) = camera.yaw.sin_cos();
    // Yaw 0 looks along -Z. Positive yaw turns toward +X. Y is up.
    let fx = sy * cp;
    let fy = sp;
    let fz = -cy * cp;
    // right = cross(forward, world_up), world_up = +Y. The opposite cross rolls the view.
    let mut rx = -fz;
    let mut ry = 0.0;
    let mut rz = fx;
    let rl = (rx * rx + ry * ry + rz * rz).sqrt().max(1.0e-6);
    rx /= rl;
    ry /= rl;
    rz /= rl;
    let ux = ry * fz - rz * fy;
    let uy = rz * fx - rx * fz;
    let uz = rx * fy - ry * fx;
    let eye = camera.position;
    [
        rx,
        ux,
        -fx,
        0.0,
        ry,
        uy,
        -fy,
        0.0,
        rz,
        uz,
        -fz,
        0.0,
        -(rx * eye[0] + ry * eye[1] + rz * eye[2]),
        -(ux * eye[0] + uy * eye[1] + uz * eye[2]),
        fx * eye[0] + fy * eye[1] + fz * eye[2],
        1.0,
    ]
}

fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let h = 1.0 / (fovy * 0.5).tan();
    let w = h / aspect;
    let mut m = [0.0; 16];
    m[0] = w;
    m[5] = -h;
    m[10] = far / (near - far);
    m[11] = -1.0;
    m[14] = (far * near) / (near - far);
    m
}

fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = a[row] * b[col * 4]
                + a[4 + row] * b[col * 4 + 1]
                + a[8 + row] * b[col * 4 + 2]
                + a[12 + row] * b[col * 4 + 3];
        }
    }
    out
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
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
        m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15],
    ]
}
