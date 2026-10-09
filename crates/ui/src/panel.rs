//! The lighting panel, the lamp sun, and the pointer step the camera frame uses.

use std::path::PathBuf;

use genos_scene::{Camera, Scene};

use crate::layout::{self, Direction, Item, Node, Pad, Place, Rect, Sizing, Space};
use crate::omarchy::{self, Palette};
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
    /// True while the red box path is advancing.
    pub box_running: bool,
    /// Omarchy `current` directory. `None` uses the real state path.
    pub omarchy_current: Option<PathBuf>,
    palette: Option<Palette>,
    palette_from: Option<PathBuf>,
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
            box_running: false,
            omarchy_current: None,
            palette: None,
            palette_from: None,
        }
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
    }
}

/// One row of a game panel: a label, then buttons left to right, then a slider.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelRow {
    pub label: String,
    pub buttons: Vec<PanelButton>,
    pub slider: Option<PanelSlider>,
}

/// A slider on a panel row. A press anywhere on the track sets the value there and a
/// held press follows the pointer, firing [`Action::Slide`] each frame it changes.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelSlider {
    /// Game id, as for a button.
    pub id: u32,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    /// Values snap to `min + k * step`; 0 is continuous.
    pub step: f64,
    /// Readout beside the track, e.g. `23.9 H`.
    pub text: String,
}

impl PanelSlider {
    fn share(&self) -> f32 {
        let span = self.max - self.min;
        if span <= 0.0 {
            0.0
        } else {
            ((self.value - self.min) / span).clamp(0.0, 1.0) as f32
        }
    }

    fn value_at(&self, track: Rect, pointer_x: f32) -> f64 {
        let (left, right) = track_ends(track);
        let t = ((pointer_x - left) / (right - left).max(1.0)).clamp(0.0, 1.0) as f64;
        let v = self.min + t * (self.max - self.min);
        if self.step > 0.0 {
            (self.min + ((v - self.min) / self.step).round() * self.step).clamp(self.min, self.max)
        } else {
            v
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
    let press_edge = pointer.down && !state.down;
    if state.dragging && pointer.down && !press_edge {
        state.panel_offset[0] += pointer.x - state.pointer_x;
        state.panel_offset[1] += pointer.y - state.pointer_y;
    }
    refresh_palette(state);
    let palette = state.palette;
    let fill = surface(palette);
    let own = u32::MAX - 4095;
    let mut panel = Node::new(own);
    panel.direction = Direction::TopToBottom;
    panel.pad = Pad::all(8.0);
    panel.gap = 6.0;
    panel.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], state.panel_offset));
    panel.draggable = true;
    paint_looks(&mut panel, chrome_looks(palette));
    panel.children.push(label(own + 1, title, fill));
    for (index, item) in rows.iter().enumerate() {
        let at = own + 2 + 3 * index as u32;
        let mut children = vec![label(at + 1, &item.label, fill)];
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
            children.push(slider(bar.id, palette));
            children.push(label(at + 2, &bar.text, fill));
        }
        panel.children.push(row(at, children, fill));
    }
    let bars: Vec<&PanelSlider> = rows.iter().filter_map(|r| r.slider.as_ref()).collect();
    let items = layout::layout(&panel, Space::Screen, viewport, Some(camera));
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
            state.sliding = None;
            state.held = None;
            if bars.iter().any(|b| b.id == item.id) {
                state.held = Some(item.id);
                state.dragging = false;
            } else if let Some(action) = item.action {
                actions.push(action);
                state.dragging = false;
            } else {
                state.dragging = draggable(&items, item.id);
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
        let held = state.held.and_then(|id| bars.iter().find(|b| b.id == id));
        let track = held.and_then(|b| items.iter().find(|i| i.id == b.id));
        if let (Some(bar), Some(track)) = (held, track) {
            let value = bar.value_at(track.rect, pointer.x);
            if value != bar.value {
                actions.push(Action::Slide { id: bar.id, value });
            }
        }
    }
    let mut shown = Vec::with_capacity(items.len());
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
        push_paints(&mut paints, item, look, palette, None, false);
        if let Some(bar) = bars.iter().find(|b| b.id == item.id) {
            // Show where a held slider is going this frame.
            let value = actions.iter().rev().find_map(|a| match a {
                Action::Slide { id, value } if *id == bar.id => Some(*value),
                _ => None,
            });
            let shown_bar = PanelSlider {
                value: value.unwrap_or(bar.value),
                ..(*bar).clone()
            };
            push_bar(&mut paints, item.rect, shown_bar.share(), palette);
        }
    }
    state.pointer_x = pointer.x;
    state.pointer_y = pointer.y;
    state.down = pointer.down;
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
    }
}

/// A game slider: the filled share of the track and a thumb at its end.
fn push_bar(out: &mut Vec<Paint>, track: Rect, share: f32, palette: Option<Palette>) {
    let (left, right) = track_ends(track);
    let x = left + (right - left) * share;
    push_rect(
        out,
        Rect {
            x: left,
            y: track.y + (track.h - NOTCH_H) * 0.5,
            w: (x - left).max(0.0),
            h: NOTCH_H,
        },
        notch_color(palette),
    );
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
        | Action::Slide { .. } => {}
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
                control(
                    id::AA_SSAA,
                    "SSAA",
                    Action::SetAntialias(PictureMode::Ssaa),
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
    let mut node = Node::new(id);
    node.width = Sizing::fixed(SLIDER_W);
    node.height = Sizing::fixed(SLIDER_H);
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
