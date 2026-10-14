//! The title bar hides the menu. A corner tab shows it again.

use genos_scene::Camera;
use genos_ui::{
    button_panel_sections, show_menu_tab, Action, PanelRow, PanelSection, Pointer, State,
    MENU_HIDE, MENU_SHOW,
};

fn row(label: &str) -> PanelRow {
    PanelRow {
        label: label.into(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: Vec::new(),
    }
}

#[test]
fn the_title_bar_hides_and_the_corner_tab_shows() {
    let sections = vec![PanelSection::new(
        10,
        "Scene/View",
        false,
        vec![row("Scene")],
    )];
    let mut state = State::default();
    let open = button_panel_sections(
        &mut state,
        "stress",
        &sections,
        [1024.0, 832.0],
        &Camera::opening(),
        Pointer::default(),
    );
    let hide = open
        .shown
        .iter()
        .find(|shown| shown.id == MENU_HIDE)
        .expect("hide");
    assert_eq!(hide.text.as_deref(), Some("hide"));
    let panel = open
        .shown
        .iter()
        .find(|shown| shown.id == u32::MAX - 4095)
        .unwrap();
    assert!(hide.rect.x + hide.rect.w <= panel.rect.x + panel.rect.w + 0.5);
    let press = button_panel_sections(
        &mut state,
        "stress",
        &sections,
        [1024.0, 832.0],
        &Camera::opening(),
        Pointer::at(hide.rect.x + 4.0, hide.rect.y + 4.0, true),
    );
    assert!(press.actions.contains(&Action::Press(MENU_HIDE)));

    let mut hidden = State::default();
    let tab = show_menu_tab(&mut hidden, [1024.0, 832.0], Pointer::default());
    let menu = tab
        .shown
        .iter()
        .find(|shown| shown.id == MENU_SHOW)
        .expect("menu tab");
    assert_eq!(menu.text.as_deref(), Some("menu"));
    assert!(menu.rect.x < 40.0);
    assert!(menu.rect.y < 40.0);
    let show = show_menu_tab(
        &mut hidden,
        [1024.0, 832.0],
        Pointer::at(menu.rect.x + 4.0, menu.rect.y + 4.0, true),
    );
    assert!(show.actions.contains(&Action::Press(MENU_SHOW)));
    assert!(!show.look_capture);
    let _ = show_menu_tab(&mut hidden, [1024.0, 832.0], Pointer::default());
    let miss = show_menu_tab(
        &mut hidden,
        [1024.0, 832.0],
        Pointer::at(400.0, 400.0, true),
    );
    assert!(miss.look_capture);
}
