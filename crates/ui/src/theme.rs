//! Role colors for new elements. The lighting panel keeps the looks it builds.

use crate::layout::Node;
use crate::omarchy::{Mode, Palette};
use crate::Look;

/// Shared colors. New elements call [`looks`]. The lighting panel does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub surface: [f32; 3],
    pub surface_raised: [f32; 3],
    pub text: [f32; 3],
    pub muted: [f32; 3],
    pub accent: [f32; 3],
    pub danger: [f32; 3],
    pub focus: [f32; 3],
    pub mode: Mode,
}

/// Idle, hover, pressed, and focus looks for one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Looks {
    pub idle: Look,
    pub hover: Look,
    pub pressed: Look,
    pub focus: Look,
}

/// Built-in dark roles.
pub fn theme_dark() -> Theme {
    Theme {
        surface: [0.16, 0.16, 0.18],
        surface_raised: [0.22, 0.22, 0.25],
        text: [0.93, 0.95, 0.92],
        muted: [0.62, 0.64, 0.66],
        accent: [0.36, 0.58, 0.95],
        danger: [0.85, 0.32, 0.28],
        focus: [0.95, 0.86, 0.40],
        mode: Mode::Dark,
    }
}

/// Built-in light roles. Surface and text swap against [`theme_dark`].
pub fn theme_light() -> Theme {
    let dark = theme_dark();
    Theme {
        surface: dark.text,
        surface_raised: [0.86, 0.87, 0.84],
        text: dark.surface,
        muted: [0.40, 0.42, 0.44],
        accent: dark.accent,
        danger: dark.danger,
        focus: dark.focus,
        mode: Mode::Light,
    }
}

/// Copy background, foreground, and accent. Danger and focus stay with the mode.
pub fn theme_from_palette(palette: Palette) -> Theme {
    let mut theme = match palette.mode {
        Mode::Dark => theme_dark(),
        Mode::Light => theme_light(),
    };
    theme.surface = palette.background;
    theme.text = palette.foreground;
    theme.accent = palette.accent;
    theme.mode = palette.mode;
    theme
}

/// Looks a new element copies onto its node.
pub fn looks(theme: &Theme) -> Looks {
    Looks {
        idle: Look {
            background: theme.surface,
            border: theme.surface_raised,
        },
        hover: Look {
            background: theme.surface_raised,
            border: theme.accent,
        },
        pressed: Look {
            background: theme.accent,
            border: theme.accent,
        },
        focus: Look {
            background: theme.surface,
            border: theme.focus,
        },
    }
}

/// Copy [`Looks`] onto a node. The lighting panel does not call this.
pub fn apply_looks(node: &mut Node, looks: Looks) {
    node.idle = looks.idle;
    node.hover = looks.hover;
    node.pressed = looks.pressed;
    node.focus_look = looks.focus;
}
