//! Touch points and the gestures an action map can bind.
//!
//! The window writes points. This module does not read the operating system.
//! A tap, a swipe, and a pinch are recognized here. The action map binds each
//! one to an action.

/// How far a finger may drift and still count as a tap, in the same units as the point.
pub const TAP_SLOP: f32 = 16.0;
/// How far a finger must travel before the lift is a swipe.
pub const SWIPE_MIN: f32 = 48.0;
/// A pinch is reported when the two-finger span changes by at least this fraction.
const PINCH_FRACTION: f32 = 0.1;

/// One sample of a finger. The platform sends the phase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchPoint {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub phase: TouchPhase,
}

/// The life of one finger for one sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

/// A gesture recognized from touch points this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Tap {
        id: u64,
        x: f32,
        y: f32,
    },
    /// Unit direction of the finger's travel. Positive axes follow the point coordinates.
    Swipe {
        id: u64,
        x: f32,
        y: f32,
    },
    /// `scale` is the new two-finger span divided by the previous span. `1` is unchanged.
    Pinch {
        scale: f32,
    },
}

/// Which gesture an action listens to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    Tap,
    Swipe,
    Pinch,
}

impl GestureKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tap => "tap",
            Self::Swipe => "swipe",
            Self::Pinch => "pinch",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "tap" => Some(Self::Tap),
            "swipe" => Some(Self::Swipe),
            "pinch" => Some(Self::Pinch),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Contact {
    id: u64,
    x: f32,
    y: f32,
    origin_x: f32,
    origin_y: f32,
}

#[derive(Clone, Debug, Default)]
pub struct TouchSurface {
    contacts: Vec<Contact>,
    reported: Vec<TouchPoint>,
    gestures: Vec<Gesture>,
    pinch_span: Option<f32>,
}

impl TouchSurface {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_frame(&mut self) {
        self.gestures.clear();
        self.reported.clear();
    }

    /// Apply this frame's points, in order. Gestures accumulate until [`Self::begin_frame`].
    pub fn apply(&mut self, points: &[TouchPoint]) {
        self.reported = points.to_vec();
        for point in points {
            match point.phase {
                TouchPhase::Began => self.track(point, true),
                TouchPhase::Moved => self.track(point, false),
                TouchPhase::Ended => self.end(point),
                TouchPhase::Cancelled => {
                    self.contacts.retain(|contact| contact.id != point.id);
                }
            }
        }
        self.pinch();
    }

    #[must_use]
    pub fn gestures(&self) -> &[Gesture] {
        &self.gestures
    }

    #[must_use]
    pub fn points(&self) -> &[TouchPoint] {
        &self.reported
    }

    #[must_use]
    pub fn connected(&self) -> bool {
        !self.contacts.is_empty()
    }

    fn track(&mut self, point: &TouchPoint, began: bool) {
        if let Some(contact) = self
            .contacts
            .iter_mut()
            .find(|contact| contact.id == point.id)
        {
            if began {
                contact.origin_x = point.x;
                contact.origin_y = point.y;
            }
            contact.x = point.x;
            contact.y = point.y;
            return;
        }
        self.contacts.push(Contact {
            id: point.id,
            x: point.x,
            y: point.y,
            origin_x: point.x,
            origin_y: point.y,
        });
    }

    fn end(&mut self, point: &TouchPoint) {
        let Some(index) = self
            .contacts
            .iter()
            .position(|contact| contact.id == point.id)
        else {
            return;
        };
        let contact = self.contacts.remove(index);
        let dx = point.x - contact.origin_x;
        let dy = point.y - contact.origin_y;
        let length = dx.hypot(dy);
        if length >= SWIPE_MIN {
            self.gestures.push(Gesture::Swipe {
                id: point.id,
                x: dx / length,
                y: dy / length,
            });
        } else if length <= TAP_SLOP {
            self.gestures.push(Gesture::Tap {
                id: point.id,
                x: point.x,
                y: point.y,
            });
        }
    }

    fn pinch(&mut self) {
        if self.contacts.len() < 2 {
            self.pinch_span = None;
            return;
        }
        let span = (self.contacts[0].x - self.contacts[1].x)
            .hypot(self.contacts[0].y - self.contacts[1].y);
        if let Some(previous) = self.pinch_span {
            if previous > 1.0 {
                let scale = span / previous;
                if (scale - 1.0).abs() >= PINCH_FRACTION {
                    self.gestures.push(Gesture::Pinch { scale });
                }
            }
        }
        self.pinch_span = Some(span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action2dBinding, ActionMap};
    use crate::device::GlyphSet;
    use crate::system::InputSystem;

    fn point(id: u64, x: f32, y: f32, phase: TouchPhase) -> TouchPoint {
        TouchPoint { id, x, y, phase }
    }

    #[test]
    fn a_short_lift_is_a_tap_and_a_long_lift_is_a_swipe() {
        let mut input = InputSystem::new();
        let mut map = ActionMap::new();
        map.set("jump", Vec::new());
        map.set_gesture("jump", Some(GestureKind::Tap)).unwrap();
        map.set_2d("move", Action2dBinding::default());
        map.set_gesture("move", Some(GestureKind::Swipe)).unwrap();

        input.set_touch_points(&[
            point(1, 10.0, 10.0, TouchPhase::Began),
            point(1, 12.0, 14.0, TouchPhase::Ended),
        ]);
        assert!(map.down(&input, "jump"));
        assert!(!map.down(&input, "move"));
        assert_eq!(input.active_glyphs(), GlyphSet::Touch);
        assert!(input
            .gestures()
            .iter()
            .any(|gesture| matches!(gesture, Gesture::Tap { .. })));

        input.begin_frame();
        input.set_touch_points(&[
            point(2, 0.0, 0.0, TouchPhase::Began),
            point(2, 0.0, 80.0, TouchPhase::Ended),
        ]);
        assert!(!map.down(&input, "jump"));
        let axis = map.axis_2d(&input, "move");
        assert!(
            (axis.y - 1.0).abs() < 1.0e-4,
            "a downward swipe is +Y in point space, got {axis:?}"
        );
        assert!(axis.x.abs() < 1.0e-4);
    }

    #[test]
    fn a_cancelled_finger_and_a_short_drag_make_no_gesture() {
        let mut surface = TouchSurface::new();
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(1, 1.0, 1.0, TouchPhase::Cancelled),
        ]);
        assert_eq!(surface.gestures(), &[] as &[Gesture]);
        assert!(!surface.connected());

        surface.begin_frame();
        surface.apply(&[
            point(3, 0.0, 0.0, TouchPhase::Began),
            point(3, 30.0, 0.0, TouchPhase::Ended),
        ]);
        assert_eq!(
            surface.gestures(),
            &[] as &[Gesture],
            "30 is past the tap slop and short of a swipe"
        );
    }

    #[test]
    fn two_fingers_moving_apart_pinch() {
        let mut input = InputSystem::new();
        let mut map = ActionMap::new();
        map.set_2d("zoom", Action2dBinding::default());
        map.set_gesture("zoom", Some(GestureKind::Pinch)).unwrap();

        input.set_touch_points(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(2, 40.0, 0.0, TouchPhase::Began),
        ]);
        assert!(map.axis_2d(&input, "zoom").y.abs() < 1.0e-4);

        input.begin_frame();
        input.set_touch_points(&[
            point(1, 0.0, 0.0, TouchPhase::Moved),
            point(2, 80.0, 0.0, TouchPhase::Moved),
        ]);
        let axis = map.axis_2d(&input, "zoom");
        assert!(
            (axis.y - 1.0).abs() < 1.0e-4,
            "span doubled, so the axis saturates at 1, got {axis:?}"
        );
        assert!(input.dump().contains("touch"));
    }
}
