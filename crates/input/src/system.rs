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
    clippy::single_match_else,
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
}

#[derive(Clone, Debug)]
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
        let glyphs = if connected {
            glyph_set_from_name(name)
        } else {
            pad.glyphs
        };
        let changed = pad.view.connected != connected || (connected && pad.view.name != name);
        if connected {
            pad.view.name = name.to_string();
            pad.glyphs = glyphs;
        } else if changed {
            pad.left_stick = Axis2d::default();
            pad.right_stick = Axis2d::default();
            pad.left_trigger = 0.0;
            pad.right_trigger = 0.0;
            pad.buttons = [ButtonState::Up; MAX_GAMEPAD_BUTTONS];
        }
        pad.view.connected = connected;
        if !changed {
            return;
        }
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
        pad.buttons[index] = if down {
            ButtonState::Down
        } else {
            ButtonState::Up
        };
        if down {
            self.note_pad_used(slot);
        }
    }

    pub fn set_gamepad_stick(&mut self, slot: usize, code: InputCode, axis: Axis2d) {
        let Some(pad) = self.gamepads.get_mut(slot) else {
            return;
        };
        if code == InputCode::gamepad_left_stick {
            pad.left_stick = axis;
        } else if code == InputCode::gamepad_right_stick {
            pad.right_stick = axis;
        } else {
            return;
        }
        self.note_pad_used(slot);
    }

    pub fn set_gamepad_trigger(&mut self, slot: usize, code: InputCode, value: f32) {
        let Some(pad) = self.gamepads.get_mut(slot) else {
            return;
        };
        let value = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if code == InputCode::gamepad_left_trigger {
            pad.left_trigger = value;
        } else if code == InputCode::gamepad_right_trigger {
            pad.right_trigger = value;
        } else {
            return;
        }
        self.note_pad_used(slot);
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
            let mut drop_slot = false;
            let sample = match self.joysticks[slot].as_mut() {
                Some(stick) => match drain_joystick(stick) {
                    Ok(()) => Some((stick.name.clone(), stick.state.clone())),
                    Err(()) => {
                        drop_slot = true;
                        None
                    }
                },
                None => None,
            };
            if drop_slot {
                self.joysticks[slot] = None;
            }
            match sample {
                Some((name, state)) => {
                    self.connect_gamepad(slot, true, &name);
                    if let Some(pad) = self.gamepads.get_mut(slot) {
                        pad.left_stick = state.left;
                        pad.right_stick = state.right;
                        pad.left_trigger = state.left_trigger;
                        pad.right_trigger = state.right_trigger;
                        pad.buttons = state.buttons;
                    }
                    self.note_pad_used(slot);
                }
                None => self.connect_gamepad(slot, false, ""),
            }
        }
    }

    fn note_pad_used(&mut self, slot: usize) {
        let used = self.gamepads.get(slot).and_then(|pad| {
            if !pad.view.connected {
                return None;
            }
            let left = pad.axis2d(InputCode::gamepad_left_stick);
            let right = pad.axis2d(InputCode::gamepad_right_stick);
            let tilted = [left, right]
                .into_iter()
                .flatten()
                .any(|axis| axis.x != 0.0 || axis.y != 0.0);
            let held = pad.buttons.contains(&ButtonState::Down)
                || pad.left_trigger >= pad.activation_threshold
                || pad.right_trigger >= pad.activation_threshold;
            if !tilted && !held {
                return None;
            }
            Some((pad.glyphs, pad.view.id, pad.view.name.clone()))
        });
        if let Some((glyphs, id, name)) = used {
            self.note_active(glyphs, id, DeviceKind::Gamepad, &name, true);
        }
    }

    fn note_active(
        &mut self,
        glyphs: GlyphSet,
        id: u32,
        kind: DeviceKind,
        name: &str,
        connected: bool,
    ) {
        if self.active_glyphs == glyphs {
            self.active_id = id;
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
    let mut stick = OpenJoystick {
        file,
        state: JsState::new(),
        name,
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
            Ok(8) => apply_js_event(&buf, &mut stick.state),
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

fn apply_js_event(buf: &[u8; 8], state: &mut JsState) {
    let value = i16::from_ne_bytes([buf[4], buf[5]]);
    let kind = buf[6] & !0x80;
    let number = buf[7];
    if kind == 1 {
        if let Some(code) = code_from_js_button(number) {
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

fn code_from_js_button(number: u8) -> Option<InputCode> {
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
        apply_js_event(&packet(2, 0, 16_384), &mut state);
        apply_js_event(&packet(2, 1, 32_767), &mut state);
        assert!(state.left.x > 0.4 && state.left.x < 0.6);
        assert!(state.left.y < -0.9);
        apply_js_event(&packet(2, 2, -32_767), &mut state);
        apply_js_event(&packet(2, 5, 32_767), &mut state);
        assert!(state.left_trigger < 0.01);
        assert!((state.right_trigger - 1.0).abs() < 0.02);
        apply_js_event(&packet(2, 7, -32_767), &mut state);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_dpad_up).unwrap()],
            ButtonState::Down
        );
        apply_js_event(&packet(1, 0, 1), &mut state);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_face_south).unwrap()],
            ButtonState::Down
        );
        apply_js_event(&packet(1, 0, 0), &mut state);
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
}
