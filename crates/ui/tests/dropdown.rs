//! Dropdown: a press opens a list over the panel, a choice reports its value,
//! and a press outside or Escape closes it.

use genos_scene::Camera;
use genos_ui::{button_panel, Action, PanelDropdown, PanelRow, Pointer, SliderStep, State};

const VIEW: [f32; 2] = [400.0, 300.0];
const ID: u32 = 40;

fn choices(names: &[&str]) -> Vec<SliderStep> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| SliderStep {
            name: (*name).into(),
            value: index as f64,
        })
        .collect()
}

fn rows(names: &[&str], value: f64) -> Vec<PanelRow> {
    vec![PanelRow {
        label: "Scene".into(),
        buttons: Vec::new(),
        slider: None,
        dropdown: Some(PanelDropdown::new(ID, value, choices(names))),
        checks: Vec::new(),
    }]
}

fn draw(state: &mut State, names: &[&str], value: f64, pointer: Pointer) -> genos_ui::Frame {
    button_panel(
        state,
        "Menu",
        &rows(names, value),
        VIEW,
        &Camera::opening(),
        pointer,
    )
}

fn box_center(state: &mut State, names: &[&str]) -> (f32, f32) {
    let frame = draw(state, names, 0.0, Pointer::default());
    let shown = frame.shown.iter().find(|item| item.id == ID).expect("box");
    (
        shown.rect.x + shown.rect.w * 0.5,
        shown.rect.y + shown.rect.h * 0.5,
    )
}

fn text_count(frame: &genos_ui::Frame, text: &str) -> usize {
    frame
        .shown
        .iter()
        .filter(|item| item.text.as_deref() == Some(text))
        .count()
}

fn select(frame: &genos_ui::Frame) -> Option<f64> {
    frame.actions.iter().find_map(|action| match action {
        Action::Select { id: ID, value } => Some(*value),
        _ => None,
    })
}

#[test]
fn a_press_opens_the_list_and_a_choice_selects_then_closes() {
    let names = ["room", "hell", "stress"];
    let mut state = State::default();
    let (x, y) = box_center(&mut state, &names);
    let closed = draw(&mut state, &names, 0.0, Pointer::at(x, y, false));
    assert_eq!(text_count(&closed, "room"), 1);
    assert_eq!(text_count(&closed, "hell"), 0);
    assert!(select(&closed).is_none());

    let opened = draw(&mut state, &names, 0.0, Pointer::at(x, y, true));
    assert_eq!(text_count(&opened, "hell"), 1);
    assert!(select(&opened).is_none());
    let hell = opened
        .shown
        .iter()
        .find(|item| item.id != ID && item.text.as_deref() == Some("hell"))
        .expect("hell row");
    let hx = hell.rect.x + hell.rect.w * 0.5;
    let hy = hell.rect.y + hell.rect.h * 0.5;

    let _ = draw(&mut state, &names, 0.0, Pointer::at(hx, hy, false));
    let picked = draw(&mut state, &names, 0.0, Pointer::at(hx, hy, true));
    assert_eq!(select(&picked), Some(1.0));

    let after = draw(&mut state, &names, 1.0, Pointer::at(hx, hy, false));
    assert_eq!(text_count(&after, "hell"), 1);
    assert_eq!(text_count(&after, "room"), 0);
    assert!(select(&after).is_none());
}

#[test]
fn a_press_outside_closes_without_a_choice() {
    let names = ["room", "hell"];
    let mut state = State::default();
    let (x, y) = box_center(&mut state, &names);
    let _ = draw(&mut state, &names, 0.0, Pointer::at(x, y, true));
    let _ = draw(&mut state, &names, 0.0, Pointer::at(x, y, false));
    let dismissed = draw(&mut state, &names, 0.0, Pointer::at(1.0, 1.0, true));
    assert!(select(&dismissed).is_none());
    let closed = draw(&mut state, &names, 0.0, Pointer::at(1.0, 1.0, false));
    assert_eq!(text_count(&closed, "hell"), 0);
}

#[test]
fn arrow_keys_move_the_highlight_and_enter_picks_it() {
    let names = ["room", "hell", "stress"];
    let mut state = State::default();
    let (x, y) = box_center(&mut state, &names);
    let _ = draw(&mut state, &names, 0.0, Pointer::at(x, y, true));
    let mut pointer = Pointer::at(x, y, false);
    pointer.key_dec = true;
    let moved = draw(&mut state, &names, 0.0, pointer);
    assert!(select(&moved).is_none());
    let hell = moved
        .shown
        .iter()
        .find(|item| item.id != ID && item.text.as_deref() == Some("hell"))
        .expect("hell");
    let room = moved
        .shown
        .iter()
        .find(|item| item.id != ID && item.text.as_deref() == Some("room"))
        .expect("room");
    assert_ne!(hell.look.background, room.look.background);

    let mut pointer = Pointer::at(0.0, 0.0, false);
    pointer.key_activate = true;
    let picked = draw(&mut state, &names, 0.0, pointer);
    assert_eq!(select(&picked), Some(1.0));
    let closed = draw(&mut state, &names, 1.0, Pointer::default());
    assert_eq!(text_count(&closed, "room"), 0);
}

#[test]
fn the_pointer_highlights_a_row_and_the_wheel_scrolls_the_window() {
    let names: Vec<String> = (0..8).map(|i| format!("c{i}")).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut state = State::default();
    let (x, y) = box_center(&mut state, &name_refs);
    let opened = draw(&mut state, &name_refs, 0.0, Pointer::at(x, y, true));
    assert_eq!(text_count(&opened, "c0"), 2);
    assert_eq!(text_count(&opened, "c6"), 0);
    let c1 = opened
        .shown
        .iter()
        .find(|item| item.id != ID && item.text.as_deref() == Some("c1"))
        .expect("c1");
    let hover = Pointer::at(c1.rect.x + 4.0, c1.rect.y + c1.rect.h * 0.5, false);
    let hovered = draw(&mut state, &name_refs, 0.0, hover);
    let hot = hovered
        .shown
        .iter()
        .find(|item| item.id != ID && item.text.as_deref() == Some("c1"))
        .expect("hot");
    let cold = hovered
        .shown
        .iter()
        .find(|item| item.id != ID && item.text.as_deref() == Some("c2"))
        .expect("cold");
    assert_ne!(hot.look.background, cold.look.background);

    let mut wheel = Pointer::at(c1.rect.x + 4.0, c1.rect.y + 4.0, false);
    wheel.wheel_y = -1.0;
    let scrolled = draw(&mut state, &name_refs, 0.0, wheel);
    assert_eq!(text_count(&scrolled, "c0"), 1);
    assert_eq!(text_count(&scrolled, "c6"), 1);
}

#[test]
fn escape_closes_the_list() {
    let names = ["room", "hell"];
    let mut state = State::default();
    let (x, y) = box_center(&mut state, &names);
    let _ = draw(&mut state, &names, 0.0, Pointer::at(x, y, true));
    let mut pointer = Pointer::at(x, y, false);
    pointer.key_dismiss = true;
    let closed = draw(&mut state, &names, 0.0, pointer);
    assert!(select(&closed).is_none());
    assert_eq!(text_count(&closed, "hell"), 0);
}
