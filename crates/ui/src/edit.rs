//! ASCII caret for a [`crate::Kind::TextField`].
//!
//! `anchor == head` is a caret. A different pair is a selection. Indexes are
//! bytes, and a non-ASCII string is left unchanged. The game copies
//! [`crate::Event::Text`] onto the kind on the next frame.

use genos_input::{InputCode, InputSystem};

use crate::kind::{Event, Kind};
use crate::layout::Item;
use crate::panel::State;
use crate::text::{self, TextFace};
use crate::theme::Theme;
use crate::Paint;

/// Selection ends. Equal ends are a caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    pub anchor: usize,
    pub head: usize,
}

impl State {
    /// Caret stored for `id`. A field that has not been edited is at `0`.
    pub fn caret(&self, id: u32) -> Caret {
        self.carets
            .get(&id)
            .copied()
            .unwrap_or(Caret { anchor: 0, head: 0 })
    }
}

/// Edit the focused field. True when this frame used a text key.
pub(crate) fn apply(
    input: &InputSystem,
    state: &mut State,
    items: &[Item],
    events: &mut Vec<Event>,
) -> bool {
    let Some(item) = focused_field(items, state.focus) else {
        return false;
    };
    let Kind::TextField { text, multiline } = &item.kind else {
        return false;
    };
    if *multiline {
        return false;
    }
    let key = edit_key(input);
    let Some(key) = key else {
        return false;
    };
    if !text.is_ascii() {
        return true;
    }
    let mut caret = clamp(state.caret(item.id), text.len());
    let edited = match key {
        Key::Left => {
            step(&mut caret, text.len(), -1, shifted(input));
            None
        }
        Key::Right => {
            step(&mut caret, text.len(), 1, shifted(input));
            None
        }
        Key::Home => {
            caret.head = 0;
            if !shifted(input) {
                caret.anchor = 0;
            }
            None
        }
        Key::End => {
            caret.head = text.len();
            if !shifted(input) {
                caret.anchor = caret.head;
            }
            None
        }
        Key::Backspace => delete_before(text, &mut caret),
        Key::Delete => delete_after(text, &mut caret),
        Key::Char(ch) => Some(insert(text, &mut caret, ch)),
    };
    state.carets.insert(item.id, caret);
    if let Some(next) = edited {
        events.push(Event::Text {
            id: item.id,
            text: next,
        });
    }
    true
}

pub(crate) fn paint(
    out: &mut Vec<Paint>,
    item: &Item,
    caret: Option<Caret>,
    theme: &Theme,
) {
    let Kind::TextField {
        text,
        multiline: false,
    } = &item.kind
    else {
        return;
    };
    let caret = caret.filter(|_| text.is_ascii()).map(|caret| clamp(caret, text.len()));
    if let Some(caret) = caret {
        let (lo, hi) = ends(caret);
        if hi > lo {
            out.push(Paint {
                x: item.content.x + cells(lo),
                y: item.content.y,
                w: cells(hi - lo),
                h: text::CELL_H,
                color: theme.accent,
                alpha: 1.0,
            });
        }
    }
    if text.is_ascii() && item.face == TextFace::Bitmap {
        for blot in text::blots(text, item.content.x, item.content.y, item.content.w.max(1.0)) {
            out.push(Paint {
                x: blot.x,
                y: blot.y,
                w: blot.w,
                h: blot.h,
                color: theme.text,
                alpha: 1.0,
            });
        }
    }
    let Some(caret) = caret else {
        return;
    };
    out.push(Paint {
        x: item.content.x + cells(caret.head),
        y: item.content.y,
        w: 2.0,
        h: text::CELL_H,
        color: theme.text,
        alpha: 1.0,
    });
}

#[derive(Clone, Copy)]
enum Key {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Char(char),
}

fn edit_key(input: &InputSystem) -> Option<Key> {
    if input.keyboard.down(InputCode::key_control_left) {
        return None;
    }
    let key = input.keyboard.pressed(InputCode::key_left).then_some(Key::Left);
    let key = key.or(input.keyboard.pressed(InputCode::key_right).then_some(Key::Right));
    let key = key.or(input.keyboard.pressed(InputCode::key_home).then_some(Key::Home));
    let key = key.or(input.keyboard.pressed(InputCode::key_end).then_some(Key::End));
    let key = key.or(input
        .keyboard
        .pressed(InputCode::key_backspace)
        .then_some(Key::Backspace));
    let key = key.or(input.keyboard.pressed(InputCode::key_delete).then_some(Key::Delete));
    key.or(letter(input).map(Key::Char))
}

fn letter(input: &InputSystem) -> Option<char> {
    const LETTERS: [(InputCode, char); 26] = [
        (InputCode::key_a, 'a'),
        (InputCode::key_b, 'b'),
        (InputCode::key_c, 'c'),
        (InputCode::key_d, 'd'),
        (InputCode::key_e, 'e'),
        (InputCode::key_f, 'f'),
        (InputCode::key_g, 'g'),
        (InputCode::key_h, 'h'),
        (InputCode::key_i, 'i'),
        (InputCode::key_j, 'j'),
        (InputCode::key_k, 'k'),
        (InputCode::key_l, 'l'),
        (InputCode::key_m, 'm'),
        (InputCode::key_n, 'n'),
        (InputCode::key_o, 'o'),
        (InputCode::key_p, 'p'),
        (InputCode::key_q, 'q'),
        (InputCode::key_r, 'r'),
        (InputCode::key_s, 's'),
        (InputCode::key_t, 't'),
        (InputCode::key_u, 'u'),
        (InputCode::key_v, 'v'),
        (InputCode::key_w, 'w'),
        (InputCode::key_x, 'x'),
        (InputCode::key_y, 'y'),
        (InputCode::key_z, 'z'),
    ];
    let shift = shifted(input);
    LETTERS.into_iter().find_map(|(code, ch)| {
        if input.keyboard.pressed(code) {
            Some(if shift { ch.to_ascii_uppercase() } else { ch })
        } else {
            None
        }
    })
}

fn shifted(input: &InputSystem) -> bool {
    input.keyboard.down(InputCode::key_shift_left) || input.keyboard.down(InputCode::key_shift_right)
}

fn focused_field(items: &[Item], id: Option<u32>) -> Option<&Item> {
    let id = id?;
    items.iter().find(|item| item.id == id && matches!(item.kind, Kind::TextField { .. }))
}

fn clamp(caret: Caret, len: usize) -> Caret {
    Caret {
        anchor: caret.anchor.min(len),
        head: caret.head.min(len),
    }
}

fn ends(caret: Caret) -> (usize, usize) {
    (caret.anchor.min(caret.head), caret.anchor.max(caret.head))
}

fn step(caret: &mut Caret, len: usize, dir: i32, keep_anchor: bool) {
    if dir < 0 {
        caret.head = caret.head.saturating_sub(1);
    } else if caret.head < len {
        caret.head += 1;
    }
    if !keep_anchor {
        caret.anchor = caret.head;
    }
}

fn insert(text: &str, caret: &mut Caret, ch: char) -> String {
    let (lo, hi) = ends(*caret);
    let mut next = text.to_string();
    if lo != hi {
        next.replace_range(lo..hi, "");
        caret.head = lo;
    }
    next.insert(caret.head, ch);
    caret.head += 1;
    caret.anchor = caret.head;
    next
}

fn delete_before(text: &str, caret: &mut Caret) -> Option<String> {
    let (lo, hi) = ends(*caret);
    let mut next = text.to_string();
    if lo != hi {
        next.replace_range(lo..hi, "");
        caret.anchor = lo;
        caret.head = lo;
        return Some(next);
    }
    if caret.head == 0 {
        return None;
    }
    let at = caret.head - 1;
    next.remove(at);
    caret.anchor = at;
    caret.head = at;
    Some(next)
}

fn delete_after(text: &str, caret: &mut Caret) -> Option<String> {
    let (lo, hi) = ends(*caret);
    if lo != hi {
        return delete_before(text, caret);
    }
    if caret.head >= text.len() {
        return None;
    }
    let mut next = text.to_string();
    next.remove(caret.head);
    caret.anchor = caret.head;
    Some(next)
}

fn cells(count: usize) -> f32 {
    let count = u16::try_from(count).unwrap_or(u16::MAX);
    f32::from(count) * text::CELL_W
}
