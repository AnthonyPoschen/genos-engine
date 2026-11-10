#![cfg_attr(
    not(any(test, target_os = "macos")),
    allow(
        dead_code,
        reason = "GameController runs on macOS. Linux still typechecks the mapper."
    )
)]

//! `GameController` pads. Connected extended gamepads fill the low slots.
//!
//! Stick Y is already positive upward. Trigger values are already `0..=1`.
//! The reported name selects the picture family. This file does not read `IOKit`,
//! so a pad that `GameController` does not publish stays disconnected.

#[cfg(target_os = "macos")]
use std::ffi::CStr;

use crate::code::InputCode;
use crate::device::Axis2d;
use crate::system::InputSystem;

const BUTTONS: [(usize, InputCode); 15] = [
    (0, InputCode::gamepad_face_south),
    (1, InputCode::gamepad_face_east),
    (2, InputCode::gamepad_face_west),
    (3, InputCode::gamepad_face_north),
    (4, InputCode::gamepad_dpad_up),
    (5, InputCode::gamepad_dpad_down),
    (6, InputCode::gamepad_dpad_left),
    (7, InputCode::gamepad_dpad_right),
    (8, InputCode::gamepad_left_shoulder),
    (9, InputCode::gamepad_right_shoulder),
    (10, InputCode::gamepad_select),
    (11, InputCode::gamepad_start),
    (12, InputCode::gamepad_home),
    (13, InputCode::gamepad_left_stick_press),
    (14, InputCode::gamepad_right_stick_press),
];

#[cfg(target_os = "macos")]
#[repr(C)]
struct RawPad {
    connected: i32,
    name: [u8; 128],
    lx: f32,
    ly: f32,
    rx: f32,
    ry: f32,
    lt: f32,
    rt: f32,
    buttons: [i32; 15],
}

/// One pad sample. `buttons` follows [`BUTTONS`].
struct Sample {
    connected: bool,
    name: String,
    left: Axis2d,
    right: Axis2d,
    left_trigger: f32,
    right_trigger: f32,
    buttons: [bool; 15],
}

#[cfg(target_os = "macos")]
extern "C" {
    fn genos_gamecontroller_poll(out: *mut RawPad, cap: i32) -> i32;
}

fn apply(system: &mut InputSystem, slot: usize, sample: &Sample) {
    let was = system
        .gamepads
        .get(slot)
        .is_some_and(|pad| pad.view.connected);
    if sample.connected != was {
        let flag = i32::from(sample.connected);
        eprintln!("genos-input: gamepad slot={slot} source=gamecontroller connected={flag}");
    }
    if !sample.connected {
        system.connect_gamepad(slot, false, "");
        return;
    }
    let name = if sample.name.is_empty() {
        "Gamepad"
    } else {
        sample.name.as_str()
    };
    system.connect_gamepad(slot, true, name);
    system.set_gamepad_stick(slot, InputCode::gamepad_left_stick, sample.left);
    system.set_gamepad_stick(slot, InputCode::gamepad_right_stick, sample.right);
    system.set_gamepad_trigger(slot, InputCode::gamepad_left_trigger, sample.left_trigger);
    system.set_gamepad_trigger(slot, InputCode::gamepad_right_trigger, sample.right_trigger);
    for (index, code) in BUTTONS {
        system.set_gamepad_button(slot, code, sample.buttons[index]);
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn poll(system: &mut InputSystem) {
    let mut raw = [RawPad {
        connected: 0,
        name: [0; 128],
        lx: 0.0,
        ly: 0.0,
        rx: 0.0,
        ry: 0.0,
        lt: 0.0,
        rt: 0.0,
        buttons: [0; 15],
    }; crate::device::MAX_GAMEPADS];
    let cap = i32::try_from(raw.len()).unwrap_or(0);
    let count = unsafe { genos_gamecontroller_poll(raw.as_mut_ptr(), cap) };
    let count = usize::try_from(count).unwrap_or(0).min(raw.len());
    for (slot, sample) in raw.iter().enumerate() {
        if slot < count && sample.connected != 0 {
            apply(system, slot, &from_raw(sample));
        } else {
            apply(system, slot, &Sample::disconnected());
        }
    }
}

#[cfg(target_os = "macos")]
fn from_raw(sample: &RawPad) -> Sample {
    let name = CStr::from_bytes_until_nul(&sample.name)
        .ok()
        .map(|text| text.to_string_lossy().trim().to_string())
        .unwrap_or_default();
    let mut buttons = [false; 15];
    for (index, down) in sample.buttons.iter().enumerate() {
        buttons[index] = *down != 0;
    }
    Sample {
        connected: true,
        name,
        left: Axis2d {
            x: sample.lx,
            y: sample.ly,
        },
        right: Axis2d {
            x: sample.rx,
            y: sample.ry,
        },
        left_trigger: sample.lt,
        right_trigger: sample.rt,
        buttons,
    }
}

impl Sample {
    fn disconnected() -> Self {
        Self {
            connected: false,
            name: String::new(),
            left: Axis2d::default(),
            right: Axis2d::default(),
            left_trigger: 0.0,
            right_trigger: 0.0,
            buttons: [false; 15],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply, Sample};
    use crate::code::InputCode;
    use crate::device::{Axis2d, GlyphSet};
    use crate::system::InputSystem;

    #[test]
    fn a_dualsense_keeps_an_upward_stick_and_playstation_pictures() {
        let mut input = InputSystem::new();
        input.begin_frame();
        let mut buttons = [false; 15];
        buttons[0] = true;
        buttons[4] = true;
        apply(
            &mut input,
            0,
            &Sample {
                connected: true,
                name: "DualSense Wireless Controller".to_string(),
                left: Axis2d { x: 0.0, y: 1.0 },
                right: Axis2d { x: -1.0, y: 0.0 },
                left_trigger: 1.0,
                right_trigger: 0.0,
                buttons,
            },
        );
        let pad = &input.gamepads[0];
        assert!(pad.view.connected);
        assert_eq!(pad.glyphs, GlyphSet::PlayStation);
        assert!(pad.down(InputCode::gamepad_face_south));
        assert!(pad.down(InputCode::gamepad_dpad_up));
        let stick = pad
            .axis2d(InputCode::gamepad_left_stick)
            .unwrap_or_default();
        assert!(stick.y > 0.99, "GameController Y is up, got {}", stick.y);
        assert!(pad.down(InputCode::gamepad_left_trigger));
        apply(&mut input, 0, &Sample::disconnected());
        assert!(!input.gamepads[0].view.connected);
        assert_eq!(input.gamepads[0].left_stick.y, 0.0);
    }
}
