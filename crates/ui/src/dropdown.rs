//! Open list for a panel dropdown.
//!
//! The closed box stays in the row. The list is a separate rectangle, so it
//! paints over whatever is under it. A window of [`VISIBLE`] rows scrolls.

use crate::layout::Rect;

/// Choice rows kept on screen. The rest scroll into this window.
pub(crate) const VISIBLE: usize = 6;
pub(crate) const ROW_H: f32 = 20.0;

/// Visible count and a scroll index clamped into range.
pub(crate) fn window(count: usize, scroll: usize) -> (usize, usize) {
    let visible = count.min(VISIBLE);
    let scroll = scroll.min(count.saturating_sub(visible));
    (visible, scroll)
}

/// List rectangle under the closed box, or above it when the viewport is short.
pub(crate) fn place(anchor: Rect, visible: usize, viewport_h: f32) -> Rect {
    let h = visible as f32 * ROW_H;
    let mut y = anchor.y + anchor.h;
    if y + h > viewport_h && anchor.y >= h {
        y = anchor.y - h;
    }
    Rect {
        x: anchor.x,
        y,
        w: anchor.w,
        h,
    }
}

/// Choice index under the pointer, or `None` when the point misses the list.
pub(crate) fn row_at(list: Rect, scroll: usize, count: usize, x: f32, y: f32) -> Option<usize> {
    if count == 0 || !list.contains(x, y) {
        return None;
    }
    let row = ((y - list.y) / ROW_H).floor() as usize;
    let index = scroll + row;
    (index < count).then_some(index)
}

/// Move the highlight by `delta` and keep it inside the choices.
pub(crate) fn move_highlight(highlight: usize, delta: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let last = count as i32 - 1;
    (highlight as i32 + delta).clamp(0, last) as usize
}

/// Scroll so `highlight` sits inside the window.
pub(crate) fn reveal(scroll: usize, highlight: usize, visible: usize, count: usize) -> usize {
    if visible == 0 || count == 0 {
        return 0;
    }
    let mut scroll = scroll.min(count.saturating_sub(visible));
    if highlight < scroll {
        scroll = highlight;
    } else if highlight >= scroll + visible {
        scroll = highlight + 1 - visible;
    }
    scroll.min(count.saturating_sub(visible))
}

/// Wheel notches. Positive `wheel_y` (wheel up) moves toward the first choice.
pub(crate) fn scroll_by(scroll: usize, wheel_steps: i32, visible: usize, count: usize) -> usize {
    let max_scroll = count.saturating_sub(visible) as i32;
    (scroll as i32 - wheel_steps).clamp(0, max_scroll) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_clamps_and_the_highlight_scrolls_into_view() {
        assert_eq!(window(2, 9), (2, 0));
        assert_eq!(window(10, 0), (VISIBLE, 0));
        assert_eq!(window(10, 8), (VISIBLE, 4));
        assert_eq!(reveal(0, 7, VISIBLE, 10), 2);
        assert_eq!(reveal(4, 1, VISIBLE, 10), 1);
        assert_eq!(move_highlight(0, -1, 4), 0);
        assert_eq!(move_highlight(3, 1, 4), 3);
        assert_eq!(scroll_by(0, -1, VISIBLE, 10), 1);
        assert_eq!(scroll_by(2, 1, VISIBLE, 10), 1);
    }

    #[test]
    fn a_short_viewport_flips_the_list_above_the_box() {
        let anchor = Rect {
            x: 10.0,
            y: 200.0,
            w: 80.0,
            h: 24.0,
        };
        let below = place(anchor, 6, 400.0);
        assert!((below.y - (anchor.y + anchor.h)).abs() < 0.1);
        let above = place(anchor, 6, 230.0);
        assert!((above.y + above.h - anchor.y).abs() < 0.1);
        assert_eq!(
            row_at(below, 0, 6, below.x + 1.0, below.y + ROW_H + 1.0),
            Some(1)
        );
        assert_eq!(row_at(below, 2, 8, below.x + 1.0, below.y + 1.0), Some(2));
        assert_eq!(row_at(below, 0, 6, below.x - 1.0, below.y + 1.0), None);
    }
}
