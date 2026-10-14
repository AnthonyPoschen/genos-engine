//! A section title folds the rows under it, and the next frame keeps that fold.

use genos_scene::Camera;
use genos_ui::{button_panel_sections, Action, PanelCheck, PanelRow, PanelSection, Pointer, State};

fn mark(id: u32, text: &str) -> PanelRow {
    PanelRow {
        label: String::new(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: vec![PanelCheck {
            id,
            text: text.into(),
            on: false,
        }],
    }
}

fn draw(state: &mut State, sections: &[PanelSection], pointer: Pointer) -> genos_ui::Frame {
    button_panel_sections(
        state,
        "stress",
        sections,
        [800.0, 600.0],
        &Camera::opening(),
        pointer,
    )
}

#[test]
fn a_section_stays_folded_on_the_next_frame() {
    let sections = vec![
        PanelSection::new(10, "Scene/View", false, vec![mark(1, "Direct")]),
        PanelSection::new(11, "Direct/Lamps", true, vec![mark(2, "Samples")]),
    ];
    let mut state = State::default();
    let idle = draw(&mut state, &sections, Pointer::default());
    assert!(idle.shown.iter().any(|shown| shown.id == 1));
    assert!(idle.shown.iter().all(|shown| shown.id != 2));
    assert!(idle
        .shown
        .iter()
        .any(|shown| shown.text.as_deref() == Some("+ Direct/Lamps")));
    let header = idle.shown.iter().find(|shown| shown.id == 11).unwrap();
    let press = draw(
        &mut state,
        &sections,
        Pointer::at(header.rect.x + 8.0, header.rect.y + 4.0, true),
    );
    assert_eq!(press.actions, vec![Action::Press(11)]);
    assert!(state.section_collapsed(11) == false);
    let open = draw(&mut state, &sections, Pointer::default());
    assert!(open.shown.iter().any(|shown| shown.id == 2));
    assert!(open
        .shown
        .iter()
        .any(|shown| shown.text.as_deref() == Some("- Direct/Lamps")));
    let header = open.shown.iter().find(|shown| shown.id == 11).unwrap();
    let _ = draw(
        &mut state,
        &sections,
        Pointer::at(header.rect.x + 8.0, header.rect.y + 4.0, true),
    );
    let folded = draw(&mut state, &sections, Pointer::default());
    assert!(folded.shown.iter().all(|shown| shown.id != 2));
    assert!(state.section_collapsed(11));
}

#[test]
fn a_remembered_fold_wins_over_the_starting_flag() {
    let sections = vec![PanelSection::new(
        10,
        "Scene/View",
        false,
        vec![mark(1, "Direct")],
    )];
    let mut state = State::default();
    state.fold_section(10, true);
    let frame = draw(&mut state, &sections, Pointer::default());
    assert!(frame.shown.iter().all(|shown| shown.id != 1));
    assert!(state.section_collapsed(10));
}
