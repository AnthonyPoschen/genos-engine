//! Clay flow plus Unity pivot and anchor placement.
//!
//! Grow siblings share leftover space until they are the same size. A sibling
//! whose fit size is already past that share stays there. Min and max can stop
//! the share. A placed child is not in the flow.

use genos_scene::{viewport_uv, Camera};

use crate::text;
use crate::Action;
use crate::Look;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    LeftToRight,
    TopToBottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug)]
pub enum Sizing {
    Fit { min: f32, max: f32 },
    Grow { min: f32, max: f32 },
    Fixed(f32),
    Percent(f32),
}

impl Sizing {
    pub fn fit() -> Self {
        Self::Fit {
            min: 0.0,
            max: f32::INFINITY,
        }
    }

    pub fn grow() -> Self {
        Self::Grow {
            min: 0.0,
            max: f32::INFINITY,
        }
    }

    pub fn fixed(value: f32) -> Self {
        Self::Fixed(value)
    }

    pub fn percent(fraction: f32) -> Self {
        Self::Percent(fraction)
    }

    fn is_grow(self) -> bool {
        matches!(self, Self::Grow { .. })
    }

    fn limits(self) -> (f32, f32) {
        match self {
            Self::Fit { min, max } | Self::Grow { min, max } => (min, max),
            Self::Fixed(value) => (value, value),
            Self::Percent(_) => (0.0, f32::INFINITY),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Pad {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Pad {
    pub fn all(value: f32) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }

    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// Pivot and anchors, in the same units as a Unity rect.
///
/// One anchor point plus `offset` places the pivot. Split anchors size the
/// element from `edge_min` and `edge_max`. Y grows down. The origin is the
/// top-left of the parent.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub anchor_min: [f32; 2],
    pub anchor_max: [f32; 2],
    pub pivot: [f32; 2],
    pub offset: [f32; 2],
    pub edge_min: [f32; 2],
    pub edge_max: [f32; 2],
}

impl Place {
    pub fn point(anchor: [f32; 2], pivot: [f32; 2], offset: [f32; 2]) -> Self {
        Self {
            anchor_min: anchor,
            anchor_max: anchor,
            pivot,
            offset,
            edge_min: [0.0, 0.0],
            edge_max: [0.0, 0.0],
        }
    }

    pub fn stretch(
        anchor_min: [f32; 2],
        anchor_max: [f32; 2],
        edge_min: [f32; 2],
        edge_max: [f32; 2],
    ) -> Self {
        Self {
            anchor_min,
            anchor_max,
            pivot: [0.5, 0.5],
            offset: [0.0, 0.0],
            edge_min,
            edge_max,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: u32,
    pub direction: Direction,
    pub pad: Pad,
    pub gap: f32,
    pub align: Align,
    pub width: Sizing,
    pub height: Sizing,
    pub place: Option<Place>,
    pub text: Option<String>,
    pub idle: Look,
    pub hover: Look,
    pub pressed: Look,
    pub draggable: bool,
    pub action: Option<Action>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            direction: Direction::TopToBottom,
            pad: Pad::default(),
            gap: 0.0,
            align: Align::Start,
            width: Sizing::fit(),
            height: Sizing::fit(),
            place: None,
            text: None,
            idle: Look::default(),
            hover: Look::default(),
            pressed: Look::default(),
            draggable: false,
            action: None,
            children: Vec::new(),
        }
    }
}

/// Where the root sits. Screen space ignores the camera. World space projects
/// `point` and places the pivot there.
#[derive(Clone, Copy, Debug)]
pub enum Space {
    Screen,
    World([f32; 3]),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u32,
    pub parent: Option<u32>,
    pub rect: Rect,
    pub content: Rect,
    pub text: Option<String>,
    pub idle: Look,
    pub hover: Look,
    pub pressed: Look,
    pub draggable: bool,
    pub action: Option<Action>,
}

struct Spec {
    basis: f32,
    grow: bool,
    max: f32,
}

/// Resolve `root` into rectangles. `viewport` is width and height in pixels.
/// A screen root keeps its viewport point when `camera` changes. A world root
/// moves with the projected point.
pub fn layout(root: &Node, space: Space, viewport: [f32; 2], camera: Option<&Camera>) -> Vec<Item> {
    let view = Rect {
        x: 0.0,
        y: 0.0,
        w: viewport[0].max(1.0),
        h: viewport[1].max(1.0),
    };
    let basis = node_basis(root, Some(view));
    let sized = (
        expand_grow(root.width, basis.0, view.w),
        expand_grow(root.height, basis.1, view.h),
    );
    let rect = match space {
        Space::Screen => position(root.place, sized, view),
        Space::World(point) => world_rect(root, sized, point, viewport, camera),
    };
    let mut items = Vec::new();
    layout_node(root, rect, None, &mut items);
    items
}

fn world_rect(
    root: &Node,
    sized: (f32, f32),
    point: [f32; 3],
    viewport: [f32; 2],
    camera: Option<&Camera>,
) -> Rect {
    let pivot = root.place.map(|place| place.pivot).unwrap_or([0.0, 0.0]);
    let offset = root.place.map(|place| place.offset).unwrap_or([0.0, 0.0]);
    let anchor = camera
        .and_then(|camera| project(camera, viewport, point))
        .unwrap_or([-1.0e5, -1.0e5]);
    Rect {
        x: anchor[0] + offset[0] - pivot[0] * sized.0,
        y: anchor[1] + offset[1] - pivot[1] * sized.1,
        w: sized.0,
        h: sized.1,
    }
}

fn project(camera: &Camera, viewport: [f32; 2], point: [f32; 3]) -> Option<[f32; 2]> {
    let aspect = viewport[0] / viewport[1].max(1.0);
    let uv = viewport_uv(camera, aspect, point)?;
    Some([uv[0] * viewport[0], uv[1] * viewport[1]])
}

fn expand_grow(sizing: Sizing, basis: f32, parent: f32) -> f32 {
    match sizing {
        Sizing::Grow { min, max } => {
            if basis > parent {
                basis.clamp(min, max)
            } else {
                parent.clamp(basis.max(min), max)
            }
        }
        _ => basis,
    }
}

fn position(place: Option<Place>, size: (f32, f32), parent: Rect) -> Rect {
    let Some(place) = place else {
        return Rect {
            x: parent.x,
            y: parent.y,
            w: size.0,
            h: size.1,
        };
    };
    let (x, w) = axis_place(
        parent.x,
        parent.w,
        place.anchor_min[0],
        place.anchor_max[0],
        place.pivot[0],
        place.offset[0],
        place.edge_min[0],
        place.edge_max[0],
        size.0,
    );
    let (y, h) = axis_place(
        parent.y,
        parent.h,
        place.anchor_min[1],
        place.anchor_max[1],
        place.pivot[1],
        place.offset[1],
        place.edge_min[1],
        place.edge_max[1],
        size.1,
    );
    Rect { x, y, w, h }
}

fn axis_place(
    origin: f32,
    parent: f32,
    anchor_min: f32,
    anchor_max: f32,
    pivot: f32,
    offset: f32,
    edge_min: f32,
    edge_max: f32,
    child: f32,
) -> (f32, f32) {
    if (anchor_max - anchor_min).abs() < 1.0e-4 {
        let anchor = origin + anchor_min * parent;
        let at = anchor + offset;
        (at - pivot * child, child)
    } else {
        let start = origin + anchor_min * parent + edge_min;
        let end = origin + anchor_max * parent + edge_max;
        (start, end - start)
    }
}

fn layout_node(node: &Node, rect: Rect, parent: Option<u32>, out: &mut Vec<Item>) {
    let content = inset(rect, node.pad);
    out.push(Item {
        id: node.id,
        parent,
        rect,
        content,
        text: node.text.clone(),
        idle: node.idle,
        hover: node.hover,
        pressed: node.pressed,
        draggable: node.draggable,
        action: node.action,
    });
    let flow = flow_children(node);
    let bases: Vec<(f32, f32)> = flow
        .iter()
        .map(|child| node_basis(child, Some(content)))
        .collect();
    let horizontal = node.direction == Direction::LeftToRight;
    let main_specs: Vec<Spec> = flow
        .iter()
        .zip(bases.iter())
        .map(|(child, basis)| {
            spec(
                if horizontal {
                    child.width
                } else {
                    child.height
                },
                if horizontal { basis.0 } else { basis.1 },
            )
        })
        .collect();
    let cross_specs: Vec<Spec> = flow
        .iter()
        .zip(bases.iter())
        .map(|(child, basis)| {
            spec(
                if horizontal {
                    child.height
                } else {
                    child.width
                },
                if horizontal { basis.1 } else { basis.0 },
            )
        })
        .collect();
    let main_space = if horizontal { content.w } else { content.h };
    let cross_space = if horizontal { content.h } else { content.w };
    let mains = distribute(main_space, node.gap, &main_specs);
    let mut cursor = if horizontal { content.x } else { content.y };
    for (index, child) in flow.iter().enumerate() {
        let main = mains[index];
        let cross = cross_size(&cross_specs[index], cross_space);
        let cross_at = align_at(
            if horizontal { content.y } else { content.x },
            cross_space,
            cross,
            node.align,
        );
        let child_rect = if horizontal {
            Rect {
                x: cursor,
                y: cross_at,
                w: main,
                h: cross,
            }
        } else {
            Rect {
                x: cross_at,
                y: cursor,
                w: cross,
                h: main,
            }
        };
        layout_node(child, child_rect, Some(node.id), out);
        cursor += main + node.gap;
    }
    for child in node.children.iter().filter(|child| child.place.is_some()) {
        let basis = node_basis(child, Some(rect));
        let child_rect = position(child.place, basis, rect);
        layout_node(child, child_rect, Some(node.id), out);
    }
}

fn spec(sizing: Sizing, basis: f32) -> Spec {
    let (_min, max) = sizing.limits();
    Spec {
        basis,
        grow: sizing.is_grow(),
        max,
    }
}

fn cross_size(spec: &Spec, content: f32) -> f32 {
    if !spec.grow {
        return spec.basis;
    }
    if spec.basis > content {
        spec.basis
    } else {
        content.clamp(spec.basis, spec.max)
    }
}

fn align_at(start: f32, content: f32, child: f32, align: Align) -> f32 {
    let extra = (content - child).max(0.0);
    match align {
        Align::Start => start,
        Align::Center => start + extra * 0.5,
        Align::End => start + extra,
    }
}

fn distribute(space: f32, gap: f32, specs: &[Spec]) -> Vec<f32> {
    let count = specs.len();
    if count == 0 {
        return Vec::new();
    }
    let gaps = if count > 1 {
        gap * (count as f32 - 1.0)
    } else {
        0.0
    };
    let mut size: Vec<f32> = specs.iter().map(|spec| spec.basis).collect();
    let mut open: Vec<bool> = specs.iter().map(|spec| spec.grow).collect();
    loop {
        let used = size.iter().sum::<f32>() + gaps;
        let leftover = space - used;
        let free: Vec<usize> = open
            .iter()
            .enumerate()
            .filter(|(_, flag)| **flag)
            .map(|(index, _)| index)
            .collect();
        if free.is_empty() || leftover <= 0.01 {
            break;
        }
        let free_sum: f32 = free.iter().map(|index| size[*index]).sum();
        let share = (free_sum + leftover) / free.len() as f32;
        let mut locked = false;
        for index in &free {
            if specs[*index].basis > share + 0.01 {
                size[*index] = specs[*index].basis;
                open[*index] = false;
                locked = true;
            } else if specs[*index].max < share - 0.01 {
                size[*index] = specs[*index].max;
                open[*index] = false;
                locked = true;
            }
        }
        if !locked {
            for index in &free {
                size[*index] = share.clamp(specs[*index].basis, specs[*index].max);
            }
            break;
        }
    }
    size
}

fn node_basis(node: &Node, parent: Option<Rect>) -> (f32, f32) {
    let width = match node.width {
        Sizing::Fixed(value) => value,
        Sizing::Percent(fraction) => parent.map(|rect| fraction * rect.w).unwrap_or(0.0),
        Sizing::Fit { min, max } | Sizing::Grow { min, max } => {
            width_from_content(node, parent).clamp(min, max)
        }
    };
    let height = match node.height {
        Sizing::Fixed(value) => value,
        Sizing::Percent(fraction) => parent.map(|rect| fraction * rect.h).unwrap_or(0.0),
        Sizing::Fit { min, max } | Sizing::Grow { min, max } => {
            height_from_content(node, width).clamp(min, max)
        }
    };
    (width, height)
}

fn width_from_content(node: &Node, parent: Option<Rect>) -> f32 {
    let flow = flow_children(node);
    if flow.is_empty() {
        let limit = text_wrap_limit(node, parent);
        let text_width = node
            .text
            .as_ref()
            .map(|text| text::measure(text, limit).0)
            .unwrap_or(0.0);
        return text_width + hpad(node);
    }
    let widths: Vec<f32> = flow.iter().map(|child| node_basis(child, None).0).collect();
    match node.direction {
        Direction::LeftToRight => {
            widths.iter().sum::<f32>() + gap_span(widths.len(), node.gap) + hpad(node)
        }
        Direction::TopToBottom => widths.into_iter().fold(0.0_f32, f32::max) + hpad(node),
    }
}

fn height_from_content(node: &Node, resolved_width: f32) -> f32 {
    let flow = flow_children(node);
    if flow.is_empty() {
        let content_w = (resolved_width - hpad(node)).max(0.0);
        let text_height = node
            .text
            .as_ref()
            .map(|text| text::measure(text, Some(content_w)).1)
            .unwrap_or(0.0);
        return text_height + vpad(node);
    }
    let inner_w = (resolved_width - hpad(node)).max(0.0);
    let hinted = Rect {
        x: 0.0,
        y: 0.0,
        w: inner_w,
        h: 0.0,
    };
    let heights: Vec<f32> = flow
        .iter()
        .map(|child| node_basis(child, Some(hinted)).1)
        .collect();
    match node.direction {
        Direction::TopToBottom => {
            heights.iter().sum::<f32>() + gap_span(heights.len(), node.gap) + vpad(node)
        }
        Direction::LeftToRight => heights.into_iter().fold(0.0_f32, f32::max) + vpad(node),
    }
}

fn text_wrap_limit(node: &Node, parent: Option<Rect>) -> Option<f32> {
    match node.width {
        Sizing::Fixed(value) => Some((value - hpad(node)).max(0.0)),
        Sizing::Percent(fraction) => parent.map(|rect| (fraction * rect.w - hpad(node)).max(0.0)),
        Sizing::Fit { max, .. } | Sizing::Grow { max, .. } => {
            let from_parent = parent.map(|rect| rect.w.max(0.0));
            let from_max = if max.is_finite() {
                Some((max - hpad(node)).max(0.0))
            } else {
                None
            };
            match (from_parent, from_max) {
                (Some(parent_width), Some(capped)) => Some(parent_width.min(capped)),
                (parent_width, capped) => parent_width.or(capped),
            }
        }
    }
}

fn flow_children(node: &Node) -> Vec<&Node> {
    node.children
        .iter()
        .filter(|child| child.place.is_none())
        .collect()
}

fn gap_span(count: usize, gap: f32) -> f32 {
    if count < 2 {
        0.0
    } else {
        gap * (count as f32 - 1.0)
    }
}

fn hpad(node: &Node) -> f32 {
    node.pad.left + node.pad.right
}

fn vpad(node: &Node) -> f32 {
    node.pad.top + node.pad.bottom
}

fn inset(rect: Rect, pad: Pad) -> Rect {
    Rect {
        x: rect.x + pad.left,
        y: rect.y + pad.top,
        w: (rect.w - pad.left - pad.right).max(0.0),
        h: (rect.h - pad.top - pad.bottom).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_scene::{viewport_uv, Camera};

    fn view() -> [f32; 2] {
        [400.0, 300.0]
    }

    fn at(items: &[Item], id: u32) -> Rect {
        items.iter().find(|item| item.id == id).expect("item").rect
    }

    fn near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.05,
            "got {actual} expected {expected}"
        );
    }

    #[test]
    fn flow_uses_direction_padding_gap_and_cross_alignment() {
        let mut row = Node::new(1);
        row.direction = Direction::LeftToRight;
        row.width = Sizing::fixed(200.0);
        row.height = Sizing::fixed(40.0);
        row.pad = Pad::new(10.0, 4.0, 10.0, 4.0);
        row.gap = 6.0;
        row.align = Align::End;
        let mut left = Node::new(2);
        left.width = Sizing::fixed(20.0);
        left.height = Sizing::fixed(10.0);
        let mut right = Node::new(3);
        right.width = Sizing::fixed(20.0);
        right.height = Sizing::fixed(10.0);
        row.children = vec![left, right];

        let items = layout(&row, Space::Screen, view(), None);
        let a = at(&items, 2);
        let b = at(&items, 3);
        near(a.x, 10.0);
        near(a.y, 26.0);
        near(b.x, 36.0);
        near(b.y, 26.0);
        near(a.w, 20.0);
        near(a.h, 10.0);

        let mut column = Node::new(1);
        column.direction = Direction::TopToBottom;
        column.width = Sizing::fixed(40.0);
        column.height = Sizing::fixed(200.0);
        column.pad = Pad::new(4.0, 10.0, 4.0, 10.0);
        column.gap = 6.0;
        column.align = Align::End;
        let mut top = Node::new(2);
        top.width = Sizing::fixed(10.0);
        top.height = Sizing::fixed(20.0);
        let mut bottom = Node::new(3);
        bottom.width = Sizing::fixed(10.0);
        bottom.height = Sizing::fixed(20.0);
        column.children = vec![top, bottom];
        let items = layout(&column, Space::Screen, view(), None);
        let a = at(&items, 2);
        let b = at(&items, 3);
        near(a.x, 26.0);
        near(a.y, 10.0);
        near(b.x, 26.0);
        near(b.y, 36.0);
    }

    #[test]
    fn a_fit_parent_grows_with_children_and_a_hard_parent_keeps_its_size() {
        let mut fit = Node::new(1);
        fit.pad = Pad::all(2.0);
        let mut child = Node::new(2);
        child.width = Sizing::fixed(30.0);
        child.height = Sizing::fixed(16.0);
        fit.children = vec![child];
        let items = layout(&fit, Space::Screen, view(), None);
        let parent = at(&items, 1);
        near(parent.w, 34.0);
        near(parent.h, 20.0);

        let mut hard = Node::new(1);
        hard.width = Sizing::fixed(50.0);
        hard.height = Sizing::fixed(50.0);
        let mut wide = Node::new(2);
        wide.width = Sizing::fixed(80.0);
        wide.height = Sizing::fixed(10.0);
        hard.children = vec![wide];
        let items = layout(&hard, Space::Screen, view(), None);
        let parent = at(&items, 1);
        near(parent.w, 50.0);
        near(parent.h, 50.0);
    }

    #[test]
    fn fixed_percent_and_grow_siblings_equalize_without_shrinking() {
        let mut row = Node::new(1);
        row.direction = Direction::LeftToRight;
        row.width = Sizing::fixed(200.0);
        row.height = Sizing::fixed(20.0);
        let mut fixed = Node::new(2);
        fixed.width = Sizing::fixed(40.0);
        fixed.height = Sizing::fixed(10.0);
        let mut percent = Node::new(3);
        percent.width = Sizing::percent(0.5);
        percent.height = Sizing::fixed(10.0);
        row.children = vec![fixed, percent];
        let items = layout(&row, Space::Screen, view(), None);
        near(at(&items, 2).w, 40.0);
        near(at(&items, 3).w, 100.0);
        near(at(&items, 3).x, 40.0);

        let mut equal = Node::new(1);
        equal.direction = Direction::LeftToRight;
        equal.width = Sizing::fixed(200.0);
        equal.height = Sizing::fixed(20.0);
        let mut a = Node::new(2);
        a.width = Sizing::grow();
        a.height = Sizing::fixed(10.0);
        let mut b = Node::new(3);
        b.width = Sizing::grow();
        b.height = Sizing::fixed(10.0);
        equal.children = vec![a, b];
        let items = layout(&equal, Space::Screen, view(), None);
        near(at(&items, 2).w, 100.0);
        near(at(&items, 3).w, 100.0);

        let mut uneven = Node::new(1);
        uneven.direction = Direction::LeftToRight;
        uneven.width = Sizing::fixed(200.0);
        uneven.height = Sizing::fixed(20.0);
        let mut wide = Node::new(2);
        wide.width = Sizing::grow();
        wide.height = Sizing::fixed(10.0);
        let mut inner = Node::new(4);
        inner.width = Sizing::fixed(140.0);
        inner.height = Sizing::fixed(10.0);
        wide.children = vec![inner];
        let mut narrow = Node::new(3);
        narrow.width = Sizing::grow();
        narrow.height = Sizing::fixed(10.0);
        let mut inner_b = Node::new(5);
        inner_b.width = Sizing::fixed(10.0);
        inner_b.height = Sizing::fixed(10.0);
        narrow.children = vec![inner_b];
        uneven.children = vec![wide, narrow];
        let items = layout(&uneven, Space::Screen, view(), None);
        near(at(&items, 2).w, 140.0);
        near(at(&items, 3).w, 60.0);

        let mut capped = Node::new(1);
        capped.direction = Direction::LeftToRight;
        capped.width = Sizing::fixed(200.0);
        capped.height = Sizing::fixed(20.0);
        let mut open = Node::new(2);
        open.width = Sizing::grow();
        open.height = Sizing::fixed(10.0);
        let mut limited = Node::new(3);
        limited.width = Sizing::Grow {
            min: 0.0,
            max: 40.0,
        };
        limited.height = Sizing::fixed(10.0);
        capped.children = vec![open, limited];
        let items = layout(&capped, Space::Screen, view(), None);
        near(at(&items, 2).w, 160.0);
        near(at(&items, 3).w, 40.0);
    }

    #[test]
    fn wrapped_text_changes_the_fit_height() {
        let mut narrow = Node::new(1);
        narrow.width = Sizing::fixed(24.0);
        narrow.height = Sizing::fit();
        let mut label = Node::new(2);
        label.text = Some("ab cd".to_string());
        narrow.children = vec![label];
        let items = layout(&narrow, Space::Screen, view(), None);
        near(at(&items, 1).h, 32.0);
        near(at(&items, 2).h, 32.0);

        let mut wide = Node::new(1);
        wide.width = Sizing::fixed(200.0);
        wide.height = Sizing::fit();
        let mut label = Node::new(2);
        label.text = Some("ab cd".to_string());
        wide.children = vec![label];
        let items = layout(&wide, Space::Screen, view(), None);
        near(at(&items, 1).h, 16.0);
        near(at(&items, 2).w, 60.0);
    }

    #[test]
    fn anchors_place_a_pivot_and_a_stretch_outside_the_flow() {
        let mut parent = Node::new(1);
        parent.width = Sizing::fixed(200.0);
        parent.height = Sizing::fixed(100.0);
        parent.direction = Direction::LeftToRight;
        let mut flow = Node::new(2);
        flow.width = Sizing::fixed(40.0);
        flow.height = Sizing::fixed(20.0);
        let mut pinned = Node::new(3);
        pinned.width = Sizing::fixed(30.0);
        pinned.height = Sizing::fixed(16.0);
        pinned.place = Some(Place::point([1.0, 0.0], [1.0, 0.0], [0.0, 0.0]));
        let mut stretch = Node::new(4);
        stretch.place = Some(Place::stretch(
            [0.0, 0.0],
            [1.0, 1.0],
            [10.0, 5.0],
            [-10.0, -5.0],
        ));
        parent.children = vec![flow, pinned, stretch];
        let items = layout(&parent, Space::Screen, view(), None);
        let flow = at(&items, 2);
        let pinned = at(&items, 3);
        let stretch = at(&items, 4);
        near(flow.x, 0.0);
        near(flow.w, 40.0);
        near(pinned.x, 170.0);
        near(pinned.y, 0.0);
        near(pinned.w, 30.0);
        near(stretch.x, 10.0);
        near(stretch.y, 5.0);
        near(stretch.w, 180.0);
        near(stretch.h, 90.0);
        near(at(&items, 1).w, 200.0);
    }

    #[test]
    fn a_screen_root_stays_and_a_world_root_follows_the_camera() {
        let mut screen = Node::new(1);
        screen.width = Sizing::fixed(20.0);
        screen.height = Sizing::fixed(10.0);
        screen.place = Some(Place::point([0.5, 0.5], [0.5, 0.5], [4.0, -6.0]));
        let first = Camera::opening();
        let mut second = Camera::opening();
        second.position.x += 3.0;
        let viewport = [200.0, 100.0];
        let a = layout(&screen, Space::Screen, viewport, Some(&first));
        let b = layout(&screen, Space::Screen, viewport, Some(&second));
        let a = at(&a, 1);
        let b = at(&b, 1);
        near(a.x, 94.0);
        near(a.y, 39.0);
        near(b.x, a.x);
        near(b.y, a.y);

        let point = world_ahead(&first);
        let mut world = Node::new(1);
        world.width = Sizing::fixed(20.0);
        world.height = Sizing::fixed(10.0);
        world.place = Some(Place::point([0.0, 0.0], [0.0, 0.0], [0.0, 0.0]));
        let a = layout(&world, Space::World(point), viewport, Some(&first));
        let b = layout(&world, Space::World(point), viewport, Some(&second));
        let a = at(&a, 1);
        let b = at(&b, 1);
        let expected = projected(&first, viewport, point);
        near(a.x, expected[0]);
        near(a.y, expected[1]);
        assert!(
            (a.x - b.x).abs() > 1.0 || (a.y - b.y).abs() > 1.0,
            "world root did not follow the camera: {a:?} {b:?}"
        );
        let moved = [point[0] + 2.0, point[1], point[2]];
        let c = layout(&world, Space::World(moved), viewport, Some(&first));
        let c = at(&c, 1);
        assert!(
            (a.x - c.x).abs() > 1.0 || (a.y - c.y).abs() > 1.0,
            "world root did not follow its world point"
        );
    }

    fn world_ahead(camera: &Camera) -> [f32; 3] {
        let (sy, cy) = camera.yaw.sin_cos();
        [
            camera.position.x + sy * 5.0,
            camera.position.y,
            camera.position.z - cy * 5.0,
        ]
    }

    fn projected(camera: &Camera, viewport: [f32; 2], point: [f32; 3]) -> [f32; 2] {
        let aspect = viewport[0] / viewport[1];
        let uv = viewport_uv(camera, aspect, point).expect("point is in front of the camera");
        [uv[0] * viewport[0], uv[1] * viewport[1]]
    }
}
