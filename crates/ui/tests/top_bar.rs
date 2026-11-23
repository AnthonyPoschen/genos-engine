#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A bar across the top opens a page; the page is a drawer on the right edge
//! or a panel that drags.

use genos_scene::Camera;
use genos_ui::{
    button_panel_blocks, top_bar, Action, PanelBlock, PanelButton, PanelCheck, PanelRow, Pointer,
    State, MENU_DOCK, PANEL_ID, TOP_BAR_HEIGHT,
};

const VIEW: [f32; 2] = [1024.0, 832.0];

fn buttons(open: Option<u32>) -> Vec<PanelButton> {
    [(1, "Scene"), (2, "Lighting"), (3, "Mesh")]
        .into_iter()
        .map(|(id, text)| PanelButton { id, text: text.into(), selected: open == Some(id) })
        .collect()
}

fn page() -> Vec<PanelBlock> {
    vec![PanelBlock::Rows(vec![PanelRow {
        label: String::new(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: vec![PanelCheck { id: 40, text: "Grid".into(), on: false }],
    }])]
}

fn at(x: f32, y: f32, down: bool) -> Pointer {
    Pointer { x, y, down, ..Pointer::default() }
}

fn rect(frame: &genos_ui::Frame, id: u32) -> genos_ui::Rect {
    frame.shown.iter().find(|shown| shown.id == id).expect("shown").rect
}

#[test]
fn the_bar_spans_the_top_and_a_press_names_its_button() {
    let mut state = State::default();
    let idle = top_bar(&mut state, &buttons(Some(2)), VIEW, Pointer::default());
    let scene = rect(&idle, 1);
    let lighting = rect(&idle, 2);
    let mesh = rect(&idle, 3);
    assert!(scene.x < lighting.x && lighting.x < mesh.x, "left to right");
    assert!(mesh.y + mesh.h <= TOP_BAR_HEIGHT, "inside the bar");
    let widest = idle.shown.iter().map(|shown| shown.rect.w).fold(0.0, f32::max);
    assert!((widest - VIEW[0]).abs() < 0.5, "the bar is as wide as the viewport, got {widest}");
    // The open page's button is drawn held; the others are not.
    let look = |id: u32| idle.shown.iter().find(|shown| shown.id == id).expect("shown").look;
    assert_ne!(look(2), look(1));
    assert_eq!(look(1), look(3));
    // A press on a button fires it, and is not a press on the picture.
    let press = top_bar(&mut state, &buttons(Some(2)), VIEW, at(mesh.x + 2.0, mesh.y + 2.0, true));
    assert_eq!(press.actions, vec![Action::Press(3)]);
    assert!(!press.look_capture);
    let _ = top_bar(&mut state, &buttons(Some(3)), VIEW, Pointer::default());
    // A press under the bar belongs to the picture.
    let miss = top_bar(&mut state, &buttons(Some(3)), VIEW, at(500.0, 400.0, true));
    assert!(miss.actions.is_empty() && miss.look_capture);
}

#[test]
fn a_docked_page_is_a_drawer_on_the_right_edge_that_a_drag_does_not_move() {
    let camera = Camera::opening();
    let mut state = State::default();
    state.dockable = true;
    state.dock = Some(TOP_BAR_HEIGHT);
    let docked = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, Pointer::default());
    let panel = rect(&docked, PANEL_ID);
    assert!((panel.x + panel.w - VIEW[0]).abs() < 0.5, "on the right edge: {panel:?}");
    assert!((panel.y - TOP_BAR_HEIGHT).abs() < 0.5, "under the bar: {panel:?}");
    assert!((panel.y + panel.h - (VIEW[1] - 8.0)).abs() < 0.5, "down to the bottom margin: {panel:?}");
    let word = |frame: &genos_ui::Frame| frame.shown.iter().find(|shown| shown.id == MENU_DOCK).expect("dock button").text.clone();
    assert_eq!(word(&docked).as_deref(), Some("float"));
    // A drag on the drawer's empty part leaves it where it is.
    let grab = [panel.x + panel.w * 0.5, panel.y + panel.h - 20.0];
    let _ = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, at(grab[0], grab[1], true));
    let dragged = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, at(grab[0] - 200.0, grab[1], true));
    assert_eq!(rect(&dragged, PANEL_ID).x, panel.x);
    let _ = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, Pointer::default());
    // The button fires MENU_DOCK; the game floats the panel, and it then drags.
    let button = rect(&docked, MENU_DOCK);
    let press = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, at(button.x + 2.0, button.y + 2.0, true));
    assert!(press.actions.contains(&Action::Press(MENU_DOCK)));
    let _ = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, Pointer::default());
    state.dock = None;
    state.panel_offset = [300.0, 100.0];
    let floating = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, Pointer::default());
    let panel = rect(&floating, PANEL_ID);
    assert_eq!(word(&floating).as_deref(), Some("dock"));
    assert!((panel.x - 300.0).abs() < 0.5 && panel.h < VIEW[1] * 0.5, "a panel as tall as its rows: {panel:?}");
    let grab = [panel.x + panel.w - 6.0, panel.y + panel.h - 4.0];
    let _ = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, at(grab[0], grab[1], true));
    let moved = button_panel_blocks(&mut state, "Mesh", &page(), VIEW, &camera, at(grab[0] - 100.0, grab[1], true));
    assert!((rect(&moved, PANEL_ID).x - 200.0).abs() < 0.5, "a floating panel drags");
}
