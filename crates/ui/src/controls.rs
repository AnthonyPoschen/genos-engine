//! Paint and hit values for [`crate::Kind`]. The lighting panel does not use this.

use crate::arc;
use crate::edit::{self, Caret};
use crate::kind::Kind;
use crate::layout::{Item, Rect};
use crate::outline::{self, OutlineFont};
use crate::slider;
use crate::text::{self, TextFace};
use crate::theme::Theme;
use crate::Paint;
use crate::Segment;

const SLIDER_INSET: f32 = 12.0;
const THUMB_W: f32 = 10.0;
const THUMB_H: f32 = 16.0;
const BORDER: f32 = 2.0;

pub(crate) struct Painted {
    pub paints: Vec<Paint>,
    pub segments: Vec<Segment>,
}

pub(crate) fn paint_item(
    item: &Item,
    look: crate::Look,
    theme: &Theme,
    font: Option<&OutlineFont>,
    caret: Option<Caret>,
) -> Painted {
    let mut paints = Vec::new();
    push_rect(&mut paints, item.rect, fill(item, look, theme));
    push_border(&mut paints, item.rect, look.border);
    match &item.kind {
        Kind::Toggle { on } => push_thumb(&mut paints, item.rect, *on, theme),
        Kind::Checkbox { checked: true } => push_mark(&mut paints, item.rect, theme),
        Kind::Slider { value, min, max, vertical, .. } => {
            push_slider_thumb(&mut paints, item.rect, *value, *min, *max, *vertical, theme);
        }
        Kind::Progress { value, radial: false } => {
            push_fill(&mut paints, item.rect, *value, theme);
        }
        Kind::Progress { value, radial: true } => {
            let painted = Painted {
                paints,
                segments: radial(*value, item.rect, theme),
            };
            return paint_text(painted, item, theme, font);
        }
        Kind::Checkbox { checked: false }
        | Kind::Box
        | Kind::Label
        | Kind::Button { .. }
        | Kind::Radio { .. }
        | Kind::TextField { .. } => {}
    }
    let mut painted = paint_text(
        Painted {
            paints,
            segments: Vec::new(),
        },
        item,
        theme,
        font,
    );
    edit::paint(&mut painted.paints, item, caret, theme);
    painted
}

fn paint_text(
    mut painted: Painted,
    item: &Item,
    theme: &Theme,
    font: Option<&OutlineFont>,
) -> Painted {
    if matches!(item.kind, Kind::TextField { .. }) {
        return painted;
    }
    if let Some(text) = &item.text {
        let color = text_color(item, theme);
        match item.face {
            TextFace::Bitmap => {
                for blot in text::blots(text, item.content.x, item.content.y, item.content.w.max(1.0)) {
                    painted.paints.push(Paint {
                        x: blot.x,
                        y: blot.y,
                        w: blot.w,
                        h: blot.h,
                        color,
                        alpha: 1.0,
                    });
                }
            }
            TextFace::Outline => {
                if let Some(font) = font {
                    outline::paint_outline_into(
                        &mut painted.paints,
                        font,
                        text,
                        [item.content.x, item.content.y],
                        item.content.w.max(1.0),
                        color,
                    );
                }
            }
        }
    }
    painted
}

pub(crate) fn slider_value(item: &Item, x: f32, y: f32) -> Option<f64> {
    let Kind::Slider {
        min,
        max,
        step,
        vertical,
        ..
    } = &item.kind
    else {
        return None;
    };
    let t = if *vertical {
        let (top, bottom) = (item.rect.y + SLIDER_INSET, item.rect.y + item.rect.h - SLIDER_INSET);
        let span = (bottom - top).max(1.0);
        ((bottom - y) / span).clamp(0.0, 1.0)
    } else {
        let (left, right) = (item.rect.x + SLIDER_INSET, item.rect.x + item.rect.w - SLIDER_INSET);
        let span = (right - left).max(1.0);
        ((x - left) / span).clamp(0.0, 1.0)
    };
    Some(slider::value_at_fraction(*min, *max, *step, &[], f64::from(t)))
}

pub(crate) fn radio_index(items: &[Item], group: u32, id: u32) -> u32 {
    let mut index = 0_u32;
    for item in items {
        let Kind::Radio {
            group: found,
            ..
        } = &item.kind
        else {
            continue;
        };
        if *found != group {
            continue;
        }
        if item.id == id {
            return index;
        }
        index = index.saturating_add(1);
    }
    0
}

fn fill(item: &Item, look: crate::Look, theme: &Theme) -> [f32; 3] {
    match &item.kind {
        Kind::Radio { selected: true, .. } => theme.accent,
        Kind::Radio { selected: false, .. }
        | Kind::Box
        | Kind::Label
        | Kind::Button { .. }
        | Kind::Toggle { .. }
        | Kind::Checkbox { .. }
        | Kind::Slider { .. }
        | Kind::Progress { .. }
        | Kind::TextField { .. } => look.background,
    }
}

fn text_color(item: &Item, theme: &Theme) -> [f32; 3] {
    match &item.kind {
        Kind::Button { enabled: false } => theme.muted,
        Kind::Button { enabled: true }
        | Kind::Box
        | Kind::Label
        | Kind::Toggle { .. }
        | Kind::Checkbox { .. }
        | Kind::Radio { .. }
        | Kind::Slider { .. }
        | Kind::Progress { .. }
        | Kind::TextField { .. } => theme.text,
    }
}

fn push_thumb(out: &mut Vec<Paint>, track: Rect, on: bool, theme: &Theme) {
    let thumb_w = (track.w * 0.5 - 2.0).max(1.0);
    let thumb = Rect {
        x: if on {
            track.x + track.w * 0.5
        } else {
            track.x + 2.0
        },
        y: track.y + 2.0,
        w: thumb_w,
        h: (track.h - 4.0).max(1.0),
    };
    push_rect(out, thumb, theme.accent);
}

fn push_mark(out: &mut Vec<Paint>, rect: Rect, theme: &Theme) {
    push_rect(
        out,
        Rect {
            x: rect.x + 4.0,
            y: rect.y + 4.0,
            w: (rect.w - 8.0).max(0.0),
            h: (rect.h - 8.0).max(0.0),
        },
        theme.accent,
    );
}

fn push_fill(out: &mut Vec<Paint>, track: Rect, value: f32, theme: &Theme) {
    let value = unit_progress(value);
    push_rect(
        out,
        Rect {
            x: track.x,
            y: track.y,
            w: track.w * value,
            h: track.h,
        },
        theme.accent,
    );
}

fn push_slider_thumb(
    out: &mut Vec<Paint>,
    track: Rect,
    value: f64,
    min: f64,
    max: f64,
    vertical: bool,
    theme: &Theme,
) {
    let share = slider::share(min, max, &[], value);
    if vertical {
        let (top, bottom) = (track.y + SLIDER_INSET, track.y + track.h - SLIDER_INSET);
        let cy = bottom - (bottom - top) * share;
        push_rect(
            out,
            Rect {
                x: track.x + (track.w - THUMB_H) * 0.5,
                y: cy - THUMB_W * 0.5,
                w: THUMB_H,
                h: THUMB_W,
            },
            theme.focus,
        );
    } else {
        let (left, right) = (track.x + SLIDER_INSET, track.x + track.w - SLIDER_INSET);
        let cx = left + (right - left) * share;
        push_rect(
            out,
            Rect {
                x: cx - THUMB_W * 0.5,
                y: track.y + (track.h - THUMB_H) * 0.5,
                w: THUMB_W,
                h: THUMB_H,
            },
            theme.focus,
        );
    }
}

fn unit_progress(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn radial(value: f32, rect: Rect, theme: &Theme) -> Vec<Segment> {
    let value = unit_progress(value);
    let radius = rect.w.min(rect.h) * 0.5 - 2.0;
    arc::arc_segments(
        [rect.x + rect.w * 0.5, rect.y + rect.h * 0.5],
        radius,
        0.0,
        value,
        3.0,
        theme.accent,
    )
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
        alpha: 1.0,
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
