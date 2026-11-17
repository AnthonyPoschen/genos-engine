//! ASCII caret for a [`crate::Kind::TextField`].
//!
//! `anchor == head` is a caret. A different pair is a selection. Indexes are
//! bytes, and a non-ASCII string is left unchanged. The game copies
//! [`crate::Event::Text`] onto the kind on the next frame.

use genos_input::{InputCode, InputSystem};

use crate::kind::{Event, Kind};
use crate::layout::{self, Item, Rect};
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
    let (held, pressed) = consumed(input);
    if !held {
        return false;
    }
    if !text.is_ascii() {
        return true;
    }
    let Some(key) = pressed else {
        return true;
    };
    if control(input) {
        return true;
    }
    let before = clamp(state.caret(item.id), text.len());
    let mut caret = before;
    let shift = shifted(input);
    let edited = match key {
        Key::Left => {
            step(&mut caret, text.len(), -1, shift);
            None
        }
        Key::Right => {
            step(&mut caret, text.len(), 1, shift);
            None
        }
        Key::Up | Key::Down => None,
        Key::Home => {
            caret.head = 0;
            if !shift {
                caret.anchor = 0;
            }
            None
        }
        Key::End => {
            caret.head = text.len();
            if !shift {
                caret.anchor = caret.head;
            }
            None
        }
        Key::Backspace => delete_before(text, &mut caret),
        Key::Delete => delete_after(text, &mut caret),
        Key::Char(ch) => Some(insert(text, &mut caret, ch)),
    };
    if caret != before {
        state.carets.insert(item.id, caret);
    }
    if let Some(next) = edited {
        events.push(Event::Text {
            id: item.id,
            text: next,
        });
    }
    true
}

pub(crate) fn paint(out: &mut Vec<Paint>, item: &Item, caret: Option<Caret>, theme: &Theme) {
    let Kind::TextField {
        text,
        multiline: false,
    } = &item.kind
    else {
        return;
    };
    let clip = item.content;
    let caret = caret
        .filter(|_| text.is_ascii())
        .map(|caret| clamp(caret, text.len()));
    if let Some(caret) = caret {
        let (lo, hi) = ends(caret);
        if hi > lo {
            push_clipped(
                out,
                Rect {
                    x: clip.x + cells(lo),
                    y: clip.y,
                    w: cells(hi - lo),
                    h: text::CELL_H,
                },
                clip,
                theme.accent,
            );
        }
    }
    if text.is_ascii() && item.face == TextFace::Bitmap {
        text::paint_ascii_line(
            out,
            text,
            clip.x,
            clip.y,
            [clip.x, clip.y, clip.w, clip.h],
            theme.text,
        );
    }
    let Some(caret) = caret else {
        return;
    };
    push_clipped(
        out,
        Rect {
            x: clip.x + cells(caret.head),
            y: clip.y,
            w: 2.0,
            h: text::CELL_H,
        },
        clip,
        theme.text,
    );
}

fn push_clipped(out: &mut Vec<Paint>, rect: Rect, clip: Rect, color: [f32; 3]) {
    let Some(rect) = layout::intersect(rect, clip) else {
        return;
    };
    out.push(Paint {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        color,
        alpha: 1.0,
    });
}

#[derive(Clone, Copy)]
enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Char(char),
}

fn consumed(input: &InputSystem) -> (bool, Option<Key>) {
    let mut held = false;
    let mut pressed = None;
    for (code, key) in COMMANDS {
        note(input, code, key, &mut held, &mut pressed);
    }
    let shift = shifted(input);
    for (code, plain, shifted_ch) in PRINTABLE {
        let ch = if shift { shifted_ch } else { plain };
        note(input, code, Key::Char(ch), &mut held, &mut pressed);
    }
    (held, pressed)
}

fn note(
    input: &InputSystem,
    code: InputCode,
    key: Key,
    held: &mut bool,
    pressed: &mut Option<Key>,
) {
    if !input.keyboard.down(code) {
        return;
    }
    *held = true;
    if pressed.is_none() && input.keyboard.pressed(code) {
        *pressed = Some(key);
    }
}

fn control(input: &InputSystem) -> bool {
    input.keyboard.down(InputCode::key_control_left)
        || input.keyboard.down(InputCode::key_control_right)
}

fn shifted(input: &InputSystem) -> bool {
    input.keyboard.down(InputCode::key_shift_left)
        || input.keyboard.down(InputCode::key_shift_right)
}

fn focused_field(items: &[Item], id: Option<u32>) -> Option<&Item> {
    let id = id?;
    items
        .iter()
        .find(|item| item.id == id && matches!(item.kind, Kind::TextField { .. }))
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
    let (lo, hi) = ends(*caret);
    if !keep_anchor && lo != hi {
        let edge = if dir < 0 { lo } else { hi };
        caret.anchor = edge;
        caret.head = edge;
        return;
    }
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

const COMMANDS: [(InputCode, Key); 8] = [
    (InputCode::key_left, Key::Left),
    (InputCode::key_right, Key::Right),
    (InputCode::key_up, Key::Up),
    (InputCode::key_down, Key::Down),
    (InputCode::key_home, Key::Home),
    (InputCode::key_end, Key::End),
    (InputCode::key_backspace, Key::Backspace),
    (InputCode::key_delete, Key::Delete),
];

/// Unshifted character, then the shifted one. Space stays a space.
const PRINTABLE: [(InputCode, char, char); 40] = [
    (InputCode::key_space, ' ', ' '),
    (InputCode::key_0, '0', ')'),
    (InputCode::key_1, '1', '!'),
    (InputCode::key_2, '2', '@'),
    (InputCode::key_3, '3', '#'),
    (InputCode::key_4, '4', '$'),
    (InputCode::key_5, '5', '%'),
    (InputCode::key_6, '6', '^'),
    (InputCode::key_7, '7', '&'),
    (InputCode::key_8, '8', '*'),
    (InputCode::key_9, '9', '('),
    (InputCode::key_a, 'a', 'A'),
    (InputCode::key_b, 'b', 'B'),
    (InputCode::key_c, 'c', 'C'),
    (InputCode::key_d, 'd', 'D'),
    (InputCode::key_e, 'e', 'E'),
    (InputCode::key_f, 'f', 'F'),
    (InputCode::key_g, 'g', 'G'),
    (InputCode::key_h, 'h', 'H'),
    (InputCode::key_i, 'i', 'I'),
    (InputCode::key_j, 'j', 'J'),
    (InputCode::key_k, 'k', 'K'),
    (InputCode::key_l, 'l', 'L'),
    (InputCode::key_m, 'm', 'M'),
    (InputCode::key_n, 'n', 'N'),
    (InputCode::key_o, 'o', 'O'),
    (InputCode::key_p, 'p', 'P'),
    (InputCode::key_q, 'q', 'Q'),
    (InputCode::key_r, 'r', 'R'),
    (InputCode::key_s, 's', 'S'),
    (InputCode::key_t, 't', 'T'),
    (InputCode::key_u, 'u', 'U'),
    (InputCode::key_v, 'v', 'V'),
    (InputCode::key_w, 'w', 'W'),
    (InputCode::key_x, 'x', 'X'),
    (InputCode::key_y, 'y', 'Y'),
    (InputCode::key_z, 'z', 'Z'),
    (InputCode::key_grave, '`', '~'),
    (InputCode::key_left_bracket, '[', '{'),
    (InputCode::key_right_bracket, ']', '}'),
];
