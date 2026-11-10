//! Cocoa smoke. There is no libtest harness, so `main` is the process main thread.
//!
//! A `#[test]` in the library runs on a worker. Sync-dispatching that worker
//! onto the main queue deadlocks, because libtest's main is blocked joining it.

#[cfg_attr(
    not(target_os = "macos"),
    expect(
        clippy::unnecessary_wraps,
        reason = "the macOS path returns the window error"
    )
)]
fn main() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    open_cocoa()?;
    #[cfg(not(target_os = "macos"))]
    println!("cocoa smoke skipped: not macOS");
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_cocoa() -> Result<(), String> {
    use genos_window::{Window, WindowMode};

    unsafe extern "C" {
        fn pthread_main_np() -> i32;
    }

    // SAFETY: `pthread_main_np` is the libSystem main-thread check. A non-zero
    // result is the process main thread, which this binary is.
    if unsafe { pthread_main_np() } == 0 {
        return Err("cocoa smoke is not the process main thread".into());
    }
    let mut window = Window::open(320, 200).map_err(|err| format!("cocoa window: {err}"))?;
    let frame = window.pump();
    if frame.width < 200 {
        return Err(format!("client width {}", frame.width));
    }
    if frame.height < 100 {
        return Err(format!("client height {}", frame.height));
    }
    if window.display.is_null() {
        return Err("CAMetalLayer".into());
    }
    if window.display != window.surface {
        return Err("display and surface differ".into());
    }
    window
        .set_mode(WindowMode::Fullscreen)
        .map_err(|err| format!("fullscreen: {err}"))?;
    let fullscreen = window.pump();
    if fullscreen.mode != WindowMode::Fullscreen {
        return Err("fullscreen was not accepted".into());
    }
    window
        .set_mode(WindowMode::Windowed)
        .map_err(|err| format!("windowed: {err}"))?;
    let windowed = window.pump();
    if windowed.mode != WindowMode::Windowed {
        return Err("windowed was not accepted".into());
    }
    window
        .lock_cursor()
        .map_err(|err| format!("capture: {err}"))?;
    let locked = window.pump();
    if !locked.pointer_locked {
        return Err("pointer was not captured".into());
    }
    window
        .unlock_cursor()
        .map_err(|err| format!("release: {err}"))?;
    let free = window.pump();
    if free.pointer_locked {
        return Err("pointer stayed captured".into());
    }
    window.set_title("Genos smoke");
    println!("cocoa smoke ok");
    Ok(())
}
