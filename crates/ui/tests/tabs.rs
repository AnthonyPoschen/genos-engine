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

fn page(id: u32, title: &str, row: u32, text: &str) -> PanelTab {
    PanelTab {
        id,
        title: title.into(),
        blocks: vec![PanelBlock::Rows(vec![mark(row, text)])],
    }
}

#[test]
fn brackets_change_the_page_and_the_choice_is_remembered() {
    let blocks = vec![PanelBlock::Tabs(PanelTabs {
        id: 100,
        tabs: vec![
            page(101, "Direct", 2, "Samples"),
            page(102, "GI", 3, "Probes"),
        ],
    })];
    let mut state = State::default();
    state.select_tab(100, 102);
    let kept = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&kept, 3), "a saved tab wins over the first page");
    assert!(!shows(&kept, 2));

    let mut next = Pointer::default();
    next.tab_next = true;
    let press = draw(&mut state, &blocks, next);
    assert!(press.actions.contains(&Action::Press(101)));
    let shown = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&shown, 2));
    assert!(!shows(&shown, 3));
    assert_eq!(state.selected_tab(100), Some(101));

    let mut prev = Pointer::default();
    prev.tab_prev = true;
    let _ = draw(&mut state, &blocks, prev);
    let back = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&back, 3));
    assert_eq!(state.selected_tab(100), Some(102));
}

#[test]
fn tab_labels_wrap_and_the_bar_does_not_move() {
    let blocks = vec![PanelBlock::Tabs(PanelTabs {
        id: 100,
        tabs: vec![
            page(101, "GI", 2, "Samples"),
            page(102, "Shadows/Ambient", 3, "Probes"),
        ],
    })];
    let mut state = State::default();
    let first = button_panel_blocks(
        &mut state,
        "stress",
        &blocks,
        [400.0, 800.0],
        &Camera::opening(),
        Pointer::default(),
    );
    let gi = first.shown.iter().find(|shown| shown.id == 101).unwrap();
    let shade = first.shown.iter().find(|shown| shown.id == 102).unwrap();
    assert!(
        shade.rect.h > 16.0 + 8.0,
        "a long label wraps inside its cell"
    );
    assert_eq!(gi.rect.w, shade.rect.w);
    assert_eq!(gi.rect.h, shade.rect.h);
    let panel = first
        .shown
        .iter()
        .find(|shown| shown.id == u32::MAX - 4095)
        .unwrap();
    let width = panel.rect.w;
    let bar = (
        gi.rect.x,
        gi.rect.y,
        gi.rect.w,
        gi.rect.h,
        shade.rect.x,
        shade.rect.y,
        shade.rect.w,
        shade.rect.h,
    );

    let mut next = Pointer::default();
    next.tab_next = true;
    let _ = button_panel_blocks(
        &mut state,
        "stress",
        &blocks,
        [400.0, 800.0],
        &Camera::opening(),
        next,
    );
    let second = button_panel_blocks(
        &mut state,
        "stress",
        &blocks,
        [400.0, 800.0],
        &Camera::opening(),
        Pointer::default(),
    );
    let gi = second.shown.iter().find(|shown| shown.id == 101).unwrap();
    let shade = second.shown.iter().find(|shown| shown.id == 102).unwrap();
    assert_eq!(
        (
            gi.rect.x,
            gi.rect.y,
            gi.rect.w,
            gi.rect.h,
            shade.rect.x,
            shade.rect.y,
            shade.rect.w,
            shade.rect.h
        ),
        bar
    );
    let panel = second
        .shown
        .iter()
        .find(|shown| shown.id == u32::MAX - 4095)
        .unwrap();
    assert_eq!(panel.rect.w, width);
    assert!(shows(&second, 3));
    assert!(!shows(&second, 2));
}

#[test]
fn the_selected_tab_uses_the_theme_accent() {
    let root = std::env::temp_dir().join(format!("genos-tab-theme-{}", std::process::id()));
    let current = root.join("current");
    let theme = current.join("theme");
    std::fs::create_dir_all(&theme).unwrap();
    std::fs::write(
        theme.join("colors.toml"),
        "background = \"#111111\"\nforeground = \"#eeeeee\"\naccent = \"#336699\"\n",
    )
    .unwrap();
    let blocks = vec![PanelBlock::Tabs(PanelTabs {
        id: 100,
        tabs: vec![
            page(101, "Direct", 2, "Samples"),
            page(102, "GI", 3, "Probes"),
        ],
    })];
    let mut state = State::default();
    state.omarchy_current = Some(current);
    let frame = draw(&mut state, &blocks, Pointer::default());
    let direct = frame.shown.iter().find(|shown| shown.id == 101).unwrap();
    let gi = frame.shown.iter().find(|shown| shown.id == 102).unwrap();
    assert_eq!(
        direct.idle.background,
        [
            0x33 as f32 / 255.0,
            0x66 as f32 / 255.0,
            0x99 as f32 / 255.0
        ]
    );
    assert_eq!(
        gi.idle.background,
        [
            0x11 as f32 / 255.0,
            0x11 as f32 / 255.0,
            0x11 as f32 / 255.0
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_page_can_hold_rows_without_a_fold_header() {
    let blocks = vec![PanelBlock::Tabs(PanelTabs {
        id: 100,
        tabs: vec![page(101, "Info", 2, "Review")],
    })];
    let mut state = State::default();
    let frame = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&frame, 2));
    assert!(frame
        .shown
        .iter()
        .all(|shown| shown.text.as_deref() != Some("+ Info") && shown.id != 50));
}
