use crate::device::{DeviceKind, DeviceView, GamepadDevice, KeyboardDevice, MouseDevice, MAX_GAMEPADS};
use crate::code::InputCode;

/// Owner of the keyboard, the mouse, and the stable gamepad slots.
#[derive(Clone, Debug)]
pub struct InputSystem {
    pub keyboard: KeyboardDevice,
    pub mouse: MouseDevice,
    pub gamepads: [GamepadDevice; MAX_GAMEPADS],
}

impl InputSystem {
    pub fn new() -> Self {
        Self {
            keyboard: KeyboardDevice::new(),
            mouse: MouseDevice::new(),
            gamepads: std::array::from_fn(GamepadDevice::new),
        }
    }

    pub fn gamepad(&self, slot: usize) -> Option<&GamepadDevice> {
        self.gamepads.get(slot)
    }

    pub fn gamepad_mut(&mut self, slot: usize) -> Option<&mut GamepadDevice> {
        self.gamepads.get_mut(slot)
    }

    /// Set the per-axis rest deadzone on every gamepad slot. The default is `0.08`. A full tilt still reaches `1`.
    pub fn set_stick_deadzone(&mut self, deadzone: f32) {
        for pad in &mut self.gamepads {
            pad.set_stick_deadzone(crate::code::InputCode::gamepad_left_stick, deadzone);
            pad.set_stick_deadzone(crate::code::InputCode::gamepad_right_stick, deadzone);
        }
    }

    pub fn gamepad_count(&self) -> usize {
        self.gamepads.iter().filter(|pad| pad.view.connected).count()
    }

    pub fn list_devices(&self, kind: DeviceKind) -> Vec<DeviceView> {
        match kind {
            DeviceKind::Keyboard => vec![self.keyboard.view.clone()],
            DeviceKind::Mouse => vec![self.mouse.view.clone()],
            DeviceKind::Gamepad => self
                .gamepads
                .iter()
                .filter(|pad| pad.view.connected)
                .map(|pad| pad.view.clone())
                .collect(),
        }
    }

    /// Start a pull frame. Previous button state is kept for pressed and released.
    pub fn begin_frame(&mut self) {
        self.keyboard.begin_frame();
        self.mouse.begin_frame();
        for pad in &mut self.gamepads {
            pad.begin_frame();
        }
    }

    /// Write keyboard state from Linux evdev codes. Unknown codes are ignored.
    pub fn apply_evdev_keys(&mut self, down: &[u8; 256]) {
        for code in 0..256 {
            if let Some(input) = crate::device::from_evdev(code as u32) {
                self.keyboard.set(input, down[code] != 0);
            }
        }
    }

    pub fn set_key(&mut self, code: InputCode, down: bool) {
        self.keyboard.set(code, down);
    }

    pub fn set_mouse_button(&mut self, code: InputCode, down: bool) {
        self.mouse.set_button(code, down);
    }

    pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse.add_delta(dx, dy);
    }

    /// Read `/dev/input/jsN` into the stable slots. A missing device leaves the slot disconnected.
    pub fn poll_gamepads(&mut self) {
        for (slot, pad) in self.gamepads.iter_mut().enumerate() {
            match read_joystick(slot) {
                Some((left, right)) => {
                    pad.view.connected = true;
                    pad.left_stick = left;
                    pad.right_stick = right;
                }
                None => {
                    pad.view.connected = false;
                    pad.left_stick = Default::default();
                    pad.right_stick = Default::default();
                }
            }
        }
    }
}

impl Default for InputSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn read_joystick(slot: usize) -> Option<(crate::device::Axis2d, crate::device::Axis2d)> {
    use crate::device::Axis2d;
    let mut file = std::fs::File::open(format!("/dev/input/js{slot}")).ok()?;
    let _ = set_nonblock(&mut file);
    let mut left = Axis2d::default();
    let mut right = Axis2d::default();
    let mut saw = false;
    let mut buf = [0u8; 8];
    loop {
        match std::io::Read::read(&mut file, &mut buf) {
            Ok(8) => {
                saw = true;
                apply_js_event(&buf, &mut left, &mut right);
            }
            _ => break,
        }
    }
    saw.then_some((left, right))
}

fn set_nonblock(file: &mut std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    unsafe {
        extern "C" {
            fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        }
        let flags = fcntl(fd, 3);
        if flags < 0 || fcntl(fd, 4, flags | 0o4000) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn apply_js_event(buf: &[u8; 8], left: &mut crate::device::Axis2d, right: &mut crate::device::Axis2d) {
    let value = i16::from_ne_bytes([buf[4], buf[5]]);
    let kind = buf[6] & !0x80;
    let number = buf[7];
    if kind != 2 {
        return;
    }
    let norm = if value < 0 {
        value as f32 / 32768.0
    } else {
        value as f32 / 32767.0
    };
    match number {
        0 => left.x = norm,
        1 => left.y = -norm,
        3 => right.x = norm,
        4 => right.y = -norm,
        _ => {}
    }
}
