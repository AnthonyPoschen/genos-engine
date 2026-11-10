//! Player settings for the action map, the stick deadzone, and the mouse scale.
//!
//! The text form is `key=value` lines, the same shape as other engine settings.
//! A file that starts with `{` is the action-map JSON from [`ActionMap::to_json`].

use std::path::Path;

use crate::action::{character_controller, format_number, ActionMap};
use crate::device::DEFAULT_STICK_DEADZONE;
use crate::system::InputSystem;

/// Deadzone, mouse scale, and the action map a player can rebind.
#[derive(Clone, Debug)]
pub struct InputSettings {
    pub stick_deadzone: f32,
    pub mouse_sensitivity: f32,
    pub map: ActionMap,
}

impl InputSettings {
    /// The character-controller map, the default deadzone, and mouse scale `1`.
    #[must_use]
    pub fn character_controller() -> Self {
        Self {
            stick_deadzone: DEFAULT_STICK_DEADZONE,
            mouse_sensitivity: 1.0,
            map: character_controller(),
        }
    }

    /// Copy the deadzone onto every gamepad slot and the mouse scale onto `input`.
    pub fn apply(&self, input: &mut InputSystem) {
        input.set_stick_deadzone(self.stick_deadzone);
        input.set_mouse_sensitivity(self.mouse_sensitivity);
        input.set_actions(&self.map);
    }

    /// The settings file text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from("stick_deadzone=");
        out.push_str(&format_number(self.stick_deadzone));
        out.push_str("\nmouse_sensitivity=");
        out.push_str(&format_number(self.mouse_sensitivity));
        out.push('\n');
        out.push_str(&self.map.to_text());
        out
    }

    /// The settings file text. This is the probe for the values in force.
    #[must_use]
    pub fn dump(&self) -> String {
        self.to_text()
    }

    /// Read settings text. JSON is accepted for the map, with the default scalars.
    ///
    /// # Errors
    ///
    /// Returns an error for a bad line, an unknown code, or an unknown field.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.trim_start().starts_with('{') {
            return Ok(Self {
                stick_deadzone: DEFAULT_STICK_DEADZONE,
                mouse_sensitivity: 1.0,
                map: ActionMap::from_json(text)?,
            });
        }
        let mut settings = Self::character_controller();
        for raw in text.split(['\n', ';']) {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("use key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
        Ok(settings)
    }

    /// Overlay one key. `stick_deadzone` and `mouse_sensitivity` stay here.
    /// Every other key is an action binding.
    ///
    /// # Errors
    ///
    /// Returns an error for a bad number or for an action key the map rejects.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        match key {
            "stick_deadzone" => {
                let deadzone = value
                    .parse::<f32>()
                    .map_err(|_| format!("stick_deadzone is a number, got {value}"))?;
                if !deadzone.is_finite() {
                    return Err("stick_deadzone must be finite".into());
                }
                self.stick_deadzone = deadzone.clamp(0.0, 0.95);
                Ok(())
            }
            "mouse_sensitivity" => {
                let sensitivity = value
                    .parse::<f32>()
                    .map_err(|_| format!("mouse_sensitivity is a number, got {value}"))?;
                if !sensitivity.is_finite() {
                    return Err("mouse_sensitivity must be finite".into());
                }
                self.mouse_sensitivity = sensitivity.clamp(0.0, 20.0);
                Ok(())
            }
            _ => self.map.set_key(key, value),
        }
    }

    /// Write the settings text to `path`.
    ///
    /// # Errors
    ///
    /// Returns the filesystem error.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_text()).map_err(|err| err.to_string())
    }

    /// Read a settings file written by [`Self::save`].
    ///
    /// # Errors
    ///
    /// Returns a filesystem error or a [`Self::parse`] error.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
        Self::parse(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::character_controller;
    use crate::code::InputCode;
    use crate::device::Axis2d;
    use crate::system::InputSystem;

    #[test]
    fn rebind_keeps_several_codes_and_scales_the_stick_after_the_deadzone() {
        let mut settings = InputSettings::character_controller();
        settings.set_key("release", "key_b mouse_right").unwrap();
        settings.set_key("look.sensitivity", "2").unwrap();
        settings.set_key("stick_deadzone", "0.2").unwrap();
        let text = settings.to_text();
        let loaded = InputSettings::parse(&text).unwrap();
        assert!(loaded.dump().contains("key_b"));
        assert!(loaded.dump().contains("mouse_right"));

        let mut input = InputSystem::new();
        loaded.apply(&mut input);
        assert!((input.gamepad(0).unwrap().left_stick_deadzone - 0.2).abs() < 1.0e-4);

        input.set_key(InputCode::key_escape, true);
        assert!(!loaded.map.down(&input, "release"));
        input.set_key(InputCode::key_escape, false);
        input.set_key(InputCode::key_b, true);
        assert!(loaded.map.down(&input, "release"));
        input.set_key(InputCode::key_b, false);
        input.set_mouse_button(InputCode::mouse_right, true);
        assert!(loaded.map.down(&input, "release"));

        let pad = input.gamepad_mut(0).unwrap();
        pad.view.connected = true;
        pad.right_stick.x = 0.15;
        let inside = loaded.map.axis_2d(&input, "look");
        assert!(
            inside.x.abs() < 1.0e-6,
            "deadzone runs before sensitivity, got {inside:?}"
        );
        let pad = input.gamepad_mut(0).unwrap();
        pad.right_stick.x = 0.25;
        let outside = loaded.map.axis_2d(&input, "look");
        assert!(
            (outside.x - 0.5).abs() < 1.0e-4,
            "0.25 times sensitivity 2, got {outside:?}"
        );

        input.set_key(InputCode::key_w, true);
        assert!(loaded.map.axis_2d(&input, "move").y > 0.5);
        input.set_key(InputCode::key_w, false);
        input.set_key(InputCode::key_i, true);
        assert!(
            loaded.map.axis_2d(&input, "move").y > 0.5,
            "the second move binding still counts"
        );
    }

    #[test]
    fn settings_file_round_trips_and_json_keeps_sensitivity() {
        let mut settings = InputSettings::character_controller();
        settings
            .set_key("look.vectors", "gamepad_right_stick*2.5")
            .unwrap();
        let path =
            std::env::temp_dir().join(format!("genos-input-settings-{}.txt", std::process::id()));
        settings.save(&path).unwrap();
        let loaded = InputSettings::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let mut input = InputSystem::new();
        let pad = input.gamepad_mut(0).unwrap();
        pad.view.connected = true;
        pad.right_stick.y = 0.2;
        let look = loaded.map.axis_2d(&input, "look");
        assert!(
            (look.y - 0.5).abs() < 1.0e-3,
            "0.2 times sensitivity 2.5, got {look:?}"
        );

        let json = loaded.map.to_json();
        assert!(json.contains("sensitivity"));
        let from_json = InputSettings::parse(&json).unwrap();
        let again = from_json.map.axis_2d(&input, "look");
        assert!((again.y - look.y).abs() < 1.0e-4);
    }

    #[test]
    fn mouse_scale_applies_from_the_settings_file() {
        let mut settings = InputSettings::character_controller();
        settings.set_key("mouse_sensitivity", "2").unwrap();
        let mut input = InputSystem::new();
        settings.apply(&mut input);
        input.add_mouse_delta(1.5, -0.25);
        assert!((input.mouse.dx - 3.0).abs() < 1.0e-4);
        assert!((input.mouse.dy + 0.5).abs() < 1.0e-4);
        assert!((input.mouse_sensitivity() - 2.0).abs() < 1.0e-4);
    }

    #[test]
    fn a_partial_file_keeps_wasd_and_accepts_sensitivity_first() {
        let loaded = InputSettings::parse(
            "stick_deadzone=0.2\nlook.sensitivity=2\njump.sensitivity=3\njump=key_space\nrelease.enabled=0\nrelease=key_b",
        )
        .unwrap();
        let mut input = InputSystem::new();
        loaded.apply(&mut input);
        input.set_key(InputCode::key_w, true);
        assert!(loaded.map.axis_2d(&input, "move").y > 0.5);
        input.set_key(InputCode::key_w, false);

        let pad = input.gamepad_mut(0).unwrap();
        pad.view.connected = true;
        pad.right_stick.x = 0.3;
        let look = loaded.map.axis_2d(&input, "look");
        assert!(
            (look.x - 0.6).abs() < 1.0e-3,
            "0.3 times the sensitivity that preceded any rebind, got {look:?}"
        );

        input.set_key(InputCode::key_space, true);
        assert!(loaded.map.down(&input, "jump"));
        assert!(loaded.dump().contains("key_space*3"));
        input.set_key(InputCode::key_b, true);
        assert!(!loaded.map.down(&input, "release"));
        let probe = input.dump();
        assert!(probe.contains("bindings"));
        assert!(probe.contains("key_w"), "{probe}");
    }

    #[test]
    fn sensitivity_clamps_a_scaled_stick_at_one() {
        let mut settings = InputSettings::character_controller();
        settings
            .set_key("look.vectors", "gamepad_right_stick*2")
            .unwrap();
        let mut input = InputSystem::new();
        let pad = input.gamepad_mut(0).unwrap();
        pad.view.connected = true;
        pad.right_stick.x = 0.8;
        let look = settings.map.axis_2d(&input, "look");
        assert!(
            (look.x - 1.0).abs() < 1.0e-4,
            "0.8 times 2 clamps at 1, got {look:?}"
        );
    }

    #[test]
    fn a_resting_stick_does_not_move_after_a_raise_in_sensitivity() {
        let mut input = InputSystem::new();
        let pad = input.gamepad_mut(0).unwrap();
        pad.view.connected = true;
        pad.left_stick = Axis2d { x: 0.07, y: 0.07 };
        let mut map = character_controller();
        map.set_sensitivity("move", 4.0).unwrap();
        let axis = map.axis_2d(&input, "move");
        assert!(axis.x.abs() < 1.0e-6 && axis.y.abs() < 1.0e-6, "{axis:?}");
    }
}
