#![cfg(not(any(target_os = "windows", target_os = "macos")))]
#![expect(
    clippy::expect_used,
    reason = "the Xvfb smoke aborts when the X11 window or the backend id is wrong"
)]

//! Headless X11 smoke. Run it under Xvfb with `WAYLAND_DISPLAY` unset.
//!
//! A normal `cargo test` skips this when Wayland is the session, because that
//! session must stay on the Wayland window. CI unsets `WAYLAND_DISPLAY`.

use genos_input::InputSystem;
use genos_window::{Window, WindowMode};

extern "C" {
    fn genos_window_backend() -> i32;
}

#[allow(
    clippy::disallowed_methods,
    reason = "the smoke reads the session DISPLAY and WAYLAND_DISPLAY. It is not a tuning knob."
)]
fn x11_session() -> bool {
    let display = std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty());
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
    display && !wayland
}

#[test]
fn an_x11_display_opens_pumps_and_writes_devices() {
    if !x11_session() {
        eprintln!("x11 smoke skipped: need DISPLAY and an empty WAYLAND_DISPLAY");
        return;
    }
    let mut window = Window::open(320, 200).expect("x11 window");
    let frame = window.pump();
    assert!(frame.width >= 1 && frame.height >= 1, "{}x{}", frame.width, frame.height);
    assert!(!frame.minimized);
    // SAFETY: the window open above set this process-wide backend id.
    let backend = unsafe { genos_window_backend() };
    assert_eq!(backend, 2, "X11 backend id");
    let mut input = InputSystem::new();
    input.begin_frame();
    window.write_devices(&mut input, &genos_window::GatedInput::default());
    window.set_mode(WindowMode::Fullscreen).expect("fullscreen request");
    window.set_title("Genos X11");
    let _ = window.pump();
    window.lock_cursor().expect("pointer grab");
    let locked = window.pump();
    assert!(locked.pointer_locked);
    window.unlock_cursor().expect("pointer release");
    let unlocked = window.pump();
    assert!(!unlocked.pointer_locked);
}
