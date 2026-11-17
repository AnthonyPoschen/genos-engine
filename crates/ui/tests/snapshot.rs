//! Canonical rectangle text. Empty and fully transparent rectangles drop out.

use genos_ui::{paint_snapshot, Paint};

#[test]
fn paint_snapshot_orders_rects_and_drops_empty_ones() {
    let paints = vec![
        Paint {
            x: 10.0,
            y: 2.0,
            w: 4.0,
            h: 5.0,
            color: [1.0, 0.0, 0.0],
            alpha: 1.0,
        },
        Paint {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            color: [0.0, 1.0, 0.5],
            alpha: 1.0,
        },
        Paint {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 4.0,
            color: [0.0, 0.0, 0.0],
            alpha: 1.0,
        },
        Paint {
            x: 4.0,
            y: 4.0,
            w: 2.0,
            h: 2.0,
            color: [0.0, 0.0, 0.0],
            alpha: 0.0,
        },
        Paint {
            x: 8.0,
            y: 1.0,
            w: 2.0,
            h: 2.0,
            color: [0.2, 0.2, 0.2],
            alpha: 0.5,
        },
    ];
    assert_eq!(
        paint_snapshot(&paints),
        "8.00 1.00 2.00 2.00 0.20 0.20 0.20 0.50\n1.00 2.00 3.00 4.00 0.00 1.00 0.50\n10.00 2.00 4.00 5.00 1.00 0.00 0.00\n"
    );
}
