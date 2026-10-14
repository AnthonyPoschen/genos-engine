#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! Tabs and folds are separate. A tab can hold a fold, and a fold can sit outside the tabs.

use genos_scene::Camera;
use genos_ui::{
    button_panel_blocks, Action, PanelBlock, PanelCheck, PanelRow, PanelSection, PanelTab,
    PanelTabs, Pointer, State, PANEL_ID,
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

    let next = Pointer {
        tab_next: true,
        ..Pointer::default()
    };
    let press = draw(&mut state, &blocks, next);
    assert!(press.actions.contains(&Action::Press(101)));
    let shown = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&shown, 2));
    assert!(!shows(&shown, 3));
    assert_eq!(state.selected_tab(100), Some(101));

    let prev = Pointer {
        tab_prev: true,
        ..Pointer::default()
    };
    let _ = draw(&mut state, &blocks, prev);
    let back = draw(&mut state, &blocks, Pointer::default());
    assert!(shows(&back, 3));
    assert_eq!(state.selected_tab(100), Some(102));
}

#[test]
fn a_tab_word_stays_one_line_and_the_bar_does_not_move() {
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
    let panel = first
        .shown
        .iter()
        .find(|shown| shown.id == u32::MAX - 4095)
        .unwrap();
    assert!(
        shade.rect.h <= 16.0 + 8.0 + 0.5,
        "a word stays on one line"
    );
    assert!((gi.rect.h - shade.rect.h).abs() < 0.5);
    assert!((gi.rect.w - (2.0 * 12.0 + 8.0)).abs() < 1.0);
    assert!(
        (shade.rect.w - label_width("Shadows/Ambient")).abs() < 1.0,
        "a tab wider than the row keeps its whole label"
    );
    assert!(panel.rect.w <= 400.0 / 3.0 + 0.5);
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

    let next = Pointer {
        tab_next: true,
        ..Pointer::default()
    };
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
            f32::from(0x33_u8) / 255.0,
            f32::from(0x66_u8) / 255.0,
            f32::from(0x99_u8) / 255.0
        ]
    );
    assert_eq!(
        gi.idle.background,
        [
            f32::from(0x11_u8) / 255.0,
            f32::from(0x11_u8) / 255.0,
            f32::from(0x11_u8) / 255.0
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

fn titled(id: u32, title: &str) -> PanelTab {
    let rows: Vec<PanelRow> = (1..18).map(|row| mark(id * 100 + row, "Row")).collect();
    PanelTab {
        id,
        title: title.into(),
        blocks: vec![PanelBlock::Rows(rows)],
    }
}

fn tab_line() -> f32 {
    16.0 + 8.0
}

fn label_width(title: &str) -> f32 {
    let chars = u16::try_from(title.chars().count()).unwrap_or(u16::MAX);
    f32::from(chars) * 12.0 + 8.0
}

fn id_at(index: usize) -> u32 {
    1000 + u32::try_from(index).unwrap_or(0)
}

fn assert_tabs_fully_visible(frame: &genos_ui::Frame, titles: &[&str], panel: genos_ui::Rect) {
    let line = tab_line();
    for (index, title) in titles.iter().enumerate() {
        let tab = frame
            .shown
            .iter()
            .find(|shown| shown.id == id_at(index))
            .unwrap_or_else(|| panic!("{title} is missing"));
        assert_eq!(tab.text.as_deref(), Some(*title));
        assert!((tab.rect.h - line).abs() < 0.5, "{title} is one line");
        assert!(
            (tab.rect.w - label_width(title)).abs() < 1.0,
            "{title} keeps its whole label"
        );
        assert!(tab.rect.x >= panel.x - 0.5);
        assert!(
            tab.rect.x + tab.rect.w <= panel.x + panel.w + 0.5,
            "{title} is cut at the panel edge"
        );
    }
}

#[test]
fn tabs_wrap_onto_another_compact_row_when_the_strip_overflows() {
    let titles = [
        "Scene", "Pipeline", "Lamps", "GI", "Denoise", "Shadows", "Perf", "Info",
    ];
    let blocks = vec![PanelBlock::Tabs(PanelTabs {
        id: 100,
        tabs: titles
            .iter()
            .enumerate()
            .map(|(index, title)| titled(id_at(index), title))
            .collect(),
    })];
    let viewport = [1024.0, 400.0];
    let mut state = State::default();
    let frame = button_panel_blocks(
        &mut state,
        "stress",
        &blocks,
        viewport,
        &Camera::opening(),
        Pointer::default(),
    );
    let panel = frame
        .shown
        .iter()
        .find(|shown| shown.id == PANEL_ID)
        .unwrap()
        .rect;
    assert!(panel.w <= viewport[0] / 3.0 + 0.5);
    let strip = frame.shown.iter().find(|shown| shown.id == 100).unwrap().rect;
    let line = tab_line();
    assert!(strip.h > line + 0.5, "overflow wraps onto a second row");
    assert!(
        strip.h <= line * 2.0 + 4.0 + 0.5,
        "the second row stays one line, not a tall box"
    );
    assert_tabs_fully_visible(&frame, &titles, panel);
    let mut rows = Vec::new();
    for id in 1000..1008 {
        let y = frame.shown.iter().find(|shown| shown.id == id).unwrap().rect.y;
        if !rows.iter().any(|row: &f32| (row - y).abs() < 0.5) {
            rows.push(y);
        }
    }
    assert_eq!(rows.len(), 2, "the eight labels use two rows");
    let scene = frame.shown.iter().find(|shown| shown.id == 1000).unwrap().rect;
    let shadows = frame.shown.iter().find(|shown| shown.id == 1005).unwrap().rect;
    assert!((shadows.y - scene.y).abs() > 1.0, "Shadows is on the next row");
    assert!(
        frame.shown.iter().all(|shown| {
            shown.id == PANEL_ID
                || shown.rect.h <= strip.h + 0.5
                || shown.rect.y > strip.y + strip.h
        }),
        "no empty filler cell beside the tabs"
    );
    assert_wide_window_is_one_row(&blocks, &titles, line);
    assert_the_bar_stays_and_the_page_still_scrolls(&mut state, &blocks, &frame, panel, strip, viewport);
}

fn assert_wide_window_is_one_row(blocks: &[PanelBlock], titles: &[&str], line: f32) {
    let wide = button_panel_blocks(
        &mut State::default(),
        "stress",
        blocks,
        [1920.0, 1080.0],
        &Camera::opening(),
        Pointer::default(),
    );
    let wide_panel = wide
        .shown
        .iter()
        .find(|shown| shown.id == PANEL_ID)
        .unwrap()
        .rect;
    let wide_strip = wide.shown.iter().find(|shown| shown.id == 100).unwrap().rect;
    assert!(wide_strip.h <= line + 0.5, "a wide panel keeps one row");
    assert_tabs_fully_visible(&wide, titles, wide_panel);
    let y = wide.shown.iter().find(|shown| shown.id == 1000).unwrap().rect.y;
    assert!(wide
        .shown
        .iter()
        .filter(|shown| (1000..1008).contains(&shown.id))
        .all(|shown| (shown.rect.y - y).abs() < 0.5));
}

fn assert_the_bar_stays_and_the_page_still_scrolls(
    state: &mut State,
    blocks: &[PanelBlock],
    frame: &genos_ui::Frame,
    panel: genos_ui::Rect,
    strip: genos_ui::Rect,
    viewport: [f32; 2],
) {
    let bar = frame
        .shown
        .iter()
        .filter(|shown| (1000..1008).contains(&shown.id))
        .map(|shown| (shown.id, shown.rect.x, shown.rect.y, shown.rect.w))
        .collect::<Vec<_>>();
    let next = Pointer {
        tab_next: true,
        ..Pointer::default()
    };
    let _ = button_panel_blocks(
        state,
        "stress",
        blocks,
        viewport,
        &Camera::opening(),
        next,
    );
    let second = button_panel_blocks(
        state,
        "stress",
        blocks,
        viewport,
        &Camera::opening(),
        Pointer::default(),
    );
    let after = second
        .shown
        .iter()
        .filter(|shown| (1000..1008).contains(&shown.id))
        .map(|shown| (shown.id, shown.rect.x, shown.rect.y, shown.rect.w))
        .collect::<Vec<_>>();
    assert_eq!(after, bar, "changing page does not move the bar");

    let row_y = frame.shown.iter().find(|shown| shown.id == 100_001).unwrap().rect.y;
    let below = Pointer {
        x: panel.x + 8.0,
        y: strip.y + strip.h + 30.0,
        wheel_y: -6.0,
        ..Pointer::default()
    };
    let page = button_panel_blocks(
        state,
        "stress",
        blocks,
        viewport,
        &Camera::opening(),
        below,
    );
    assert_ne!(
        page.shown
            .iter()
            .find(|shown| shown.id == 100_001)
            .map(|shown| shown.rect.y),
        Some(row_y)
    );
}
