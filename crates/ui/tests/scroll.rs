//! A page taller than the window scrolls. The tab bar stays where it was.
//! The wheel follows the panel's wheel policy.

use genos_scene::{Camera, FocusPolicy};
use genos_ui::{
    button_panel, button_panel_blocks, Action, InputPolicy, PanelBlock, PanelCheck, PanelRow,
    PanelSection, PanelSlider, PanelTab, PanelTabs, Pointer, State, PANEL_ID,
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
    assert!(scrolled.consumed.wheel);
    let panel = scrolled
        .shown
        .iter()
        .find(|shown| shown.id == u32::MAX - 4095)
        .unwrap();
    assert!(panel.rect.y + panel.rect.h <= 220.0);
}

fn policy(wheel: FocusPolicy) -> InputPolicy {
    InputPolicy {
        wheel,
        keys: FocusPolicy::Passive,
    }
}

fn wheel_at(x: f32, y: f32) -> Pointer {
    let mut pointer = Pointer::at(x, y, false);
    pointer.wheel_y = -6.0;
    pointer
}

fn row_y(state: &mut State, policy: Option<InputPolicy>, pointer: Pointer) -> (f32, bool) {
    if let Some(policy) = policy {
        state.set_input_policy(PANEL_ID, policy);
    }
    let frame = draw(state, pointer);
    (y_of(&frame, 1), frame.consumed.wheel)
}

#[test]
fn the_wheel_policy_decides_whether_the_page_moves() {
    let mut state = State::default();
    let top = draw(&mut state, Pointer::default());
    let start = y_of(&top, 1);
    let panel = top
        .shown
        .iter()
        .find(|shown| shown.id == PANEL_ID)
        .unwrap()
        .rect;
    let over = wheel_at(panel.x + 8.0, panel.y + 8.0);
    let outside = wheel_at(panel.x + panel.w + 40.0, panel.y + 8.0);

    state.set_input_policy(PANEL_ID, policy(FocusPolicy::Passive));
    let (passive_y, passive_wheel) = row_y(&mut state, None, over);
    assert_eq!(passive_y, start);
    assert!(!passive_wheel, "a passive wheel falls through");

    state.set_input_policy(PANEL_ID, policy(FocusPolicy::Hover));
    let (away_y, away_wheel) = row_y(&mut state, None, outside);
    assert_eq!(away_y, start);
    assert!(!away_wheel, "hover off the panel falls through");
    let (hover_y, hover_wheel) = row_y(&mut state, None, over);
    assert_ne!(hover_y, start);
    assert!(hover_wheel);

    let mut focused = State::default();
    focused.set_input_policy(PANEL_ID, policy(FocusPolicy::ClickToFocus));
    let (before, before_wheel) = row_y(&mut focused, None, over);
    assert_eq!(before, start);
    assert!(!before_wheel, "click-to-focus ignores the wheel until focused");
    focused.set_focused_panel(Some(PANEL_ID));
    let (armed_y, armed_wheel) = row_y(&mut focused, None, outside);
    assert_ne!(armed_y, start);
    assert!(armed_wheel);

    let mut modal = State::default();
    modal.set_input_policy(PANEL_ID, policy(FocusPolicy::Modal));
    let (modal_y, modal_wheel) = row_y(&mut modal, None, outside);
    assert_ne!(modal_y, start);
    assert!(modal_wheel);
}

#[test]
fn a_slider_wheel_follows_the_same_policy() {
    let rows = vec![PanelRow {
        label: "Rate".into(),
        buttons: Vec::new(),
        slider: Some(PanelSlider::range(9, 0.0, 0.0, 10.0, 1.0, "0")),
        dropdown: None,
        checks: Vec::new(),
    }];
    let mut state = State::default();
    state.set_input_policy(PANEL_ID, policy(FocusPolicy::Hover));
    let laid = button_panel(
        &mut state,
        "Menu",
        &rows,
        [480.0, 320.0],
        &Camera::opening(),
        Pointer::default(),
    );
    let bar = laid.shown.iter().find(|shown| shown.id == 9).unwrap().rect;
    let mut wheel = Pointer::at(bar.x + bar.w * 0.5, bar.y + bar.h * 0.5, false);
    wheel.wheel_y = 1.0;
    let stepped = button_panel(
        &mut state,
        "Menu",
        &rows,
        [480.0, 320.0],
        &Camera::opening(),
        wheel,
    );
    assert!(stepped.actions.iter().any(|action| matches!(
        action,
        Action::Slide { id: 9, value } if *value > 0.0
    )));
    assert!(stepped.consumed.wheel);

    state.set_input_policy(PANEL_ID, policy(FocusPolicy::Passive));
    let held = button_panel(
        &mut state,
        "Menu",
        &rows,
        [480.0, 320.0],
        &Camera::opening(),
        wheel,
    );
    assert!(held.actions.is_empty());
    assert!(!held.consumed.wheel);
}
