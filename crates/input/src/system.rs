#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_lossless,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::match_same_arms,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::needless_continue,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::needless_range_loop,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::undocumented_unsafe_blocks,
    reason = "TODO(lint-2026-10-14): Vulkan or OS FFI; add a SAFETY comment. docs/LINT_RULES.md"
)]

use std::fs::File;
use std::io::{ErrorKind, Read};

use crate::code::InputCode;
use crate::device::{
    gamepad_button_index, glyph_set_from_name, Axis2d, ButtonState, DeviceKind, DeviceView,
    GamepadDevice, GlyphSet, KeyboardDevice, MouseDevice, MAX_GAMEPADS, MAX_GAMEPAD_BUTTONS,
};

/// One open `/dev/input/jsN` file and the latest control positions.
#[derive(Debug)]
struct OpenJoystick {
    file: File,
    state: JsState,
    name: String,
    glyphs: GlyphSet,
}

#[derive(Clone, Copy, Debug)]
struct JsState {
    left: Axis2d,
    right: Axis2d,
    left_trigger: f32,
    right_trigger: f32,
    buttons: [ButtonState; MAX_GAMEPAD_BUTTONS],
}

impl JsState {
    fn new() -> Self {
        Self {
            left: Axis2d::default(),
            right: Axis2d::default(),
            left_trigger: 0.0,
            right_trigger: 0.0,
            buttons: [ButtonState::Up; MAX_GAMEPAD_BUTTONS],
        }
    }
}

/// A gamepad connected, disconnected, or the picture family changed.
///
/// The UI reads [`InputSystem::active_glyphs`] for the family to draw, and
/// this record for which slot changed.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceChange {
    pub id: u32,
    pub kind: DeviceKind,
    pub connected: bool,
    pub name: String,
    pub glyphs: GlyphSet,
}

/// Owner of the keyboard, the mouse, and the stable gamepad slots.
#[derive(Debug)]
pub struct InputSystem {
    pub keyboard: KeyboardDevice,
    pub mouse: MouseDevice,
    pub gamepads: [GamepadDevice; MAX_GAMEPADS],
    joysticks: [Option<OpenJoystick>; MAX_GAMEPADS],
    mouse_sensitivity: f32,
    active_glyphs: GlyphSet,
    active_id: u32,
    action_text: String,
    changes: Vec<DeviceChange>,
}

impl Clone for InputSystem {
    /// Copy the device state. The clone does not keep the open joystick files.
    fn clone(&self) -> Self {
        Self {
            keyboard: self.keyboard.clone(),
            mouse: self.mouse.clone(),
            gamepads: self.gamepads.clone(),
            joysticks: std::array::from_fn(|_| None),
            mouse_sensitivity: self.mouse_sensitivity,
            active_glyphs: self.active_glyphs,
            active_id: self.active_id,
            action_text: self.action_text.clone(),
            changes: self.changes.clone(),
        }
    }
}

impl InputSystem {
    pub fn new() -> Self {
        Self {
            keyboard: KeyboardDevice::new(),
            mouse: MouseDevice::new(),
            gamepads: std::array::from_fn(GamepadDevice::new),
            joysticks: std::array::from_fn(|_| None),
            mouse_sensitivity: 1.0,
            active_glyphs: GlyphSet::KeyboardMouse,
            active_id: 0,
            action_text: String::new(),
            changes: Vec::new(),
        }
    }

    /// Scale applied in [`Self::add_mouse_delta`]. `1` keeps the platform delta.
    pub fn set_mouse_sensitivity(&mut self, sensitivity: f32) {
        if sensitivity.is_finite() {
            self.mouse_sensitivity = sensitivity.clamp(0.0, 20.0);
        }
    }

    #[must_use]
    pub fn mouse_sensitivity(&self) -> f32 {
        self.mouse_sensitivity
    }

    /// Picture family for the device the player used last.
    #[must_use]
    pub fn active_glyphs(&self) -> GlyphSet {
        self.active_glyphs
    }

    /// Id of the device that last changed the picture family. The keyboard is 0,
    /// the mouse is 1, and gamepad slots start at [`crate::FIRST_GAMEPAD_ID`].
    #[must_use]
    pub fn active_id(&self) -> u32 {
        self.active_id
    }

    /// Remember the action-map text so [`Self::dump`] can list the bindings.
    pub fn set_actions(&mut self, map: &crate::action::ActionMap) {
        self.action_text = map.dump();
    }

    /// Connection changes and picture-family changes since the last take.
    #[must_use]
    pub fn take_device_changes(&mut self) -> Vec<DeviceChange> {
        std::mem::take(&mut self.changes)
    }

    /// Text probe: picture family, deadzone, mouse scale, and connected pads.
    #[must_use]
    pub fn dump(&self) -> String {
        let mut out = format!(
            "glyphs={}\nactive_id={}\nstick_deadzone={}\nmouse_sensitivity={}\n",
            self.active_glyphs.name(),
            self.active_id,
            crate::action::format_number(self.gamepads[0].left_stick_deadzone),
            crate::action::format_number(self.mouse_sensitivity),
        );
        for pad in self.gamepads.iter().filter(|pad| pad.view.connected) {
            out.push_str("pad ");
            out.push_str(&pad.view.id.to_string());
            out.push(' ');
            out.push_str(pad.glyphs.name());
            out.push(' ');
            out.push_str(&pad.view.name);
            out.push('\n');
        }
        out.push_str("bindings\n");
        out.push_str(&self.action_text);
        out
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
        self.gamepads
            .iter()
            .filter(|pad| pad.view.connected)
            .count()
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
        let was = self.keyboard.down(code);
        self.keyboard.set(code, down);
        if down && !was {
            self.note_active(
                GlyphSet::KeyboardMouse,
                0,
                DeviceKind::Keyboard,
                "keyboard",
                true,
            );
        }
    }

    pub fn set_mouse_button(&mut self, code: InputCode, down: bool) {
        let was = self.mouse.down(code);
        self.mouse.set_button(code, down);
        if down && !was {
            self.note_active(GlyphSet::KeyboardMouse, 1, DeviceKind::Mouse, "mouse", true);
        }
    }

    pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
        let dx = dx * self.mouse_sensitivity;
        let dy = dy * self.mouse_sensitivity;
        self.mouse.add_delta(dx, dy);
        if dx != 0.0 || dy != 0.0 {
            self.note_active(GlyphSet::KeyboardMouse, 1, DeviceKind::Mouse, "mouse", true);
        }
    }

    /// Mark a slot connected or disconnected and remember its reported name.
    ///
    /// A new connection selects that pad's picture family. Disconnecting the
    /// active pad falls back to the keyboard family, or to another connected pad.
    pub fn connect_gamepad(&mut self, slot: usize, connected: bool, name: &str) {
        let Some(pad) = self.gamepads.get_mut(slot) else {
            return;
        };
        let changed = pad.view.connected != connected || (connected && pad.view.name != name);
        if !changed {
            return;
        }
        let glyphs = if connected {
            glyph_set_from_name(name)
        } else {
            pad.glyphs
        };
        if connected {
            pad.view.name = name.to_string();
            pad.glyphs = glyphs;
        } else {
            pad.left_stick = Axis2d::default();
            pad.right_stick = Axis2d::default();
            pad.left_trigger = 0.0;
            pad.right_trigger = 0.0;
            pad.buttons = [ButtonState::Up; MAX_GAMEPAD_BUTTONS];
        }
        pad.view.connected = connected;
        let id = pad.view.id;
        let stored_name = pad.view.name.clone();
        let glyphs = pad.glyphs;
        self.push_change(DeviceChange {
            id,
            kind: DeviceKind::Gamepad,
            connected,
            name: stored_name,
            glyphs,
        });
        if connected {
            self.active_glyphs = glyphs;
            self.active_id = id;
            return;
        }
        if self.active_id != id {
            return;
        }
        if let Some(other) = self
            .gamepads
            .iter()
            .find(|pad| pad.view.connected)
            .map(|pad| (pad.glyphs, pad.view.id, pad.view.name.clone()))
        {
            self.note_active(other.0, other.1, DeviceKind::Gamepad, &other.2, true);
        } else {
            self.note_active(
                GlyphSet::KeyboardMouse,
                0,
                DeviceKind::Keyboard,
                "keyboard",
                true,
            );
        }
    }

    pub fn set_gamepad_button(&mut self, slot: usize, code: InputCode, down: bool) {
        let Some(index) = gamepad_button_index(code) else {
            return;
        };
        let Some(pad) = self.gamepads.get_mut(slot) else {
            return;
        };
        let was = pad.buttons[index] == ButtonState::Down;
        pad.buttons[index] = if down {
            ButtonState::Down
        } else {
            ButtonState::Up
        };
        if down && !was && pad.view.connected {
            self.note_pad(slot);
        }
    }

    pub fn set_gamepad_stick(&mut self, slot: usize, code: InputCode, axis: Axis2d) {
        let Some(pad) = self.gamepads.get_mut(slot) else {
            return;
        };
        if code != InputCode::gamepad_left_stick && code != InputCode::gamepad_right_stick {
            return;
        }
        let was = axis_live(pad, code);
        if code == InputCode::gamepad_left_stick {
            pad.left_stick = axis;
        } else {
            pad.right_stick = axis;
        }
        let now = axis_live(pad, code);
        if now && !was && pad.view.connected {
            self.note_pad(slot);
        }
    }

    pub fn set_gamepad_trigger(&mut self, slot: usize, code: InputCode, value: f32) {
        let Some(pad) = self.gamepads.get_mut(slot) else {
            return;
        };
        if code != InputCode::gamepad_left_trigger && code != InputCode::gamepad_right_trigger {
            return;
        }
        let value = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let was = trigger_live(pad, code);
        if code == InputCode::gamepad_left_trigger {
            pad.left_trigger = value;
        } else {
            pad.right_trigger = value;
        }
        let now = trigger_live(pad, code);
        if now && !was && pad.view.connected {
            self.note_pad(slot);
        }
    }

    /// Read `/dev/input/jsN` into the stable slots. A missing device leaves the slot disconnected.
    ///
    /// The first poll opens a present device and keeps the file. A later poll
    /// reads new events from that file. An empty read keeps the last sticks.
    pub fn poll_gamepads(&mut self) {
        for slot in 0..MAX_GAMEPADS {
            if self.joysticks[slot].is_none() {
                self.joysticks[slot] = open_joystick(slot);
            }
            let ready = match self.joysticks[slot].as_mut() {
                Some(stick) => drain_joystick(stick).is_ok(),
                None => false,
            };
            if !ready {
                self.joysticks[slot] = None;
                self.connect_gamepad(slot, false, "");
                continue;
            }
            let same = self.joysticks[slot].as_ref().is_some_and(|stick| {
                let pad = &self.gamepads[slot];
                pad.view.connected && pad.view.name == stick.name
            });
            if !same {
                if let Some(name) = self.joysticks[slot]
                    .as_ref()
                    .map(|stick| stick.name.clone())
                {
                    self.connect_gamepad(slot, true, &name);
                }
            }
            self.absorb_joystick(slot);
        }
    }

    /// Copy one polled pad. A stick that was already outside the deadzone, and a
    /// button that was already down, do not select the family again.
    fn absorb_joystick(&mut self, slot: usize) {
        let Some(state) = self.joysticks[slot].as_ref().map(|stick| stick.state) else {
            return;
        };
        let pad = &self.gamepads[slot];
        let left_was = axis_live(pad, InputCode::gamepad_left_stick);
        let right_was = axis_live(pad, InputCode::gamepad_right_stick);
        let left_trigger_was = trigger_live(pad, InputCode::gamepad_left_trigger);
        let right_trigger_was = trigger_live(pad, InputCode::gamepad_right_trigger);
        let previous = pad.buttons;
        let connected = pad.view.connected;
        let pad = &mut self.gamepads[slot];
        pad.left_stick = state.left;
        pad.right_stick = state.right;
        pad.left_trigger = state.left_trigger;
        pad.right_trigger = state.right_trigger;
        pad.buttons = state.buttons;
        if !connected {
            return;
        }
        let pad = &self.gamepads[slot];
        let stick_edge = (!left_was && axis_live(pad, InputCode::gamepad_left_stick))
            || (!right_was && axis_live(pad, InputCode::gamepad_right_stick));
        let trigger_edge = (!left_trigger_was
            && trigger_live(pad, InputCode::gamepad_left_trigger))
            || (!right_trigger_was && trigger_live(pad, InputCode::gamepad_right_trigger));
        let button_edge = state
            .buttons
            .iter()
            .zip(previous.iter())
            .any(|(now, was)| *now == ButtonState::Down && *was == ButtonState::Up);
        if stick_edge || trigger_edge || button_edge {
            self.note_pad(slot);
        }
    }

    fn note_pad(&mut self, slot: usize) {
        let Some(pad) = self.gamepads.get(slot) else {
            return;
        };
        if !pad.view.connected
            || (self.active_glyphs == pad.glyphs && self.active_id == pad.view.id)
        {
            return;
        }
        let glyphs = pad.glyphs;
        let id = pad.view.id;
        let name = pad.view.name.clone();
        self.note_active(glyphs, id, DeviceKind::Gamepad, &name, true);
    }

    fn note_active(
        &mut self,
        glyphs: GlyphSet,
        id: u32,
        kind: DeviceKind,
        name: &str,
        connected: bool,
    ) {
        if self.active_glyphs == glyphs && self.active_id == id {
            return;
        }
        self.active_glyphs = glyphs;
        self.active_id = id;
        self.push_change(DeviceChange {
            id,
            kind,
            connected,
            name: name.to_string(),
            glyphs,
        });
    }

    fn push_change(&mut self, change: DeviceChange) {
        if self.changes.len() == 64 {
            self.changes.remove(0);
        }
        self.changes.push(change);
    }
}

impl Default for InputSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn open_joystick(slot: usize) -> Option<OpenJoystick> {
    let mut file = File::open(format!("/dev/input/js{slot}")).ok()?;
    set_nonblock(&mut file).ok()?;
    let name = joystick_name(&file, slot);
    let glyphs = glyph_set_from_name(&name);
    let mut stick = OpenJoystick {
        file,
        state: JsState::new(),
        name,
        glyphs,
    };
    drain_joystick(&mut stick).ok()?;
    Some(stick)
}

/// `JSIOCGNAME(128)` from `linux/joystick.h`.
fn joystick_name(file: &File, slot: usize) -> String {
    use std::os::fd::AsRawFd;
    let mut buf = [0u8; 128];
    let rc = unsafe {
        extern "C" {
            fn ioctl(fd: i32, request: usize, ...) -> i32;
        }
        // SAFETY: `fd` is the open joystick. The request reads the device name
        // into `buf`. A failure leaves the buffer zeroed and we use the slot name.
        ioctl(file.as_raw_fd(), 0x8080_6a13, buf.as_mut_ptr())
    };
    if rc <= 0 {
        return format!("gamepad {slot}");
    }
    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    if name.is_empty() {
        format!("gamepad {slot}")
    } else {
        name
    }
}

/// Read queued events. [`ErrorKind::WouldBlock`] keeps the last stick positions.
fn drain_joystick(stick: &mut OpenJoystick) -> Result<(), ()> {
    let mut buf = [0u8; 8];
    loop {
        match stick.file.read(&mut buf) {
            Ok(8) => {
                let glyphs = stick.glyphs;
                apply_js_event(&buf, &mut stick.state, glyphs);
            }
            Ok(0) => return Err(()),
            Ok(_) => return Err(()),
            Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(()),
        }
    }
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

fn apply_js_event(buf: &[u8; 8], state: &mut JsState, glyphs: GlyphSet) {
    let value = i16::from_ne_bytes([buf[4], buf[5]]);
    let kind = buf[6] & !0x80;
    let number = buf[7];
    if kind == 1 {
        if let Some(code) = code_from_js_button(glyphs, number) {
            set_js_button(&mut state.buttons, code, value != 0);
        }
        return;
    }
    if kind != 2 {
        return;
    }
    let norm = if value < 0 {
        value as f32 / 32768.0
    } else {
        value as f32 / 32767.0
    };
    match number {
        0 => state.left.x = norm,
        1 => state.left.y = -norm,
        2 => state.left_trigger = trigger_from_axis(norm),
        3 => state.right.x = norm,
        4 => state.right.y = -norm,
        5 => state.right_trigger = trigger_from_axis(norm),
        6 => {
            set_js_button(
                &mut state.buttons,
                InputCode::gamepad_dpad_left,
                norm <= -0.5,
            );
            set_js_button(
                &mut state.buttons,
                InputCode::gamepad_dpad_right,
                norm >= 0.5,
            );
        }
        7 => {
            set_js_button(&mut state.buttons, InputCode::gamepad_dpad_up, norm <= -0.5);
            set_js_button(
                &mut state.buttons,
                InputCode::gamepad_dpad_down,
                norm >= 0.5,
            );
        }
        _ => {}
    }
}

/// Linux `js` triggers rest at `-1` and reach `1`. That range becomes `0..=1`.
fn trigger_from_axis(norm: f32) -> f32 {
    f32::midpoint(norm, 1.0).clamp(0.0, 1.0)
}

/// joydev numbers the buttons that are actually set, from `BTN_MISC` upward.
///
/// xpad sets `BTN_X` (`0x133`) for the west face and does not set the trigger
/// buttons, so index 2 is west and the clicks are 9 and 10. hid-playstation
/// sets `BTN_NORTH` before `BTN_WEST` and also sets `BTN_TL2` / `BTN_TR2`, so
/// index 2 is north and the clicks land at 11 and 12. A Steam Deck and a
/// generic pad follow the Linux gamepad spec without those trigger buttons:
/// index 2 is north and select stays at 6.
fn code_from_js_button(glyphs: GlyphSet, number: u8) -> Option<InputCode> {
    match glyphs {
        GlyphSet::Xbox => xpad_button(number),
        GlyphSet::PlayStation => playstation_button(number),
        GlyphSet::SteamDeck | GlyphSet::GenericGamepad | GlyphSet::KeyboardMouse => {
            spec_button(number)
        }
    }
}

fn xpad_button(number: u8) -> Option<InputCode> {
    Some(match number {
        0 => InputCode::gamepad_face_south,
        1 => InputCode::gamepad_face_east,
        2 => InputCode::gamepad_face_west,
        3 => InputCode::gamepad_face_north,
        4 => InputCode::gamepad_left_shoulder,
        5 => InputCode::gamepad_right_shoulder,
        6 => InputCode::gamepad_select,
        7 => InputCode::gamepad_start,
        8 => InputCode::gamepad_home,
        9 => InputCode::gamepad_left_stick_press,
        10 => InputCode::gamepad_right_stick_press,
        _ => return None,
    })
}

fn playstation_button(number: u8) -> Option<InputCode> {
    Some(match number {
        0 => InputCode::gamepad_face_south,
        1 => InputCode::gamepad_face_east,
        2 => InputCode::gamepad_face_north,
        3 => InputCode::gamepad_face_west,
        4 => InputCode::gamepad_left_shoulder,
        5 => InputCode::gamepad_right_shoulder,
        6 => InputCode::gamepad_left_trigger,
        7 => InputCode::gamepad_right_trigger,
        8 => InputCode::gamepad_select,
        9 => InputCode::gamepad_start,
        10 => InputCode::gamepad_home,
        11 => InputCode::gamepad_left_stick_press,
        12 => InputCode::gamepad_right_stick_press,
        _ => return None,
    })
}

fn spec_button(number: u8) -> Option<InputCode> {
    Some(match number {
        0 => InputCode::gamepad_face_south,
        1 => InputCode::gamepad_face_east,
        2 => InputCode::gamepad_face_north,
        3 => InputCode::gamepad_face_west,
        4 => InputCode::gamepad_left_shoulder,
        5 => InputCode::gamepad_right_shoulder,
        6 => InputCode::gamepad_select,
        7 => InputCode::gamepad_start,
        8 => InputCode::gamepad_home,
        9 => InputCode::gamepad_left_stick_press,
        10 => InputCode::gamepad_right_stick_press,
        _ => return None,
    })
}

fn axis_live(pad: &GamepadDevice, code: InputCode) -> bool {
    pad.axis2d(code)
        .is_some_and(|axis| axis.x != 0.0 || axis.y != 0.0)
}

fn trigger_live(pad: &GamepadDevice, code: InputCode) -> bool {
    let value = if code == InputCode::gamepad_left_trigger {
        pad.left_trigger
    } else if code == InputCode::gamepad_right_trigger {
        pad.right_trigger
    } else {
        return false;
    };
    value >= pad.activation_threshold
}

fn set_js_button(buttons: &mut [ButtonState; MAX_GAMEPAD_BUTTONS], code: InputCode, down: bool) {
    if let Some(index) = gamepad_button_index(code) {
        buttons[index] = if down {
            ButtonState::Down
        } else {
            ButtonState::Up
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(kind: u8, number: u8, value: i16) -> [u8; 8] {
        let bytes = value.to_ne_bytes();
        [0, 0, 0, 0, bytes[0], bytes[1], kind, number]
    }

    #[test]
    fn js_axes_fill_sticks_triggers_and_the_dpad() {
        let mut state = JsState::new();
        apply_js_event(&packet(2, 0, 16_384), &mut state, GlyphSet::Xbox);
        apply_js_event(&packet(2, 1, 32_767), &mut state, GlyphSet::Xbox);
        assert!(state.left.x > 0.4 && state.left.x < 0.6);
        assert!(state.left.y < -0.9);
        apply_js_event(&packet(2, 2, -32_767), &mut state, GlyphSet::Xbox);
        apply_js_event(&packet(2, 5, 32_767), &mut state, GlyphSet::Xbox);
        assert!(state.left_trigger < 0.01);
        assert!((state.right_trigger - 1.0).abs() < 0.02);
        apply_js_event(&packet(2, 7, -32_767), &mut state, GlyphSet::Xbox);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_dpad_up).unwrap()],
            ButtonState::Down
        );
        apply_js_event(&packet(1, 0, 1), &mut state, GlyphSet::Xbox);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_face_south).unwrap()],
            ButtonState::Down
        );
        apply_js_event(&packet(1, 0, 0), &mut state, GlyphSet::Xbox);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_face_south).unwrap()],
            ButtonState::Up
        );
    }

    #[test]
    fn names_pick_xbox_playstation_and_steam_deck_pictures() {
        assert_eq!(
            glyph_set_from_name("Microsoft X-Box 360 pad"),
            GlyphSet::Xbox
        );
        assert_eq!(
            glyph_set_from_name("Xbox Wireless Controller"),
            GlyphSet::Xbox
        );
        assert_eq!(
            glyph_set_from_name("Sony Interactive Entertainment DualSense"),
            GlyphSet::PlayStation
        );
        assert_eq!(
            glyph_set_from_name("Wireless Controller"),
            GlyphSet::PlayStation
        );
        assert_eq!(glyph_set_from_name("Steam Deck"), GlyphSet::SteamDeck);
        assert_eq!(
            glyph_set_from_name("Generic USB Joystick"),
            GlyphSet::GenericGamepad
        );
    }

    #[test]
    fn connecting_a_pad_and_pressing_a_key_changes_the_picture_family() {
        let mut input = InputSystem::new();
        assert_eq!(input.active_glyphs(), GlyphSet::KeyboardMouse);
        assert_eq!(input.take_device_changes(), Vec::new());

        input.connect_gamepad(0, true, "Xbox Wireless Controller");
        let connected = input.take_device_changes();
        assert_eq!(connected.len(), 1);
        assert!(connected[0].connected);
        assert_eq!(connected[0].glyphs, GlyphSet::Xbox);
        assert_eq!(input.active_glyphs(), GlyphSet::Xbox);
        assert!(input.dump().contains("xbox"));

        input.set_gamepad_stick(0, InputCode::gamepad_left_stick, Axis2d { x: 0.07, y: 0.0 });
        assert_eq!(
            input.take_device_changes(),
            Vec::new(),
            "a rest inside the deadzone is not a new device"
        );

        input.set_key(InputCode::key_w, true);
        let typed = input.take_device_changes();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].glyphs, GlyphSet::KeyboardMouse);
        assert_eq!(input.active_glyphs(), GlyphSet::KeyboardMouse);

        input.set_gamepad_stick(0, InputCode::gamepad_left_stick, Axis2d { x: 0.0, y: 1.0 });
        assert_eq!(input.active_glyphs(), GlyphSet::Xbox);

        input.connect_gamepad(1, true, "Steam Deck");
        assert_eq!(input.active_glyphs(), GlyphSet::SteamDeck);
        input.connect_gamepad(1, false, "Steam Deck");
        assert_eq!(
            input.active_glyphs(),
            GlyphSet::Xbox,
            "the other connected pad keeps its pictures"
        );
        input.connect_gamepad(0, false, "Xbox Wireless Controller");
        assert_eq!(input.active_glyphs(), GlyphSet::KeyboardMouse);
        let changes = input.take_device_changes();
        assert!(changes
            .iter()
            .any(|change| change.glyphs == GlyphSet::SteamDeck && change.connected));
        assert!(changes
            .iter()
            .any(|change| change.glyphs == GlyphSet::KeyboardMouse));
    }

    #[test]
    fn a_held_stick_does_not_steal_the_family_back_from_the_mouse() {
        let mut input = InputSystem::new();
        input.connect_gamepad(0, true, "Xbox Wireless Controller");
        input.set_gamepad_stick(0, InputCode::gamepad_left_stick, Axis2d { x: 0.0, y: 1.0 });
        assert_eq!(input.active_id(), crate::FIRST_GAMEPAD_ID);
        let _ = input.take_device_changes();

        input.add_mouse_delta(3.0, 0.0);
        assert_eq!(input.active_glyphs(), GlyphSet::KeyboardMouse);
        assert_eq!(input.active_id(), 1);
        let mouse = input.take_device_changes();
        assert_eq!(mouse.len(), 1);
        assert_eq!(mouse[0].glyphs, GlyphSet::KeyboardMouse);

        input.set_gamepad_stick(0, InputCode::gamepad_left_stick, Axis2d { x: 0.0, y: 1.0 });
        assert_eq!(input.active_glyphs(), GlyphSet::KeyboardMouse);
        assert_eq!(input.take_device_changes(), Vec::new());
    }

    #[test]
    fn a_held_key_stays_until_the_stick_crosses_the_deadzone() {
        let mut input = InputSystem::new();
        input.connect_gamepad(0, true, "Xbox Wireless Controller");
        let _ = input.take_device_changes();

        input.set_key(InputCode::key_w, true);
        input.set_key(InputCode::key_w, true);
        let typed = input.take_device_changes();
        assert_eq!(typed.len(), 1);
        assert_eq!(input.active_glyphs(), GlyphSet::KeyboardMouse);
        assert_eq!(input.active_id(), 0);

        input.set_gamepad_stick(0, InputCode::gamepad_left_stick, Axis2d { x: 0.08, y: 0.0 });
        assert_eq!(input.active_glyphs(), GlyphSet::KeyboardMouse);
        assert_eq!(input.take_device_changes(), Vec::new());

        input.set_gamepad_stick(0, InputCode::gamepad_left_stick, Axis2d { x: 0.2, y: 0.0 });
        assert_eq!(input.active_glyphs(), GlyphSet::Xbox);
        assert_eq!(input.active_id(), crate::FIRST_GAMEPAD_ID);
        let crossed = input.take_device_changes();
        assert_eq!(crossed.len(), 1);
        assert_eq!(crossed[0].id, crate::FIRST_GAMEPAD_ID);
        assert_eq!(crossed[0].glyphs, GlyphSet::Xbox);
    }

    #[test]
    fn a_second_pad_emits_a_change_when_it_becomes_the_active_slot() {
        let mut input = InputSystem::new();
        input.connect_gamepad(0, true, "Xbox Wireless Controller");
        input.connect_gamepad(1, true, "Xbox Wireless Controller");
        let _ = input.take_device_changes();

        input.set_gamepad_stick(0, InputCode::gamepad_left_stick, Axis2d { x: 0.2, y: 0.0 });
        assert_eq!(input.active_glyphs(), GlyphSet::Xbox);
        assert_eq!(input.active_id(), crate::FIRST_GAMEPAD_ID);
        let first = input.take_device_changes();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, crate::FIRST_GAMEPAD_ID);
        assert_eq!(first[0].glyphs, GlyphSet::Xbox);

        input.set_gamepad_stick(1, InputCode::gamepad_left_stick, Axis2d { x: 0.2, y: 0.0 });
        assert_eq!(input.active_id(), crate::FIRST_GAMEPAD_ID + 1);
        let second = input.take_device_changes();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, crate::FIRST_GAMEPAD_ID + 1);
        assert_eq!(second[0].glyphs, GlyphSet::Xbox);
    }

    #[test]
    fn joydev_buttons_follow_the_glyph_family() {
        assert_eq!(
            pressed(GlyphSet::Xbox, 2),
            Some(InputCode::gamepad_face_west)
        );
        assert_eq!(
            pressed(GlyphSet::Xbox, 10),
            Some(InputCode::gamepad_right_stick_press)
        );
        assert_eq!(pressed(GlyphSet::Xbox, 11), None);

        assert_eq!(
            pressed(GlyphSet::PlayStation, 2),
            Some(InputCode::gamepad_face_north)
        );
        assert_eq!(
            pressed(GlyphSet::PlayStation, 3),
            Some(InputCode::gamepad_face_west)
        );
        assert_eq!(
            pressed(GlyphSet::PlayStation, 6),
            Some(InputCode::gamepad_left_trigger)
        );
        assert_eq!(
            pressed(GlyphSet::PlayStation, 11),
            Some(InputCode::gamepad_left_stick_press)
        );
        assert_eq!(
            pressed(GlyphSet::PlayStation, 12),
            Some(InputCode::gamepad_right_stick_press)
        );

        assert_eq!(
            pressed(GlyphSet::SteamDeck, 2),
            Some(InputCode::gamepad_face_north)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 6),
            Some(InputCode::gamepad_select)
        );
        assert_eq!(
            pressed(GlyphSet::GenericGamepad, 2),
            Some(InputCode::gamepad_face_north)
        );
        assert_eq!(
            pressed(GlyphSet::GenericGamepad, 10),
            Some(InputCode::gamepad_right_stick_press)
        );
    }

    #[test]
    fn a_trigger_is_down_from_the_button_or_the_axis_threshold() {
        let mut pad = GamepadDevice::new(0);
        assert!(!pad.down(InputCode::gamepad_left_trigger));
        pad.left_trigger = 0.49;
        assert!(!pad.down(InputCode::gamepad_left_trigger));
        pad.left_trigger = 0.5;
        assert!(pad.down(InputCode::gamepad_left_trigger));
        pad.left_trigger = 0.0;
        let index = gamepad_button_index(InputCode::gamepad_left_trigger).unwrap();
        pad.buttons[index] = ButtonState::Down;
        assert!(pad.down(InputCode::gamepad_left_trigger));
        pad.buttons[index] = ButtonState::Up;
        pad.right_trigger = 1.0;
        assert!(pad.down(InputCode::gamepad_right_trigger));
    }

    #[test]
    fn a_steady_poll_of_an_active_pad_does_not_allocate() {
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        let mut input = InputSystem::new();
        let mut peers = Vec::new();
        for slot in 0..MAX_GAMEPADS {
            let (local, peer) = UnixStream::pair().unwrap();
            peers.push(peer);
            let mut file = File::from(OwnedFd::from(local));
            set_nonblock(&mut file).unwrap();
            let name = if slot == 0 {
                "Xbox Wireless Controller".to_string()
            } else {
                format!("pad {slot}")
            };
            let glyphs = glyph_set_from_name(&name);
            let mut state = JsState::new();
            if slot == 0 {
                state.left.y = 1.0;
            }
            input.joysticks[slot] = Some(OpenJoystick {
                file,
                state,
                name,
                glyphs,
            });
        }
        input.poll_gamepads();
        let _ = input.take_device_changes();
        let ((), bytes) = crate::alloc_guard(|| {
            for _ in 0..8 {
                input.poll_gamepads();
            }
        });
        assert_eq!(bytes, 0, "steady poll allocated {bytes} bytes");
        assert!(input.gamepad(0).unwrap().view.connected);
        assert!((input.gamepad(0).unwrap().left_stick.y - 1.0).abs() < 1.0e-6);
        let _keep = peers;
    }

    fn pressed(glyphs: GlyphSet, number: u8) -> Option<InputCode> {
        let mut state = JsState::new();
        apply_js_event(&packet(1, number, 1), &mut state, glyphs);
        let codes = [
            InputCode::gamepad_face_south,
            InputCode::gamepad_face_east,
            InputCode::gamepad_face_west,
            InputCode::gamepad_face_north,
            InputCode::gamepad_left_shoulder,
            InputCode::gamepad_right_shoulder,
            InputCode::gamepad_left_trigger,
            InputCode::gamepad_right_trigger,
            InputCode::gamepad_select,
            InputCode::gamepad_start,
            InputCode::gamepad_home,
            InputCode::gamepad_left_stick_press,
            InputCode::gamepad_right_stick_press,
        ];
        codes.into_iter().find(|code| {
            gamepad_button_index(*code)
                .is_some_and(|index| state.buttons[index] == ButtonState::Down)
        })
    }
}
