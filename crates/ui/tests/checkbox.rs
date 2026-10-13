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
