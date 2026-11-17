#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::float_cmp, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! A checkbox reports the next on state and paints a mark only while it is on.

use genos_scene::Camera;
use genos_ui::{button_panel, Action, PanelCheck, PanelRow, Pointer, State};

const VIEW: [f32; 2] = [400.0, 200.0];

fn rows(direct: bool) -> Vec<PanelRow> {
    vec![PanelRow {
        label: String::new(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: vec![
            PanelCheck {
                id: 1,
                text: "Direct".into(),
                on: direct,
            },
            PanelCheck {
                id: 2,
                text: "Shadows".into(),
                on: false,
            },
        ],
    }]
}

fn draw(state: &mut State, direct: bool, pointer: Pointer) -> genos_ui::Frame {
    button_panel(
        state,
        "Menu",
        &rows(direct),
        VIEW,
        &Camera::opening(),
        pointer,
    )
}

fn mark_inside(frame: &genos_ui::Frame, id: u32) -> bool {
    let shown = frame
        .shown
        .iter()
        .find(|item| item.id == id)
        .expect("check");
    frame.paints.iter().any(|paint| {
        paint.w > 4.0
            && paint.h > 4.0
            && paint.w < 12.0
            && paint.h < 12.0
            && paint.x >= shown.rect.x
            && paint.y >= shown.rect.y
            && paint.x + paint.w <= shown.rect.x + 22.0
    })
}

#[test]
fn a_press_reports_the_next_state_and_the_mark_follows_on() {
    let mut state = State::default();
    let idle = draw(&mut state, false, Pointer::default());
    assert!(!mark_inside(&idle, 1));
    let shown = idle.shown.iter().find(|item| item.id == 1).expect("direct");
    let x = shown.rect.x + shown.rect.w * 0.5;
    let y = shown.rect.y + shown.rect.h * 0.5;
    let pressed = draw(&mut state, false, Pointer::at(x, y, true));
    assert_eq!(pressed.actions, vec![Action::Toggle { id: 1, on: true }]);
    let on = draw(&mut state, true, Pointer::at(x, y, false));
    assert!(mark_inside(&on, 1));
    assert!(!mark_inside(&on, 2));
    let again = draw(&mut state, true, Pointer::at(x, y, true));
    assert_eq!(again.actions, vec![Action::Toggle { id: 1, on: false }]);
}

#[test]
fn a_kind_checkbox_paints_an_inset_mark_and_emits_toggle() {
    use genos_ui::{frame, theme_dark, Event, Kind, Node, Sizing, Space};
    let theme = theme_dark();
    let mut checked = Node::new(11);
    checked.kind = Kind::Checkbox { checked: true };
    checked.width = Sizing::fixed(16.0);
    checked.height = Sizing::fixed(16.0);
    let mut state = State::default();
    state.theme = theme;
    let on = frame(
        &checked,
        &mut state,
        Space::Screen,
        [40.0, 40.0],
        None,
        Pointer::at(4.0, 4.0, true),
        0.0,
        None,
    );
    assert_eq!(on.events, vec![Event::Toggle { id: 11, on: false }]);
    assert!(on.paints.iter().any(|paint| {
        paint.color == theme.accent && paint.x >= 4.0 && paint.w <= 8.0
    }));
    let mut clear = Node::new(11);
    clear.kind = Kind::Checkbox { checked: false };
    clear.width = Sizing::fixed(16.0);
    clear.height = Sizing::fixed(16.0);
    let mut state = State::default();
    state.theme = theme;
    let off = frame(
        &clear,
        &mut state,
        Space::Screen,
        [40.0, 40.0],
        None,
        Pointer::at(40.0, 40.0, false),
        0.0,
        None,
    );
    assert!(off.paints.iter().all(|paint| paint.color != theme.accent));
}
