//! Each focus policy, and the events that fall through when the panel does not take them.

use genos_scene::{Camera, FocusPolicy};
use genos_ui::{
    button_panel, Action, InputPolicy, PanelCheck, PanelDropdown, PanelRow, PanelSlider, Pointer,
    SliderStep, State, PANEL_ID,
};

const VIEW: [f32; 2] = [480.0, 320.0];
const OUTSIDE: [f32; 2] = [460.0, 300.0];

fn checks() -> Vec<PanelRow> {
    vec![PanelRow {
        label: String::new(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: vec![
            PanelCheck {
                id: 1,
                text: "Direct".into(),
                on: false,
            },
            PanelCheck {
                id: 2,
                text: "Shadows".into(),
                on: false,
            },
        ],
    }]
}

fn with_slider() -> Vec<PanelRow> {
    let mut rows = checks();
    rows.push(PanelRow {
        label: "Rate".into(),
        buttons: Vec::new(),
        slider: Some(PanelSlider::range(9, 0.0, 0.0, 10.0, 1.0, "0")),
        dropdown: None,
        checks: Vec::new(),
    });
    rows
}

fn with_list() -> Vec<PanelRow> {
    vec![PanelRow {
        label: "Scene".into(),
        buttons: Vec::new(),
        slider: None,
        dropdown: Some(PanelDropdown::new(
            40,
            0.0,
            vec![
                SliderStep {
                    name: "Hall".into(),
                    value: 0.0,
                },
                SliderStep {
                    name: "Sponza".into(),
                    value: 1.0,
                },
            ],
        )),
        checks: Vec::new(),
    }]
}

fn arm(state: &mut State, wheel: FocusPolicy, keys: FocusPolicy) {
    state.set_input_policy(
        PANEL_ID,
        InputPolicy { wheel, keys },
    );
}

fn draw(state: &mut State, rows: &[PanelRow], pointer: Pointer) -> genos_ui::Frame {
    button_panel(state, "Menu", rows, VIEW, &Camera::opening(), pointer)
}

fn center(frame: &genos_ui::Frame, id: u32) -> (f32, f32) {
    let shown = frame
        .shown
        .iter()
        .find(|item| item.id == id)
        .unwrap_or_else(|| panic!("missing {id}"));
    (
        shown.rect.x + shown.rect.w * 0.5,
        shown.rect.y + shown.rect.h * 0.5,
    )
}

/// Press, then release, so the next call is a new edge. Returns the press frame.
fn click(state: &mut State, rows: &[PanelRow], x: f32, y: f32) -> genos_ui::Frame {
    let pressed = draw(state, rows, Pointer::at(x, y, true));
    let _ = draw(state, rows, Pointer::at(x, y, false));
    pressed
}

fn list_open(frame: &genos_ui::Frame) -> bool {
    frame
        .shown
        .iter()
        .any(|item| (u32::MAX - 8192..u32::MAX - 8000).contains(&item.id))
}

fn place(state: &mut State, rows: &[PanelRow], id: u32) -> (f32, f32) {
    let idle = draw(state, rows, Pointer::default());
    center(&idle, id)
}

#[test]
fn a_panel_without_a_policy_keeps_the_old_path() {
    let rows = checks();
    let mut state = State::default();
    let (x, y) = place(&mut state, &rows, 1);
    let pressed = click(&mut state, &rows, x, y);
    assert_eq!(
        pressed.actions,
        vec![Action::Toggle { id: 1, on: true }]
    );
    assert!(!pressed.consumed.pointer);
    assert!(!pressed.consumed.keys);
    assert!(!pressed.consumed.wheel);
    assert!(!pressed.consumed.blur);
    assert_eq!(state.focused_panel(), None);
    let mut key = Pointer::default();
    key.key_inc = true;
    let moved = draw(&mut state, &rows, key);
    assert!(!moved.consumed.keys);
    assert_eq!(state.focused_control(), Some(1));
}

#[test]
fn click_to_focus_takes_keys_after_a_click_and_arrows_move() {
    let rows = checks();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Hover, FocusPolicy::ClickToFocus);
    let mut early = Pointer::default();
    early.key_inc = true;
    let before = draw(&mut state, &rows, early);
    assert!(!before.consumed.keys, "keys fall through before the click");
    assert_eq!(state.focused_panel(), None);
    assert!(before.actions.is_empty());

    let (x, y) = place(&mut state, &rows, 1);
    let pressed = click(&mut state, &rows, x, y);
    assert!(pressed.consumed.pointer);
    assert!(!pressed.look_capture);
    assert_eq!(state.focused_panel(), Some(PANEL_ID));
    assert_eq!(state.focused_control(), Some(1));

    let mut key = Pointer::default();
    key.key_inc = true;
    let moved = draw(&mut state, &rows, key);
    assert!(moved.consumed.keys);
    assert!(moved.actions.is_empty());
    assert_eq!(state.focused_control(), Some(2));

    let mut enter = Pointer::default();
    enter.key_activate = true;
    let picked = draw(&mut state, &rows, enter);
    assert_eq!(
        picked.actions,
        vec![Action::Toggle { id: 2, on: true }]
    );
    assert!(picked.consumed.keys);
}

#[test]
fn a_focused_slider_steps_instead_of_moving() {
    let rows = with_slider();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Hover, FocusPolicy::ClickToFocus);
    let (x, y) = place(&mut state, &rows, 9);
    let _ = click(&mut state, &rows, x, y);
    assert_eq!(state.focused_control(), Some(9));
    let mut key = Pointer::default();
    key.key_inc = true;
    let stepped = draw(&mut state, &rows, key);
    assert!(
        stepped.actions.iter().any(|action| matches!(
            action,
            Action::Slide { id: 9, value } if *value > 0.0
        )),
        "{:?}",
        stepped.actions
    );
    assert!(stepped.consumed.keys);
    assert_eq!(state.focused_control(), Some(9));
}

#[test]
fn escape_closes_a_list_before_it_blurs_click_to_focus() {
    let rows = with_list();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Hover, FocusPolicy::ClickToFocus);
    let (x, y) = place(&mut state, &rows, 40);
    let opened = click(&mut state, &rows, x, y);
    assert!(list_open(&opened));
    assert_eq!(state.focused_panel(), Some(PANEL_ID));
    let mut escape = Pointer::default();
    escape.key_dismiss = true;
    let closed = draw(&mut state, &rows, escape);
    assert!(closed.consumed.keys);
    assert!(!closed.consumed.blur, "the list took Escape");
    assert_eq!(state.focused_panel(), Some(PANEL_ID));
    assert!(!list_open(&closed));
    let blurred = draw(&mut state, &rows, escape);
    assert!(blurred.consumed.blur);
    assert!(blurred.consumed.keys);
    assert_eq!(state.focused_panel(), None);
}

#[test]
fn grave_releases_click_to_focus_and_reports_blur() {
    let rows = checks();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Hover, FocusPolicy::ClickToFocus);
    let (x, y) = place(&mut state, &rows, 1);
    let _ = click(&mut state, &rows, x, y);
    let mut grave = Pointer::default();
    grave.key_blur = true;
    let frame = draw(&mut state, &rows, grave);
    assert!(frame.consumed.blur);
    assert!(frame.consumed.keys);
    assert_eq!(state.focused_panel(), None);
    assert_eq!(state.focused_control(), None);
}

#[test]
fn a_click_outside_releases_click_to_focus_and_falls_through() {
    let rows = checks();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Hover, FocusPolicy::ClickToFocus);
    let (x, y) = place(&mut state, &rows, 1);
    let _ = click(&mut state, &rows, x, y);
    let outside = draw(&mut state, &rows, Pointer::at(OUTSIDE[0], OUTSIDE[1], true));
    assert_eq!(state.focused_panel(), None);
    assert!(!outside.consumed.pointer);
    assert!(outside.look_capture);
    assert!(outside.actions.is_empty());
}

#[test]
fn hover_delivers_keys_only_while_the_pointer_is_over_the_panel() {
    let rows = checks();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Hover, FocusPolicy::Hover);
    let (x, y) = place(&mut state, &rows, 1);
    let mut over = Pointer::at(x, y, false);
    over.key_inc = true;
    let on = draw(&mut state, &rows, over);
    assert!(on.consumed.keys);
    assert_eq!(state.focused_control(), Some(1));
    let mut away = Pointer::at(OUTSIDE[0], OUTSIDE[1], false);
    away.key_inc = true;
    let off = draw(&mut state, &rows, away);
    assert!(!off.consumed.keys, "keys fall through off the panel");
}

#[test]
fn passive_never_takes_the_pointer_or_the_keys() {
    let rows = checks();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Passive, FocusPolicy::Passive);
    let (x, y) = place(&mut state, &rows, 1);
    let pressed = draw(&mut state, &rows, Pointer::at(x, y, true));
    assert!(pressed.actions.is_empty());
    assert!(pressed.look_capture);
    assert!(!pressed.consumed.pointer);
    let mut key = Pointer::at(x, y, false);
    key.key_inc = true;
    let keyed = draw(&mut state, &rows, key);
    assert!(!keyed.consumed.keys);
    assert!(keyed.actions.is_empty());
}

#[test]
fn modal_owns_a_click_outside_and_an_unused_key() {
    let rows = checks();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Modal, FocusPolicy::Modal);
    let outside = draw(&mut state, &rows, Pointer::at(OUTSIDE[0], OUTSIDE[1], true));
    assert!(outside.consumed.pointer);
    assert!(!outside.look_capture);
    assert!(outside.actions.is_empty());
    let mut grave = Pointer::default();
    grave.key_blur = true;
    let keyed = draw(&mut state, &rows, grave);
    assert!(keyed.consumed.keys);
    assert!(keyed.consumed.blur);
}

#[test]
fn set_policy_and_set_focus_change_the_next_frame() {
    let rows = checks();
    let mut state = State::default();
    arm(&mut state, FocusPolicy::Hover, FocusPolicy::Hover);
    assert_eq!(
        state.input_policy(PANEL_ID),
        Some(InputPolicy {
            wheel: FocusPolicy::Hover,
            keys: FocusPolicy::Hover,
        })
    );
    let (x, y) = place(&mut state, &rows, 1);
    let mut key = Pointer::at(x, y, false);
    key.key_inc = true;
    assert!(draw(&mut state, &rows, key).consumed.keys);

    arm(&mut state, FocusPolicy::Passive, FocusPolicy::Passive);
    let after = draw(&mut state, &rows, key);
    assert!(!after.consumed.keys);
    let pressed = draw(&mut state, &rows, Pointer::at(x, y, true));
    assert!(pressed.actions.is_empty());
    assert!(!pressed.consumed.pointer);

    arm(&mut state, FocusPolicy::Hover, FocusPolicy::ClickToFocus);
    state.set_focused_panel(Some(PANEL_ID));
    assert_eq!(state.focused_panel(), Some(PANEL_ID));
    let mut bare = Pointer::default();
    bare.key_inc = true;
    let armed = draw(&mut state, &rows, bare);
    assert!(armed.consumed.keys);
    state.set_focused_panel(None);
    let dropped = draw(&mut state, &rows, bare);
    assert!(!dropped.consumed.keys);
}
