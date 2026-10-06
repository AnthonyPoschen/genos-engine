//! The screen-space lighting panel and the pointer step the camera frame uses.

use genos_scene::{Camera, Scene};

use crate::layout::{self, Direction, Item, Node, Pad, Place, Rect, Space};
use crate::text::{self, Blot};
use crate::{id, Action, Look};

const MOVE: f32 = 0.75;
const BRIGHTER: f32 = 1.25;
const DIMMER: f32 = 0.8;
const BORDER: f32 = 2.0;
const TEXT: [f32; 3] = [0.93, 0.95, 0.92];

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
    focus: Option<u32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            panel_offset: [16.0, 16.0],
            pointer_x: 0.0,
            pointer_y: 0.0,
            down: false,
            dragging: false,
            focus: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Shown {
    pub id: u32,
    pub rect: Rect,
    pub look: Look,
    pub idle: Look,
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

/// Lay out the lighting panel, apply hover, press, and drag, and report controls.
///
/// A press on the panel sets `look_capture` false. The camera frame must not
/// capture the pointer in that case.
pub fn lighting_frame(
    state: &mut State,
    viewport: [f32; 2],
    camera: &Camera,
    pointer: Pointer,
) -> Frame {
    let press_edge = pointer.down && !state.down;
    if state.dragging && pointer.down && !press_edge {
        state.panel_offset[0] += pointer.x - state.pointer_x;
        state.panel_offset[1] += pointer.y - state.pointer_y;
    }
    let root = lighting_panel(state.panel_offset);
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
            look_capture = false;
            if let Some(action) = item.action {
                actions.push(action);
            } else if draggable(&items, item.id) {
                state.dragging = true;
            }
        } else {
            state.focus = None;
            state.dragging = false;
            look_capture = true;
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
        });
        push_paints(&mut paints, item, look);
    }
    state.pointer_x = pointer.x;
    state.pointer_y = pointer.y;
    state.down = pointer.down;
    if !pointer.down {
        state.dragging = false;
    }
    Frame {
        shown,
        paints,
        actions,
        look_capture,
    }
}

/// Write one control onto the first lamp. Intensity scales `Light.color`.
pub fn apply_lamp(scene: &mut Scene, action: Action) {
    let Some(light) = scene.lights.first_mut() else {
        return;
    };
    match action {
        Action::MoveLamp { axis, delta } => match axis {
            0 => light.position.x += delta,
            1 => light.position.y += delta,
            2 => light.position.z += delta,
            _ => {}
        },
        Action::ScaleIntensity(scale) => {
            for channel in &mut light.color {
                *channel = (*channel * scale).clamp(0.0, 8.0);
            }
        }
    }
}

fn lighting_panel(offset: [f32; 2]) -> Node {
    let mut panel = Node::new(id::PANEL);
    panel.direction = Direction::TopToBottom;
    panel.pad = Pad::all(8.0);
    panel.gap = 6.0;
    panel.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], offset));
    panel.draggable = true;
    panel.idle = Look {
        background: [0.08, 0.09, 0.11],
        border: [0.42, 0.46, 0.52],
    };
    panel.hover = Look {
        background: [0.14, 0.15, 0.18],
        border: [0.55, 0.60, 0.68],
    };
    panel.pressed = Look {
        background: [0.18, 0.20, 0.24],
        border: [0.70, 0.75, 0.82],
    };
    panel.children = vec![
        label(id::TITLE, "Light"),
        row(
            21,
            vec![
                control(
                    id::X_NEG,
                    "X-",
                    Action::MoveLamp {
                        axis: 0,
                        delta: -MOVE,
                    },
                ),
                control(
                    id::X_POS,
                    "X+",
                    Action::MoveLamp {
                        axis: 0,
                        delta: MOVE,
                    },
                ),
            ],
        ),
        row(
            22,
            vec![
                control(
                    id::Y_NEG,
                    "Y-",
                    Action::MoveLamp {
                        axis: 1,
                        delta: -MOVE,
                    },
                ),
                control(
                    id::Y_POS,
                    "Y+",
                    Action::MoveLamp {
                        axis: 1,
                        delta: MOVE,
                    },
                ),
            ],
        ),
        row(
            23,
            vec![
                control(
                    id::Z_NEG,
                    "Z-",
                    Action::MoveLamp {
                        axis: 2,
                        delta: -MOVE,
                    },
                ),
                control(
                    id::Z_POS,
                    "Z+",
                    Action::MoveLamp {
                        axis: 2,
                        delta: MOVE,
                    },
                ),
            ],
        ),
        row(
            24,
            vec![
                control(id::DIM, "Dim", Action::ScaleIntensity(DIMMER)),
                control(id::BRIGHT, "Bright", Action::ScaleIntensity(BRIGHTER)),
            ],
        ),
    ];
    panel
}

fn label(id: u32, text: &str) -> Node {
    let mut node = Node::new(id);
    node.text = Some(text.to_string());
    node.idle = Look {
        background: [0.08, 0.09, 0.11],
        border: [0.08, 0.09, 0.11],
    };
    node.hover = node.idle;
    node.pressed = node.idle;
    node
}

fn row(id: u32, children: Vec<Node>) -> Node {
    let mut node = Node::new(id);
    node.direction = Direction::LeftToRight;
    node.gap = 4.0;
    node.align = layout::Align::Center;
    node.idle = Look {
        background: [0.08, 0.09, 0.11],
        border: [0.08, 0.09, 0.11],
    };
    node.hover = node.idle;
    node.pressed = node.idle;
    node.children = children;
    node
}

fn control(id: u32, text: &str, action: Action) -> Node {
    let mut node = Node::new(id);
    node.pad = Pad::all(4.0);
    node.text = Some(text.to_string());
    node.action = Some(action);
    node.idle = Look {
        background: [0.16, 0.17, 0.20],
        border: [0.35, 0.40, 0.45],
    };
    node.hover = Look {
        background: [0.28, 0.32, 0.38],
        border: [0.55, 0.65, 0.75],
    };
    node.pressed = Look {
        background: [0.42, 0.55, 0.32],
        border: [0.75, 0.90, 0.45],
    };
    node
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

fn push_paints(out: &mut Vec<Paint>, item: &Item, look: Look) {
    push_rect(out, item.rect, look.background);
    push_border(out, item.rect, look.border);
    if let Some(text) = &item.text {
        for blot in text::blots(text, item.content.x, item.content.y, item.content.w) {
            push_blot(out, blot);
        }
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

fn push_blot(out: &mut Vec<Paint>, blot: Blot) {
    out.push(Paint {
        x: blot.x,
        y: blot.y,
        w: blot.w,
        h: blot.h,
        color: TEXT,
    });
}
