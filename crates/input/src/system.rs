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
    layout: JsLayout,
}

const MAX_JS_AXES: usize = 16;

/// One joydev index mapped to a control. Built once, from the family table or
/// from `JSIOCGBTNMAP` / `JSIOCGAXMAP`.
#[derive(Clone, Copy, Debug)]
struct JsLayout {
    buttons: [Option<InputCode>; MAX_GAMEPAD_BUTTONS],
    axes: [JsAxis; MAX_JS_AXES],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsAxis {
    None,
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
    DpadX,
    DpadY,
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
    let layout = read_layout(&file, glyphs);
    let mut stick = OpenJoystick {
        file,
        state: JsState::new(),
        name,
        layout,
    };
    drain_joystick(&mut stick).ok()?;
    Some(stick)
}

/// `JSIOCGNAME(128)` from `linux/joystick.h`.
fn joystick_name(file: &File, slot: usize) -> String {
    let mut buf = [0u8; 128];
    // SAFETY: `fd` is the open joystick. The request reads the device name
    // into `buf`. A failure leaves the buffer zeroed and we use the slot name.
    let rc = js_ioctl(file, 0x8080_6a13, buf.as_mut_ptr());
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
                apply_js_event(&buf, &mut stick.state, &stick.layout);
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

fn js_ioctl(file: &File, request: usize, arg: *mut u8) -> i32 {
    use std::os::fd::AsRawFd;
    unsafe {
        extern "C" {
            fn ioctl(fd: i32, request: usize, ...) -> i32;
        }
        // SAFETY: `fd` is an open joystick. `request` is a `JSIOC*` read and
        // `arg` points at a buffer sized for that request. A negative return
        // leaves the buffer unchanged.
        ioctl(file.as_raw_fd(), request, arg)
    }
}

fn js_u8(file: &File, request: usize) -> Option<usize> {
    let mut count = 0u8;
    if js_ioctl(file, request, &raw mut count) < 0 {
        return None;
    }
    Some(usize::from(count))
}

/// `JSIOCGBUTTONS` then `JSIOCGBTNMAP`. The map is the evdev code at each
/// joydev index. A failure keeps the family table.
fn read_button_codes(file: &File) -> Option<([u16; MAX_GAMEPAD_BUTTONS], usize)> {
    let count = js_u8(file, 0x8001_6a12)?;
    let mut raw = [0u16; 512];
    if js_ioctl(file, 0x8400_6a34, raw.as_mut_ptr().cast()) < 0 {
        return None;
    }
    let n = count.min(MAX_GAMEPAD_BUTTONS);
    let mut out = [0u16; MAX_GAMEPAD_BUTTONS];
    out[..n].copy_from_slice(&raw[..n]);
    Some((out, n))
}

/// `JSIOCGAXES` then `JSIOCGAXMAP`. `ABS_X` is code 0, so the count is the
/// length. A failure keeps the family axes.
fn read_axis_codes(file: &File) -> Option<([u8; MAX_JS_AXES], usize)> {
    let count = js_u8(file, 0x8001_6a11)?;
    let mut raw = [0u8; 64];
    if js_ioctl(file, 0x8040_6a32, raw.as_mut_ptr().cast()) < 0 {
        return None;
    }
    let n = count.min(MAX_JS_AXES);
    let mut out = [0u8; MAX_JS_AXES];
    out[..n].copy_from_slice(&raw[..n]);
    Some((out, n))
}

fn read_layout(file: &File, glyphs: GlyphSet) -> JsLayout {
    let mut layout = layout_for(glyphs);
    if let Some((codes, count)) = read_button_codes(file) {
        layout.buttons = [None; MAX_GAMEPAD_BUTTONS];
        for index in 0..count {
            layout.buttons[index] = evdev_button(codes[index], glyphs);
        }
    }
    if let Some((codes, count)) = read_axis_codes(file) {
        layout.axes = [JsAxis::None; MAX_JS_AXES];
        for index in 0..count {
            layout.axes[index] = evdev_axis(codes[index], glyphs);
        }
    }
    layout
}

fn layout_for(glyphs: GlyphSet) -> JsLayout {
    let mut buttons = [None; MAX_GAMEPAD_BUTTONS];
    for number in 0..MAX_GAMEPAD_BUTTONS {
        buttons[number] = button_from_family(glyphs, number);
    }
    JsLayout {
        buttons,
        axes: if glyphs == GlyphSet::SteamDeck {
            deck_axes()
        } else {
            standard_axes()
        },
    }
}

/// `ABS_X`, `ABS_Y`, `ABS_Z` trigger, `ABS_RX`, `ABS_RY`, `ABS_RZ` trigger,
/// `ABS_HAT0X`, `ABS_HAT0Y`. This is the xpad order.
fn standard_axes() -> [JsAxis; MAX_JS_AXES] {
    let mut axes = [JsAxis::None; MAX_JS_AXES];
    axes[0] = JsAxis::LeftX;
    axes[1] = JsAxis::LeftY;
    axes[2] = JsAxis::LeftTrigger;
    axes[3] = JsAxis::RightX;
    axes[4] = JsAxis::RightY;
    axes[5] = JsAxis::RightTrigger;
    axes[6] = JsAxis::DpadX;
    axes[7] = JsAxis::DpadY;
    axes
}

/// The Deck sets no `ABS_Z` or `ABS_RZ`. Sticks are `ABS_X/Y` and `ABS_RX/RY`.
/// `ABS_HAT0` and `ABS_HAT1` are the trackpads. `ABS_HAT2X` is the right
/// trigger and `ABS_HAT2Y` is the left trigger, joydev indices 8 and 9.
fn deck_axes() -> [JsAxis; MAX_JS_AXES] {
    let mut axes = [JsAxis::None; MAX_JS_AXES];
    axes[0] = JsAxis::LeftX;
    axes[1] = JsAxis::LeftY;
    axes[2] = JsAxis::RightX;
    axes[3] = JsAxis::RightY;
    axes[8] = JsAxis::RightTrigger;
    axes[9] = JsAxis::LeftTrigger;
    axes
}

#[cfg(test)]
fn layout_from_bitmap(glyphs: GlyphSet, buttons: &[u16], axes: &[u8]) -> JsLayout {
    let mut layout = JsLayout {
        buttons: [None; MAX_GAMEPAD_BUTTONS],
        axes: [JsAxis::None; MAX_JS_AXES],
    };
    for (index, code) in buttons.iter().enumerate().take(MAX_GAMEPAD_BUTTONS) {
        layout.buttons[index] = evdev_button(*code, glyphs);
    }
    for (index, code) in axes.iter().enumerate().take(MAX_JS_AXES) {
        layout.axes[index] = evdev_axis(*code, glyphs);
    }
    layout
}

/// Evdev code at a joydev index. On the Deck, `BTN_X` (`0x133`) is the
/// physical west face and `BTN_Y` (`0x134`) is the physical north face.
fn evdev_button(code: u16, glyphs: GlyphSet) -> Option<InputCode> {
    let deck = glyphs == GlyphSet::SteamDeck;
    Some(match code {
        0x126 if deck => InputCode::gamepad_capture,
        0x130 => InputCode::gamepad_face_south,
        0x131 => InputCode::gamepad_face_east,
        0x133 if deck => InputCode::gamepad_face_west,
        0x133 => InputCode::gamepad_face_north,
        0x134 if deck => InputCode::gamepad_face_north,
        0x134 => InputCode::gamepad_face_west,
        0x136 => InputCode::gamepad_left_shoulder,
        0x137 => InputCode::gamepad_right_shoulder,
        0x138 => InputCode::gamepad_left_trigger,
        0x139 => InputCode::gamepad_right_trigger,
        0x13a => InputCode::gamepad_select,
        0x13b => InputCode::gamepad_start,
        0x13c => InputCode::gamepad_home,
        0x13d => InputCode::gamepad_left_stick_press,
        0x13e => InputCode::gamepad_right_stick_press,
        0x220 => InputCode::gamepad_dpad_up,
        0x221 => InputCode::gamepad_dpad_down,
        0x222 => InputCode::gamepad_dpad_left,
        0x223 => InputCode::gamepad_dpad_right,
        _ => return None,
    })
}

fn evdev_axis(code: u8, glyphs: GlyphSet) -> JsAxis {
    let deck = glyphs == GlyphSet::SteamDeck;
    match code {
        0x00 => JsAxis::LeftX,
        0x01 => JsAxis::LeftY,
        0x02 => JsAxis::LeftTrigger,
        0x03 => JsAxis::RightX,
        0x04 => JsAxis::RightY,
        0x05 => JsAxis::RightTrigger,
        0x10 if !deck => JsAxis::DpadX,
        0x11 if !deck => JsAxis::DpadY,
        0x14 => JsAxis::RightTrigger,
        0x15 => JsAxis::LeftTrigger,
        _ => JsAxis::None,
    }
}

fn apply_js_event(buf: &[u8; 8], state: &mut JsState, layout: &JsLayout) {
    let value = i16::from_ne_bytes([buf[4], buf[5]]);
    let kind = buf[6] & !0x80;
    let number = usize::from(buf[7]);
    if kind == 1 {
        if let Some(code) = layout.buttons.get(number).copied().flatten() {
            set_js_button(&mut state.buttons, code, value != 0);
        }
        return;
    }
    if kind != 2 {
        return;
    }
    let Some(role) = layout.axes.get(number).copied() else {
        return;
    };
    let norm = if value < 0 {
        value as f32 / 32768.0
    } else {
        value as f32 / 32767.0
    };
    match role {
        JsAxis::None => {}
        JsAxis::LeftX => state.left.x = norm,
        JsAxis::LeftY => state.left.y = -norm,
        JsAxis::RightX => state.right.x = norm,
        JsAxis::RightY => state.right.y = -norm,
        JsAxis::LeftTrigger => state.left_trigger = trigger_from_axis(norm),
        JsAxis::RightTrigger => state.right_trigger = trigger_from_axis(norm),
        JsAxis::DpadX => {
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
        JsAxis::DpadY => {
            set_js_button(&mut state.buttons, InputCode::gamepad_dpad_up, norm <= -0.5);
            set_js_button(
                &mut state.buttons,
                InputCode::gamepad_dpad_down,
                norm >= 0.5,
            );
        }
    }
}

/// Linux `js` triggers rest at `-1` and reach `1`. That range becomes `0..=1`.
fn trigger_from_axis(norm: f32) -> f32 {
    f32::midpoint(norm, 1.0).clamp(0.0, 1.0)
}

/// Family table used when `JSIOCGBTNMAP` fails. joydev numbers the buttons
/// that are set, from `BTN_JOYSTICK` upward and then the lower codes.
///
/// xpad sets `BTN_X` (`0x133`) for the west face and does not set the trigger
/// buttons, so index 2 is west and the clicks are 9 and 10. hid-playstation
/// sets `BTN_NORTH` before `BTN_WEST` and also sets `BTN_TL2` / `BTN_TR2`, so
/// index 2 is north and the clicks land at 11 and 12. A generic pad without
/// those trigger buttons keeps select at 6. The Deck follows `hid-steam`:
/// the pad touches and the quick-access button come first, so south is index
/// 3 and select is index 11.
fn button_from_family(glyphs: GlyphSet, number: usize) -> Option<InputCode> {
    match glyphs {
        GlyphSet::Xbox => xpad_button(number),
        GlyphSet::PlayStation => playstation_button(number),
        GlyphSet::SteamDeck => deck_button(number),
        GlyphSet::GenericGamepad | GlyphSet::KeyboardMouse => spec_button(number),
    }
}

/// `drivers/hid/hid-steam.c` for `STEAM_QUIRK_DECK`. Index 0 is `BTN_THUMB`
/// (left pad touch) and index 1 is `BTN_THUMB2` (right pad touch). Index 2
/// is `BTN_BASE` (quick access). `BTN_X` is the physical west face and
/// `BTN_Y` is the physical north face. Grip codes have no `InputCode`.
fn deck_button(number: usize) -> Option<InputCode> {
    Some(match number {
        2 => InputCode::gamepad_capture,
        3 => InputCode::gamepad_face_south,
        4 => InputCode::gamepad_face_east,
        5 => InputCode::gamepad_face_west,
        6 => InputCode::gamepad_face_north,
        7 => InputCode::gamepad_left_shoulder,
        8 => InputCode::gamepad_right_shoulder,
        9 => InputCode::gamepad_left_trigger,
        10 => InputCode::gamepad_right_trigger,
        11 => InputCode::gamepad_select,
        12 => InputCode::gamepad_start,
        13 => InputCode::gamepad_home,
        14 => InputCode::gamepad_left_stick_press,
        15 => InputCode::gamepad_right_stick_press,
        16 => InputCode::gamepad_dpad_up,
        17 => InputCode::gamepad_dpad_down,
        18 => InputCode::gamepad_dpad_left,
        19 => InputCode::gamepad_dpad_right,
        _ => return None,
    })
}

fn xpad_button(number: usize) -> Option<InputCode> {
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

fn playstation_button(number: usize) -> Option<InputCode> {
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

fn spec_button(number: usize) -> Option<InputCode> {
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
        let layout = layout_for(GlyphSet::Xbox);
        let mut state = JsState::new();
        apply_js_event(&packet(2, 0, 16_384), &mut state, &layout);
        apply_js_event(&packet(2, 1, 32_767), &mut state, &layout);
        assert!(state.left.x > 0.4 && state.left.x < 0.6);
        assert!(state.left.y < -0.9);
        apply_js_event(&packet(2, 2, -32_767), &mut state, &layout);
        apply_js_event(&packet(2, 5, 32_767), &mut state, &layout);
        assert!(state.left_trigger < 0.01);
        assert!((state.right_trigger - 1.0).abs() < 0.02);
        apply_js_event(&packet(2, 7, -32_767), &mut state, &layout);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_dpad_up).unwrap()],
            ButtonState::Down
        );
        apply_js_event(&packet(1, 0, 1), &mut state, &layout);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_face_south).unwrap()],
            ButtonState::Down
        );
        apply_js_event(&packet(1, 0, 0), &mut state, &layout);
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

        assert_eq!(pressed(GlyphSet::SteamDeck, 0), None);
        assert_eq!(pressed(GlyphSet::SteamDeck, 1), None);
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 2),
            Some(InputCode::gamepad_capture)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 3),
            Some(InputCode::gamepad_face_south)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 5),
            Some(InputCode::gamepad_face_west)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 6),
            Some(InputCode::gamepad_face_north)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 9),
            Some(InputCode::gamepad_left_trigger)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 10),
            Some(InputCode::gamepad_right_trigger)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 11),
            Some(InputCode::gamepad_select)
        );
        assert_eq!(
            pressed(GlyphSet::SteamDeck, 16),
            Some(InputCode::gamepad_dpad_up)
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
    fn steam_deck_axes_keep_the_trackpads_off_the_sticks() {
        let layout = layout_for(GlyphSet::SteamDeck);
        let mut state = JsState::new();
        apply_js_event(&packet(2, 2, 32_767), &mut state, &layout);
        assert!(state.right.x > 0.9);
        assert!(state.left_trigger < 0.01);
        assert!(state.right_trigger < 0.01);
        apply_js_event(&packet(2, 4, 32_767), &mut state, &layout);
        apply_js_event(&packet(2, 6, 32_767), &mut state, &layout);
        assert!(state.right.y.abs() < 0.01);
        assert!(state.left.x.abs() < 0.01);
        assert_eq!(
            state.buttons[gamepad_button_index(InputCode::gamepad_dpad_right).unwrap()],
            ButtonState::Up
        );
        apply_js_event(&packet(2, 8, 32_767), &mut state, &layout);
        apply_js_event(&packet(2, 9, 32_767), &mut state, &layout);
        assert!((state.right_trigger - 1.0).abs() < 0.02);
        assert!((state.left_trigger - 1.0).abs() < 0.02);
    }

    #[test]
    fn a_button_bitmap_keeps_select_on_btn_select() {
        let with_triggers = [
            0x130u16, 0x131, 0x133, 0x134, 0x136, 0x137, 0x138, 0x139, 0x13a, 0x13b, 0x13c, 0x13d,
            0x13e,
        ];
        let layout = layout_from_bitmap(GlyphSet::GenericGamepad, &with_triggers, &[]);
        assert_eq!(layout.buttons[6], Some(InputCode::gamepad_left_trigger));
        assert_eq!(layout.buttons[8], Some(InputCode::gamepad_select));
        assert_eq!(layout.buttons[9], Some(InputCode::gamepad_start));
        assert_eq!(layout.buttons[10], Some(InputCode::gamepad_home));
        assert_eq!(
            layout.buttons[11],
            Some(InputCode::gamepad_left_stick_press)
        );
        assert_eq!(
            layout.buttons[12],
            Some(InputCode::gamepad_right_stick_press)
        );

        let without_triggers = [
            0x130u16, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e,
        ];
        let layout = layout_from_bitmap(GlyphSet::GenericGamepad, &without_triggers, &[]);
        assert_eq!(layout.buttons[6], Some(InputCode::gamepad_select));
        assert_eq!(layout.buttons[8], Some(InputCode::gamepad_home));
        assert_eq!(layout.buttons[9], Some(InputCode::gamepad_left_stick_press));
    }

    #[test]
    fn a_deck_bitmap_matches_hid_steam() {
        let buttons = [
            0x121u16, 0x122, 0x126, 0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x138, 0x139, 0x13a,
            0x13b, 0x13c, 0x13d, 0x13e, 0x220, 0x221, 0x222, 0x223, 0x224, 0x225, 0x226, 0x227,
        ];
        let axes = [0x00u8, 0x01, 0x03, 0x04, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15];
        let layout = layout_from_bitmap(GlyphSet::SteamDeck, &buttons, &axes);
        assert_eq!(layout.buttons[0], None);
        assert_eq!(layout.buttons[1], None);
        assert_eq!(layout.buttons[2], Some(InputCode::gamepad_capture));
        assert_eq!(layout.buttons[3], Some(InputCode::gamepad_face_south));
        assert_eq!(layout.buttons[5], Some(InputCode::gamepad_face_west));
        assert_eq!(layout.buttons[6], Some(InputCode::gamepad_face_north));
        assert_eq!(layout.buttons[9], Some(InputCode::gamepad_left_trigger));
        assert_eq!(layout.buttons[10], Some(InputCode::gamepad_right_trigger));
        assert_eq!(layout.buttons[11], Some(InputCode::gamepad_select));
        assert_eq!(layout.buttons[16], Some(InputCode::gamepad_dpad_up));
        assert_eq!(layout.buttons[20], None);
        assert_eq!(layout.axes[2], JsAxis::RightX);
        assert_eq!(layout.axes[4], JsAxis::None);
        assert_eq!(layout.axes[6], JsAxis::None);
        assert_eq!(layout.axes[8], JsAxis::RightTrigger);
        assert_eq!(layout.axes[9], JsAxis::LeftTrigger);
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
                layout: layout_for(glyphs),
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
        let layout = layout_for(glyphs);
        let mut state = JsState::new();
        apply_js_event(&packet(1, number, 1), &mut state, &layout);
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
            InputCode::gamepad_capture,
            InputCode::gamepad_left_stick_press,
            InputCode::gamepad_right_stick_press,
            InputCode::gamepad_dpad_up,
            InputCode::gamepad_dpad_down,
            InputCode::gamepad_dpad_left,
            InputCode::gamepad_dpad_right,
        ];
        codes.into_iter().find(|code| {
            gamepad_button_index(*code)
                .is_some_and(|index| state.buttons[index] == ButtonState::Down)
        })
    }
}
