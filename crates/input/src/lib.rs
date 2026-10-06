//! Pull-based input component.
//!
//! Platform code writes one keyboard, one mouse, and the gamepad slots.
//! Callers read those devices or an [`ActionMap`]. The camera is not part of this crate.

mod action;
mod code;
mod device;
mod system;

pub use action::{character_controller, Action2dBinding, ActionMap, BoundInput};
pub use code::{name, parse, InputCode};
pub use device::{Axis2d, DeviceKind, DeviceView, DEFAULT_STICK_DEADZONE, FIRST_GAMEPAD_ID};
pub use system::InputSystem;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_input_zig_names_and_values() {
        assert_eq!(InputCode::key_w as u16, 87);
        assert_eq!(InputCode::key_escape as u16, 27);
        assert_eq!(InputCode::mouse_left as u16, 1000);
        assert_eq!(InputCode::gamepad_left_stick as u16, 2018);
        assert_eq!(parse("key_a"), Some(InputCode::key_a));
        assert_eq!(name(InputCode::key_d), "key_d");
        assert_eq!(from_letter_evdev(), InputCode::key_w);
    }

    fn from_letter_evdev() -> InputCode {
        device::from_evdev(17).unwrap()
    }

    #[test]
    fn slots_stay_stable_while_disconnected() {
        let input = InputSystem::new();
        assert_eq!(input.list_devices(DeviceKind::Keyboard).len(), 1);
        assert_eq!(input.list_devices(DeviceKind::Mouse).len(), 1);
        assert_eq!(input.gamepad_count(), 0);
        let first = input.gamepad(0).unwrap();
        let second = input.gamepad(1).unwrap();
        assert_eq!(first.view.id, FIRST_GAMEPAD_ID);
        assert_eq!(second.view.id, FIRST_GAMEPAD_ID + 1);
        assert!(!first.view.connected);
        assert!(input.list_devices(DeviceKind::Gamepad).is_empty());
    }

    #[test]
    fn a_tiny_stick_rest_is_zero_until_the_deadzone_is_raised() {
        let mut input = InputSystem::new();
        let pad = input.gamepad_mut(0).unwrap();
        assert_eq!(pad.left_stick_deadzone, DEFAULT_STICK_DEADZONE);
        pad.left_stick.x = 0.07;
        pad.left_stick.y = 0.07;
        let resting = pad.axis2d(InputCode::gamepad_left_stick).unwrap();
        assert_eq!(resting.x, 0.0);
        assert_eq!(resting.y, 0.0);
        pad.left_stick.y = 0.04;
        pad.left_stick.x = 0.2;
        let mixed = pad.axis2d(InputCode::gamepad_left_stick).unwrap();
        assert!(
            (mixed.x - 0.2).abs() < 1.0e-5,
            "a real tilt was dropped: {mixed:?}"
        );
        assert_eq!(mixed.y, 0.0);
        pad.left_stick.x = 1.0;
        let full = pad.axis2d(InputCode::gamepad_left_stick).unwrap();
        assert!(
            (full.x - 1.0).abs() < 1.0e-5,
            "full tilt was reduced: {full:?}"
        );
        pad.left_stick.x = 0.2;
        let partial = pad.axis2d(InputCode::gamepad_left_stick).unwrap();
        assert!(
            (partial.x - 0.2).abs() < 1.0e-5,
            "a tilt past the rest was scaled: {partial:?}"
        );
        pad.set_stick_deadzone(InputCode::gamepad_left_stick, 0.0);
        pad.left_stick.x = 0.04;
        let open = pad.axis2d(InputCode::gamepad_left_stick).unwrap();
        assert!(
            open.x > 0.03,
            "clearing the deadzone should keep the drift, got {open:?}"
        );
        input.set_stick_deadzone(0.2);
        let pad = input.gamepad(0).unwrap();
        assert_eq!(pad.left_stick_deadzone, 0.2);
        assert_eq!(pad.right_stick_deadzone, 0.2);
    }

    #[test]
    fn move_action_reads_wasd_ijkl_and_the_left_stick() {
        let mut input = InputSystem::new();
        let map = character_controller();
        input.set_key(InputCode::key_w, true);
        let forward = map.axis_2d(&input, "move");
        assert!(forward.y > 0.0);
        assert_eq!(forward.x, 0.0);

        input.set_key(InputCode::key_w, false);
        input.set_key(InputCode::key_s, true);
        assert!(map.axis_2d(&input, "move").y < 0.0);

        input.set_key(InputCode::key_s, false);
        input.set_key(InputCode::key_a, true);
        assert!(map.axis_2d(&input, "move").x < 0.0);
        input.set_key(InputCode::key_a, false);
        input.set_key(InputCode::key_d, true);
        assert!(map.axis_2d(&input, "move").x > 0.0);

        input.set_key(InputCode::key_d, false);
        input.set_key(InputCode::key_i, true);
        assert!(map.axis_2d(&input, "move").y > 0.0);
        input.set_key(InputCode::key_i, false);
        input.set_key(InputCode::key_k, true);
        assert!(map.axis_2d(&input, "move").y < 0.0);
        input.set_key(InputCode::key_k, false);
        input.set_key(InputCode::key_j, true);
        assert!(map.axis_2d(&input, "move").x < 0.0);
        input.set_key(InputCode::key_j, false);
        input.set_key(InputCode::key_l, true);
        assert!(map.axis_2d(&input, "move").x > 0.0);

        input.set_key(InputCode::key_l, false);
        let pad = input.gamepad_mut(0).unwrap();
        pad.view.connected = true;
        pad.left_stick.y = 1.0;
        pad.right_stick.x = 0.5;
        assert!(map.axis_2d(&input, "move").y > 0.5);
        assert!(map.axis_2d(&input, "look").x > 0.4);

        let other = input.gamepad_mut(1).unwrap();
        other.view.connected = true;
        other.left_stick.x = 1.0;
        let both = map.axis_2d(&input, "move");
        assert!(
            both.x > 0.5,
            "a second stick still contributes, got {both:?}"
        );
        assert!(both.y > 0.5);
    }

    #[test]
    fn character_map_round_trips_json() {
        let map = character_controller();
        let text = map.to_json();
        let loaded = ActionMap::from_json(&text).unwrap();
        let mut input = InputSystem::new();
        input.set_key(InputCode::key_w, true);
        let original = map.axis_2d(&input, "move");
        let restored = loaded.axis_2d(&input, "move");
        assert!((original.y - restored.y).abs() < 1.0e-4);
        input.set_key(InputCode::key_w, false);
        input.set_key(InputCode::key_i, true);
        let original_i = map.axis_2d(&input, "move");
        let restored_i = loaded.axis_2d(&input, "move");
        assert!((original_i.y - restored_i.y).abs() < 1.0e-4);
        assert!(restored_i.y > 0.0);
        assert!(loaded.down(&input, "release") == false);
        input.set_key(InputCode::key_escape, true);
        assert!(loaded.down(&input, "release"));
    }
}
