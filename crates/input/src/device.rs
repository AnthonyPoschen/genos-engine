use crate::code::InputCode;

pub const MAX_KEYS: usize = 256;
pub const MAX_MOUSE_BUTTONS: usize = 16;
pub const MAX_GAMEPADS: usize = 16;
pub const MAX_GAMEPAD_BUTTONS: usize = 32;
pub const FIRST_GAMEPAD_ID: u32 = 100;
pub const DEFAULT_STICK_DEADZONE: f32 = 0.08;
pub const DEFAULT_ACTIVATION_THRESHOLD: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axis2d {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Keyboard,
    Mouse,
    Gamepad,
}

#[derive(Clone, Debug)]
pub struct DeviceView {
    pub id: u32,
    pub kind: DeviceKind,
    pub connected: bool,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct KeyboardDevice {
    pub view: DeviceView,
    keys: [ButtonState; MAX_KEYS],
    prev_keys: [ButtonState; MAX_KEYS],
}

impl KeyboardDevice {
    pub fn new() -> Self {
        Self {
            view: DeviceView {
                id: 0,
                kind: DeviceKind::Keyboard,
                connected: true,
                name: "keyboard".into(),
            },
            keys: [ButtonState::Up; MAX_KEYS],
            prev_keys: [ButtonState::Up; MAX_KEYS],
        }
    }

    pub fn begin_frame(&mut self) {
        self.prev_keys = self.keys;
    }

    pub fn set(&mut self, code: InputCode, down: bool) {
        let index = code as u16 as usize;
        if index < self.keys.len() {
            self.keys[index] = if down {
                ButtonState::Down
            } else {
                ButtonState::Up
            };
        }
    }

    pub fn down(&self, code: InputCode) -> bool {
        self.state(self.keys, code) == Some(true)
    }

    pub fn pressed(&self, code: InputCode) -> bool {
        self.state(self.prev_keys, code) == Some(false) && self.down(code)
    }

    pub fn released(&self, code: InputCode) -> bool {
        self.state(self.prev_keys, code) == Some(true) && !self.down(code)
    }

    fn state(&self, keys: [ButtonState; MAX_KEYS], code: InputCode) -> Option<bool> {
        let index = code as u16 as usize;
        if index >= keys.len() {
            return None;
        }
        Some(keys[index] == ButtonState::Down)
    }
}

#[derive(Clone, Debug)]
pub struct MouseDevice {
    pub view: DeviceView,
    buttons: [ButtonState; MAX_MOUSE_BUTTONS],
    prev_buttons: [ButtonState; MAX_MOUSE_BUTTONS],
    pub x: f32,
    pub y: f32,
    pub dx: f32,
    pub dy: f32,
}

impl MouseDevice {
    pub fn new() -> Self {
        Self {
            view: DeviceView {
                id: 1,
                kind: DeviceKind::Mouse,
                connected: true,
                name: "mouse".into(),
            },
            buttons: [ButtonState::Up; MAX_MOUSE_BUTTONS],
            prev_buttons: [ButtonState::Up; MAX_MOUSE_BUTTONS],
            x: 0.0,
            y: 0.0,
            dx: 0.0,
            dy: 0.0,
        }
    }

    pub fn begin_frame(&mut self) {
        self.prev_buttons = self.buttons;
        self.dx = 0.0;
        self.dy = 0.0;
    }

    pub fn add_delta(&mut self, dx: f32, dy: f32) {
        self.dx += dx;
        self.dy += dy;
        self.x += dx;
        self.y += dy;
    }

    pub fn set_button(&mut self, code: InputCode, down: bool) {
        if let Some(index) = mouse_index(code) {
            self.buttons[index] = if down {
                ButtonState::Down
            } else {
                ButtonState::Up
            };
        }
    }

    pub fn down(&self, code: InputCode) -> bool {
        mouse_index(code).is_some_and(|index| self.buttons[index] == ButtonState::Down)
    }

    pub fn pressed(&self, code: InputCode) -> bool {
        mouse_index(code).is_some_and(|index| {
            self.prev_buttons[index] == ButtonState::Up && self.buttons[index] == ButtonState::Down
        })
    }
}

#[derive(Clone, Debug)]
pub struct GamepadDevice {
    pub view: DeviceView,
    pub buttons: [ButtonState; MAX_GAMEPAD_BUTTONS],
    prev_buttons: [ButtonState; MAX_GAMEPAD_BUTTONS],
    pub left_stick: Axis2d,
    pub right_stick: Axis2d,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub left_stick_deadzone: f32,
    pub right_stick_deadzone: f32,
    pub activation_threshold: f32,
}

impl GamepadDevice {
    pub fn new(slot: usize) -> Self {
        Self {
            view: DeviceView {
                id: FIRST_GAMEPAD_ID + slot as u32,
                kind: DeviceKind::Gamepad,
                connected: false,
                name: format!("gamepad {slot}"),
            },
            buttons: [ButtonState::Up; MAX_GAMEPAD_BUTTONS],
            prev_buttons: [ButtonState::Up; MAX_GAMEPAD_BUTTONS],
            left_stick: Axis2d::default(),
            right_stick: Axis2d::default(),
            left_trigger: 0.0,
            right_trigger: 0.0,
            left_stick_deadzone: DEFAULT_STICK_DEADZONE,
            right_stick_deadzone: DEFAULT_STICK_DEADZONE,
            activation_threshold: DEFAULT_ACTIVATION_THRESHOLD,
        }
    }

    pub fn begin_frame(&mut self) {
        self.prev_buttons = self.buttons;
    }

    pub fn down(&self, code: InputCode) -> bool {
        if let Some(index) = gamepad_button_index(code) {
            return self.buttons[index] == ButtonState::Down;
        }
        match code {
            InputCode::gamepad_left_stick_up => self.left_stick.y >= self.activation_threshold,
            InputCode::gamepad_left_stick_down => self.left_stick.y <= -self.activation_threshold,
            InputCode::gamepad_left_stick_left => self.left_stick.x <= -self.activation_threshold,
            InputCode::gamepad_left_stick_right => self.left_stick.x >= self.activation_threshold,
            InputCode::gamepad_right_stick_up => self.right_stick.y >= self.activation_threshold,
            InputCode::gamepad_right_stick_down => self.right_stick.y <= -self.activation_threshold,
            InputCode::gamepad_right_stick_left => self.right_stick.x <= -self.activation_threshold,
            InputCode::gamepad_right_stick_right => self.right_stick.x >= self.activation_threshold,
            InputCode::gamepad_left_trigger => self.left_trigger >= self.activation_threshold,
            InputCode::gamepad_right_trigger => self.right_trigger >= self.activation_threshold,
            _ => false,
        }
    }

    /// Per-axis rest deadzone. Each axis inside it becomes zero. A larger tilt on that axis stays as it is.
    pub fn set_stick_deadzone(&mut self, code: InputCode, deadzone: f32) {
        let deadzone = deadzone.clamp(0.0, 0.95);
        if code == InputCode::gamepad_right_stick {
            self.right_stick_deadzone = deadzone;
        } else {
            self.left_stick_deadzone = deadzone;
        }
    }

    pub fn axis2d(&self, code: InputCode) -> Option<Axis2d> {
        let stick = match code {
            InputCode::gamepad_left_stick => self.left_stick,
            InputCode::gamepad_right_stick => self.right_stick,
            _ => return None,
        };
        Some(apply_deadzone(stick, self.deadzone(code)))
    }

    fn deadzone(&self, code: InputCode) -> f32 {
        if code == InputCode::gamepad_right_stick {
            self.right_stick_deadzone
        } else {
            self.left_stick_deadzone
        }
    }
}

fn apply_deadzone(stick: Axis2d, deadzone: f32) -> Axis2d {
    Axis2d {
        x: if stick.x.abs() <= deadzone {
            0.0
        } else {
            stick.x
        },
        y: if stick.y.abs() <= deadzone {
            0.0
        } else {
            stick.y
        },
    }
}

pub fn mouse_index(code: InputCode) -> Option<usize> {
    let value = code as u16;
    if (1000..1016).contains(&value) {
        Some((value - 1000) as usize)
    } else {
        None
    }
}

pub fn gamepad_button_index(code: InputCode) -> Option<usize> {
    match code {
        InputCode::gamepad_face_south => Some(0),
        InputCode::gamepad_face_east => Some(1),
        InputCode::gamepad_face_west => Some(2),
        InputCode::gamepad_face_north => Some(3),
        InputCode::gamepad_dpad_up => Some(4),
        InputCode::gamepad_dpad_down => Some(5),
        InputCode::gamepad_dpad_left => Some(6),
        InputCode::gamepad_dpad_right => Some(7),
        InputCode::gamepad_left_shoulder => Some(8),
        InputCode::gamepad_right_shoulder => Some(9),
        InputCode::gamepad_left_trigger => Some(10),
        InputCode::gamepad_right_trigger => Some(11),
        InputCode::gamepad_select => Some(12),
        InputCode::gamepad_start => Some(13),
        InputCode::gamepad_home => Some(14),
        InputCode::gamepad_left_stick_press => Some(15),
        InputCode::gamepad_right_stick_press => Some(16),
        InputCode::gamepad_capture => Some(17),
        _ => None,
    }
}

/// Linux evdev code to the shared input code. Unknown codes are ignored.
pub fn from_evdev(code: u32) -> Option<InputCode> {
    Some(match code {
        1 => InputCode::key_escape,
        14 => InputCode::key_backspace,
        15 => InputCode::key_tab,
        28 => InputCode::key_enter,
        57 => InputCode::key_space,
        42 => InputCode::key_shift_left,
        54 => InputCode::key_shift_right,
        29 => InputCode::key_control_left,
        97 => InputCode::key_control_right,
        56 => InputCode::key_alt_left,
        100 => InputCode::key_alt_right,
        103 => InputCode::key_up,
        108 => InputCode::key_down,
        105 => InputCode::key_left,
        106 => InputCode::key_right,
        2 => InputCode::key_1,
        3 => InputCode::key_2,
        4 => InputCode::key_3,
        5 => InputCode::key_4,
        6 => InputCode::key_5,
        7 => InputCode::key_6,
        8 => InputCode::key_7,
        9 => InputCode::key_8,
        10 => InputCode::key_9,
        11 => InputCode::key_0,
        16 => InputCode::key_q,
        17 => InputCode::key_w,
        18 => InputCode::key_e,
        19 => InputCode::key_r,
        20 => InputCode::key_t,
        21 => InputCode::key_y,
        22 => InputCode::key_u,
        23 => InputCode::key_i,
        24 => InputCode::key_o,
        25 => InputCode::key_p,
        30 => InputCode::key_a,
        31 => InputCode::key_s,
        32 => InputCode::key_d,
        33 => InputCode::key_f,
        34 => InputCode::key_g,
        35 => InputCode::key_h,
        36 => InputCode::key_j,
        37 => InputCode::key_k,
        38 => InputCode::key_l,
        44 => InputCode::key_z,
        45 => InputCode::key_x,
        46 => InputCode::key_c,
        47 => InputCode::key_v,
        48 => InputCode::key_b,
        49 => InputCode::key_n,
        50 => InputCode::key_m,
        59 => InputCode::key_f1,
        60 => InputCode::key_f2,
        61 => InputCode::key_f3,
        62 => InputCode::key_f4,
        63 => InputCode::key_f5,
        64 => InputCode::key_f6,
        65 => InputCode::key_f7,
        66 => InputCode::key_f8,
        67 => InputCode::key_f9,
        68 => InputCode::key_f10,
        87 => InputCode::key_f11,
        88 => InputCode::key_f12,
        _ => return None,
    })
}
