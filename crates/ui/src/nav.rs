//! Keyboard and gamepad focus for [`crate::frame`].
//!
//! `key_a` stays on the camera move action. Shift-Tab is the previous control.
//! [`genos_input::ActionMap`] has no chord, so Tab plus a held shift is read here.
//! The map is built once. A frame does not allocate a new one.

use std::sync::OnceLock;

use genos_input::{ActionMap, BoundInput, InputCode, InputSystem};

use crate::controls::radio_index;
use crate::kind::{Event, Kind};
use crate::layers;
use crate::layout::Item;
use crate::panel::State;
use crate::slider;

/// Navigation actions. `ui_next` is Tab. Shift-Tab is handled in [`apply`].
///
/// The map is process-wide and built on the first call.
#[must_use]
pub fn ui_actions() -> &'static ActionMap {
    static MAP: OnceLock<ActionMap> = OnceLock::new();
    MAP.get_or_init(build_ui_actions)
}

fn build_ui_actions() -> ActionMap {
    let mut map = ActionMap::new();
    map.set(
        "ui_left",
        vec![
            bound(InputCode::key_left),
            bound(InputCode::gamepad_dpad_left),
            bound(InputCode::gamepad_left_stick_left),
        ],
    );
    map.set(
        "ui_right",
        vec![
            bound(InputCode::key_right),
            bound(InputCode::gamepad_dpad_right),
            bound(InputCode::gamepad_left_stick_right),
        ],
    );
    map.set(
        "ui_up",
        vec![
            bound(InputCode::key_up),
            bound(InputCode::gamepad_dpad_up),
            bound(InputCode::gamepad_left_stick_up),
        ],
    );
    map.set(
        "ui_down",
        vec![
            bound(InputCode::key_down),
            bound(InputCode::gamepad_dpad_down),
            bound(InputCode::gamepad_left_stick_down),
        ],
    );
    map.set(
        "ui_activate",
        vec![
            bound(InputCode::key_enter),
            bound(InputCode::key_space),
            bound(InputCode::gamepad_face_south),
        ],
    );
    map.set(
        "ui_back",
        vec![
            bound(InputCode::key_escape),
            bound(InputCode::gamepad_face_east),
        ],
    );
    map.set("ui_next", vec![bound(InputCode::key_tab)]);
    map
}

/// Move focus or fire the focused control. Returns whether a navigation code was used.
pub(crate) fn apply(
    input: &InputSystem,
    state: &mut State,
    items: &[Item],
    events: &mut Vec<Event>,
) -> bool {
    let map = ui_actions();
    let shift = input.keyboard.down(InputCode::key_shift_left)
        || input.keyboard.down(InputCode::key_shift_right);
    let tab = map.pressed(input, "ui_next");
    let forward = map.pressed(input, "ui_down") || (tab && !shift);
    let backward = map.pressed(input, "ui_up") || (tab && shift);
    let activate = map.pressed(input, "ui_activate");
    let left = map.pressed(input, "ui_left");
    let right = map.pressed(input, "ui_right");
    if forward {
        move_focus(state, items, 1);
        return true;
    }
    if backward {
        move_focus(state, items, -1);
        return true;
    }
    if let Some(item) = focused(items, state.focus) {
        if (left || right) && step_axis(item, items, state, events, if right { 1 } else { -1 }) {
            return true;
        }
        if activate && activate_item(item, items, events) {
            return true;
        }
    }
    false
}

pub(crate) fn back(input: &InputSystem) -> bool {
    ui_actions().pressed(input, "ui_back")
}

/// True when this frame pressed a UI navigation code.
pub(crate) fn took_key(input: &InputSystem) -> bool {
    let map = ui_actions();
    map.pressed(input, "ui_left")
        || map.pressed(input, "ui_right")
        || map.pressed(input, "ui_up")
        || map.pressed(input, "ui_down")
        || map.pressed(input, "ui_activate")
        || map.pressed(input, "ui_back")
        || map.pressed(input, "ui_next")
}

fn wrap_index(index: usize, len: usize, delta: i32) -> usize {
    let Ok(len_i) = i32::try_from(len) else {
        return index;
    };
    let Ok(index_i) = i32::try_from(index) else {
        return 0;
    };
    let step = (index_i + delta).rem_euclid(len_i.max(1));
    usize::try_from(step).unwrap_or(0)
}

fn bound(code: InputCode) -> BoundInput {
    BoundInput {
        code,
        activation_threshold: None,
    }
}

fn move_focus(state: &mut State, items: &[Item], delta: i32) {
    let ids = focus_ids(items);
    if ids.is_empty() {
        return;
    }
    let next = match state.focus.and_then(|id| ids.iter().position(|found| *found == id)) {
        Some(index) => ids[wrap_index(index, ids.len(), delta)],
        None => {
            if delta < 0 {
                ids[ids.len() - 1]
            } else {
                ids[0]
            }
        }
    };
    state.focus = Some(next);
}

fn focus_ids(items: &[Item]) -> Vec<u32> {
    items
        .iter()
        .filter(|item| layers::focusable_kind(&item.kind))
        .map(|item| item.id)
        .collect()
}

fn focused(items: &[Item], id: Option<u32>) -> Option<&Item> {
    let id = id?;
    items.iter().find(|item| item.id == id)
}

fn step_axis(item: &Item, items: &[Item], state: &mut State, events: &mut Vec<Event>, delta: i32) -> bool {
    match &item.kind {
        Kind::Slider { value, min, max, step, .. } => {
            let next = slider::step_by(*min, *max, *step, &[], *value, delta);
            if (next - *value).abs() > 1.0e-4 {
                events.push(Event::Change {
                    id: item.id,
                    value: next,
                });
            }
            true
        }
        Kind::Radio { group, .. } => {
            let ids: Vec<u32> = items
                .iter()
                .filter_map(|other| match &other.kind {
                    Kind::Radio { group: found, .. } if found == group => Some(other.id),
                    Kind::Box
                    | Kind::Label
                    | Kind::Button { .. }
                    | Kind::Toggle { .. }
                    | Kind::Checkbox { .. }
                    | Kind::Radio { .. }
                    | Kind::Slider { .. }
                    | Kind::Progress { .. } => None,
                })
                .collect();
            if ids.is_empty() {
                return false;
            }
            let pos = ids.iter().position(|id| *id == item.id).unwrap_or(0);
            let next = ids[wrap_index(pos, ids.len(), delta)];
            state.focus = Some(next);
            events.push(Event::Select {
                id: next,
                index: radio_index(items, *group, next),
            });
            true
        }
        Kind::Box
        | Kind::Label
        | Kind::Button { .. }
        | Kind::Toggle { .. }
        | Kind::Checkbox { .. }
        | Kind::Progress { .. } => false,
    }
}

fn activate_item(item: &Item, items: &[Item], events: &mut Vec<Event>) -> bool {
    match &item.kind {
        Kind::Button { enabled: true } => {
            events.push(Event::Press { id: item.id });
            true
        }
        Kind::Toggle { on } => {
            events.push(Event::Toggle {
                id: item.id,
                on: !on,
            });
            true
        }
        Kind::Checkbox { checked } => {
            events.push(Event::Toggle {
                id: item.id,
                on: !checked,
            });
            true
        }
        Kind::Radio { group, .. } => {
            events.push(Event::Select {
                id: item.id,
                index: radio_index(items, *group, item.id),
            });
            true
        }
        Kind::Button { enabled: false }
        | Kind::Box
        | Kind::Label
        | Kind::Slider { .. }
        | Kind::Progress { .. } => false,
    }
}
