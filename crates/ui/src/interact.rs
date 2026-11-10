//! One pass over a retained tree: layout, hit, event, paint.
//!
//! `lighting_frame` keeps its own pass. A game menu calls [`frame`].

#![expect(clippy::too_many_arguments, reason = "frame lists the pass inputs until FrameArgs folds them")]
#![expect(clippy::too_many_lines, reason = "the pass stays in one function so the order of hit, keys, and paint stays visible")]

use genos_input::InputSystem;
use genos_scene::Camera;

use crate::controls;
use crate::focus::Consumed;
use crate::kind::{Event, Kind};
use crate::layers::{self, LayerRole, OpenLayer};
use crate::layout::{self, Item, Node, Rect, Space};
use crate::nav;
use crate::panel::{Frame, Paint, Pointer, Shown, State};
use crate::Look;
use crate::Segment;

const TIP_DELAY: f32 = 0.4;

/// Lay out `root`, hit-test `pointer`, and paint. A press edge fires once.
///
/// `keys` moves UI focus when it is `Some`. `damage` stays empty until PR #5.
/// `dt` is seconds and feeds the tooltip delay.
pub fn frame(
    root: &Node,
    state: &mut State,
    space: Space,
    viewport: [f32; 2],
    camera: Option<&Camera>,
    pointer: Pointer,
    dt: f32,
    keys: Option<&InputSystem>,
) -> Frame {
    let press_edge = pointer.down && !state.down;
    let root_items = layout::layout(root, space, viewport, camera);
    settle_tooltip(state, &root_items, pointer, viewport, dt);
    let layer_items = state.layer.as_ref().map(|layer| {
        layout::layout(&layer.node, Space::Screen, viewport, camera)
    });
    let role = state.layer.as_ref().map(|layer| layer.role);
    let layer_root = state.layer.as_ref().map(|layer| layer.node.id);
    let layer_hit = layer_items
        .as_ref()
        .and_then(|items| hit_at(items, pointer.x, pointer.y))
        .cloned();
    let root_hit = hit_at(&root_items, pointer.x, pointer.y).cloned();
    let mut events = Vec::new();
    let mut actions = Vec::new();
    let mut look_capture = false;
    let mut keys_captured = false;
    if press_edge {
        match role {
            Some(LayerRole::Popup) => {
                if let Some(item) = layer_hit.as_ref() {
                    state.focus = Some(item.id);
                    state.look_hold = false;
                    on_press(item, layer_items.as_deref().unwrap_or(&[]), pointer, state, &mut events, &mut actions);
                } else if let Some(id) = layer_root {
                    events.push(Event::Dismiss { id });
                    state.close_layer();
                }
            }
            Some(LayerRole::Dialog) => {
                if let Some(item) = layer_hit.as_ref() {
                    state.focus = Some(item.id);
                    state.look_hold = false;
                    on_press(item, layer_items.as_deref().unwrap_or(&[]), pointer, state, &mut events, &mut actions);
                }
            }
            Some(LayerRole::Tooltip) | None => {
                if let Some(item) = root_hit.as_ref() {
                    state.focus = Some(item.id);
                    state.look_hold = false;
                    state.held = None;
                    on_press(item, &root_items, pointer, state, &mut events, &mut actions);
                } else {
                    state.focus = None;
                    state.dragging = false;
                    state.held = None;
                    state.look_hold = true;
                    look_capture = true;
                }
            }
        }
    } else if pointer.down && state.look_hold {
        look_capture = true;
    } else if pointer.down {
        let on_layer = layer_items.as_ref().is_some_and(|items| {
            items.iter().any(|item| state.held == Some(item.id))
        });
        if on_layer {
            slide_held(layer_items.as_deref().unwrap_or(&[]), pointer, state, &mut events);
        } else {
            slide_held(&root_items, pointer, state, &mut events);
        }
    }
    if let Some(input) = keys {
        if role == Some(LayerRole::Dialog) && nav::back(input) {
            if let Some(id) = layer_root {
                events.push(Event::Dismiss { id });
                state.close_layer();
                keys_captured = true;
            }
        }
        if !keys_captured {
            let pool: &[Item] = if role == Some(LayerRole::Dialog) {
                layer_items.as_deref().unwrap_or(&root_items)
            } else {
                &root_items
            };
            if nav::apply(input, state, pool, &mut events) {
                keys_captured = true;
            }
        }
    }
    let theme = state.theme;
    let focus = state.focus;
    let layer_open = state.layer.is_some();
    let dialog = state
        .layer
        .as_ref()
        .is_some_and(|layer| layer.role == LayerRole::Dialog);
    let mut shown = Vec::new();
    let mut paints = Vec::new();
    let mut segments = Vec::new();
    paint_items(
        &root_items,
        pointer,
        root_hit.as_ref().map(|hit| hit.id),
        focus,
        &theme,
        &mut shown,
        &mut paints,
        &mut segments,
    );
    if dialog {
        paints.push(Paint {
            x: 0.0,
            y: 0.0,
            w: viewport[0],
            h: viewport[1],
            color: theme.surface,
            alpha: 1.0,
        });
    }
    if layer_open {
        if let Some(items) = &layer_items {
            let hit_id = if role == Some(LayerRole::Tooltip) {
                None
            } else {
                layer_hit.as_ref().map(|hit| hit.id)
            };
            paint_items(
                items,
                pointer,
                hit_id,
                focus,
                &theme,
                &mut shown,
                &mut paints,
                &mut segments,
            );
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
        tooltip: None,
        consumed: Consumed::default(),
        damage: Vec::new(),
        events,
        segments,
        keys_captured,
    }
}

fn settle_tooltip(state: &mut State, items: &[Item], pointer: Pointer, viewport: [f32; 2], dt: f32) {
    if state
        .layer
        .as_ref()
        .is_some_and(|layer| layer.role == LayerRole::Dialog)
    {
        state.close_tooltip();
        return;
    }
    let hovered = hit_at(items, pointer.x, pointer.y).filter(|item| item.tooltip.is_some());
    let Some(item) = hovered else {
        state.close_tooltip();
        return;
    };
    if state.tooltip_id == Some(item.id) {
        state.tooltip_secs += dt.max(0.0);
    } else {
        state.tooltip_id = Some(item.id);
        state.tooltip_secs = dt.max(0.0);
    }
    if state.tooltip_secs < TIP_DELAY {
        return;
    }
    if state.layer.is_some() {
        return;
    }
    let Some(text) = item.tooltip.clone() else {
        return;
    };
    state.layer = Some(OpenLayer {
        node: layers::tooltip_node(&text, [pointer.x, pointer.y], viewport, &state.theme),
        role: LayerRole::Tooltip,
        focus_before: state.focus,
    });
}

fn hit_at(items: &[Item], x: f32, y: f32) -> Option<&Item> {
    items.iter().rev().find(|item| {
        item.rect.contains(x, y) && item.clip_to.is_none_or(|clip| clip.contains(x, y))
    })
}

fn paint_items(
    items: &[Item],
    pointer: Pointer,
    hit: Option<u32>,
    focus: Option<u32>,
    theme: &crate::theme::Theme,
    shown: &mut Vec<Shown>,
    paints: &mut Vec<Paint>,
    segments: &mut Vec<Segment>,
) {
    for item in items {
        let look = pick_look(item, pointer.down, hit, focus);
        shown.push(Shown {
            id: item.id,
            rect: item.rect,
            look,
            idle: item.idle,
            text: item.text.clone(),
        });
        let painted = controls::paint_item(item, look, theme);
        for paint in painted.paints {
            if let Some(paint) = clip_paint(paint, item.clip_to) {
                paints.push(paint);
            }
        }
        for segment in painted.segments {
            if let Some(segment) = clip_segment(segment, item.clip_to) {
                segments.push(segment);
            }
        }
    }
}

fn clip_paint(paint: Paint, clip: Option<Rect>) -> Option<Paint> {
    let Some(clip) = clip else {
        return Some(paint);
    };
    let rect = layout::intersect(
        Rect {
            x: paint.x,
            y: paint.y,
            w: paint.w,
            h: paint.h,
        },
        clip,
    )?;
    Some(Paint {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        color: paint.color,
        alpha: paint.alpha,
    })
}

fn clip_segment(segment: Segment, clip: Option<Rect>) -> Option<Segment> {
    let Some(clip) = clip else {
        return Some(segment);
    };
    let dx = segment.b[0] - segment.a[0];
    let dy = segment.b[1] - segment.a[1];
    let mut t0 = 0.0_f32;
    let mut t1 = 1.0_f32;
    let edges = [
        (-dx, segment.a[0] - clip.x),
        (dx, clip.x + clip.w - segment.a[0]),
        (-dy, segment.a[1] - clip.y),
        (dy, clip.y + clip.h - segment.a[1]),
    ];
    for (p, q) in edges {
        if p.abs() < 1.0e-6 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            if t > t1 {
                return None;
            }
            if t > t0 {
                t0 = t;
            }
        } else {
            if t < t0 {
                return None;
            }
            if t < t1 {
                t1 = t;
            }
        }
    }
    Some(Segment {
        a: [segment.a[0] + t0 * dx, segment.a[1] + t0 * dy],
        b: [segment.a[0] + t1 * dx, segment.a[1] + t1 * dy],
        width: segment.width,
        color: segment.color,
    })
}

fn on_press(
    item: &Item,
    items: &[Item],
    pointer: Pointer,
    state: &mut State,
    events: &mut Vec<Event>,
    actions: &mut Vec<crate::Action>,
) {
    match &item.kind {
        Kind::Button { enabled: true } => events.push(Event::Press { id: item.id }),
        Kind::Button { enabled: false } | Kind::Label | Kind::Progress { .. } => {}
        Kind::Toggle { on } => events.push(Event::Toggle {
            id: item.id,
            on: !on,
        }),
        Kind::Checkbox { checked } => events.push(Event::Toggle {
            id: item.id,
            on: !checked,
        }),
        Kind::Radio { group, .. } => events.push(Event::Select {
            id: item.id,
            index: controls::radio_index(items, *group, item.id),
        }),
        Kind::Slider { .. } => {
            state.held = Some(item.id);
            if let Some(value) = controls::slider_value(item, pointer.x, pointer.y) {
                events.push(Event::Change { id: item.id, value });
            }
        }
        Kind::Box => {
            if let Some(action) = item.action {
                actions.push(action);
            } else if item.draggable {
                state.dragging = true;
            }
        }
    }
}

fn slide_held(items: &[Item], pointer: Pointer, state: &State, events: &mut Vec<Event>) {
    let Some(id) = state.held else {
        return;
    };
    let Some(item) = items.iter().find(|item| item.id == id) else {
        return;
    };
    let Kind::Slider { value: current, .. } = &item.kind else {
        return;
    };
    let Some(value) = controls::slider_value(item, pointer.x, pointer.y) else {
        return;
    };
    if (value - *current).abs() > 1.0e-4 {
        events.push(Event::Change { id, value });
    }
}

fn pick_look(item: &Item, down: bool, hit: Option<u32>, focus: Option<u32>) -> Look {
    if down && focus == Some(item.id) && hit == Some(item.id) {
        item.pressed
    } else if focus == Some(item.id) && !down {
        item.focus_look
    } else if hit == Some(item.id) {
        item.hover
    } else {
        item.idle
    }
}
