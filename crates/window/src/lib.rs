//! Wayland window. Each pump pulls the seat, then tells subscribers what changed.

mod events;

pub use events::{WindowEvent, WindowMode};

use std::os::raw::{c_int, c_void};

use events::EventHub;

#[repr(C)]
struct RawPump {
    width: i32,
    height: i32,
    closing: i32,
    focused: i32,
    mouse_dx: i32,
    mouse_dy: i32,
    capture_click: i32,
    key_w: i32,
    key_a: i32,
    key_s: i32,
    key_d: i32,
    key_escape: i32,
    resized: i32,
    mouse_left: i32,
    keys_down: [u8; 256],
    pointer_locked: i32,
    clicked_while_focused: i32,
    fullscreen: i32,
    pointer_x: i32,
    pointer_y: i32,
    display: *mut c_void,
    surface: *mut c_void,
}

extern "C" {
    fn genos_window_open(width: c_int, height: c_int, app_id: *const i8, title: *const i8) -> *mut c_void;
    fn genos_window_pump(window: *mut c_void, out: *mut RawPump);
    #[cfg_attr(not(test), allow(dead_code))]
    fn genos_window_pump_size() -> c_int;
    fn genos_window_set_pointer_capture(window: *mut c_void, capture: c_int) -> c_int;
    fn genos_window_set_fullscreen(window: *mut c_void, fullscreen: c_int) -> c_int;
    fn genos_window_destroy(window: *mut c_void);
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub closing: bool,
    pub focused: bool,
    pub mouse_dx: f32,
    pub mouse_dy: f32,
    pub capture_click: bool,
    pub key_w: bool,
    pub key_a: bool,
    pub key_s: bool,
    pub key_d: bool,
    pub escape: bool,
    pub resized: bool,
    pub mouse_left: bool,
    pub keys_down: [u8; 256],
    pub pointer_locked: bool,
    /// Left click landed on this surface. Keyboard focus can be late or already gone.
    pub clicked_while_focused: bool,
    pub mode: WindowMode,
    /// Pointer position in window pixels. The origin is the top-left.
    pub pointer_x: f32,
    pub pointer_y: f32,
}

pub struct Window {
    raw: *mut c_void,
    pub display: *mut c_void,
    pub surface: *mut c_void,
    hub: EventHub,
}

impl Window {
    /// A normal desktop window. Hyprland can focus and tile it.
    pub fn open(width: u32, height: u32) -> Result<Self, String> {
        Self::open_named(width, height, "genos", "Genos")
    }

    /// The check window. Its class and title match the compositor rule that floats it and refuses focus.
    pub fn open_proof(width: u32, height: u32) -> Result<Self, String> {
        Self::open_named(width, height, "genos-camera", "Genos Engine")
    }

    pub fn open_named(width: u32, height: u32, app_id: &str, title: &str) -> Result<Self, String> {
        let app_id = std::ffi::CString::new(app_id).map_err(|_| "app id contains a null".to_string())?;
        let title = std::ffi::CString::new(title).map_err(|_| "title contains a null".to_string())?;
        let raw = unsafe { genos_window_open(width as c_int, height as c_int, app_id.as_ptr(), title.as_ptr()) };
        if raw.is_null() {
            return Err("Wayland window failed to open".into());
        }
        let pump = unsafe {
            let mut out = std::mem::zeroed();
            genos_window_pump(raw, &mut out);
            out
        };
        if pump.display.is_null() || pump.surface.is_null() {
            unsafe { genos_window_destroy(raw) };
            return Err("Wayland surface is missing".into());
        }
        let frame = decode(&pump);
        Ok(Self {
            raw,
            display: pump.display,
            surface: pump.surface,
            hub: EventHub::from_frame(&frame),
        })
    }

    pub fn pump(&mut self) -> Frame {
        let frame = self.read_frame();
        self.hub.apply(&frame);
        frame
    }

    /// Receive later changes, plus the size and any active focus, lock, or fullscreen state.
    ///
    /// The listener runs inside `pump`. It must not call `pump` again.
    pub fn on(&mut self, listener: impl FnMut(&WindowEvent) + 'static) {
        self.hub.on(listener);
    }

    pub fn size(&self) -> (u32, u32) {
        self.hub.size()
    }

    pub fn focused(&self) -> bool {
        self.hub.focused()
    }

    pub fn cursor_locked(&self) -> bool {
        self.hub.cursor_locked()
    }

    pub fn mode(&self) -> WindowMode {
        self.hub.mode()
    }

    /// Hide the cursor and confine it to this window until [`Self::unlock_cursor`].
    pub fn lock_cursor(&mut self) -> Result<(), String> {
        self.set_pointer_capture(true)
    }

    /// Show the cursor and let it leave the window.
    pub fn unlock_cursor(&mut self) -> Result<(), String> {
        self.set_pointer_capture(false)
    }

    /// Ask the compositor for fullscreen or a normal window.
    ///
    /// [`WindowEvent::ModeChanged`] arrives on a later pump, after the compositor applies it.
    pub fn set_mode(&mut self, mode: WindowMode) -> Result<(), String> {
        let fullscreen = if mode == WindowMode::Fullscreen { 1 } else { 0 };
        let rc = unsafe { genos_window_set_fullscreen(self.raw, fullscreen) };
        if rc != 0 {
            Err("fullscreen request failed".into())
        } else {
            Ok(())
        }
    }

    fn read_frame(&mut self) -> Frame {
        let raw = unsafe {
            let mut out = std::mem::zeroed();
            genos_window_pump(self.raw, &mut out);
            out
        };
        decode(&raw)
    }

    /// Lock and hide the Wayland pointer, or destroy that lock and show it again.
    ///
    /// The window does not call this when it opens. `false` always releases.
    /// `true` fails when the compositor has no pointer-constraint or relative-pointer global.
    pub fn set_pointer_capture(&mut self, captured: bool) -> Result<(), String> {
        let rc = unsafe { genos_window_set_pointer_capture(self.raw, if captured { 1 } else { 0 }) };
        if rc != 0 {
            Err("Wayland pointer lock failed".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe { genos_window_destroy(self.raw) };
    }
}

fn decode(raw: &RawPump) -> Frame {
    Frame {
        width: raw.width.max(1) as u32,
        height: raw.height.max(1) as u32,
        closing: raw.closing != 0,
        focused: raw.focused != 0,
        mouse_dx: raw.mouse_dx as f32,
        mouse_dy: raw.mouse_dy as f32,
        capture_click: raw.capture_click != 0,
        key_w: raw.key_w != 0,
        key_a: raw.key_a != 0,
        key_s: raw.key_s != 0,
        key_d: raw.key_d != 0,
        escape: raw.key_escape != 0,
        resized: raw.resized != 0,
        mouse_left: raw.mouse_left != 0,
        keys_down: raw.keys_down,
        pointer_locked: raw.pointer_locked != 0,
        clicked_while_focused: raw.clicked_while_focused != 0,
        pointer_x: raw.pointer_x as f32,
        pointer_y: raw.pointer_y as f32,
        mode: if raw.fullscreen != 0 {
            WindowMode::Fullscreen
        } else {
            WindowMode::Windowed
        },
    }
}

/// True when a configure event changes the drawable size.
pub fn extent_changed(old_w: u32, old_h: u32, new_w: u32, new_h: u32) -> bool {
    new_w > 0 && new_h > 0 && (new_w != old_w || new_h != old_h)
}

/// Input after the Wayland focus rule.
///
/// A click can arrive in the pump before keyboard focus. That click is kept
/// and applied on the first focused frame, so the first click both focuses
/// and captures the pointer. Keys and mouse motion act only while focused.
#[derive(Clone, Debug, Default)]
pub struct FocusGate {
    pending_click: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GatedInput {
    pub key_w: bool,
    pub key_a: bool,
    pub key_s: bool,
    pub key_d: bool,
    pub escape: bool,
    pub mouse_dx: f32,
    pub mouse_dy: f32,
    pub capture_click: bool,
    pub mouse_left: bool,
    pub keys_down: [u8; 256],
}

impl Default for GatedInput {
    fn default() -> Self {
        Self {
            key_w: false,
            key_a: false,
            key_s: false,
            key_d: false,
            escape: false,
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            capture_click: false,
            mouse_left: false,
            keys_down: [0; 256],
        }
    }
}

impl FocusGate {
    pub fn gate(&mut self, frame: &Frame) -> GatedInput {
        if frame.capture_click {
            self.pending_click = true;
        }
        let focused = frame.focused;
        let capture_click = frame.clicked_while_focused || (focused && self.pending_click);
        if capture_click {
            self.pending_click = false;
        }
        GatedInput {
            key_w: focused && frame.key_w,
            key_a: focused && frame.key_a,
            key_s: focused && frame.key_s,
            key_d: focused && frame.key_d,
            escape: focused && frame.escape,
            mouse_dx: if focused { frame.mouse_dx } else { 0.0 },
            mouse_dy: if focused { frame.mouse_dy } else { 0.0 },
            capture_click,
            mouse_left: focused && frame.mouse_left,
            keys_down: if focused { frame.keys_down } else { [0; 256] },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extent_changed, genos_window_pump_size, FocusGate, Frame, GatedInput, RawPump, WindowMode,
    };

    #[test]
    fn resize_is_a_real_extent_change() {
        assert!(!extent_changed(1280, 720, 1280, 720));
        assert!(extent_changed(1280, 720, 800, 600));
        assert!(!extent_changed(1280, 720, 0, 600));
    }

    #[test]
    fn a_click_before_focus_captures_on_the_focused_frame() {
        let mut gate = FocusGate::default();
        let ignored = gate.gate(&frame(false, true, 12.0, true));
        assert!(!ignored.capture_click);
        assert_eq!(ignored.mouse_dx, 0.0);
        assert!(!ignored.key_w);

        let focused = gate.gate(&frame(true, false, 4.0, true));
        assert!(focused.capture_click);
        assert_eq!(focused.mouse_dx, 4.0);
        assert!(focused.key_w);
        assert!(focused.escape);
    }

    #[test]
    fn keys_and_mouse_stay_idle_without_focus() {
        let mut gate = FocusGate::default();
        let idle = gate.gate(&frame(false, false, 9.0, true));
        assert_eq!(idle, GatedInput::default());
    }

    fn frame(focused: bool, click: bool, mouse_dx: f32, key_w: bool) -> Frame {
        Frame {
            width: 100,
            height: 100,
            closing: false,
            focused,
            mouse_dx,
            mouse_dy: 0.0,
            capture_click: click,
            key_w,
            key_a: false,
            key_s: false,
            key_d: false,
            escape: focused,
            resized: false,
            mouse_left: click,
            keys_down: [0; 256],
            pointer_locked: false,
            clicked_while_focused: false,
            mode: WindowMode::Windowed,
            pointer_x: 0.0,
            pointer_y: 0.0,
        }
    }

    #[test]
    fn the_pump_record_matches_the_wayland_struct() {
        assert_eq!(
            unsafe { genos_window_pump_size() } as usize,
            std::mem::size_of::<RawPump>()
        );
    }

    #[test]
    fn a_click_that_lands_as_focus_drops_still_captures() {
        let mut gate = FocusGate::default();
        let mut frame = frame(false, true, 6.0, false);
        frame.clicked_while_focused = true;
        let gated = gate.gate(&frame);
        assert!(gated.capture_click);
    }

    #[test]
    fn the_first_click_locks_the_pointer_and_escape_releases_it() {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return;
        }
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/genos-camera");
        if !binary.exists() {
            return;
        }
        let cursor = cursor_pos();
        set_focus_rule(true);
        let _restore = CursorRestore { x: cursor.0, y: cursor.1 };
        let child = std::process::Command::new(&binary)
            .args(["--frames", "400", "--trace", "--proof"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn shipped camera");
        let pid = child.id();
        let mut placed = None;
        // Scene load and the first device setup run before the toplevel is mapped.
        for _ in 0..400 {
            if let Some(found) = client_center(pid) {
                placed = Some(found);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        let (cx, cy, address) = placed.expect("shipped window was not mapped");
        let focus = std::process::Command::new("hyprctl")
            .args(["dispatch", &format!("hl.dsp.focus({{ window = \"address:{address}\" }})")])
            .output();
        let moved = std::process::Command::new("hyprctl")
            .args(["dispatch", &format!("hl.dsp.cursor.move({{ x = {cx}, y = {cy} }})")])
            .output();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let cursor_now = cursor_pos();
        eprintln!(
            "center {cx},{cy} address {address} cursor {cursor_now:?} focus {:?} move {:?}",
            focus.ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()),
            moved.ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        );
        ydotool(&["click", "0xC0"]);
        // The frame locks the pointer after the pump. The next pump reports the lock.
        std::thread::sleep(std::time::Duration::from_millis(5000));
        ydotool(&["key", "1:1"]);
        // A frame can be slower than a short key tap. Hold Escape across one pump.
        std::thread::sleep(std::time::Duration::from_millis(4000));
        ydotool(&["key", "1:0"]);
        std::thread::sleep(std::time::Duration::from_millis(4000));
        dispatch(&format!(
            "hl.dsp.window.close({{ window = \"address:{address}\" }})"
        ));
        let output = child.wait_with_output().expect("camera output");
        let trace = String::from_utf8_lossy(&output.stdout);
        let mut saw_open = false;
        let mut saw_lock = false;
        let mut saw_release = false;
        for line in trace.lines() {
            if !line.contains("trace ") {
                continue;
            }
            let locked = line.contains("locked=1");
            let captured = line.contains("captured=1");
            if !locked && !captured {
                saw_open = true;
            }
            if locked && captured {
                saw_lock = true;
            }
            if saw_lock && line.contains("locked=0") && line.contains("captured=0") {
                saw_release = true;
            }
        }
        assert!(saw_open, "trace never showed an unlocked open: {trace}");
        assert!(saw_lock, "first click did not lock the pointer: {trace}");
        assert!(saw_release, "Escape did not release the pointer: {trace}");
    }

    struct CursorRestore {
        x: i32,
        y: i32,
    }

    impl Drop for CursorRestore {
        fn drop(&mut self) {
            dispatch(&format!(
                "hl.dsp.cursor.move({{ x = {}, y = {} }})",
                self.x, self.y
            ));
            set_focus_rule(false);
        }
    }

    fn set_focus_rule(allow: bool) {
        let no_focus = if allow { "false" } else { "true" };
        let _ = std::process::Command::new("hyprctl")
            .args([
                "eval",
                &format!(
                    "hl.window_rule({{ name = \"genos-allow-focus\", match = {{ class = \"^genos-camera$\" }}, no_focus = {no_focus}, no_initial_focus = {no_focus}, focus_on_activate = {allow} }})"
                ),
            ])
            .status();
    }

    fn dispatch(command: &str) {
        let _ = std::process::Command::new("hyprctl")
            .args(["dispatch", command])
            .status();
    }

    fn ydotool(args: &[&str]) {
        let _ = std::process::Command::new("ydotool").args(args).status();
    }

    fn cursor_pos() -> (i32, i32) {
        let out = std::process::Command::new("hyprctl")
            .arg("cursorpos")
            .output()
            .expect("cursorpos");
        let text = String::from_utf8_lossy(&out.stdout);
        let mut parts = text.split(',');
        let x = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
        let y = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
        (x, y)
    }

    fn client_center(pid: u32) -> Option<(i32, i32, String)> {
        let out = std::process::Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()
            .ok()?;
        let text = String::from_utf8(out.stdout).ok()?;
        let needle = format!("\"pid\": {pid}");
        let idx = text.find(&needle)?;
        let start = idx.saturating_sub(8000);
        let blob = &text[start..idx];
        let at = blob.rfind("\"at\": [")?;
        let size = blob.rfind("\"size\": [")?;
        let addr = blob.rfind("\"address\": \"")?;
        let (ax, ay) = pair_after(&blob[at..])?;
        let (sx, sy) = pair_after(&blob[size..])?;
        let addr_text = &blob[addr..];
        let q1 = addr_text.find('"')? + 1;
        let q2 = addr_text[q1..].find('"')? + q1;
        let q3 = addr_text[q2 + 1..].find('"')? + q2 + 1;
        let q4 = addr_text[q3 + 1..].find('"')? + q3 + 1;
        let address = addr_text[q3 + 1..q4].to_string();
        Some((ax + sx / 2, ay + sy / 2, address))
    }

    fn pair_after(text: &str) -> Option<(i32, i32)> {
        let open = text.find('[')?;
        let close = text[open..].find(']')? + open;
        let mut nums = text[open + 1..close].split(',');
        let x = nums.next()?.trim().parse().ok()?;
        let y = nums.next()?.trim().parse().ok()?;
        Some((x, y))
    }
}
