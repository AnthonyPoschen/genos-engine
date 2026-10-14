//! The lighting panel, the lamp sun, and the pointer step the camera frame uses.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use genos_scene::{Camera, FocusPolicy, Scene};

use crate::dropdown;
use crate::focus::{Consumed, FocusManager, InputKind, InputPolicy};
use crate::layout::{self, Direction, Item, Node, Pad, Place, Rect, Sizing, Space};
use crate::omarchy::{self, Palette};
use crate::slider::{self, SliderStep};
use crate::text::{self, Blot};
use crate::{id, Action, Look, PictureMode};

const BRIGHTER: f32 = 1.25;
const DIMMER: f32 = 0.8;
const BORDER: f32 = 2.0;
const TEXT: [f32; 3] = [0.93, 0.95, 0.92];
const SLIDER_W: f32 = 168.0;
const SLIDER_H: f32 = 24.0;
const SLIDER_INSET: f32 = 12.0;
const NOTCH_W: f32 = 2.0;
const NOTCH_H: f32 = 8.0;
const THUMB_W: f32 = 10.0;
const THUMB_H: f32 = 16.0;
const SUN_DIAMETER: f32 = 32.0;
const SUN_BANDS: usize = 9;
const SUN_COLOR: [f32; 3] = [1.0, 0.92, 0.35];
const THUMB_COLOR: [f32; 3] = [0.95, 0.86, 0.40];
/// Press on the panel title bar. The caller hides the menu.
pub const MENU_HIDE: u32 = u32::MAX - 5000;
/// Press on the corner tab while the menu is hidden. The caller shows the menu.
pub const MENU_SHOW: u32 = u32::MAX - 5001;
const TITLE_ROW: u32 = u32::MAX - 5002;
/// The settings panel. A game sets this id's [`InputPolicy`].
pub const PANEL_ID: u32 = u32::MAX - 4095;

/// X and Z reach both edges of the 16 by 16 floor. Neighbor notches are 1 meter apart.
const XZ_NOTCHES: [f32; 17] = [
    -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0,
];
/// Y includes the scripted height 7 and positions under the floor. Neighbor notches are 1 meter apart.
const Y_NOTCHES: [f32; 15] = [
    -4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0,
];

#[derive(Clone, Copy, Debug)]
pub struct Pointer {
    pub x: f32,
    pub y: f32,
    pub down: bool,
    /// Vertical wheel this frame. Positive steps a slider toward its maximum.
    /// Callers pass the pressed edge, not a held repeat, for the keys below.
    pub wheel_y: f32,
    /// Decrease key pressed this frame (left or down).
    pub key_dec: bool,
    /// Increase key pressed this frame (right or up).
    pub key_inc: bool,
    /// Activate key pressed this frame (enter). Opens or picks the highlighted choice.
    pub key_activate: bool,
    /// Dismiss key pressed this frame (escape). Closes an open list.
    pub key_dismiss: bool,
    /// Grave pressed this frame. Click-to-focus releases on this key.
    pub key_blur: bool,
    /// Previous page of the tab strip. A press edge, not a held key.
    pub tab_prev: bool,
    /// Next page of the tab strip. A press edge, not a held key.
    pub tab_next: bool,
}

impl Default for Pointer {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            down: false,
            wheel_y: 0.0,
            key_dec: false,
            key_inc: false,
            key_activate: false,
            key_dismiss: false,
            key_blur: false,
            tab_prev: false,
            tab_next: false,
        }
    }
}

impl Pointer {
    pub fn at(x: f32, y: f32, down: bool) -> Self {
        Self {
            x,
            y,
            down,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub panel_offset: [f32; 2],
    pointer_x: f32,
    pointer_y: f32,
    down: bool,
    dragging: bool,
    /// Axis of the slider held by the current press.
    sliding: Option<u8>,
    /// Game slider held by the current press.
    held: Option<u32>,
    /// The current press missed the panel, so look capture stays on.
    look_hold: bool,
    focus: Option<u32>,
    /// Dropdown whose list is open. The list paints over the rest of the panel.
    open_drop: Option<u32>,
    drop_highlight: usize,
    drop_scroll: usize,
    /// True while the red box path is advancing.
    pub box_running: bool,
    /// Omarchy `current` directory. `None` uses the real state path.
    pub omarchy_current: Option<PathBuf>,
    palette: Option<Palette>,
    palette_from: Option<PathBuf>,
    /// Section ids that have already taken their first-frame fold.
    section_seen: BTreeSet<u32>,
    /// Section ids that are folded. Later frames keep this set.
    collapsed_sections: BTreeSet<u32>,
    /// Selected tab id for each tab strip.
    selected_tabs: BTreeMap<u32, u32>,
    /// The pointer was over the panel on the last frame.
    panel_hover: bool,
    /// Pixels the page has moved up under the tab bar. The bar stays put.
    scroll: f32,
    /// Per-panel wheel and key policy. Empty means the older path, which takes
    /// keys and the pointer the way it did before policies existed.
    pub input: FocusManager,
}

impl Default for State {
    fn default() -> Self {
        Self {
            panel_offset: [16.0, 16.0],
            pointer_x: 0.0,
            pointer_y: 0.0,
            down: false,
            dragging: false,
            sliding: None,
            held: None,
            look_hold: false,
            focus: None,
            open_drop: None,
            drop_highlight: 0,
            drop_scroll: 0,
            box_running: false,
            omarchy_current: None,
            palette: None,
            palette_from: None,
            section_seen: BTreeSet::new(),
            collapsed_sections: BTreeSet::new(),
            selected_tabs: BTreeMap::new(),
            panel_hover: false,
            scroll: 0.0,
            input: FocusManager::default(),
        }
    }
}

impl State {
    /// Remember a fold before the first frame. A later frame keeps this choice
    /// instead of the section's starting fold.
    pub fn fold_section(&mut self, id: u32, collapsed: bool) {
        self.section_seen.insert(id);
        if collapsed {
            self.collapsed_sections.insert(id);
        } else {
            self.collapsed_sections.remove(&id);
        }
    }

    /// True when this section is folded.
    pub fn section_collapsed(&self, id: u32) -> bool {
        self.collapsed_sections.contains(&id)
    }

    /// True after this section has been laid out or folded from outside.
    /// A fold that has not been seen yet keeps its layout default.
    pub fn section_known(&self, id: u32) -> bool {
        self.section_seen.contains(&id)
    }

    /// The tab this strip is showing. `None` until the first frame picks one.
    pub fn selected_tab(&self, group: u32) -> Option<u32> {
        self.selected_tabs.get(&group).copied()
    }

    /// Show this tab before the first frame. A later press keeps the new choice.
    pub fn select_tab(&mut self, group: u32, tab: u32) {
        self.selected_tabs.insert(group, tab);
    }

    /// True when the pointer was over the panel on the last frame.
    pub fn panel_hovered(&self) -> bool {
        self.panel_hover
    }

    /// Replace how this panel takes the wheel and the keys. The next frame uses it.
    pub fn set_input_policy(&mut self, panel: u32, policy: InputPolicy) {
        self.input.set_policy(panel, policy);
    }

    /// The policy last set for this panel.
    pub fn input_policy(&self, panel: u32) -> Option<InputPolicy> {
        self.input.policy(panel)
    }

    /// The panel click-to-focus is holding.
    pub fn focused_panel(&self) -> Option<u32> {
        self.input.focused()
    }

    /// Arm click-to-focus, or pass `None` to release it.
    pub fn set_focused_panel(&mut self, panel: Option<u32>) {
        self.input.set_focus(panel);
    }

    /// The control inside the focused panel that arrows move.
    pub fn focused_control(&self) -> Option<u32> {
        self.focus
    }

    fn toggle_section(&mut self, id: u32) {
        self.section_seen.insert(id);
        if !self.collapsed_sections.remove(&id) {
            self.collapsed_sections.insert(id);
        }
    }

    /// First sight uses `start_collapsed`. After that, the remembered fold wins.
    fn section_is_open(&mut self, id: u32, start_collapsed: bool) -> bool {
        if self.section_seen.insert(id) && start_collapsed {
            self.collapsed_sections.insert(id);
        }
        !self.collapsed_sections.contains(&id)
    }
}

#[derive(Clone, Debug)]
pub struct Shown {
    pub id: u32,
    pub rect: Rect,
    pub look: Look,
    pub idle: Look,
    pub text: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct Paint {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub shown: Vec<Shown>,
    pub paints: Vec<Paint>,
    pub actions: Vec<Action>,
    /// True when this press missed the panel and the camera may capture look.
    pub look_capture: bool,
    /// Name and exact value while the pointer is on a slider. `None` otherwise.
    pub tooltip: Option<String>,
    /// Events the panel took. A false field belongs to the game.
    pub consumed: Consumed,
}

/// Lay out the lighting panel and the lamp sun. Apply hover, press, and drag.
///
/// `lamp` is the first lamp in world space. `None` draws the panel without a sun.
/// A press on the panel sets `look_capture` false. A press on the sun does not.
/// The camera frame must not capture the pointer when `look_capture` is false.
pub fn lighting_frame(
    state: &mut State,
    viewport: [f32; 2],
    camera: &Camera,
    lamp: Option<[f32; 3]>,
    pointer: Pointer,
) -> Frame {
    let press_edge = pointer.down && !state.down;
    if state.dragging && pointer.down && !press_edge {
        state.panel_offset[0] += pointer.x - state.pointer_x;
        state.panel_offset[1] += pointer.y - state.pointer_y;
    }
    refresh_palette(state);
    let root = lighting_panel(state.panel_offset, state.palette, state.box_running);
    let items = layout::layout(&root, Space::Screen, viewport, Some(camera));
    let hit = items
        .iter()
        .rev()
        .find(|item| item.rect.contains(pointer.x, pointer.y));
    let mut actions = Vec::new();
    let mut look_capture = false;
    if press_edge {
        if let Some(item) = hit {
            state.focus = Some(item.id);
            state.look_hold = false;
            if let Some(axis) = slider_axis(item.id) {
                state.sliding = Some(axis);
                state.dragging = false;
            } else if let Some(action) = item.action {
                actions.push(action);
                state.sliding = None;
            } else if draggable(&items, item.id) {
                state.dragging = true;
                state.sliding = None;
            } else {
                state.sliding = None;
            }
        } else {
            state.focus = None;
            state.dragging = false;
            state.sliding = None;
            state.look_hold = true;
            look_capture = true;
        }
    } else if pointer.down && state.look_hold {
        look_capture = true;
    }
    if pointer.down {
        if let Some(axis) = state.sliding {
            if let Some(position) = snap_axis(&items, axis, pointer.x) {
                actions.push(Action::SetLamp { axis, position });
            }
        }
    }
    let painted_lamp = lamp_after(lamp, &actions);
    let mut shown = Vec::with_capacity(items.len() + 1);
    let mut paints = Vec::new();
    for item in &items {
        let look = choose(item, pointer.down, hit.map(|hit| hit.id), state.focus);
        shown.push(Shown {
            id: item.id,
            rect: item.rect,
            look,
            idle: item.idle,
            text: item.text.clone(),
        });
        push_paints(&mut paints, item, look, state.palette, painted_lamp, true);
    }
    // The sun is not in `items`, so a press on it misses the panel.
    push_sun(&mut shown, &mut paints, viewport, camera, painted_lamp);
    state.pointer_x = pointer.x;
    state.pointer_y = pointer.y;
    state.down = pointer.down;
    if !pointer.down {
        state.dragging = false;
        state.sliding = None;
        state.look_hold = false;
    }
    Frame {
        shown,
        paints,
        actions,
        look_capture,
        tooltip: None,
        consumed: Consumed::default(),
    }
}

/// One row of a game panel: a label, then buttons left to right, then a slider.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelRow {
    pub label: String,
    pub buttons: Vec<PanelButton>,
    pub slider: Option<PanelSlider>,
    /// Closed box in the row. The open list is not part of the row's layout.
    pub dropdown: Option<PanelDropdown>,
    /// On/off marks laid out left to right. A press reports the next state.
    pub checks: Vec<PanelCheck>,
}

/// A named group of rows. A press on the title folds the rows.
///
/// `collapsed` applies the first time this id is shown. [`State`] remembers the
/// fold after that, including across later calls with the same id.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelSection {
    pub id: u32,
    pub title: String,
    /// Folded on the first frame this id is seen.
    pub collapsed: bool,
    pub rows: Vec<PanelRow>,
    /// Folded with the rows. A fold can hold tabs, and a tab can hold folds.
    pub blocks: Vec<PanelBlock>,
}

impl PanelSection {
    pub fn new(id: u32, title: impl Into<String>, collapsed: bool, rows: Vec<PanelRow>) -> Self {
        Self {
            id,
            title: title.into(),
            collapsed,
            rows,
            blocks: Vec::new(),
        }
    }

    /// Put tabs or more folds inside this fold.
    pub fn with_blocks(mut self, blocks: Vec<PanelBlock>) -> Self {
        self.blocks = blocks;
        self
    }
}

/// One page in a [`PanelTabs`] strip. Its blocks are folds, rows are not required.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelTab {
    pub id: u32,
    pub title: String,
    pub blocks: Vec<PanelBlock>,
}

/// A row of tabs. Only the selected tab's blocks are laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelTabs {
    pub id: u32,
    pub tabs: Vec<PanelTab>,
}

/// A fold, a tab strip, or rows with no fold header. Either container can hold the others.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelBlock {
    Section(PanelSection),
    Tabs(PanelTabs),
    /// Controls on a tab, or outside one, with no title to fold.
    Rows(Vec<PanelRow>),
}

/// A compact on/off mark and a label. The panel does not store `on`.
/// The next frame passes the new value.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelCheck {
    pub id: u32,
    pub text: String,
    pub on: bool,
}

/// A closed choice box. A press opens a scrollable list over the panel.
///
/// Arrow keys move the highlight. Enter picks it. Escape, or a press outside
/// the list, closes it without a choice. The wheel scrolls the list. The row
/// under the pointer is highlighted.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelDropdown {
    pub id: u32,
    /// Selected value. The closed box shows the nearest choice name.
    pub value: f64,
    pub choices: Vec<SliderStep>,
}

impl PanelDropdown {
    pub fn new(id: u32, value: f64, choices: Vec<SliderStep>) -> Self {
        Self { id, value, choices }
    }

    fn selected_index(&self) -> usize {
        slider::nearest_index(&self.choices, self.value).unwrap_or(0)
    }

    fn selected_name(&self) -> String {
        self.choices
            .get(self.selected_index())
            .map(|choice| choice.name.clone())
            .unwrap_or_default()
    }
}

/// A slider on a panel row. A press anywhere on the track sets the value there and a
/// held press follows the pointer, firing [`Action::Slide`] each frame it changes.
///
/// `steps` empty is a numeric slider: `step` of 0 is continuous, and a positive
/// `step` snaps to `min + k * step`. Named `steps` are discrete. The track spaces
/// them evenly. A click, a drag, the wheel, and the arrow keys snap to a step and
/// report that step's `value`. The names are drawn small and dim under the ticks.
/// The current name is brighter. A hover tip shows the name and the exact value.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelSlider {
    /// Game id, as for a button.
    pub id: u32,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    /// Values snap to `min + k * step`; 0 is continuous. Ignored when `steps` is set.
    pub step: f64,
    /// Readout beside the track, e.g. `23.9 H`.
    pub text: String,
    /// Named steps in track order. Empty for a numeric slider.
    pub steps: Vec<SliderStep>,
}

impl PanelSlider {
    /// A numeric slider. `steps` stays empty.
    pub fn range(
        id: u32,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        text: impl Into<String>,
    ) -> Self {
        Self {
            id,
            value,
            min,
            max,
            step,
            text: text.into(),
            steps: Vec::new(),
        }
    }

    /// Discrete named steps. `value` selects the nearest step.
    pub fn named(id: u32, value: f64, steps: Vec<SliderStep>) -> Self {
        let text = slider::nearest_index(&steps, value)
            .map(|index| steps[index].name.clone())
            .unwrap_or_default();
        let min = steps.first().map(|step| step.value).unwrap_or(0.0);
        let max = steps.last().map(|step| step.value).unwrap_or(min);
        Self {
            id,
            value,
            min,
            max,
            step: 0.0,
            text,
            steps,
        }
    }

    fn share(&self) -> f32 {
        slider::share(self.min, self.max, &self.steps, self.value)
    }

    fn value_at(&self, track: Rect, pointer_x: f32) -> f64 {
        let band = thumb_band(track, !self.steps.is_empty());
        let (left, right) = track_ends(band);
        let t = ((pointer_x - left) / (right - left).max(1.0)).clamp(0.0, 1.0) as f64;
        slider::value_at_fraction(self.min, self.max, self.step, &self.steps, t)
    }

    /// Move `delta` steps from the current value. Positive moves toward `max`.
    pub fn step_by(&self, delta: i32) -> f64 {
        slider::step_by(
            self.min,
            self.max,
            self.step,
            &self.steps,
            self.value,
            delta,
        )
    }

    fn step_from(&self, value: f64, delta: i32) -> f64 {
        slider::step_by(self.min, self.max, self.step, &self.steps, value, delta)
    }

    /// Hover text: the step name and its exact value, or the exact numeric value.
    pub fn tip(&self) -> String {
        slider::tip(&self.steps, self.value)
    }

    fn at(&self, value: f64) -> Self {
        Self {
            value,
            ..self.clone()
        }
    }
}

/// A game button. A press fires [`Action::Press`] with `id`. `selected` draws the
/// button in its pressed look, for a choice that is on.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelButton {
    /// Game id. Ids from `u32::MAX - 4095` up are the panel's own rows and labels.
    pub id: u32,
    pub text: String,
    pub selected: bool,
}

/// Lay out a game panel of labelled button rows at `state.panel_offset`, apply hover,
/// press and drag, and paint it. A press on a button fires [`Action::Press`]; a press
/// elsewhere on the panel drags it. A press that misses the panel sets `look_capture`.
pub fn button_panel(
    state: &mut State,
    title: &str,
    rows: &[PanelRow],
    viewport: [f32; 2],
    camera: &Camera,
    pointer: Pointer,
) -> Frame {
    drag_panel(state, pointer);
    refresh_palette(state);
    let panel = panel_tree(state, title, rows, &[], None, false);
    finish_panel(state, panel, rows, &[], &[], viewport, camera, pointer)
}

/// Lay out the same panel as [`button_panel`], grouped under folding section titles.
///
/// A press on a title fires [`Action::Press`] with the section id and folds the
/// rows under it. The next call keeps that fold.
pub fn button_panel_sections(
    state: &mut State,
    title: &str,
    sections: &[PanelSection],
    viewport: [f32; 2],
    camera: &Camera,
    pointer: Pointer,
) -> Frame {
    drag_panel(state, pointer);
    refresh_palette(state);
    let mut visible = Vec::new();
    let mut ids = Vec::new();
    for section in sections {
        ids.push(section.id);
        if state.section_is_open(section.id, section.collapsed) {
            visible.extend(section.rows.iter().cloned());
        }
    }
    let max_width = viewport[0] / 3.0;
    let panel = panel_tree(state, title, &visible, sections, Some(max_width), true);
    finish_panel(state, panel, &visible, &ids, &[], viewport, camera, pointer)
}

/// Lay out folds and tab strips. A tab's blocks can be folds. A fold's blocks
/// can be a tab strip. A fold can also sit outside every tab.
///
/// A press on a tab fires [`Action::Press`] with the tab id and shows that tab
/// on the next frame. [`Pointer::tab_next`] and [`Pointer::tab_prev`] do the
/// same for the strip. A press on a fold title still folds that fold. Tab
/// labels share equal cells and wrap inside them, so a short page does not
/// move the bar.
pub fn button_panel_blocks(
    state: &mut State,
    title: &str,
    blocks: &[PanelBlock],
    viewport: [f32; 2],
    camera: &Camera,
    pointer: Pointer,
) -> Frame {
    drag_panel(state, pointer);
    refresh_palette(state);
    let mut visible = Vec::new();
    let mut section_ids = Vec::new();
    let mut tab_ids = Vec::new();
    collect_blocks(state, blocks, &mut visible, &mut section_ids, &mut tab_ids);
    let max_width = viewport[0] / 3.0;
    let panel = block_panel(state, title, blocks, Some(max_width));
    finish_panel(
        state,
        panel,
        &visible,
        &section_ids,
        &tab_ids,
        viewport,
        camera,
        pointer,
    )
}

fn drag_panel(state: &mut State, pointer: Pointer) {
    let press_edge = pointer.down && !state.down;
    if state.dragging && pointer.down && !press_edge {
        state.panel_offset[0] += pointer.x - state.pointer_x;
        state.panel_offset[1] += pointer.y - state.pointer_y;
    }
}

fn panel_tree(
    state: &State,
    title: &str,
    rows: &[PanelRow],
    sections: &[PanelSection],
    max_width: Option<f32>,
    hide: bool,
) -> Node {
    let palette = state.palette;
    let fill = surface(palette);
    let own = PANEL_ID;
    let mut panel = Node::new(own);
    panel.direction = Direction::TopToBottom;
    panel.pad = Pad::all(8.0);
    panel.gap = 6.0;
    panel.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], state.panel_offset));
    panel.draggable = true;
    paint_looks(&mut panel, chrome_looks(palette));
    if let Some(max_width) = max_width {
        panel.width = Sizing::Fit {
            min: 0.0,
            max: max_width,
        };
    }
    let inner = max_width.map(|width| (width - 16.0).max(text::CELL_W));
    panel
        .children
        .push(title_bar(own + 1, title, fill, inner, palette, hide));
    if sections.is_empty() {
        for (index, item) in rows.iter().enumerate() {
            panel.children.push(row_node(
                own + 2 + 3 * index as u32,
                item,
                fill,
                palette,
                None,
                None,
            ));
        }
        return panel;
    }
    let columns = inner.map(check_columns).unwrap_or(CHECK_COLUMNS);
    let cell = inner
        .map(|width| check_cell(width, columns))
        .unwrap_or(CHECK_CELL);
    let mut row_index = 0usize;
    for section in sections {
        panel
            .children
            .push(section_header(section, state, palette, inner));
        if state.section_collapsed(section.id) {
            continue;
        }
        for item in &section.rows {
            if check_grid(item) {
                for chunk in item.checks.chunks(columns) {
                    let mut part = item.clone();
                    part.checks = chunk.to_vec();
                    panel.children.push(row_node(
                        own + 2 + 3 * row_index as u32,
                        &part,
                        fill,
                        palette,
                        Some(cell),
                        inner,
                    ));
                    row_index += 1;
                }
            } else {
                panel.children.push(row_node(
                    own + 2 + 3 * row_index as u32,
                    item,
                    fill,
                    palette,
                    None,
                    inner,
                ));
                row_index += 1;
            }
        }
    }
    panel
}

fn block_panel(state: &State, title: &str, blocks: &[PanelBlock], max_width: Option<f32>) -> Node {
    let palette = state.palette;
    let fill = surface(palette);
    let own = PANEL_ID;
    let mut panel = Node::new(own);
    panel.direction = Direction::TopToBottom;
    panel.pad = Pad::all(8.0);
    panel.gap = 6.0;
    panel.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], state.panel_offset));
    panel.draggable = true;
    paint_looks(&mut panel, chrome_looks(palette));
    if let Some(max_width) = max_width {
        panel.width = Sizing::Fit {
            min: 0.0,
            max: max_width,
        };
    }
    let inner = max_width.map(|width| (width - 16.0).max(text::CELL_W));
    panel
        .children
        .push(title_bar(own + 1, title, fill, inner, palette, true));
    let columns = inner.map(check_columns).unwrap_or(CHECK_COLUMNS);
    let cell = inner
        .map(|width| check_cell(width, columns))
        .unwrap_or(CHECK_CELL);
    let mut lay = BlockLay {
        row_index: 0,
        columns,
        cell,
        inner,
        fill,
        palette,
        own,
    };
    push_blocks(&mut panel, state, blocks, &mut lay);
    panel
}

struct BlockLay {
    row_index: usize,
    columns: usize,
    cell: f32,
    inner: Option<f32>,
    fill: [f32; 3],
    palette: Option<Palette>,
    own: u32,
}

fn collect_blocks(
    state: &mut State,
    blocks: &[PanelBlock],
    visible: &mut Vec<PanelRow>,
    section_ids: &mut Vec<u32>,
    tab_ids: &mut Vec<(u32, u32)>,
) {
    for block in blocks {
        match block {
            PanelBlock::Section(section) => {
                section_ids.push(section.id);
                if state.section_is_open(section.id, section.collapsed) {
                    visible.extend(section.rows.iter().cloned());
                    collect_blocks(state, &section.blocks, visible, section_ids, tab_ids);
                }
            }
            PanelBlock::Tabs(tabs) => {
                for tab in &tabs.tabs {
                    tab_ids.push((tabs.id, tab.id));
                }
                let selected = state
                    .selected_tab(tabs.id)
                    .filter(|id| tabs.tabs.iter().any(|tab| tab.id == *id))
                    .or_else(|| tabs.tabs.first().map(|tab| tab.id));
                if let Some(id) = selected {
                    state.select_tab(tabs.id, id);
                    if let Some(tab) = tabs.tabs.iter().find(|tab| tab.id == id) {
                        collect_blocks(state, &tab.blocks, visible, section_ids, tab_ids);
                    }
                }
            }
            PanelBlock::Rows(rows) => visible.extend(rows.iter().cloned()),
        }
    }
}

fn push_blocks(panel: &mut Node, state: &State, blocks: &[PanelBlock], lay: &mut BlockLay) {
    for block in blocks {
        match block {
            PanelBlock::Section(section) => push_section(panel, state, section, lay),
            PanelBlock::Tabs(tabs) => {
                panel
                    .children
                    .push(tab_strip(tabs, state, lay.palette, lay.inner));
                let selected = state.selected_tab(tabs.id);
                if let Some(tab) = tabs.tabs.iter().find(|tab| Some(tab.id) == selected) {
                    push_blocks(panel, state, &tab.blocks, lay);
                }
            }
            PanelBlock::Rows(rows) => {
                for item in rows {
                    push_row(panel, item, lay);
                }
            }
        }
    }
}

fn push_section(panel: &mut Node, state: &State, section: &PanelSection, lay: &mut BlockLay) {
    panel
        .children
        .push(section_header(section, state, lay.palette, lay.inner));
    if state.section_collapsed(section.id) {
        return;
    }
    for item in &section.rows {
        push_row(panel, item, lay);
    }
    push_blocks(panel, state, &section.blocks, lay);
}

fn push_row(panel: &mut Node, item: &PanelRow, lay: &mut BlockLay) {
    if check_grid(item) {
        for chunk in item.checks.chunks(lay.columns) {
            let mut part = item.clone();
            part.checks = chunk.to_vec();
            panel.children.push(row_node(
                lay.own + 2 + 3 * lay.row_index as u32,
                &part,
                lay.fill,
                lay.palette,
                Some(lay.cell),
                lay.inner,
            ));
            lay.row_index += 1;
        }
    } else {
        panel.children.push(row_node(
            lay.own + 2 + 3 * lay.row_index as u32,
            item,
            lay.fill,
            lay.palette,
            None,
            lay.inner,
        ));
        lay.row_index += 1;
    }
}

/// Narrowest tab cell. A longer label wraps inside the cell instead of growing it.
const TAB_MIN: f32 = 56.0;
const TAB_GAP: f32 = 4.0;

fn tab_columns(inner: f32, count: usize) -> usize {
    let fit = ((inner + TAB_GAP) / (TAB_MIN + TAB_GAP)).floor() as usize;
    fit.clamp(1, count.max(1))
}

fn tab_strip(
    tabs: &PanelTabs,
    state: &State,
    palette: Option<Palette>,
    inner: Option<f32>,
) -> Node {
    let selected = state.selected_tab(tabs.id);
    let Some(inner) = inner else {
        return tab_row(tabs, selected, palette, None, None);
    };
    let count = tabs.tabs.len().max(1);
    let cols = tab_columns(inner, count);
    let cell_w = ((inner - TAB_GAP * cols.saturating_sub(1) as f32) / cols as f32).max(TAB_MIN);
    let content_w = (cell_w - 8.0).max(text::CELL_W);
    let mut strip = Node::new(tabs.id);
    strip.direction = Direction::TopToBottom;
    strip.gap = TAB_GAP;
    strip.width = Sizing::Fixed(inner);
    for (row_index, row_tabs) in tabs.tabs.chunks(cols).enumerate() {
        let row_h = row_tabs
            .iter()
            .map(|tab| text::measure(&tab.title, Some(content_w)).1 + 8.0)
            .fold(text::CELL_H + 8.0, f32::max);
        let mut row = Node::new(tabs.id.wrapping_mul(16).wrapping_add(row_index as u32 + 3));
        row.direction = Direction::LeftToRight;
        row.gap = TAB_GAP;
        row.width = Sizing::Fixed(inner);
        row.height = Sizing::Fixed(row_h);
        for tab in row_tabs {
            row.children.push(tab_button(
                tab,
                Some(tab.id) == selected,
                palette,
                cell_w,
                row_h,
            ));
        }
        strip.children.push(row);
    }
    strip
}

fn tab_row(
    tabs: &PanelTabs,
    selected: Option<u32>,
    palette: Option<Palette>,
    cell_w: Option<f32>,
    cell_h: Option<f32>,
) -> Node {
    let mut row = Node::new(tabs.id);
    row.direction = Direction::LeftToRight;
    row.gap = TAB_GAP;
    for tab in &tabs.tabs {
        let mut node = tab_button(
            tab,
            Some(tab.id) == selected,
            palette,
            cell_w.unwrap_or(0.0),
            cell_h.unwrap_or(0.0),
        );
        if cell_w.is_none() {
            node.width = Sizing::fit();
            node.height = Sizing::fit();
        }
        row.children.push(node);
    }
    row
}

fn tab_button(
    tab: &PanelTab,
    selected: bool,
    palette: Option<Palette>,
    width: f32,
    height: f32,
) -> Node {
    let mut node = control(tab.id, &tab.title, Action::Press(tab.id), palette);
    if width > 0.0 {
        node.width = Sizing::Fixed(width);
        node.height = Sizing::Fixed(height);
    }
    if selected {
        let looks = control_looks(palette);
        node.idle = looks.pressed;
        node.hover = looks.pressed;
        node.pressed = looks.pressed;
    }
    node
}

/// Arrow keys step a focused slider. Otherwise they walk checks, sliders,
/// dropdowns, and press targets from top to bottom, then left to right.
fn step_or_move_focus(
    state: &mut State,
    items: &[Item],
    page: &PageScroll,
    bars: &[&PanelSlider],
    drops: &[&PanelDropdown],
    checks: &[&PanelCheck],
    pointer: Pointer,
    actions: &mut Vec<Action>,
) -> bool {
    if !(pointer.key_dec || pointer.key_inc) || pointer.key_dec == pointer.key_inc {
        return false;
    }
    if let Some(bar) = state
        .focus
        .and_then(|id| bars.iter().copied().find(|bar| bar.id == id))
    {
        let delta = i32::from(pointer.key_inc) - i32::from(pointer.key_dec);
        push_step(actions, bar, delta);
        return true;
    }
    let ids = focusable_ids(items, page, bars, drops, checks);
    if ids.is_empty() {
        return false;
    }
    let forward = pointer.key_inc;
    let next = match state
        .focus
        .and_then(|id| ids.iter().position(|item| *item == id))
    {
        Some(index) if forward => ids[(index + 1) % ids.len()],
        Some(index) => ids[(index + ids.len() - 1) % ids.len()],
        None if forward => ids[0],
        None => ids[ids.len() - 1],
    };
    state.focus = Some(next);
    true
}

fn focusable_ids(
    items: &[Item],
    page: &PageScroll,
    bars: &[&PanelSlider],
    drops: &[&PanelDropdown],
    checks: &[&PanelCheck],
) -> Vec<u32> {
    let mut ranked = Vec::new();
    for item in items {
        let rect = page.hit_rect(item);
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        let widget = bars.iter().any(|bar| bar.id == item.id)
            || drops.iter().any(|drop| drop.id == item.id)
            || checks.iter().any(|check| check.id == item.id)
            || matches!(
                item.action,
                Some(Action::Press(_)) | Some(Action::Toggle { .. })
            );
        if widget {
            ranked.push((rect.y, rect.x, item.id));
        }
    }
    ranked.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    ranked.into_iter().map(|(_, _, id)| id).collect()
}

/// Enter opens a focused dropdown or fires the focused control's action.
fn activate_focused(
    state: &mut State,
    items: &[Item],
    tab_ids: &[(u32, u32)],
    section_ids: &[u32],
    drops: &[&PanelDropdown],
    actions: &mut Vec<Action>,
) -> bool {
    let Some(id) = state.focus else {
        return false;
    };
    if let Some(drop) = drops.iter().find(|drop| drop.id == id) {
        open_dropdown(state, drop);
        return true;
    }
    let Some(item) = items.iter().find(|item| item.id == id) else {
        return false;
    };
    let Some(action) = item.action else {
        return false;
    };
    if let Action::Press(press_id) = action {
        if let Some((group, _)) = tab_ids.iter().find(|(_, tab)| *tab == press_id) {
            state.select_tab(*group, press_id);
        } else if section_ids.contains(&press_id) {
            state.toggle_section(press_id);
        }
    }
    actions.push(action);
    true
}

/// Escape or grave leaves click-to-focus. Escape that just closed a list does not.
fn release_click_focus(
    state: &mut State,
    panel_id: u32,
    policy: Option<InputPolicy>,
    keys_open: bool,
    pointer: Pointer,
    list_closed: bool,
    consumed: &mut Consumed,
) {
    let click = policy.is_some_and(|policy| policy.keys == FocusPolicy::ClickToFocus);
    if !(keys_open && click && state.input.focused() == Some(panel_id)) {
        return;
    }
    let grave = pointer.key_blur;
    let escape = pointer.key_dismiss && !list_closed && state.open_drop.is_none();
    if !(grave || escape) {
        return;
    }
    if grave {
        state.open_drop = None;
    }
    state.input.set_focus(None);
    state.focus = None;
    consumed.blur = true;
    consumed.keys = true;
}

fn cycle_tab(
    state: &mut State,
    tab_ids: &[(u32, u32)],
    items: &[Item],
    pointer: &Pointer,
) -> Option<Action> {
    let dir = if pointer.tab_next {
        1isize
    } else if pointer.tab_prev {
        -1
    } else {
        return None;
    };
    let mut groups: Vec<(u32, Vec<u32>)> = Vec::new();
    for (group, tab) in tab_ids {
        if let Some((_, tabs)) = groups.iter_mut().find(|(id, _)| id == group) {
            if !tabs.contains(tab) {
                tabs.push(*tab);
            }
        } else {
            groups.push((*group, vec![*tab]));
        }
    }
    let hit = items
        .iter()
        .rev()
        .find(|item| item.rect.contains(pointer.x, pointer.y));
    let over_header = hit.and_then(|item| tab_ids.iter().find(|(_, tab)| *tab == item.id));
    let (group, tabs) = if let Some((group, _)) = over_header {
        groups.into_iter().find(|(id, _)| id == group)?
    } else {
        groups.into_iter().next()?
    };
    if tabs.len() < 2 {
        return None;
    }
    let current = state.selected_tab(group).unwrap_or(tabs[0]);
    let index = tabs.iter().position(|id| *id == current).unwrap_or(0);
    let next = if dir > 0 {
        tabs[(index + 1) % tabs.len()]
    } else {
        tabs[(index + tabs.len() - 1) % tabs.len()]
    };
    state.select_tab(group, next);
    Some(Action::Press(next))
}

fn check_columns(inner: f32) -> usize {
    let cols = ((inner + 4.0) / (CHECK_CELL + 4.0)).floor() as usize;
    cols.clamp(2, CHECK_COLUMNS)
}

fn check_cell(inner: f32, columns: usize) -> f32 {
    let gaps = 4.0 * columns.saturating_sub(1) as f32;
    ((inner - gaps) / columns as f32).max(text::CELL_W)
}

fn title_bar(
    label_id: u32,
    title: &str,
    fill: [f32; 3],
    inner: Option<f32>,
    palette: Option<Palette>,
    hide: bool,
) -> Node {
    if !hide {
        return capped_label(label_id, title, fill, inner);
    }
    let mut row = Node::new(TITLE_ROW);
    row.direction = Direction::LeftToRight;
    row.gap = 4.0;
    if let Some(inner) = inner {
        row.width = Sizing::Fit {
            min: 0.0,
            max: inner,
        };
    }
    let label_max = inner.map(|width| (width - 64.0).max(text::CELL_W));
    row.children
        .push(capped_label(label_id, title, fill, label_max));
    row.children.push(control(
        MENU_HIDE,
        "hide",
        Action::Press(MENU_HIDE),
        palette,
    ));
    row
}

/// A small tab in the top-left corner. A press fires [`Action::Press`] with [`MENU_SHOW`].
pub fn show_menu_tab(state: &mut State, viewport: [f32; 2], pointer: Pointer) -> Frame {
    refresh_palette(state);
    let mut tab = control(MENU_SHOW, "menu", Action::Press(MENU_SHOW), state.palette);
    tab.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [8.0, 8.0]));
    let items = layout::layout(&tab, Space::Screen, viewport, None);
    let press_edge = pointer.down && !state.down;
    let hit = items
        .iter()
        .rev()
        .find(|item| item.rect.contains(pointer.x, pointer.y));
    let mut actions = Vec::new();
    let mut look_capture = false;
    if press_edge {
        if let Some(item) = hit {
            state.look_hold = false;
            if let Some(action) = item.action {
                actions.push(action);
            }
        } else {
            state.look_hold = true;
            look_capture = true;
        }
    } else if pointer.down && state.look_hold {
        look_capture = true;
    }
    let mut shown = Vec::new();
    let mut paints = Vec::new();
    for item in &items {
        let look = choose(item, pointer.down, hit.map(|hit| hit.id), None);
        shown.push(Shown {
            id: item.id,
            rect: item.rect,
            look,
            idle: item.idle,
            text: item.text.clone(),
        });
        push_paints(&mut paints, item, look, state.palette, None, false);
    }
    state.pointer_x = pointer.x;
    state.pointer_y = pointer.y;
    state.down = pointer.down;
    state.panel_hover = false;
    if !pointer.down {
        state.look_hold = false;
    }
    Frame {
        shown,
        paints,
        actions,
        look_capture,
        tooltip: None,
        consumed: Consumed::default(),
    }
}

fn section_header(
    section: &PanelSection,
    state: &State,
    palette: Option<Palette>,
    max_width: Option<f32>,
) -> Node {
    let mark = if state.section_collapsed(section.id) {
        "+"
    } else {
        "-"
    };
    let mut node = control(
        section.id,
        &format!("{mark} {}", section.title),
        Action::Press(section.id),
        palette,
    );
    if let Some(max_width) = max_width {
        node.width = Sizing::Fit {
            min: 0.0,
            max: max_width,
        };
    }
    node
}

fn row_node(
    at: u32,
    item: &PanelRow,
    fill: [f32; 3],
    palette: Option<Palette>,
    cell: Option<f32>,
    inner: Option<f32>,
) -> Node {
    let fit = inner.map(|inner| fit_row(item, inner));
    let mut children = Vec::new();
    if !item.label.is_empty() {
        children.push(capped_label(
            at + 1,
            &item.label,
            fill,
            fit.as_ref().map(|fit| fit.label_max),
        ));
    }
    for button in &item.buttons {
        let mut node = control(button.id, &button.text, Action::Press(button.id), palette);
        if button.selected {
            let looks = control_looks(palette);
            node.idle = looks.pressed;
            node.hover = looks.pressed;
        }
        children.push(node);
    }
    if let Some(bar) = &item.slider {
        let width = fit
            .as_ref()
            .map(|fit| fit.slider_w)
            .unwrap_or(slider_width(bar));
        children.push(sized_slider(
            bar.id,
            palette,
            width,
            slider_height_at(bar, width),
        ));
        children.push(capped_label(
            at + 2,
            &bar.text,
            fill,
            fit.as_ref().map(|fit| fit.value_max),
        ));
    }
    if let Some(drop) = &item.dropdown {
        children.push(dropdown_box(
            drop,
            palette,
            fit.as_ref().map(|fit| fit.drop_w),
        ));
    }
    for check in &item.checks {
        children.push(check_node(check, fill, palette, cell));
    }
    let mut node = row(at, children, fill);
    if let Some(inner) = inner {
        node.width = Sizing::Fit {
            min: 0.0,
            max: inner,
        };
    }
    node
}

struct RowFit {
    label_max: f32,
    slider_w: f32,
    value_max: f32,
    drop_w: f32,
}

fn fit_row(item: &PanelRow, inner: f32) -> RowFit {
    let gap = 4.0;
    let has_label = !item.label.is_empty();
    let has_value = item.slider.as_ref().is_some_and(|bar| !bar.text.is_empty());
    let parts = usize::from(has_label)
        + usize::from(item.slider.is_some())
        + usize::from(has_value)
        + usize::from(item.dropdown.is_some())
        + item.buttons.len()
        + item.checks.len();
    let gaps = gap * parts.saturating_sub(1) as f32;
    let value_natural = item
        .slider
        .as_ref()
        .map(|bar| text::measure(&bar.text, None).0)
        .unwrap_or(0.0);
    let value_max = if has_value {
        value_natural.min(inner * 0.4).max(text::CELL_W)
    } else {
        0.0
    };
    let mut slider_w = item.slider.as_ref().map(slider_width).unwrap_or(0.0);
    let mut drop_w = item.dropdown.as_ref().map(dropdown_width).unwrap_or(0.0);
    let label_natural = if has_label {
        text::measure(&item.label, None).0
    } else {
        0.0
    };
    let checks_w: f32 = item
        .checks
        .iter()
        .map(|check| text::measure(&check.text, None).0 + 30.0)
        .sum();
    let buttons_w: f32 = item
        .buttons
        .iter()
        .map(|button| text::measure(&button.text, None).0 + 8.0)
        .sum();
    let used = label_natural + slider_w + drop_w + value_max + checks_w + buttons_w + gaps;
    if used <= inner {
        return RowFit {
            label_max: label_natural.max(text::CELL_W),
            slider_w,
            value_max: value_natural.max(if has_value { text::CELL_W } else { 0.0 }),
            drop_w,
        };
    }
    let label_floor = if has_label {
        (text::CELL_W * 8.0).min(inner * 0.45)
    } else {
        0.0
    };
    let controls = (inner - label_floor - value_max - checks_w - buttons_w - gaps).max(72.0);
    if item.slider.is_some() && item.dropdown.is_some() {
        slider_w = (controls * 0.55).clamp(72.0, slider_w.max(72.0));
        drop_w = (controls - slider_w).max(96.0);
    } else if item.slider.is_some() {
        slider_w = controls.min(slider_w).max(72.0);
    } else if item.dropdown.is_some() {
        drop_w = controls.min(drop_w.max(96.0)).max(96.0);
    }
    let mut label_max =
        (inner - slider_w - drop_w - value_max - checks_w - buttons_w - gaps).max(text::CELL_W);
    let mut sum = label_max + slider_w + drop_w + value_max + checks_w + buttons_w + gaps;
    if sum > inner && slider_w > 48.0 {
        let cut = (sum - inner).min(slider_w - 48.0);
        slider_w -= cut;
        sum -= cut;
    }
    if sum > inner && drop_w > 96.0 {
        let cut = (sum - inner).min(drop_w - 96.0);
        drop_w -= cut;
        sum -= cut;
    }
    if sum > inner {
        label_max = (label_max - (sum - inner)).max(text::CELL_W);
    }
    RowFit {
        label_max,
        slider_w,
        value_max,
        drop_w,
    }
}

fn check_grid(item: &PanelRow) -> bool {
    !item.checks.is_empty()
        && item.buttons.is_empty()
        && item.slider.is_none()
        && item.dropdown.is_none()
        && item.label.is_empty()
}

fn capped_label(id: u32, text: &str, fill: [f32; 3], max_width: Option<f32>) -> Node {
    let mut node = label(id, text, fill);
    if let Some(max_width) = max_width {
        node.width = Sizing::Fit {
            min: 0.0,
            max: max_width,
        };
    }
    node
}

struct PageScroll {
    clip: Option<Rect>,
    body: BTreeSet<u32>,
}

impl PageScroll {
    fn hit_rect(&self, item: &Item) -> Rect {
        let Some(clip) = self.clip else {
            return item.rect;
        };
        if !self.body.contains(&item.id) {
            return item.rect;
        }
        intersect(item.rect, clip).unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        })
    }
}

/// One wheel notch moves the page by this many pixels.
const SCROLL_STEP: f32 = 48.0;

fn scroll_page(
    state: &mut State,
    items: &mut [Item],
    panel_id: u32,
    tab_ids: &[(u32, u32)],
    bar_ids: &[u32],
    viewport_h: f32,
    pointer: Pointer,
) -> PageScroll {
    let mut strips = Vec::new();
    for (group, _) in tab_ids {
        if !strips.contains(group) {
            strips.push(*group);
        }
    }
    let Some(panel_index) = items.iter().position(|item| item.id == panel_id) else {
        return PageScroll {
            clip: None,
            body: BTreeSet::new(),
        };
    };
    let panel_rect = items[panel_index].rect;
    let view_bottom = viewport_h - 8.0;
    if panel_rect.y + panel_rect.h <= view_bottom + 0.5 {
        state.scroll = 0.0;
        return PageScroll {
            clip: None,
            body: BTreeSet::new(),
        };
    }
    let mut body = BTreeSet::new();
    let mut sticky_bottom = panel_rect.y;
    let mut content_top = f32::MAX;
    let mut content_bottom = panel_rect.y;
    for item in items.iter() {
        if item.id == panel_id {
            continue;
        }
        if sticky_item(items, item.id, panel_id, &strips) {
            sticky_bottom = sticky_bottom.max(item.rect.y + item.rect.h);
        } else {
            body.insert(item.id);
            content_top = content_top.min(item.rect.y);
            content_bottom = content_bottom.max(item.rect.y + item.rect.h);
        }
    }
    if body.is_empty() || content_top == f32::MAX {
        state.scroll = 0.0;
        return PageScroll {
            clip: None,
            body: BTreeSet::new(),
        };
    }
    let clip = Rect {
        x: panel_rect.x,
        y: content_top,
        w: panel_rect.w,
        h: (view_bottom - content_top).max(32.0),
    };
    let max_scroll = (content_bottom - content_top - clip.h).max(0.0);
    let over_panel = panel_rect.contains(pointer.x, pointer.y);
    let over_slider = items
        .iter()
        .any(|item| bar_ids.contains(&item.id) && item.rect.contains(pointer.x, pointer.y));
    if max_scroll > 0.0
        && over_panel
        && !over_slider
        && state.open_drop.is_none()
        && pointer.wheel_y != 0.0
    {
        state.scroll -= pointer.wheel_y * SCROLL_STEP;
    }
    state.scroll = state.scroll.clamp(0.0, max_scroll);
    let scroll = state.scroll;
    for item in items.iter_mut() {
        if !body.contains(&item.id) {
            continue;
        }
        item.rect.y -= scroll;
        item.content.y -= scroll;
    }
    if let Some(panel) = items.iter_mut().find(|item| item.id == panel_id) {
        let bottom = view_bottom.max(clip.y + clip.h);
        panel.rect.h = (bottom - panel.rect.y).max(0.0);
    }
    PageScroll {
        clip: Some(clip),
        body,
    }
}

fn sticky_item(items: &[Item], id: u32, panel_id: u32, strips: &[u32]) -> bool {
    if id == panel_id {
        return true;
    }
    let mut current = Some(id);
    for _ in 0..64 {
        let Some(now) = current else {
            return false;
        };
        if now == panel_id {
            return false;
        }
        if strips.contains(&now) || now == TITLE_ROW || now == panel_id.wrapping_add(1) {
            return true;
        }
        current = items
            .iter()
            .find(|item| item.id == now)
            .and_then(|item| item.parent);
    }
    false
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some(Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }
}

fn finish_panel(
    state: &mut State,
    panel: Node,
    rows: &[PanelRow],
    section_ids: &[u32],
    tab_ids: &[(u32, u32)],
    viewport: [f32; 2],
    camera: &Camera,
    pointer: Pointer,
) -> Frame {
    let panel_id = panel.id;
    let press_edge = pointer.down && !state.down;
    let palette = state.palette;
    let bars: Vec<&PanelSlider> = rows.iter().filter_map(|r| r.slider.as_ref()).collect();
    let drops: Vec<&PanelDropdown> = rows.iter().filter_map(|r| r.dropdown.as_ref()).collect();
    let checks: Vec<&PanelCheck> = rows.iter().flat_map(|row| row.checks.iter()).collect();
    let mut items = layout::layout(&panel, Space::Screen, viewport, Some(camera));
    let bar_ids: Vec<u32> = bars.iter().map(|bar| bar.id).collect();
    let page = scroll_page(
        state,
        &mut items,
        panel_id,
        tab_ids,
        &bar_ids,
        viewport[1],
        pointer,
    );
    if state
        .open_drop
        .is_some_and(|id| drops.iter().all(|drop| drop.id != id))
    {
        state.open_drop = None;
    }
    let hit = items
        .iter()
        .rev()
        .find(|item| page.hit_rect(item).contains(pointer.x, pointer.y));
    let mut actions = Vec::new();
    let mut look_capture = false;
    let mut took_press = false;
    let mut consumed = Consumed::default();
    let over = hit.is_some();
    let policy = state.input.policy(panel_id);
    let legacy = policy.is_none();
    let keys_open = legacy || state.input.allows(panel_id, InputKind::Keys, over);
    if press_edge {
        if let Some(policy) = policy {
            if policy.keys == FocusPolicy::ClickToFocus {
                if over {
                    state.input.set_focus(Some(panel_id));
                } else if state.input.focused() == Some(panel_id) {
                    state.input.set_focus(None);
                    state.focus = None;
                }
            }
        }
    }
    let pointer_open = legacy
        || match policy.map(|policy| policy.keys) {
            Some(FocusPolicy::Passive) => false,
            Some(FocusPolicy::Hover) => over,
            Some(FocusPolicy::Modal) | Some(FocusPolicy::ClickToFocus) => true,
            None => true,
        };
    if press_edge && !pointer_open {
        look_capture = true;
        state.look_hold = true;
    } else if press_edge && policy.is_some_and(|policy| policy.keys == FocusPolicy::Modal) && !over
    {
        consumed.pointer = true;
        took_press = true;
    } else if press_edge {
        if let Some((drop, list, _)) = popup_geom(state, &drops, &items, viewport) {
            if let Some(index) = dropdown::row_at(
                list,
                state.drop_scroll,
                drop.choices.len(),
                pointer.x,
                pointer.y,
            ) {
                if let Some(choice) = drop.choices.get(index) {
                    actions.push(Action::Select {
                        id: drop.id,
                        value: choice.value,
                    });
                }
                state.open_drop = None;
            } else if hit.map(|item| item.id) == Some(drop.id) {
                state.open_drop = None;
            } else {
                state.open_drop = None;
            }
            state.dragging = false;
            state.look_hold = false;
            state.held = None;
            state.focus = Some(drop.id);
            took_press = true;
            if !legacy {
                consumed.pointer = true;
            }
        }
    }
    if !took_press && press_edge && pointer_open {
        if let Some(item) = hit {
            if !legacy {
                consumed.pointer = true;
            }
            state.focus = Some(item.id);
            state.look_hold = false;
            state.sliding = None;
            state.held = None;
            if let Some(drop) = drops.iter().find(|drop| drop.id == item.id) {
                open_dropdown(state, drop);
                state.dragging = false;
            } else if bars.iter().any(|b| b.id == item.id) {
                state.held = Some(item.id);
                state.dragging = false;
            } else if let Some(action) = item.action {
                if let Action::Press(id) = action {
                    if let Some((group, _)) = tab_ids.iter().find(|(_, tab)| *tab == id) {
                        state.select_tab(*group, id);
                    } else if section_ids.contains(&id) {
                        state.toggle_section(id);
                    }
                }
                actions.push(action);
                state.dragging = false;
            } else {
                state.dragging = draggable(&items, item.id);
            }
        } else {
            state.focus = None;
            state.open_drop = None;
            state.dragging = false;
            state.sliding = None;
            state.look_hold = true;
            look_capture = true;
        }
    } else if pointer.down && state.look_hold {
        look_capture = true;
    }
    let mut list_closed = false;
    if pointer.down {
        let held = state.held.and_then(|id| bars.iter().find(|b| b.id == id));
        let track = held.and_then(|b| items.iter().find(|i| i.id == b.id));
        if let (Some(bar), Some(track)) = (held, track) {
            let value = bar.value_at(track.rect, pointer.x);
            if value != bar.value {
                actions.push(Action::Slide { id: bar.id, value });
            }
        }
    } else if state.open_drop.is_some() {
        let mut drive = pointer;
        if !keys_open {
            drive.key_dec = false;
            drive.key_inc = false;
            drive.key_activate = false;
            drive.key_dismiss = false;
        }
        let was_open = state.open_drop;
        drive_open_dropdown(state, &drops, &items, viewport, drive, &mut actions);
        list_closed = was_open.is_some() && state.open_drop.is_none() && pointer.key_dismiss;
        if keys_open && !legacy && (pointer.key_dec || pointer.key_inc || pointer.key_activate || list_closed)
        {
            consumed.keys = true;
        }
    } else {
        let hovered = hit.and_then(|item| bars.iter().copied().find(|bar| bar.id == item.id));
        if let Some(bar) = hovered {
            let notches = wheel_steps(pointer.wheel_y);
            if notches != 0 {
                state.focus = Some(bar.id);
                push_step(&mut actions, bar, notches);
            }
        }
        if keys_open && legacy {
            if let Some(action) = cycle_tab(state, tab_ids, &items, &pointer) {
                actions.push(action);
            }
            if pointer.key_dec || pointer.key_inc {
                let keyed = hovered.or_else(|| {
                    state
                        .focus
                        .and_then(|id| bars.iter().copied().find(|bar| bar.id == id))
                });
                if let Some(bar) = keyed {
                    let delta = i32::from(pointer.key_inc) - i32::from(pointer.key_dec);
                    state.focus = Some(bar.id);
                    push_step(&mut actions, bar, delta);
                }
            }
            if pointer.key_activate {
                if let Some(drop) = state
                    .focus
                    .and_then(|id| drops.iter().find(|drop| drop.id == id))
                {
                    open_dropdown(state, drop);
                }
            }
        } else if keys_open {
            if let Some(action) = cycle_tab(state, tab_ids, &items, &pointer) {
                actions.push(action);
                consumed.keys = true;
            }
            if step_or_move_focus(state, &items, &page, &bars, &drops, &checks, pointer, &mut actions)
            {
                consumed.keys = true;
            }
            if pointer.key_activate
                && activate_focused(state, &items, tab_ids, section_ids, &drops, &mut actions)
            {
                consumed.keys = true;
            }
        }
    }
    release_click_focus(
        state,
        panel_id,
        policy,
        keys_open,
        pointer,
        list_closed,
        &mut consumed,
    );
    if keys_open && policy.is_some_and(|policy| policy.keys == FocusPolicy::Modal) {
        let key_edge = pointer.key_dec
            || pointer.key_inc
            || pointer.key_activate
            || pointer.key_dismiss
            || pointer.key_blur
            || pointer.tab_prev
            || pointer.tab_next;
        if key_edge {
            consumed.keys = true;
            if pointer.key_blur {
                consumed.blur = true;
            }
        }
    }
    let mut shown = Vec::with_capacity(items.len());
    let mut paints = Vec::new();
    for item in &items {
        let visible = page.hit_rect(item);
        if visible.h <= 0.0 || visible.w <= 0.0 {
            continue;
        }
        let look = choose(item, pointer.down, hit.map(|hit| hit.id), state.focus);
        shown.push(Shown {
            id: item.id,
            rect: visible,
            look,
            idle: item.idle,
            text: item.text.clone(),
        });
        let mut part = Vec::new();
        push_paints(&mut part, item, look, palette, None, false);
        if let Some(bar) = bars.iter().find(|b| b.id == item.id) {
            // Show where a held slider is going this frame.
            push_bar(&mut part, item.rect, &shown_slider(bar, &actions), palette);
        }
        if drops.iter().any(|drop| drop.id == item.id) {
            push_chevron(&mut part, item.rect, state.open_drop == Some(item.id));
        }
        if let Some(check) = checks.iter().find(|check| check.id == item.id) {
            push_check(&mut part, item.rect, check.on);
        }
        if page.body.contains(&item.id) {
            if let Some(clip) = page.clip {
                for paint in &mut part {
                    match intersect(
                        Rect {
                            x: paint.x,
                            y: paint.y,
                            w: paint.w,
                            h: paint.h,
                        },
                        clip,
                    ) {
                        Some(rect) => {
                            paint.x = rect.x;
                            paint.y = rect.y;
                            paint.w = rect.w;
                            paint.h = rect.h;
                        }
                        None => {
                            paint.w = 0.0;
                            paint.h = 0.0;
                        }
                    }
                }
            }
        }
        paints.extend(
            part.into_iter()
                .filter(|paint| paint.w > 0.0 && paint.h > 0.0),
        );
    }
    if let Some((drop, list, scroll)) = popup_geom(state, &drops, &items, viewport) {
        state.drop_scroll = scroll;
        push_popup(
            &mut paints,
            &mut shown,
            drop,
            list,
            scroll,
            state.drop_highlight,
        );
    }
    let mut tooltip = None;
    if let Some(item) = hit {
        if let Some(bar) = bars.iter().find(|b| b.id == item.id) {
            let shown_bar = shown_slider(bar, &actions);
            let text = shown_bar.tip();
            let band = thumb_band(item.rect, !shown_bar.steps.is_empty());
            let (left, right) = track_ends(band);
            let x = left + (right - left) * shown_bar.share();
            push_tip(&mut paints, viewport, x, band.y, &text);
            tooltip = Some(text);
        }
    }
    state.pointer_x = pointer.x;
    state.pointer_y = pointer.y;
    state.down = pointer.down;
    state.panel_hover = hit.is_some();
    if !pointer.down {
        state.dragging = false;
        state.look_hold = false;
        state.held = None;
    }
    Frame {
        shown,
        paints,
        actions,
        look_capture,
        tooltip,
        consumed,
    }
}

const POPUP_ID: u32 = u32::MAX - 8192;
const LIST_BG: [f32; 3] = [0.10, 0.11, 0.14];
const LIST_HOVER: [f32; 3] = [0.30, 0.38, 0.48];
const LIST_BORDER: [f32; 3] = [0.55, 0.62, 0.72];

/// Equal column width for on/off marks in a section. The label wraps inside it.
const CHECK_CELL: f32 = 156.0;
const CHECK_COLUMNS: usize = 3;
const CHECK_BOX: f32 = 16.0;
const CHECK_MARK: [f32; 3] = [0.62, 0.78, 0.42];

fn check_node(
    check: &PanelCheck,
    fill: [f32; 3],
    palette: Option<Palette>,
    cell: Option<f32>,
) -> Node {
    let mut node = Node::new(check.id);
    node.pad = Pad {
        left: 22.0,
        right: 8.0,
        top: 2.0,
        bottom: 2.0,
    };
    if let Some(cell) = cell {
        node.width = Sizing::fixed(cell);
    }
    node.text = Some(check.text.clone());
    node.action = Some(Action::Toggle {
        id: check.id,
        on: !check.on,
    });
    let mut looks = flat_look(fill);
    let accent = palette
        .map(|palette| palette.accent)
        .unwrap_or([0.55, 0.65, 0.75]);
    looks.hover.border = accent;
    looks.pressed.border = accent;
    paint_looks(&mut node, looks);
    node
}

fn push_check(out: &mut Vec<Paint>, rect: Rect, on: bool) {
    let box_rect = Rect {
        x: rect.x + 2.0,
        y: rect.y + (rect.h - CHECK_BOX) * 0.5,
        w: CHECK_BOX,
        h: CHECK_BOX,
    };
    push_border(out, box_rect, LIST_BORDER);
    if on {
        push_rect(
            out,
            Rect {
                x: box_rect.x + 4.0,
                y: box_rect.y + 4.0,
                w: CHECK_BOX - 8.0,
                h: CHECK_BOX - 8.0,
            },
            CHECK_MARK,
        );
    }
}

fn dropdown_box(drop: &PanelDropdown, palette: Option<Palette>, max_width: Option<f32>) -> Node {
    let mut width = dropdown_width(drop);
    if let Some(max_width) = max_width {
        width = width.min(max_width).max(96.0);
    }
    let mut node = Node::new(drop.id);
    node.pad = Pad {
        left: 6.0,
        right: 16.0,
        top: 4.0,
        bottom: 4.0,
    };
    node.text = Some(drop.selected_name());
    node.width = Sizing::fixed(width);
    paint_looks(&mut node, control_looks(palette));
    node
}

fn dropdown_width(drop: &PanelDropdown) -> f32 {
    let text = drop
        .choices
        .iter()
        .map(|choice| text::measure(&choice.name, None).0)
        .fold(0.0_f32, f32::max);
    (text + 22.0).clamp(160.0, 320.0)
}

fn open_dropdown(state: &mut State, drop: &PanelDropdown) {
    let index = drop.selected_index();
    let visible = drop.choices.len().min(dropdown::VISIBLE);
    state.open_drop = Some(drop.id);
    state.drop_highlight = index;
    state.drop_scroll = dropdown::reveal(0, index, visible, drop.choices.len());
    state.focus = Some(drop.id);
}

fn popup_geom<'a>(
    state: &State,
    drops: &[&'a PanelDropdown],
    items: &[Item],
    viewport: [f32; 2],
) -> Option<(&'a PanelDropdown, Rect, usize)> {
    let id = state.open_drop?;
    let drop = drops.iter().find(|drop| drop.id == id)?;
    let anchor = items.iter().find(|item| item.id == id)?.rect;
    let (visible, scroll) = dropdown::window(drop.choices.len(), state.drop_scroll);
    if visible == 0 {
        return None;
    }
    let list = dropdown::place(anchor, visible, viewport[1]);
    Some((drop, list, scroll))
}

fn drive_open_dropdown(
    state: &mut State,
    drops: &[&PanelDropdown],
    items: &[Item],
    viewport: [f32; 2],
    pointer: Pointer,
    actions: &mut Vec<Action>,
) {
    let Some((drop, list, scroll)) = popup_geom(state, drops, items, viewport) else {
        return;
    };
    state.drop_scroll = scroll;
    if pointer.key_dismiss {
        state.open_drop = None;
        return;
    }
    let count = drop.choices.len();
    let (visible, _) = dropdown::window(count, state.drop_scroll);
    let delta = i32::from(pointer.key_dec) - i32::from(pointer.key_inc);
    if delta != 0 {
        state.drop_highlight = dropdown::move_highlight(state.drop_highlight, delta, count);
        state.drop_scroll =
            dropdown::reveal(state.drop_scroll, state.drop_highlight, visible, count);
    }
    let on_list = list.contains(pointer.x, pointer.y);
    let on_box = items
        .iter()
        .find(|item| item.id == drop.id)
        .is_some_and(|item| item.rect.contains(pointer.x, pointer.y));
    if on_list || on_box {
        let notches = wheel_steps(pointer.wheel_y);
        if notches != 0 {
            state.drop_scroll = dropdown::scroll_by(state.drop_scroll, notches, visible, count);
        }
    }
    if let Some(index) = dropdown::row_at(list, state.drop_scroll, count, pointer.x, pointer.y) {
        state.drop_highlight = index;
    }
    if pointer.key_activate {
        if let Some(choice) = drop.choices.get(state.drop_highlight) {
            actions.push(Action::Select {
                id: drop.id,
                value: choice.value,
            });
            state.open_drop = None;
        }
    }
}

fn push_chevron(out: &mut Vec<Paint>, rect: Rect, open: bool) {
    let x = rect.x + rect.w - 12.0;
    let mid = rect.y + rect.h * 0.5;
    if open {
        push_rect(
            out,
            Rect {
                x: x + 3.0,
                y: mid - 5.0,
                w: 2.0,
                h: 2.0,
            },
            TEXT,
        );
        push_rect(
            out,
            Rect {
                x: x + 2.0,
                y: mid - 3.0,
                w: 4.0,
                h: 2.0,
            },
            TEXT,
        );
        push_rect(
            out,
            Rect {
                x,
                y: mid - 1.0,
                w: 8.0,
                h: 2.0,
            },
            TEXT,
        );
    } else {
        push_rect(
            out,
            Rect {
                x,
                y: mid - 1.0,
                w: 8.0,
                h: 2.0,
            },
            TEXT,
        );
        push_rect(
            out,
            Rect {
                x: x + 2.0,
                y: mid + 1.0,
                w: 4.0,
                h: 2.0,
            },
            TEXT,
        );
        push_rect(
            out,
            Rect {
                x: x + 3.0,
                y: mid + 3.0,
                w: 2.0,
                h: 2.0,
            },
            TEXT,
        );
    }
}

fn push_popup(
    paints: &mut Vec<Paint>,
    shown: &mut Vec<Shown>,
    drop: &PanelDropdown,
    list: Rect,
    scroll: usize,
    highlight: usize,
) {
    push_rect(paints, list, LIST_BG);
    push_border(paints, list, LIST_BORDER);
    let (visible, scroll) = dropdown::window(drop.choices.len(), scroll);
    for row in 0..visible {
        let index = scroll + row;
        let Some(choice) = drop.choices.get(index) else {
            break;
        };
        let rect = Rect {
            x: list.x + 1.0,
            y: list.y + row as f32 * dropdown::ROW_H,
            w: (list.w - 2.0).max(0.0),
            h: dropdown::ROW_H,
        };
        let hot = index == highlight;
        let background = if hot { LIST_HOVER } else { LIST_BG };
        if hot {
            push_rect(paints, rect, background);
        }
        for blot in text::blots(&choice.name, rect.x + 6.0, rect.y + 2.0, rect.w - 8.0) {
            push_blot(paints, blot, TEXT);
        }
        let look = Look {
            background,
            border: LIST_BORDER,
        };
        shown.push(Shown {
            id: POPUP_ID + index as u32,
            rect,
            look,
            idle: Look {
                background: LIST_BG,
                border: LIST_BORDER,
            },
            text: Some(choice.name.clone()),
        });
    }
}

fn shown_slider(bar: &PanelSlider, actions: &[Action]) -> PanelSlider {
    let value = actions.iter().rev().find_map(|action| match action {
        Action::Slide { id, value } if *id == bar.id => Some(*value),
        _ => None,
    });
    bar.at(value.unwrap_or(bar.value))
}

fn push_step(actions: &mut Vec<Action>, bar: &PanelSlider, delta: i32) {
    if delta == 0 {
        return;
    }
    let current = actions
        .iter()
        .rev()
        .find_map(|action| match action {
            Action::Slide { id, value } if *id == bar.id => Some(*value),
            _ => None,
        })
        .unwrap_or(bar.value);
    let value = bar.step_from(current, delta);
    if value != current {
        actions.push(Action::Slide { id: bar.id, value });
    }
}

fn wheel_steps(wheel_y: f32) -> i32 {
    if wheel_y.abs() < 1.0e-4 {
        0
    } else {
        let steps = wheel_y.round() as i32;
        if steps == 0 {
            if wheel_y > 0.0 {
                1
            } else {
                -1
            }
        } else {
            steps.clamp(-24, 24)
        }
    }
}

fn slider_width(bar: &PanelSlider) -> f32 {
    if bar.steps.is_empty() {
        SLIDER_W
    } else {
        (bar.steps.len() as f32 * 64.0).clamp(220.0, 480.0)
    }
}

fn slider_height_at(bar: &PanelSlider, width: f32) -> f32 {
    if bar.steps.is_empty() {
        return SLIDER_H;
    }
    let rows = slider::label_rows(&bar.steps, SLIDER_INSET, width - SLIDER_INSET).max(1);
    SLIDER_H + rows as f32 * text::SMALL_LINE + 2.0
}

/// Named sliders keep tick names under the thumb track. The hit target is the whole box.
fn thumb_band(track: Rect, named: bool) -> Rect {
    if named {
        Rect {
            h: SLIDER_H.min(track.h),
            ..track
        }
    } else {
        track
    }
}

/// A game slider: the filled share of the track and a thumb at its end.
/// Named steps also draw a tick and a small name under each step.
fn push_bar(out: &mut Vec<Paint>, track: Rect, bar: &PanelSlider, palette: Option<Palette>) {
    let band = thumb_band(track, !bar.steps.is_empty());
    let (left, right) = track_ends(band);
    let x = left + (right - left) * bar.share();
    push_rect(
        out,
        Rect {
            x: left,
            y: band.y + (band.h - NOTCH_H) * 0.5,
            w: (x - left).max(0.0),
            h: NOTCH_H,
        },
        notch_color(palette),
    );
    if !bar.steps.is_empty() {
        let tick = [0.34, 0.36, 0.40];
        for index in 0..bar.steps.len() {
            let tick_x = slider::notch_x(band, index, bar.steps.len(), SLIDER_INSET);
            push_rect(
                out,
                Rect {
                    x: tick_x - 1.0,
                    y: band.y + (band.h - 6.0) * 0.5,
                    w: 2.0,
                    h: 6.0,
                },
                tick,
            );
        }
    }
    push_rect(
        out,
        Rect {
            x: x - THUMB_W * 0.5,
            y: band.y + (band.h - THUMB_H) * 0.5,
            w: THUMB_W,
            h: THUMB_H,
        },
        THUMB_COLOR,
    );
    if bar.steps.is_empty() {
        return;
    }
    let current = slider::nearest_index(&bar.steps, bar.value);
    let labels = slider::place_labels(&bar.steps, current, left, right, band.y + band.h + 1.0);
    for label in &labels {
        let color = if label.current {
            slider::LABEL_HOT
        } else {
            slider::LABEL_DIM
        };
        for blot in slider::label_blots(label) {
            push_blot(out, blot, color);
        }
    }
}

fn push_tip(out: &mut Vec<Paint>, viewport: [f32; 2], anchor_x: f32, anchor_y: f32, text: &str) {
    let (text_w, text_h) = text::measure(text, None);
    let pad = 4.0;
    let box_w = text_w + pad * 2.0;
    let box_h = text_h + pad * 2.0;
    let mut x = anchor_x - box_w * 0.5;
    let mut y = anchor_y - box_h - 6.0;
    if y < 0.0 {
        y = anchor_y + SLIDER_H + 4.0;
    }
    x = x.clamp(0.0, (viewport[0] - box_w).max(0.0));
    y = y.clamp(0.0, (viewport[1] - box_h).max(0.0));
    let rect = Rect {
        x,
        y,
        w: box_w,
        h: box_h,
    };
    push_rect(out, rect, [0.05, 0.06, 0.08]);
    push_border(out, rect, [0.55, 0.58, 0.64]);
    for blot in text::blots(text, x + pad, y + pad, text_w.max(1.0)) {
        push_blot(out, blot, TEXT);
    }
}

/// Write one control onto the first lamp. Intensity scales `Light.color`.
pub fn apply_lamp(scene: &mut Scene, action: Action) {
    let Some(light) = scene.lights.first_mut() else {
        return;
    };
    match action {
        Action::SetLamp { axis, position } => match axis {
            0 => light.position.x = position,
            1 => light.position.y = position,
            2 => light.position.z = position,
            _ => {}
        },
        Action::ScaleIntensity(scale) => {
            for channel in &mut light.color {
                *channel = (*channel * scale).clamp(0.0, 8.0);
            }
        }
        Action::SetAntialias(_)
        | Action::ToggleBoxRun
        | Action::Press(_)
        | Action::Slide { .. }
        | Action::Select { .. }
        | Action::Toggle { .. } => {}
    }
}

/// Apply one lighting-panel action.
///
/// A picture selection returns the mode. The caller sets that mode on the
/// renderer before the draw. Lamp actions stay on the first lamp.
pub fn apply_frame_action(scene: &mut Scene, action: Action) -> Option<PictureMode> {
    match action {
        Action::SetAntialias(mode) => Some(mode),
        other => {
            apply_lamp(scene, other);
            None
        }
    }
}

fn lamp_after(lamp: Option<[f32; 3]>, actions: &[Action]) -> Option<[f32; 3]> {
    let mut lamp = lamp?;
    for action in actions {
        if let Action::SetLamp { axis, position } = *action {
            if let Some(slot) = lamp.get_mut(axis as usize) {
                *slot = position;
            }
        }
    }
    Some(lamp)
}

fn refresh_palette(state: &mut State) {
    let current = match &state.omarchy_current {
        Some(path) => path.clone(),
        None => omarchy::omarchy_current_dir(),
    };
    if !omarchy::on_omarchy(&current) {
        state.palette = None;
        state.palette_from = None;
        return;
    }
    if let Some(palette) = omarchy::omarchy_palette(&current) {
        state.palette = Some(palette);
        state.palette_from = Some(current);
        return;
    }
    // Omarchy removes `current/theme` before the next theme lands.
    // Keep the last good palette for this directory. The next read replaces it.
    let same_dir = state.palette_from.as_ref() == Some(&current);
    if !same_dir {
        state.palette = None;
        state.palette_from = None;
    }
}

fn lighting_panel(offset: [f32; 2], palette: Option<Palette>, box_running: bool) -> Node {
    let mut panel = Node::new(id::PANEL);
    panel.direction = Direction::TopToBottom;
    panel.pad = Pad::all(8.0);
    panel.gap = 6.0;
    panel.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], offset));
    panel.draggable = true;
    paint_looks(&mut panel, chrome_looks(palette));
    let fill = surface(palette);
    panel.children = vec![
        label(id::TITLE, "Light", fill),
        row(21, vec![label(31, "X", fill), slider(id::X, palette)], fill),
        row(22, vec![label(32, "Y", fill), slider(id::Y, palette)], fill),
        row(23, vec![label(33, "Z", fill), slider(id::Z, palette)], fill),
        row(
            24,
            vec![
                control(id::DIM, "Dim", Action::ScaleIntensity(DIMMER), palette),
                control(
                    id::BRIGHT,
                    "Bright",
                    Action::ScaleIntensity(BRIGHTER),
                    palette,
                ),
            ],
            fill,
        ),
        row(
            25,
            vec![
                control(
                    id::AA_OFF,
                    "off",
                    Action::SetAntialias(PictureMode::Off),
                    palette,
                ),
                control(
                    id::AA_FXAA,
                    "FXAA",
                    Action::SetAntialias(PictureMode::Fxaa),
                    palette,
                ),
            ],
            fill,
        ),
        row(
            26,
            vec![control(
                id::BOX_RUN,
                if box_running { "Stop" } else { "Run" },
                Action::ToggleBoxRun,
                palette,
            )],
            fill,
        ),
    ];
    panel
}

fn label(id: u32, text: &str, fill: [f32; 3]) -> Node {
    let mut node = Node::new(id);
    node.text = Some(text.to_string());
    paint_looks(&mut node, flat_look(fill));
    node
}

fn row(id: u32, children: Vec<Node>, fill: [f32; 3]) -> Node {
    let mut node = Node::new(id);
    node.direction = Direction::LeftToRight;
    node.gap = 4.0;
    node.align = layout::Align::Center;
    paint_looks(&mut node, flat_look(fill));
    node.children = children;
    node
}

fn slider(id: u32, palette: Option<Palette>) -> Node {
    sized_slider(id, palette, SLIDER_W, SLIDER_H)
}

fn sized_slider(id: u32, palette: Option<Palette>, width: f32, height: f32) -> Node {
    let mut node = Node::new(id);
    node.width = Sizing::fixed(width);
    node.height = Sizing::fixed(height);
    paint_looks(&mut node, control_looks(palette));
    node
}

fn control(id: u32, text: &str, action: Action, palette: Option<Palette>) -> Node {
    let mut node = Node::new(id);
    node.pad = Pad::all(4.0);
    node.text = Some(text.to_string());
    node.action = Some(action);
    paint_looks(&mut node, control_looks(palette));
    node
}

fn slider_axis(id: u32) -> Option<u8> {
    match id {
        id::X => Some(0),
        id::Y => Some(1),
        id::Z => Some(2),
        _ => None,
    }
}

fn notches(axis: u8) -> &'static [f32] {
    match axis {
        0 | 2 => &XZ_NOTCHES,
        1 => &Y_NOTCHES,
        _ => &[],
    }
}

fn track_rect(items: &[Item], axis: u8) -> Option<Rect> {
    let id = match axis {
        0 => id::X,
        1 => id::Y,
        2 => id::Z,
        _ => return None,
    };
    items
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.rect)
}

fn track_ends(track: Rect) -> (f32, f32) {
    (track.x + SLIDER_INSET, track.x + track.w - SLIDER_INSET)
}

fn notch_x(track: Rect, index: usize, count: usize) -> f32 {
    let (left, right) = track_ends(track);
    if count <= 1 {
        return (left + right) * 0.5;
    }
    let t = index as f32 / (count - 1) as f32;
    left + (right - left) * t
}

fn index_at(track: Rect, count: usize, pointer_x: f32) -> usize {
    let (left, right) = track_ends(track);
    let t = ((pointer_x - left) / (right - left).max(1.0)).clamp(0.0, 1.0);
    let last = count.saturating_sub(1);
    ((t * last as f32).round() as usize).min(last)
}

fn nearest_index(values: &[f32], value: f32) -> usize {
    let mut best = 0;
    let mut best_dist = f32::MAX;
    for (index, notch) in values.iter().enumerate() {
        let dist = (notch - value).abs();
        if dist < best_dist {
            best = index;
            best_dist = dist;
        }
    }
    best
}

fn snap_axis(items: &[Item], axis: u8, pointer_x: f32) -> Option<f32> {
    let values = notches(axis);
    if values.is_empty() {
        return None;
    }
    let track = track_rect(items, axis)?;
    Some(values[index_at(track, values.len(), pointer_x)])
}

struct Looks {
    idle: Look,
    hover: Look,
    pressed: Look,
}

fn paint_looks(node: &mut Node, looks: Looks) {
    node.idle = looks.idle;
    node.hover = looks.hover;
    node.pressed = looks.pressed;
}

fn flat_look(fill: [f32; 3]) -> Looks {
    let look = Look {
        background: fill,
        border: fill,
    };
    Looks {
        idle: look,
        hover: look,
        pressed: look,
    }
}

fn surface(palette: Option<Palette>) -> [f32; 3] {
    palette
        .map(|palette| palette.background)
        .unwrap_or([0.08, 0.09, 0.11])
}

/// Idle shows background and foreground. Hover and pressed show accent.
fn theme_looks(palette: Palette) -> Looks {
    Looks {
        idle: Look {
            background: palette.background,
            border: palette.foreground,
        },
        hover: Look {
            background: palette.background,
            border: palette.accent,
        },
        pressed: Look {
            background: palette.accent,
            border: palette.foreground,
        },
    }
}

fn chrome_looks(palette: Option<Palette>) -> Looks {
    if let Some(palette) = palette {
        return theme_looks(palette);
    }
    Looks {
        idle: Look {
            background: [0.08, 0.09, 0.11],
            border: [0.42, 0.46, 0.52],
        },
        hover: Look {
            background: [0.14, 0.15, 0.18],
            border: [0.55, 0.60, 0.68],
        },
        pressed: Look {
            background: [0.18, 0.20, 0.24],
            border: [0.70, 0.75, 0.82],
        },
    }
}

fn control_looks(palette: Option<Palette>) -> Looks {
    if let Some(palette) = palette {
        return theme_looks(palette);
    }
    Looks {
        idle: Look {
            background: [0.16, 0.17, 0.20],
            border: [0.35, 0.40, 0.45],
        },
        hover: Look {
            background: [0.28, 0.32, 0.38],
            border: [0.55, 0.65, 0.75],
        },
        pressed: Look {
            background: [0.42, 0.55, 0.32],
            border: [0.75, 0.90, 0.45],
        },
    }
}

fn draggable(items: &[Item], id: u32) -> bool {
    let mut current = Some(id);
    while let Some(id) = current {
        let Some(item) = items.iter().find(|item| item.id == id) else {
            return false;
        };
        if item.action.is_some() {
            return false;
        }
        if item.draggable {
            return true;
        }
        current = item.parent;
    }
    false
}

fn choose(item: &Item, down: bool, hit: Option<u32>, focus: Option<u32>) -> Look {
    if down && focus == Some(item.id) {
        item.pressed
    } else if hit == Some(item.id) {
        item.hover
    } else if focus == Some(item.id) {
        item.pressed
    } else {
        item.idle
    }
}

fn push_paints(
    out: &mut Vec<Paint>,
    item: &Item,
    look: Look,
    palette: Option<Palette>,
    lamp: Option<[f32; 3]>,
    sliders: bool,
) {
    push_rect(out, item.rect, look.background);
    push_border(out, item.rect, look.border);
    if let Some(text) = &item.text {
        let color = text_color(palette, look);
        for blot in text::blots(text, item.content.x, item.content.y, item.content.w) {
            push_blot(out, blot, color);
        }
    }
    if let Some(axis) = slider_axis(item.id).filter(|_| sliders) {
        push_slider(out, item.rect, axis, lamp, palette);
    }
}

fn push_slider(
    out: &mut Vec<Paint>,
    track: Rect,
    axis: u8,
    lamp: Option<[f32; 3]>,
    palette: Option<Palette>,
) {
    let values = notches(axis);
    if values.is_empty() {
        return;
    }
    let color = notch_color(palette);
    for index in 0..values.len() {
        let x = notch_x(track, index, values.len());
        push_rect(
            out,
            Rect {
                x: x - NOTCH_W * 0.5,
                y: track.y + (track.h - NOTCH_H) * 0.5,
                w: NOTCH_W,
                h: NOTCH_H,
            },
            color,
        );
    }
    let value = lamp
        .map(|lamp| lamp[axis as usize])
        .unwrap_or(values[values.len() / 2]);
    let index = nearest_index(values, value);
    let x = notch_x(track, index, values.len());
    push_rect(
        out,
        Rect {
            x: x - THUMB_W * 0.5,
            y: track.y + (track.h - THUMB_H) * 0.5,
            w: THUMB_W,
            h: THUMB_H,
        },
        THUMB_COLOR,
    );
}

fn notch_color(palette: Option<Palette>) -> [f32; 3] {
    palette
        .map(|palette| palette.foreground)
        .unwrap_or([0.72, 0.76, 0.70])
}

fn text_color(palette: Option<Palette>, look: Look) -> [f32; 3] {
    let Some(palette) = palette else {
        return TEXT;
    };
    if look.background == palette.accent {
        palette.background
    } else {
        palette.foreground
    }
}

fn push_sun(
    shown: &mut Vec<Shown>,
    paints: &mut Vec<Paint>,
    viewport: [f32; 2],
    camera: &Camera,
    lamp: Option<[f32; 3]>,
) {
    let Some(lamp) = lamp else {
        return;
    };
    let mut node = Node::new(id::SUN);
    node.width = Sizing::fixed(SUN_DIAMETER);
    node.height = Sizing::fixed(SUN_DIAMETER);
    node.place = Some(Place::point([0.5, 0.5], [0.5, 0.5], [0.0, 0.0]));
    let placed = layout::layout(&node, Space::World(lamp), viewport, Some(camera));
    let Some(item) = placed.into_iter().find(|item| item.id == id::SUN) else {
        return;
    };
    if !overlaps_view(item.rect, viewport) {
        return;
    }
    let center = [
        item.rect.x + item.rect.w * 0.5,
        item.rect.y + item.rect.h * 0.5,
    ];
    let radius = item.rect.w.min(item.rect.h) * 0.5;
    push_disk(paints, center, radius, SUN_COLOR);
    let look = Look {
        background: SUN_COLOR,
        border: SUN_COLOR,
    };
    shown.push(Shown {
        id: id::SUN,
        rect: item.rect,
        look,
        idle: look,
        text: None,
    });
}

fn overlaps_view(rect: Rect, viewport: [f32; 2]) -> bool {
    rect.w > 0.0
        && rect.h > 0.0
        && rect.x < viewport[0]
        && rect.y < viewport[1]
        && rect.x + rect.w > 0.0
        && rect.y + rect.h > 0.0
}

fn push_disk(out: &mut Vec<Paint>, center: [f32; 2], radius: f32, color: [f32; 3]) {
    if radius <= 0.0 {
        return;
    }
    let band_h = radius * 2.0 / SUN_BANDS as f32;
    for index in 0..SUN_BANDS {
        let y = center[1] - radius + index as f32 * band_h;
        let mid = y + band_h * 0.5;
        let dy = ((mid - center[1]) / radius).clamp(-1.0, 1.0);
        let half = radius * (1.0 - dy * dy).max(0.0).sqrt();
        push_rect(
            out,
            Rect {
                x: center[0] - half,
                y,
                w: half * 2.0,
                h: band_h,
            },
            color,
        );
    }
}

fn push_rect(out: &mut Vec<Paint>, rect: Rect, color: [f32; 3]) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    out.push(Paint {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        color,
    });
}

fn push_border(out: &mut Vec<Paint>, rect: Rect, color: [f32; 3]) {
    if rect.w <= BORDER || rect.h <= BORDER {
        return;
    }
    push_rect(
        out,
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: BORDER,
        },
        color,
    );
    push_rect(
        out,
        Rect {
            x: rect.x,
            y: rect.y + rect.h - BORDER,
            w: rect.w,
            h: BORDER,
        },
        color,
    );
    push_rect(
        out,
        Rect {
            x: rect.x,
            y: rect.y,
            w: BORDER,
            h: rect.h,
        },
        color,
    );
    push_rect(
        out,
        Rect {
            x: rect.x + rect.w - BORDER,
            y: rect.y,
            w: BORDER,
            h: rect.h,
        },
        color,
    );
}

fn push_blot(out: &mut Vec<Paint>, blot: Blot, color: [f32; 3]) {
    out.push(Paint {
        x: blot.x,
        y: blot.y,
        w: blot.w,
        h: blot.h,
        color,
    });
}
