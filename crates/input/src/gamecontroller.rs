#![cfg_attr(
    not(any(test, target_os = "macos")),
    allow(
        dead_code,
        reason = "GameController runs on macOS. Linux still typechecks the mapper."
    )
)]

//! `GameController` pads. A controller keeps its slot until it disconnects.
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

#[derive(Clone, Copy)]
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
    // SAFETY: `raw` is a writable buffer of `cap` slots. The poll fills that many.
    let count = unsafe { genos_gamecontroller_poll(raw.as_mut_ptr(), cap) };
    let count = usize::try_from(count).unwrap_or(0).min(raw.len());
    for (slot, sample) in raw.iter().enumerate().take(count) {
        apply_raw(system, slot, sample);
    }
}

#[cfg(target_os = "macos")]
fn apply_raw(system: &mut InputSystem, slot: usize, raw: &RawPad) {
    if raw.connected == 0 {
        let was = system
            .gamepads
            .get(slot)
            .is_some_and(|pad| pad.view.connected);
        if was {
            eprintln!("genos-input: gamepad slot={slot} source=gamecontroller connected=0");
            system.connect_gamepad(slot, false, "");
        }
        return;
    }
    let name = CStr::from_bytes_until_nul(&raw.name)
        .ok()
        .and_then(|text| text.to_str().ok())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("Gamepad");
    let was = system
        .gamepads
        .get(slot)
        .is_some_and(|pad| pad.view.connected);
    if !was {
        eprintln!("genos-input: gamepad slot={slot} source=gamecontroller connected=1");
    }
    system.connect_gamepad(slot, true, name);
    system.set_gamepad_stick(
        slot,
        InputCode::gamepad_left_stick,
        Axis2d {
            x: raw.lx,
            y: raw.ly,
        },
    );
    system.set_gamepad_stick(
        slot,
        InputCode::gamepad_right_stick,
        Axis2d {
            x: raw.rx,
            y: raw.ry,
        },
    );
    system.set_gamepad_trigger(slot, InputCode::gamepad_left_trigger, raw.lt);
    system.set_gamepad_trigger(slot, InputCode::gamepad_right_trigger, raw.rt);
    for (index, code) in BUTTONS {
        system.set_gamepad_button(slot, code, raw.buttons[index] != 0);
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

    #[test]
    fn a_steady_gamecontroller_sample_does_not_allocate() {
        let mut input = InputSystem::new();
        let mut buttons = [false; 15];
        buttons[0] = true;
        let sample = Sample {
            connected: true,
            name: "DualSense Wireless Controller".to_string(),
            left: Axis2d { x: 0.0, y: 1.0 },
            right: Axis2d::default(),
            left_trigger: 1.0,
            right_trigger: 0.0,
            buttons,
        };
        apply(&mut input, 0, &sample);
        let _ = input.take_device_changes();
        let ((), bytes) = crate::alloc_guard(|| {
            for _ in 0..8 {
                apply(&mut input, 0, &sample);
            }
        });
        assert_eq!(bytes, 0, "steady GameController sample allocated {bytes} bytes");
    }

    #[test]
    fn the_poll_buffer_initializes_without_a_clone() {
        let pads = [super::RawPad {
            connected: 0,
            name: [0; 128],
            lx: 0.0,
            ly: 0.0,
            rx: 0.0,
            ry: 0.0,
            lt: 0.0,
            rt: 0.0,
            buttons: [0; 15],
        }; 4];
        assert_eq!(pads[3].connected, 0);
        assert_eq!(pads.len(), 4);
    }
}
