//! Named debug keys. The first chord is the primary key. The rest are aliases.
//!
//! `GENOS_DEBUG_KEYS` points at a text file that replaces chords. A line is
//! `action key [key...]`. A key is an [`InputCode`] name. `+` before a name
//! requires shift. `#` starts a comment.

use crate::code::{name, parse, InputCode};
use crate::system::InputSystem;

/// One key, with or without shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub code: InputCode,
    pub shift: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActionKeys {
    name: String,
    chords: Vec<Chord>,
}

/// Actions a debug binary reads instead of hard-coded function keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBindings {
    actions: Vec<ActionKeys>,
}

impl KeyBindings {
    /// Menu, frame-rate plate, and unfilled count. Function keys stay as aliases.
    pub fn builtin() -> Self {
        Self {
            actions: vec![
                action(
                    "menu",
                    &[
                        Chord {
                            code: InputCode::key_grave,
                            shift: false,
                        },
                        Chord {
                            code: InputCode::key_f1,
                            shift: false,
                        },
                    ],
                ),
                action(
                    "fps",
                    &[
                        Chord {
                            code: InputCode::key_f,
                            shift: false,
                        },
                        Chord {
                            code: InputCode::key_f3,
                            shift: false,
                        },
                    ],
                ),
                action(
                    "unfilled",
                    &[
                        Chord {
                            code: InputCode::key_u,
                            shift: false,
                        },
                        Chord {
                            code: InputCode::key_f8,
                            shift: false,
                        },
                    ],
                ),
                action(
                    "unfilled_tint",
                    &[
                        Chord {
                            code: InputCode::key_u,
                            shift: true,
                        },
                        Chord {
                            code: InputCode::key_f8,
                            shift: true,
                        },
                    ],
                ),
                action(
                    "tab_next",
                    &[Chord {
                        code: InputCode::key_right_bracket,
                        shift: false,
                    }],
                ),
                action(
                    "tab_prev",
                    &[Chord {
                        code: InputCode::key_left_bracket,
                        shift: false,
                    }],
                ),
                action(
                    "tab_focus_next",
                    &[Chord {
                        code: InputCode::key_tab,
                        shift: false,
                    }],
                ),
                action(
                    "tab_focus_prev",
                    &[Chord {
                        code: InputCode::key_tab,
                        shift: true,
                    }],
                ),
            ],
        }
    }

    /// True when one of the action's chords was pressed this frame.
    ///
    /// A chord with `shift` matches only while shift is down. A chord without
    /// shift matches only while shift is up, so shift-U does not also fire `unfilled`.
    pub fn pressed(&self, input: &InputSystem, action: &str) -> bool {
        let Some(action) = self.actions.iter().find(|item| item.name == action) else {
            return false;
        };
        let shift = input.keyboard.down(InputCode::key_shift_left)
            || input.keyboard.down(InputCode::key_shift_right);
        action
            .chords
            .iter()
            .any(|chord| input.keyboard.pressed(chord.code) && chord.shift == shift)
    }

    /// Replace the chords for `name`, or add the action when it is new.
    pub fn rebind(&mut self, name: &str, chords: Vec<Chord>) {
        if let Some(action) = self.actions.iter_mut().find(|item| item.name == name) {
            action.chords = chords;
        } else {
            self.actions.push(ActionKeys {
                name: name.to_string(),
                chords,
            });
        }
    }

    /// Apply a config file. Each listed action replaces its chords.
    pub fn apply_text(&mut self, text: &str) -> Result<(), String> {
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let name = parts
                .next()
                .ok_or_else(|| format!("line {}: missing action", index + 1))?;
            let mut chords = Vec::new();
            for token in parts {
                let (shift, code_name) = token
                    .strip_prefix('+')
                    .map(|rest| (true, rest))
                    .unwrap_or((false, token));
                let code = parse(code_name)
                    .ok_or_else(|| format!("line {}: unknown key {code_name}", index + 1))?;
                chords.push(Chord { code, shift });
            }
            if chords.is_empty() {
                return Err(format!("line {}: {name} has no keys", index + 1));
            }
            self.rebind(name, chords);
        }
        Ok(())
    }

    /// Words for the help line. The primary key is first.
    pub fn hint(&self, action: &str) -> String {
        let Some(action) = self.actions.iter().find(|item| item.name == action) else {
            return String::new();
        };
        action
            .chords
            .iter()
            .map(chord_label)
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

/// Built-in debug keys, then `GENOS_DEBUG_KEYS` when that path is set.
pub fn debug_keys() -> Result<KeyBindings, String> {
    let mut keys = KeyBindings::builtin();
    if let Ok(path) = std::env::var("GENOS_DEBUG_KEYS") {
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("GENOS_DEBUG_KEYS {path}: {err}"))?;
        keys.apply_text(&text)?;
    }
    Ok(keys)
}

fn action(name: &str, chords: &[Chord]) -> ActionKeys {
    ActionKeys {
        name: name.to_string(),
        chords: chords.to_vec(),
    }
}

fn chord_label(chord: &Chord) -> String {
    let word = match chord.code {
        InputCode::key_grave => "grave".to_string(),
        InputCode::key_left_bracket => "left bracket".to_string(),
        InputCode::key_right_bracket => "right bracket".to_string(),
        InputCode::key_tab => "tab".to_string(),
        InputCode::key_f => "F".to_string(),
        InputCode::key_u => "U".to_string(),
        InputCode::key_f1 => "F1".to_string(),
        InputCode::key_f3 => "F3".to_string(),
        InputCode::key_f8 => "F8".to_string(),
        other => {
            let raw = name(other);
            raw.strip_prefix("key_").unwrap_or(raw).to_string()
        }
    };
    if chord.shift {
        format!("shift-{word}")
    } else {
        word
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: InputCode, shift: bool) -> InputSystem {
        let mut input = InputSystem::new();
        input.set_key(code, true);
        if shift {
            input.set_key(InputCode::key_shift_left, true);
        }
        input
    }

    #[test]
    fn grave_hides_the_menu_and_f1_is_the_alias() {
        let keys = KeyBindings::builtin();
        assert!(keys.pressed(&press(InputCode::key_grave, false), "menu"));
        assert!(keys.pressed(&press(InputCode::key_f1, false), "menu"));
        assert!(!keys.pressed(&press(InputCode::key_f, false), "menu"));
        assert_eq!(keys.hint("menu"), "grave or F1");
    }

    #[test]
    fn the_letter_f_toggles_the_plate_and_f3_is_the_alias() {
        let keys = KeyBindings::builtin();
        assert!(keys.pressed(&press(InputCode::key_f, false), "fps"));
        assert!(keys.pressed(&press(InputCode::key_f3, false), "fps"));
        assert_eq!(keys.hint("fps"), "F or F3");
    }

    #[test]
    fn shift_u_tints_and_does_not_also_toggle_the_count() {
        let keys = KeyBindings::builtin();
        let shifted = press(InputCode::key_u, true);
        assert!(keys.pressed(&shifted, "unfilled_tint"));
        assert!(!keys.pressed(&shifted, "unfilled"));
        let plain = press(InputCode::key_u, false);
        assert!(keys.pressed(&plain, "unfilled"));
        assert!(!keys.pressed(&plain, "unfilled_tint"));
        assert!(keys.pressed(&press(InputCode::key_f8, true), "unfilled_tint"));
        assert_eq!(keys.hint("unfilled"), "U or F8");
        assert_eq!(keys.hint("unfilled_tint"), "shift-U or shift-F8");
        assert!(keys.pressed(&press(InputCode::key_right_bracket, false), "tab_next"));
        assert!(keys.pressed(&press(InputCode::key_left_bracket, false), "tab_prev"));
        assert!(keys.pressed(&press(InputCode::key_tab, false), "tab_focus_next"));
        assert!(!keys.pressed(&press(InputCode::key_tab, false), "tab_focus_prev"));
        assert!(keys.pressed(&press(InputCode::key_tab, true), "tab_focus_prev"));
        assert!(!keys.pressed(&press(InputCode::key_tab, true), "tab_focus_next"));
    }

    #[test]
    fn a_text_file_replaces_one_action_and_leaves_the_rest() {
        let mut keys = KeyBindings::builtin();
        keys.apply_text("# remap\nmenu key_b\n").unwrap();
        assert!(keys.pressed(&press(InputCode::key_b, false), "menu"));
        assert!(!keys.pressed(&press(InputCode::key_grave, false), "menu"));
        assert!(keys.pressed(&press(InputCode::key_f, false), "fps"));
        assert!(keys.apply_text("fps not_a_key\n").is_err());
    }
}
