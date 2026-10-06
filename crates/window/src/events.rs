//! Window facts a game or a widget can subscribe to.
//!
//! The compositor still arrives through [`crate::Window::pump`]. These events are the edges in that stream.

use crate::Frame;

/// How the compositor is showing the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMode {
    Windowed,
    Fullscreen,
}

/// One change since the previous pump, or the current fact given to a new subscriber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    Resized { width: u32, height: u32 },
    Focused,
    Unfocused,
    CursorLocked,
    CursorUnlocked,
    ModeChanged(WindowMode),
}

#[derive(Clone, Copy, Debug)]
struct Observed {
    width: u32,
    height: u32,
    focused: bool,
    cursor_locked: bool,
    mode: WindowMode,
    seen: bool,
}

impl Default for Observed {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            focused: false,
            cursor_locked: false,
            mode: WindowMode::Windowed,
            seen: false,
        }
    }
}

pub(crate) struct EventHub {
    listeners: Vec<Box<dyn FnMut(&WindowEvent)>>,
    observed: Observed,
}

impl Default for EventHub {
    fn default() -> Self {
        Self {
            listeners: Vec::new(),
            observed: Observed::default(),
        }
    }
}

impl EventHub {
    pub(crate) fn from_frame(frame: &Frame) -> Self {
        Self {
            listeners: Vec::new(),
            observed: Observed {
                width: frame.width,
                height: frame.height,
                focused: frame.focused,
                cursor_locked: frame.pointer_locked,
                mode: frame.mode,
                seen: true,
            },
        }
    }

    pub(crate) fn on(&mut self, mut listener: impl FnMut(&WindowEvent) + 'static) {
        if self.observed.seen {
            listener(&WindowEvent::Resized {
                width: self.observed.width,
                height: self.observed.height,
            });
            if self.observed.focused {
                listener(&WindowEvent::Focused);
            }
            if self.observed.cursor_locked {
                listener(&WindowEvent::CursorLocked);
            }
            if self.observed.mode == WindowMode::Fullscreen {
                listener(&WindowEvent::ModeChanged(WindowMode::Fullscreen));
            }
        }
        self.listeners.push(Box::new(listener));
    }

    pub(crate) fn apply(&mut self, frame: &Frame) {
        let events = changes(&mut self.observed, frame);
        for event in &events {
            for listener in &mut self.listeners {
                listener(event);
            }
        }
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.observed.width, self.observed.height)
    }

    pub(crate) fn focused(&self) -> bool {
        self.observed.focused
    }

    pub(crate) fn cursor_locked(&self) -> bool {
        self.observed.cursor_locked
    }

    pub(crate) fn mode(&self) -> WindowMode {
        self.observed.mode
    }
}

fn changes(observed: &mut Observed, frame: &Frame) -> Vec<WindowEvent> {
    let mut events = Vec::new();
    if !observed.seen || frame.width != observed.width || frame.height != observed.height {
        events.push(WindowEvent::Resized {
            width: frame.width,
            height: frame.height,
        });
    }
    if observed.seen && frame.focused != observed.focused {
        events.push(if frame.focused {
            WindowEvent::Focused
        } else {
            WindowEvent::Unfocused
        });
    } else if !observed.seen && frame.focused {
        events.push(WindowEvent::Focused);
    }
    if observed.seen && frame.pointer_locked != observed.cursor_locked {
        events.push(if frame.pointer_locked {
            WindowEvent::CursorLocked
        } else {
            WindowEvent::CursorUnlocked
        });
    } else if !observed.seen && frame.pointer_locked {
        events.push(WindowEvent::CursorLocked);
    }
    if observed.seen && frame.mode != observed.mode {
        events.push(WindowEvent::ModeChanged(frame.mode));
    } else if !observed.seen && frame.mode == WindowMode::Fullscreen {
        events.push(WindowEvent::ModeChanged(WindowMode::Fullscreen));
    }
    observed.width = frame.width;
    observed.height = frame.height;
    observed.focused = frame.focused;
    observed.cursor_locked = frame.pointer_locked;
    observed.mode = frame.mode;
    observed.seen = true;
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn frame(width: u32, height: u32, focused: bool, locked: bool, mode: WindowMode) -> Frame {
        Frame {
            width,
            height,
            closing: false,
            focused,
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            capture_click: false,
            key_w: false,
            key_a: false,
            key_s: false,
            key_d: false,
            escape: false,
            resized: false,
            mouse_left: false,
            keys_down: [0; 256],
            pointer_locked: locked,
            clicked_while_focused: false,
            mode,
            pointer_x: 0.0,
            pointer_y: 0.0,
        }
    }

    #[test]
    fn subscribers_hear_resize_focus_cursor_and_mode() {
        let mut hub = EventHub::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let seen = log.clone();
        hub.on(move |event| seen.borrow_mut().push(*event));

        hub.apply(&frame(1280, 720, false, false, WindowMode::Windowed));
        hub.apply(&frame(1280, 720, false, false, WindowMode::Windowed));
        hub.apply(&frame(1280, 720, true, false, WindowMode::Windowed));
        hub.apply(&frame(800, 600, true, false, WindowMode::Windowed));
        hub.apply(&frame(800, 600, false, false, WindowMode::Windowed));
        hub.apply(&frame(800, 600, false, true, WindowMode::Windowed));
        hub.apply(&frame(800, 600, false, false, WindowMode::Windowed));
        hub.apply(&frame(800, 600, false, false, WindowMode::Fullscreen));
        hub.apply(&frame(1920, 1080, false, false, WindowMode::Windowed));

        assert_eq!(
            log.borrow().as_slice(),
            &[
                WindowEvent::Resized {
                    width: 1280,
                    height: 720
                },
                WindowEvent::Focused,
                WindowEvent::Resized {
                    width: 800,
                    height: 600
                },
                WindowEvent::Unfocused,
                WindowEvent::CursorLocked,
                WindowEvent::CursorUnlocked,
                WindowEvent::ModeChanged(WindowMode::Fullscreen),
                WindowEvent::Resized {
                    width: 1920,
                    height: 1080
                },
                WindowEvent::ModeChanged(WindowMode::Windowed),
            ]
        );
    }

    #[test]
    fn a_late_subscriber_receives_the_current_window() {
        let mut hub = EventHub::from_frame(&frame(800, 600, true, true, WindowMode::Fullscreen));
        let log = Rc::new(RefCell::new(Vec::new()));
        let seen = log.clone();
        hub.on(move |event| seen.borrow_mut().push(*event));
        assert_eq!(
            log.borrow().as_slice(),
            &[
                WindowEvent::Resized {
                    width: 800,
                    height: 600
                },
                WindowEvent::Focused,
                WindowEvent::CursorLocked,
                WindowEvent::ModeChanged(WindowMode::Fullscreen),
            ]
        );
        assert_eq!(hub.size(), (800, 600));
        assert!(hub.focused());
        assert!(hub.cursor_locked());
        assert_eq!(hub.mode(), WindowMode::Fullscreen);
    }
}
