#![cfg_attr(
    not(any(test, target_os = "windows")),
    allow(
        dead_code,
        reason = "XInput runs on Windows. Linux still typechecks the mapper."
    )
)]

//! `XInput` gamepads. Slots 0..3 are the `XInput` user indices. The rest stay disconnected.
//!
//! Stick Y is already positive upward, so it is not negated the way a Linux
//! `/dev/input/jsN` axis is. The engine deadzone still applies on read.
//! The name `XInput` selects Xbox pictures.

use crate::code::InputCode;
use crate::device::Axis2d;
use crate::system::InputSystem;

const XINPUT_A: u16 = 0x1000;
const XINPUT_B: u16 = 0x2000;
const XINPUT_X: u16 = 0x4000;
const XINPUT_Y: u16 = 0x8000;
const XINPUT_UP: u16 = 0x0001;
const XINPUT_DOWN: u16 = 0x0002;
const XINPUT_LEFT: u16 = 0x0004;
const XINPUT_RIGHT: u16 = 0x0008;
const XINPUT_START: u16 = 0x0010;
const XINPUT_BACK: u16 = 0x0020;
const XINPUT_LEFT_THUMB: u16 = 0x0040;
const XINPUT_RIGHT_THUMB: u16 = 0x0080;
const XINPUT_LEFT_SHOULDER: u16 = 0x0100;
const XINPUT_RIGHT_SHOULDER: u16 = 0x0200;
const XINPUT_GUIDE: u16 = 0x0400;

const BUTTONS: [(u16, InputCode); 14] = [
    (XINPUT_A, InputCode::gamepad_face_south),
    (XINPUT_B, InputCode::gamepad_face_east),
    (XINPUT_X, InputCode::gamepad_face_west),
    (XINPUT_Y, InputCode::gamepad_face_north),
    (XINPUT_UP, InputCode::gamepad_dpad_up),
    (XINPUT_DOWN, InputCode::gamepad_dpad_down),
    (XINPUT_LEFT, InputCode::gamepad_dpad_left),
    (XINPUT_RIGHT, InputCode::gamepad_dpad_right),
    (XINPUT_LEFT_SHOULDER, InputCode::gamepad_left_shoulder),
    (XINPUT_RIGHT_SHOULDER, InputCode::gamepad_right_shoulder),
    (XINPUT_BACK, InputCode::gamepad_select),
    (XINPUT_START, InputCode::gamepad_start),
    (XINPUT_LEFT_THUMB, InputCode::gamepad_left_stick_press),
    (XINPUT_RIGHT_THUMB, InputCode::gamepad_right_stick_press),
];

#[cfg(target_os = "windows")]
#[repr(C)]
struct XInputGamepad {
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumb_lx: i16,
    thumb_ly: i16,
    thumb_rx: i16,
    thumb_ry: i16,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct XInputState {
    packet: u32,
    gamepad: XInputGamepad,
}

#[cfg(all(target_os = "windows", target_env = "msvc"))]
#[link(name = "Xinput")]
extern "system" {
    fn XInputGetState(user: u32, state: *mut XInputState) -> u32;
}

#[cfg(all(target_os = "windows", not(target_env = "msvc")))]
#[link(name = "xinput1_4")]
extern "system" {
    fn XInputGetState(user: u32, state: *mut XInputState) -> u32;
}

/// Write one `XInput` sample through the public gamepad setters.
#[expect(
    clippy::too_many_arguments,
    reason = "one XInput packet: slot, connection, buttons, four axes, two triggers"
)]
pub(crate) fn apply(
    system: &mut InputSystem,
    slot: usize,
    connected: bool,
    buttons: u16,
    lx: i16,
    ly: i16,
    rx: i16,
    ry: i16,
    lt: u8,
    rt: u8,
) {
    let was = system
        .gamepads
        .get(slot)
        .is_some_and(|pad| pad.view.connected);
    if connected != was {
        let flag = i32::from(connected);
        eprintln!("genos-input: gamepad slot={slot} source=xinput connected={flag}");
    }
    if !connected {
        system.connect_gamepad(slot, false, "");
        return;
    }
    system.connect_gamepad(slot, true, "XInput");
    system.set_gamepad_stick(
        slot,
        InputCode::gamepad_left_stick,
        Axis2d {
            x: stick(lx),
            y: stick(ly),
        },
    );
    system.set_gamepad_stick(
        slot,
        InputCode::gamepad_right_stick,
        Axis2d {
            x: stick(rx),
            y: stick(ry),
        },
    );
    system.set_gamepad_trigger(slot, InputCode::gamepad_left_trigger, f32::from(lt) / 255.0);
    system.set_gamepad_trigger(
        slot,
        InputCode::gamepad_right_trigger,
        f32::from(rt) / 255.0,
    );
    for (mask, code) in BUTTONS {
        system.set_gamepad_button(slot, code, buttons & mask != 0);
    }
    if buttons & XINPUT_GUIDE != 0 {
        system.set_gamepad_button(slot, InputCode::gamepad_home, true);
    } else {
        system.set_gamepad_button(slot, InputCode::gamepad_home, false);
    }
}

fn stick(value: i16) -> f32 {
    (f32::from(value) / 32767.0).clamp(-1.0, 1.0)
}

#[cfg(target_os = "windows")]
pub(crate) fn poll(system: &mut InputSystem) {
    for slot in 0..crate::device::MAX_GAMEPADS {
        if slot < 4 {
            read_user(system, slot);
        } else {
            apply(system, slot, false, 0, 0, 0, 0, 0, 0, 0);
        }
    }
}

#[cfg(target_os = "windows")]
fn read_user(system: &mut InputSystem, slot: usize) {
    let mut state = XInputState {
        packet: 0,
        gamepad: XInputGamepad {
            buttons: 0,
            left_trigger: 0,
            right_trigger: 0,
            thumb_lx: 0,
            thumb_ly: 0,
            thumb_rx: 0,
            thumb_ry: 0,
        },
    };
    let user = u32::try_from(slot).unwrap_or(0);
    // SAFETY: `state` is a valid XINPUT_STATE. A non-zero code means the slot is empty.
    let code = unsafe { XInputGetState(user, &mut state) };
    if code != 0 {
        apply(system, slot, false, 0, 0, 0, 0, 0, 0, 0);
        return;
    }
    let pad = &state.gamepad;
    apply(
        system,
        slot,
        true,
        pad.buttons,
        pad.thumb_lx,
        pad.thumb_ly,
        pad.thumb_rx,
        pad.thumb_ry,
        pad.left_trigger,
        pad.right_trigger,
    );
}

#[cfg(test)]
mod tests {
    use super::apply;
    use crate::code::InputCode;
    use crate::device::GlyphSet;
    use crate::system::InputSystem;

    #[test]
    fn an_a_button_and_an_upward_stick_keep_their_sign() {
        let mut input = InputSystem::new();
        input.begin_frame();
        apply(
            &mut input,
            0,
            true,
            0x1000 | 0x0001,
            0,
            32767,
            -32768,
            0,
            255,
            0,
        );
        let pad = &input.gamepads[0];
        assert!(pad.view.connected);
        assert_eq!(pad.glyphs, GlyphSet::Xbox);
        assert!(pad.down(InputCode::gamepad_face_south));
        assert!(pad.down(InputCode::gamepad_dpad_up));
        assert!(!pad.down(InputCode::gamepad_face_east));
        let stick = pad
            .axis2d(InputCode::gamepad_left_stick)
            .unwrap_or_default();
        assert!(stick.y > 0.99, "XInput Y is up, got {}", stick.y);
        let right = pad
            .axis2d(InputCode::gamepad_right_stick)
            .unwrap_or_default();
        assert!(right.x < -0.99, "full left tilt, got {}", right.x);
        assert!(pad.left_trigger > 0.99);
        assert_eq!(pad.right_trigger, 0.0);
        assert!(pad.down(InputCode::gamepad_left_trigger));
        apply(&mut input, 0, false, 0x1000, 100, 100, 0, 0, 10, 10);
        assert!(!input.gamepads[0].view.connected);
        assert_eq!(input.gamepads[0].left_stick.y, 0.0);
        assert!(!input.gamepads[0].down(InputCode::gamepad_face_south));
    }
}
