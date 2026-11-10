//! Touch points and the gestures an action map can bind.
//!
//! The window writes points. This module does not read the operating system.
//! A tap, a swipe, and a pinch are recognized here. The action map binds each
//! one to an action.
//!
//! Coordinates are points, the pixel space the window already uses. Positive Y
//! follows that space. This module does not flip Y.

/// How far a finger may drift and still count as a tap, in points (pixels).
pub const TAP_SLOP: f32 = 16.0;
/// How far a finger must travel before the lift is a swipe, in points (pixels).
pub const SWIPE_MIN: f32 = 48.0;
/// A pinch is reported when the span has changed by at least this fraction
/// from the span at the start of the gesture, or from the last span that emitted.
const PINCH_FRACTION: f32 = 0.1;
/// A span at or below this, in points, is not a pinch baseline. Fingers that
/// start together set the baseline on the first sample that exceeds it.
const PINCH_MIN_SPAN: f32 = 1.0;

/// One sample of a finger. The platform sends the phase.
///
/// `x` and `y` are in points (pixels), the same space as [`TAP_SLOP`].
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
    /// `scale` is the new span divided by the span at gesture start, or by the
    /// last span that emitted. `1` is unchanged. It is not the previous sample.
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
    /// Span at the start of the pinch, or the span of the last emitted pinch.
    pinch_span: Option<f32>,
    /// The two contact ids that define the pinch. A lift of either one drops it.
    pinch_pair: Option<(u64, u64)>,
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
    ///
    /// Positions are written first, including an `Ended` sample. The pinch runs
    /// while those contacts are still present. Lifts are classified after that.
    pub fn apply(&mut self, points: &[TouchPoint]) {
        self.reported = points.to_vec();
        for point in points {
            match point.phase {
                TouchPhase::Began => self.track(point, true),
                TouchPhase::Moved => self.track(point, false),
                TouchPhase::Ended => self.place(point),
                TouchPhase::Cancelled => {}
            }
        }
        self.pinch();
        for point in points {
            if point.phase == TouchPhase::Ended {
                self.classify_lift(point);
            }
        }
        self.contacts.retain(|contact| {
            !points.iter().any(|point| {
                point.id == contact.id
                    && matches!(point.phase, TouchPhase::Ended | TouchPhase::Cancelled)
            })
        });
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

    fn place(&mut self, point: &TouchPoint) {
        if let Some(contact) = self
            .contacts
            .iter_mut()
            .find(|contact| contact.id == point.id)
        {
            contact.x = point.x;
            contact.y = point.y;
        }
    }

    fn classify_lift(&mut self, point: &TouchPoint) {
        let Some(contact) = self.contacts.iter().find(|contact| contact.id == point.id) else {
            return;
        };
        let dx = point.x - contact.origin_x;
        let dy = point.y - contact.origin_y;
        let length = dx.hypot(dy);
        if self.in_pinch_pair(point.id) {
            return;
        }
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

    fn in_pinch_pair(&self, id: u64) -> bool {
        self.pinch_pair
            .is_some_and(|(left, right)| id == left || id == right)
    }

    fn pinch(&mut self) {
        let Some((left, right)) = self.pinch_ids() else {
            self.pinch_span = None;
            self.pinch_pair = None;
            return;
        };
        if self.pinch_pair != Some((left, right)) {
            self.pinch_span = None;
            self.pinch_pair = Some((left, right));
        }
        let Some(span) = self.span_of(left, right) else {
            self.pinch_span = None;
            return;
        };
        let Some(baseline) = self.pinch_span else {
            if span > PINCH_MIN_SPAN {
                self.pinch_span = Some(span);
            }
            return;
        };
        let scale = span / baseline;
        if (scale - 1.0).abs() >= PINCH_FRACTION {
            self.gestures.push(Gesture::Pinch { scale });
            self.pinch_span = Some(span);
        }
    }

    /// Keep the same two ids while both are down. A lift picks a new pair.
    fn pinch_ids(&self) -> Option<(u64, u64)> {
        if let Some((left, right)) = self.pinch_pair {
            let left_down = self.contacts.iter().any(|contact| contact.id == left);
            let right_down = self.contacts.iter().any(|contact| contact.id == right);
            if left_down && right_down {
                return Some((left, right));
            }
        }
        self.fresh_pair()
    }

    fn fresh_pair(&self) -> Option<(u64, u64)> {
        let mut low = [u64::MAX; 2];
        let mut count = 0usize;
        for contact in &self.contacts {
            if count < 2 {
                low[count] = contact.id;
                count += 1;
                if count == 2 && low[1] < low[0] {
                    low.swap(0, 1);
                }
                continue;
            }
            if contact.id < low[0] {
                low[1] = low[0];
                low[0] = contact.id;
            } else if contact.id < low[1] {
                low[1] = contact.id;
            }
        }
        (count == 2).then_some((low[0], low[1]))
    }

    fn span_of(&self, left: u64, right: u64) -> Option<f32> {
        let a = self.contacts.iter().find(|contact| contact.id == left)?;
        let b = self.contacts.iter().find(|contact| contact.id == right)?;
        Some((a.x - b.x).hypot(a.y - b.y))
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

    #[test]
    fn a_slow_pinch_emits_before_the_span_doubles() {
        let mut surface = TouchSurface::new();
        let mut span = 100.0_f32;
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(2, span, 0.0, TouchPhase::Began),
        ]);
        assert_eq!(surface.gestures(), &[] as &[Gesture]);
        let mut emitted = false;
        for _ in 0..40 {
            span *= 1.023;
            surface.begin_frame();
            surface.apply(&[
                point(1, 0.0, 0.0, TouchPhase::Moved),
                point(2, span, 0.0, TouchPhase::Moved),
            ]);
            if surface
                .gestures()
                .iter()
                .any(|gesture| matches!(gesture, Gesture::Pinch { scale } if *scale >= 1.1))
            {
                emitted = true;
                break;
            }
        }
        assert!(
            emitted,
            "2.3% per frame should accumulate past a tenth, span {span}"
        );
        assert!(
            span < 200.0,
            "the pinch should emit before the span doubles, span {span}"
        );
    }

    #[test]
    fn fingers_that_start_together_can_pinch_out() {
        let mut surface = TouchSurface::new();
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(2, 0.0, 0.0, TouchPhase::Began),
        ]);
        surface.begin_frame();
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Moved),
            point(2, 0.5, 0.0, TouchPhase::Moved),
        ]);
        assert_eq!(
            surface.gestures(),
            &[] as &[Gesture],
            "a span of 0.5 is not a baseline"
        );
        surface.begin_frame();
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Moved),
            point(2, 20.0, 0.0, TouchPhase::Moved),
        ]);
        assert_eq!(
            surface.gestures(),
            &[] as &[Gesture],
            "the first span past the threshold only sets the baseline"
        );
        surface.begin_frame();
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Moved),
            point(2, 24.0, 0.0, TouchPhase::Moved),
        ]);
        let mut scale = None;
        for gesture in surface.gestures() {
            if let Gesture::Pinch { scale: value } = *gesture {
                scale = Some(value);
            }
        }
        assert!(
            scale.is_some(),
            "a pinch out from the baseline should emit, got {:?}",
            surface.gestures()
        );
        let scale = scale.unwrap();
        assert!((scale - 1.2).abs() < 1.0e-4, "24/20, got {scale}");
    }

    #[test]
    fn a_pinch_finger_does_not_also_swipe() {
        let mut input = InputSystem::new();
        let mut map = ActionMap::new();
        map.set_2d("look", Action2dBinding::default());
        map.set_gesture("look", Some(GestureKind::Swipe)).unwrap();
        map.set_sensitivity("look", 0.5).unwrap();
        input.set_touch_points(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(2, 80.0, 0.0, TouchPhase::Began),
        ]);
        input.begin_frame();
        input.set_touch_points(&[
            point(1, 0.0, 80.0, TouchPhase::Ended),
            point(2, 80.0, 0.0, TouchPhase::Moved),
        ]);
        assert!(
            !input
                .gestures()
                .iter()
                .any(|gesture| matches!(gesture, Gesture::Swipe { .. })),
            "a pinch finger must not arm a swipe, got {:?}",
            input.gestures()
        );
        assert!(map.axis_2d(&input, "look").x.abs() < 1.0e-4);
        assert!(map.axis_2d(&input, "look").y.abs() < 1.0e-4);
    }

    #[test]
    fn swipe_sensitivity_scales_before_the_clamp() {
        let mut input = InputSystem::new();
        let mut map = ActionMap::new();
        map.set_2d("look", Action2dBinding::default());
        map.set_gesture("look", Some(GestureKind::Swipe)).unwrap();
        map.set_sensitivity("look", 0.5).unwrap();
        input.set_touch_points(&[point(1, 0.0, 0.0, TouchPhase::Began)]);
        input.begin_frame();
        input.set_touch_points(&[point(1, 0.0, 80.0, TouchPhase::Ended)]);
        let axis = map.axis_2d(&input, "look");
        assert!(
            (axis.y - 0.5).abs() < 1.0e-4,
            "unit swipe times 0.5, got {axis:?}"
        );
        assert!(axis.x.abs() < 1.0e-4, "no sideways travel, got {axis:?}");
    }

    #[test]
    fn lifting_one_finger_does_not_pinch_the_remaining_pair() {
        let mut surface = TouchSurface::new();
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(2, 100.0, 0.0, TouchPhase::Began),
        ]);
        surface.begin_frame();
        surface.apply(&[
            point(3, 110.0, 0.0, TouchPhase::Began),
            point(1, 0.0, 0.0, TouchPhase::Moved),
            point(2, 100.0, 0.0, TouchPhase::Moved),
        ]);
        assert_eq!(surface.gestures(), &[] as &[Gesture]);
        surface.begin_frame();
        surface.apply(&[
            point(1, 0.0, 0.0, TouchPhase::Ended),
            point(2, 100.0, 0.0, TouchPhase::Moved),
            point(3, 110.0, 0.0, TouchPhase::Moved),
        ]);
        assert!(
            !surface
                .gestures()
                .iter()
                .any(|gesture| matches!(gesture, Gesture::Pinch { .. })),
            "the leftover fingers are a new pair, got {:?}",
            surface.gestures()
        );
    }

    #[test]
    fn action_json_keeps_the_gesture() {
        let mut map = ActionMap::new();
        map.set("jump", Vec::new());
        map.set_gesture("jump", Some(GestureKind::Pinch)).unwrap();
        let loaded = ActionMap::from_json(&map.to_json()).unwrap();
        assert!(
            loaded.to_text().contains("jump.gesture=pinch"),
            "{}",
            loaded.to_text()
        );
    }

    #[test]
    fn a_pinch_that_ends_in_the_same_batch_still_emits() {
        let mut input = InputSystem::new();
        let mut map = ActionMap::new();
        map.set("jump", Vec::new());
        map.set_gesture("jump", Some(GestureKind::Tap)).unwrap();
        map.set_2d("zoom", Action2dBinding::default());
        map.set_gesture("zoom", Some(GestureKind::Pinch)).unwrap();

        input.set_touch_points(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(2, 40.0, 0.0, TouchPhase::Began),
        ]);
        assert!(map.axis_2d(&input, "zoom").y.abs() < 1.0e-4);

        input.begin_frame();
        input.set_touch_points(&[
            point(1, 0.0, 0.0, TouchPhase::Ended),
            point(2, 80.0, 0.0, TouchPhase::Ended),
        ]);
        let mut scale = None;
        for gesture in input.gestures() {
            if let Gesture::Pinch { scale: value } = *gesture {
                scale = Some(value);
            }
        }
        assert!(
            scale.is_some(),
            "ending in the emitting batch should still pinch, got {:?}",
            input.gestures()
        );
        let scale = scale.unwrap();
        assert!((scale - 2.0).abs() < 1.0e-4, "80/40, got {scale}");
        let axis = map.axis_2d(&input, "zoom");
        assert!(
            (axis.y - 1.0).abs() < 1.0e-4,
            "scale 2 saturates the axis, got {axis:?}"
        );
        assert!(
            !map.down(&input, "jump"),
            "a pinch finger that lifts inside the tap slop is not a tap"
        );
    }

    #[test]
    fn pinch_scale_follows_the_action_sensitivity() {
        let mut input = InputSystem::new();
        let mut map = ActionMap::new();
        map.set_2d("zoom", Action2dBinding::default());
        map.set_gesture("zoom", Some(GestureKind::Pinch)).unwrap();
        map.set_sensitivity("zoom", 2.0).unwrap();

        input.set_touch_points(&[
            point(1, 0.0, 0.0, TouchPhase::Began),
            point(2, 100.0, 0.0, TouchPhase::Began),
        ]);
        input.begin_frame();
        input.set_touch_points(&[
            point(1, 0.0, 0.0, TouchPhase::Moved),
            point(2, 120.0, 0.0, TouchPhase::Moved),
        ]);
        let axis = map.axis_2d(&input, "zoom");
        assert!(
            (axis.y - 0.4).abs() < 1.0e-4,
            "(1.2 - 1) times sensitivity 2, got {axis:?}"
        );
    }
}
