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
fn toggles_share_column_width_and_a_long_label_wraps() {
    let sections = vec![        PanelSection::new(
        10,
        "Pipeline toggles",
        false,
        vec![PanelRow {
            label: String::new(),
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: vec![
                PanelCheck {
                    id: 1,
                    text: "Direct".into(),
                    on: true,
                },
                PanelCheck {
                    id: 2,
                    text: "Adaptive denoise".into(),
                    on: false,
                },
                PanelCheck {
                    id: 3,
                    text: "GI".into(),
                    on: true,
                },
            ],
        }],
    )];
    let frame = draw(&mut State::default(), &sections, Pointer::default());
    let rect = |id: u32| {
        frame
            .shown
            .iter()
            .find(|shown| shown.id == id)
            .unwrap()
            .rect
    };
    let direct = rect(1);
    let adaptive = rect(2);
    let gi = rect(3);
    assert!((direct.w - adaptive.w).abs() < 0.5);
    assert!((adaptive.w - gi.w).abs() < 0.5);
    assert!(adaptive.x > direct.x + direct.w - 1.0);
    assert!(gi.x > adaptive.x + adaptive.w - 1.0);
    assert!(adaptive.h > direct.h + 8.0);
    assert!(adaptive.w < 180.0);
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
