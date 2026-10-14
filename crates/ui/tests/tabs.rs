//! Tabs and folds are separate. A tab can hold a fold, and a fold can sit outside the tabs.

use genos_scene::Camera;
use genos_ui::{
    button_panel_blocks, Action, PanelBlock, PanelCheck, PanelRow, PanelSection, PanelTab,
    PanelTabs, Pointer, State,
};

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

fn draw(state: &mut State, blocks: &[PanelBlock], pointer: Pointer) -> genos_ui::Frame {
    button_panel_blocks(
        state,
        "stress",
        blocks,
        [1024.0, 832.0],
        &Camera::opening(),
        pointer,
    )
}

fn blocks() -> Vec<PanelBlock> {
    vec![
        PanelBlock::Section(PanelSection::new(
            10,
            "Scene/View",
            false,
            vec![mark(1, "Scene")],
        )),
        PanelBlock::Tabs(PanelTabs {
            id: 100,
            tabs: vec![
                PanelTab {
                    id: 101,
                    title: "Direct".into(),
                    blocks: vec![PanelBlock::Section(PanelSection::new(
                        11,
                        "Lamps",
                        false,
                        vec![mark(2, "Samples")],
                    ))],
                },
                PanelTab {
                    id: 102,
                    title: "GI".into(),
                    blocks: vec![PanelBlock::Section(
                        PanelSection::new(12, "Cache", true, vec![mark(3, "Probes")]).with_blocks(
                            vec![PanelBlock::Tabs(PanelTabs {
                                id: 200,
                                tabs: vec![PanelTab {
                                    id: 201,
                                    title: "Rays".into(),
                                    blocks: vec![PanelBlock::Section(PanelSection::new(
                                        13,
                                        "Count",
                                        false,
                                        vec![mark(4, "Rays")],
                                    ))],
                                }],
                            })],
                        ),
                    )],
                },
            ],
        }),
    ]
}

fn shows(frame: &genos_ui::Frame, id: u32) -> bool {
    frame.shown.iter().any(|shown| shown.id == id)
}

#[test]
fn a_tab_holds_a_fold_and_a_fold_can_sit_outside() {
    let blocks = blocks();
    let mut state = State::default();
    let first = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&first, 1), "outside fold stays up");
    assert!(shows(&first, 2), "the first tab is open");
    assert!(!shows(&first, 3), "the other tab is hidden");
    assert_eq!(state.selected_tab(100), Some(101));

    let gi = first.shown.iter().find(|shown| shown.id == 102).unwrap();
    let press = draw(
        &mut state,
        &blocks,
        Pointer::at(gi.rect.x + 4.0, gi.rect.y + 4.0, true),
    );
    assert!(press.actions.contains(&Action::Press(102)));
    let _ = draw(&mut state, &blocks, Pointer::default());
    let open = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&open, 1));
    assert!(!shows(&open, 2));
    assert!(shows(&open, 12), "the folded header is visible");
    assert!(!shows(&open, 3));
    assert!(
        !shows(&open, 4),
        "a folded section hides the tabs inside it"
    );

    let header = open.shown.iter().find(|shown| shown.id == 12).unwrap();
    let _ = draw(
        &mut state,
        &blocks,
        Pointer::at(header.rect.x + 4.0, header.rect.y + 4.0, true),
    );
    let unfolded = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&unfolded, 3));
    assert!(shows(&unfolded, 4));
    assert!(shows(&unfolded, 1));
}
