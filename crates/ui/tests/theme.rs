//! Theme roles sit beside the Omarchy palette. The lighting panel does not use them.

use genos_ui::omarchy::{Mode, Palette};
use genos_ui::{looks, theme_dark, theme_from_palette};

#[test]
fn palette_channels_win_and_hover_uses_the_accent() {
    let theme = theme_from_palette(Palette {
        background: [0.1, 0.1, 0.1],
        foreground: [0.9, 0.9, 0.9],
        accent: [1.0, 0.0, 0.0],
        mode: Mode::Dark,
    });
    assert_eq!(theme.surface, [0.1, 0.1, 0.1]);
    assert_eq!(theme.text, [0.9, 0.9, 0.9]);
    assert_eq!(theme.accent, [1.0, 0.0, 0.0]);
    assert_eq!(theme.danger, theme_dark().danger);
    assert_eq!(theme.focus, theme_dark().focus);
    assert_eq!(looks(&theme_dark()).hover.border, theme_dark().accent);
}
