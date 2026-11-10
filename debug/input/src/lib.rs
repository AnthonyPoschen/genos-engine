//! Headless input session: devices, actions, the glyph set, and rebinding.
//!
//! The window is not involved. A script writes the same calls a platform
//! backend writes. `genos-input` prints one text view per `frame` line and
//! serves `get_input`, `set_input`, and `get_glyphs` over MCP.

#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

use genos_input::{Axis2d, InputCode, InputSettings, InputSystem, TouchPhase, TouchPoint};

/// Commands this binary registers. Lighting's catalogue does not own the devices.
pub const COMMANDS: &[(&str, &str)] = &[
    (
        "get_input",
        "Live view: glyph set, each action's axis and button, devices, touch points, gestures, and the settings text.",
    ),
    (
        "set_input",
        "Rebind. Pass key and value (one settings line) or text (a whole settings document).",
    ),
    (
        "get_glyphs",
        "The glyph set to draw: keyboard_mouse, xbox, playstation, steam_deck, generic_gamepad, or touch.",
    ),
];

/// The input the debug binary shows and edits.
#[derive(Debug)]
pub struct Session {
    pub input: InputSystem,
    pub settings: InputSettings,
    pending_touches: Vec<TouchPoint>,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        let settings = InputSettings::character_controller();
        let mut input = InputSystem::new();
        settings.apply(&mut input);
        Self {
            input,
            settings,
            pending_touches: Vec::new(),
        }
    }

    /// Overlay one settings key and copy the deadzone and mouse scale onto the devices.
    ///
    /// # Errors
    ///
    /// Returns the settings error for an unknown key or code.
    pub fn rebind(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.settings.set_key(key, value)?;
        self.settings.apply(&mut self.input);
        Ok(())
    }

    /// One script line. `frame` is not a line here; [`Self::run_script`] handles it.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown verb, code, or number.
    pub fn apply_line(&mut self, line: &str) -> Result<(), String> {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let verb = parts.next().ok_or_else(|| format!("empty line: {line}"))?;
        match verb {
            "key" => {
                let code = code_from(parts.next(), line)?;
                let down = down_from(parts.next(), line)?;
                self.input.set_key(code, down);
            }
            "button" => {
                let code = code_from(parts.next(), line)?;
                let down = down_from(parts.next(), line)?;
                self.input.set_mouse_button(code, down);
            }
            "mouse" => {
                let dx = number_from(parts.next(), line)?;
                let dy = number_from(parts.next(), line)?;
                self.input.add_mouse_delta(dx, dy);
            }
            "pad" => {
                let slot = usize_from(parts.next(), line)?;
                let action = parts
                    .next()
                    .ok_or_else(|| format!("pad needs connect or disconnect: {line}"))?;
                match action {
                    "connect" => {
                        let name = parts.collect::<Vec<_>>().join(" ");
                        if name.is_empty() {
                            return Err(format!("pad connect needs a name: {line}"));
                        }
                        self.input.connect_gamepad(slot, true, &name);
                    }
                    "disconnect" => self.input.connect_gamepad(slot, false, ""),
                    _ => return Err(format!("pad action is connect or disconnect: {line}")),
                }
            }
            "stick" => {
                let slot = usize_from(parts.next(), line)?;
                let side = parts
                    .next()
                    .ok_or_else(|| format!("stick needs left or right: {line}"))?;
                let x = number_from(parts.next(), line)?;
                let y = number_from(parts.next(), line)?;
                let code = match side {
                    "left" => InputCode::gamepad_left_stick,
                    "right" => InputCode::gamepad_right_stick,
                    _ => return Err(format!("stick side is left or right: {line}")),
                };
                self.input.set_gamepad_stick(slot, code, Axis2d { x, y });
            }
            "touch" => {
                let id = u64_from(parts.next(), line)?;
                let x = number_from(parts.next(), line)?;
                let y = number_from(parts.next(), line)?;
                let phase = phase_from(parts.next(), line)?;
                self.pending_touches.push(TouchPoint { id, x, y, phase });
            }
            "rebind" => {
                let Some(rest) = line.strip_prefix("rebind") else {
                    return Err(format!("rebind needs key=value: {line}"));
                };
                let Some((key, value)) = rest.trim().split_once('=') else {
                    return Err(format!("rebind needs key=value: {line}"));
                };
                self.rebind(key.trim(), value.trim())?;
            }
            _ => return Err(format!("unknown line: {line}")),
        }
        Ok(())
    }

    /// Run a script. Each `frame` line appends one view, then starts the next frame.
    ///
    /// # Errors
    ///
    /// Returns the first bad line.
    pub fn run_script(&mut self, text: &str) -> Result<String, String> {
        let mut out = String::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line == "frame" {
                out.push_str(&self.finish_frame());
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                continue;
            }
            self.apply_line(line)
                .map_err(|err| format!("line {}: {err}", index + 1))?;
        }
        Ok(out)
    }

    /// Apply queued touch points, return the view, then start the next frame.
    #[must_use]
    pub fn finish_frame(&mut self) -> String {
        if !self.pending_touches.is_empty() {
            let points = std::mem::take(&mut self.pending_touches);
            self.input.set_touch_points(&points);
        }
        let text = self.frame_text();
        self.input.begin_frame();
        text
    }

    /// Glyph set, actions, devices, touches, gestures, and the settings text.
    #[must_use]
    pub fn frame_text(&self) -> String {
        let mut out = String::from("glyphs=");
        out.push_str(self.input.active_glyphs().name());
        out.push('\n');
        for name in self.settings.map.names() {
            let axis = self.settings.map.axis_2d(&self.input, &name);
            let down = u8::from(self.settings.map.down(&self.input, &name));
            out.push_str(&name);
            out.push('=');
            out.push_str(&num(axis.x));
            out.push(',');
            out.push_str(&num(axis.y));
            out.push_str(" down=");
            out.push_str(&down.to_string());
            out.push('\n');
        }
        out.push_str("touch_connected=");
        out.push_str(
            if self.input.touch_points().is_empty() && !self.touch_held() {
                "0"
            } else {
                "1"
            },
        );
        out.push('\n');
        for point in self.input.touch_points() {
            out.push_str("touch ");
            out.push_str(&point.id.to_string());
            out.push(' ');
            out.push_str(&num(point.x));
            out.push(' ');
            out.push_str(&num(point.y));
            out.push(' ');
            out.push_str(phase_name(point.phase));
            out.push('\n');
        }
        for gesture in self.input.gestures() {
            out.push_str("gesture ");
            match *gesture {
                genos_input::Gesture::Tap { id, x, y } => {
                    out.push_str("tap ");
                    out.push_str(&id.to_string());
                    out.push(' ');
                    out.push_str(&num(x));
                    out.push(' ');
                    out.push_str(&num(y));
                }
                genos_input::Gesture::Swipe { id, x, y } => {
                    out.push_str("swipe ");
                    out.push_str(&id.to_string());
                    out.push(' ');
                    out.push_str(&num(x));
                    out.push(' ');
                    out.push_str(&num(y));
                }
                genos_input::Gesture::Pinch { scale } => {
                    out.push_str("pinch ");
                    out.push_str(&num(scale));
                }
            }
            out.push('\n');
        }
        let pads = self.input.list_devices(genos_input::DeviceKind::Gamepad);
        for pad in pads {
            out.push_str("pad ");
            out.push_str(&pad.id.to_string());
            out.push(' ');
            out.push_str(&pad.name);
            out.push('\n');
        }
        out.push_str("--- settings\n");
        out.push_str(&self.settings.dump());
        out
    }

    /// Run one catalogue command. `text` replaces the settings document.
    /// `key` and `value` overlay one line.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown command or a bad rebind.
    pub fn command(
        &mut self,
        name: &str,
        key: Option<&str>,
        value: Option<&str>,
        text: Option<&str>,
    ) -> Result<String, String> {
        match name {
            "get_input" => Ok(self.frame_text()),
            "get_glyphs" => Ok(self.input.active_glyphs().name().to_string()),
            "set_input" => {
                if let Some(text) = text {
                    self.settings = InputSettings::parse(text)?;
                    self.settings.apply(&mut self.input);
                    return Ok(self.settings.to_text());
                }
                let key = key.ok_or("set_input needs key and value, or text")?;
                let value = value.ok_or("set_input needs a value")?;
                self.rebind(key, value)?;
                Ok(self.settings.to_text())
            }
            _ => Err(format!("unknown command {name}")),
        }
    }

    fn touch_held(&self) -> bool {
        self.input
            .list_devices(genos_input::DeviceKind::Touch)
            .iter()
            .any(|device| device.connected)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

fn num(value: f32) -> String {
    let rounded = (value * 1_000.0).round() / 1_000.0;
    format!("{rounded}")
}

fn code_from(token: Option<&str>, line: &str) -> Result<InputCode, String> {
    let token = token.ok_or_else(|| format!("missing code: {line}"))?;
    genos_input::parse(token).ok_or_else(|| format!("unknown code {token}"))
}

fn down_from(token: Option<&str>, line: &str) -> Result<bool, String> {
    match token {
        Some("down" | "1") => Ok(true),
        Some("up" | "0") => Ok(false),
        _ => Err(format!("expected down or up: {line}")),
    }
}

fn number_from(token: Option<&str>, line: &str) -> Result<f32, String> {
    let token = token.ok_or_else(|| format!("missing number: {line}"))?;
    token
        .parse::<f32>()
        .map_err(|_| format!("bad number {token} in {line}"))
}

fn usize_from(token: Option<&str>, line: &str) -> Result<usize, String> {
    let token = token.ok_or_else(|| format!("missing slot: {line}"))?;
    token
        .parse::<usize>()
        .map_err(|_| format!("bad slot {token} in {line}"))
}

fn u64_from(token: Option<&str>, line: &str) -> Result<u64, String> {
    let token = token.ok_or_else(|| format!("missing id: {line}"))?;
    token
        .parse::<u64>()
        .map_err(|_| format!("bad id {token} in {line}"))
}

fn phase_from(token: Option<&str>, line: &str) -> Result<TouchPhase, String> {
    match token {
        Some("began") => Ok(TouchPhase::Began),
        Some("moved") => Ok(TouchPhase::Moved),
        Some("ended") => Ok(TouchPhase::Ended),
        Some("cancelled") => Ok(TouchPhase::Cancelled),
        _ => Err(format!(
            "touch phase is began, moved, ended, or cancelled: {line}"
        )),
    }
}

fn phase_name(phase: TouchPhase) -> &'static str {
    match phase {
        TouchPhase::Began => "began",
        TouchPhase::Moved => "moved",
        TouchPhase::Ended => "ended",
        TouchPhase::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_inspect_rebind_and_glyphs() {
        let names: Vec<_> = COMMANDS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["get_input", "set_input", "get_glyphs"]);
    }

    #[test]
    fn a_script_shows_wasd_a_rebind_a_pad_and_a_tap() {
        let mut session = Session::new();
        let view = session
            .run_script(
                "
                key key_w down
                frame
                rebind release=key_b
                key key_w up
                key key_b down
                frame
                pad 0 connect Xbox Wireless Controller
                stick 0 left 0 1
                frame
                rebind jump.gesture=tap
                touch 1 10 10 began
                touch 1 12 11 ended
                frame
                ",
            )
            .unwrap();
        let frames: Vec<_> = view.split("--- settings").collect();
        assert!(frames.len() >= 4, "{view}");
        assert!(frames[0].contains("glyphs=keyboard_mouse"), "{view}");
        assert!(frames[0].contains("move=0,1"), "{view}");
        assert!(frames[1].contains("release=0,0 down=1"), "{view}");
        assert!(frames[2].contains("glyphs=xbox"), "{view}");
        assert!(
            frames[2].contains("pad 100 Xbox Wireless Controller"),
            "{view}"
        );
        assert!(frames[3].contains("glyphs=touch"), "{view}");
        assert!(frames[3].contains("gesture tap 1"), "{view}");
        assert!(frames[3].contains("jump=0,0 down=1"), "{view}");
        assert_eq!(
            session.command("get_glyphs", None, None, None).unwrap(),
            "touch"
        );
        let settings = session
            .command("set_input", Some("mouse_sensitivity"), Some("2"), None)
            .unwrap();
        assert!(settings.contains("mouse_sensitivity=2"));
        let again = session.command("get_input", None, None, None).unwrap();
        assert!(again.contains("glyphs=touch"), "{again}");
    }
}
