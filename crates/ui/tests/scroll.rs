//! A page taller than the window scrolls. The tab bar stays where it was.

use genos_scene::Camera;
use genos_ui::{
    button_panel_blocks, PanelBlock, PanelCheck, PanelRow, PanelSection, PanelTab, PanelTabs,
    Pointer, State,
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

fn blocks() -> Vec<PanelBlock> {
    let rows: Vec<PanelRow> = (1..24).map(|id| mark(id, "Row")).collect();
    vec![PanelBlock::Tabs(PanelTabs {
        id: 100,
        tabs: vec![
            PanelTab {
                id: 101,
                title: "Direct".into(),
                blocks: vec![PanelBlock::Section(PanelSection::new(
                    11, "Lamps", false, rows,
                ))],
            },
            PanelTab {
                id: 102,
                title: "GI".into(),
                blocks: vec![PanelBlock::Rows(vec![mark(80, "Probes")])],
            },
        ],
    })]
}

fn draw(state: &mut State, pointer: Pointer) -> genos_ui::Frame {
    button_panel_blocks(
        state,
        "stress",
        &blocks(),
        [800.0, 220.0],
        &Camera::opening(),
        pointer,
    )
}

fn y_of(frame: &genos_ui::Frame, id: u32) -> f32 {
    frame
        .shown
        .iter()
        .find(|shown| shown.id == id)
        .map(|shown| shown.rect.y)
        .unwrap_or(-1.0)
}

#[test]
fn the_page_scrolls_and_the_tab_bar_stays() {
    let mut state = State::default();
    let top = draw(&mut state, Pointer::default());
    assert!(
        top.shown.iter().any(|shown| shown.id == 1),
        "the first row is on screen"
    );
    assert!(
        top.shown.iter().all(|shown| shown.id != 23),
        "the last row is below the window"
    );
    let tab_y = y_of(&top, 101);
    assert!(tab_y >= 0.0);
    let panel_bottom = top
        .shown
        .iter()
        .find(|shown| shown.id == u32::MAX - 4095)
        .unwrap()
        .rect;
    assert!(
        panel_bottom.y + panel_bottom.h <= 220.0,
        "the panel stays inside the window"
    );

    let mut wheel = Pointer::default();
    wheel.x = panel_bottom.x + 8.0;
    wheel.y = panel_bottom.y + 8.0;
    wheel.wheel_y = -6.0;
    let scrolled = draw(&mut state, wheel);
    assert_eq!(y_of(&scrolled, 101), tab_y, "the tab bar moved");
    assert!(
        scrolled.shown.iter().all(|shown| shown.id != 1),
        "the first row stayed on screen"
    );
    assert!(
        scrolled.shown.iter().any(|shown| shown.id > 8),
        "a lower row did not come into view"
    );
    let panel = scrolled
        .shown
        .iter()
        .find(|shown| shown.id == u32::MAX - 4095)
        .unwrap();
    assert!(panel.rect.y + panel.rect.h <= 220.0);
}
