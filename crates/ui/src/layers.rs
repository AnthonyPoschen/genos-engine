//! One layer in front of the root: a popup, a dialog, or a tooltip.
//!
//! A second `open_layer` replaces the first. A tooltip is not a press target.

use crate::kind::Kind;
use crate::layout::Node;
use crate::panel::State;
use crate::theme::{self, Theme};

/// Which layer is open. A dialog traps the pointer. A tooltip does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerRole {
    Popup,
    Dialog,
    Tooltip,
}

/// The node drawn above the root.
#[derive(Clone, Debug)]
pub struct OpenLayer {
    pub node: Node,
    pub role: LayerRole,
    /// Focus to restore when a dialog closes.
    pub focus_before: Option<u32>,
}

/// Id of the tooltip label `frame` builds. It is not a game node.
///
/// Panel chrome owns `u32::MAX - 4095` and up (`PANEL_ID`). This id sits below that band.
pub const TIP_ID: u32 = u32::MAX - 12_000;

impl State {
    /// Open a popup. A press outside it emits [`crate::Event::Dismiss`] and closes it.
    ///
    /// A second call while a popup is open replaces the node and leaves focus where it is.
    pub fn open_layer(&mut self, node: Node) {
        if let Some(layer) = self.layer.as_mut() {
            if layer.role == LayerRole::Popup {
                layer.node = node;
                return;
            }
        }
        self.close_layer();
        self.layer = Some(OpenLayer {
            node,
            role: LayerRole::Popup,
            focus_before: self.focus,
        });
    }

    /// Open a dialog. An outside press does not reach the root and does not close it.
    ///
    /// A second call while a dialog is open replaces the node and does not move focus.
    pub fn open_dialog(&mut self, node: Node) {
        if let Some(layer) = self.layer.as_mut() {
            if layer.role == LayerRole::Dialog {
                layer.node = node;
                return;
            }
        }
        self.close_layer();
        let focus_before = self.focus;
        self.focus = first_focusable(&node).or(focus_before);
        self.layer = Some(OpenLayer {
            node,
            role: LayerRole::Dialog,
            focus_before,
        });
    }

    /// Close the popup, dialog, or tooltip. A dialog returns focus to the id it saved.
    pub fn close_layer(&mut self) {
        let Some(layer) = self.layer.take() else {
            return;
        };
        if layer.role == LayerRole::Dialog {
            self.focus = layer.focus_before;
        }
    }

    pub(crate) fn close_tooltip(&mut self) {
        if self
            .layer
            .as_ref()
            .is_some_and(|layer| layer.role == LayerRole::Tooltip)
        {
            self.layer = None;
        }
        self.tooltip_id = None;
        self.tooltip_secs = 0.0;
    }
}

pub(crate) fn first_focusable(node: &Node) -> Option<u32> {
    if focusable_kind(&node.kind) {
        return Some(node.id);
    }
    for child in &node.children {
        if let Some(id) = first_focusable(child) {
            return Some(id);
        }
    }
    None
}

pub(crate) fn focusable_kind(kind: &Kind) -> bool {
    match kind {
        Kind::Button { enabled } => *enabled,
        Kind::Toggle { .. }
        | Kind::Checkbox { .. }
        | Kind::Radio { .. }
        | Kind::Slider { .. }
        | Kind::TextField { .. } => true,
        Kind::Box | Kind::Label | Kind::Progress { .. } => false,
    }
}

pub(crate) fn tooltip_node(
    text: &str,
    at: [f32; 2],
    viewport: [f32; 2],
    theme: &Theme,
) -> Node {
    let (width, height) = crate::text::measure(text, Some(viewport[0].max(1.0)));
    let width = width.max(12.0);
    let height = height.max(16.0);
    let mut x = at[0];
    let mut y = at[1] + 8.0;
    if y + height > viewport[1] {
        y = (at[1] - height).max(0.0);
    }
    if x + width > viewport[0] {
        x = (viewport[0] - width).max(0.0);
    }
    let mut node = Node::new(TIP_ID);
    node.kind = Kind::Label;
    node.text = Some(text.to_string());
    node.width = crate::Sizing::fixed(width);
    node.height = crate::Sizing::fixed(height);
    node.place = Some(crate::Place::point([0.0, 0.0], [0.0, 0.0], [x, y]));
    theme::apply_looks(&mut node, theme::looks(theme));
    node
}
