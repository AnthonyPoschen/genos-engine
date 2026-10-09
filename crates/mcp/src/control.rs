//! Requests from a client that the picture frame applies.
//!
//! The listener thread only stores the request. The frame thread takes it, draws
//! the shot, and writes the new settings. A client waits for that frame.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::discover;

/// One change to the stress picture. Absent fields stay as they are.
///
/// These are the panel switches. They do not change the gather.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightingSettings {
    pub sun_frozen: Option<bool>,
    pub sky_on: Option<bool>,
    pub boxes_still: Option<bool>,
    /// Percent of lamps that move, 0 to 100.
    pub dynamic: Option<u32>,
    /// Fraction of the day, 0 to 1.
    pub day: Option<f32>,
}

struct State {
    pending: LightingSettings,
    /// Text of the last accepted request.
    posted: String,
    /// Text the frame last published.
    live: String,
    shot_wanted: bool,
    shot_serial: u64,
    shot: Vec<u8>,
}

static STATE: Mutex<State> = Mutex::new(State {
    pending: LightingSettings {
        sun_frozen: None,
        sky_on: None,
        boxes_still: None,
        dynamic: None,
        day: None,
    },
    posted: String::new(),
    live: String::new(),
    shot_wanted: false,
    shot_serial: 0,
    shot: Vec::new(),
});
static WAKE: Condvar = Condvar::new();

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|err| err.into_inner())
}

/// Parse `key=value` lines or a query string. Unknown keys and bad values fail.
pub fn post_settings(text: &str) -> Result<String, String> {
    let patch = parse_settings(text)?;
    let mut state = lock();
    merge(&mut state.pending, &patch);
    state.posted = format_settings(&patch);
    state.live.clear();
    Ok(state.posted.clone())
}

/// The settings the frame should apply, if a client sent any.
pub fn take_settings() -> Option<LightingSettings> {
    let mut state = lock();
    let patch = std::mem::take(&mut state.pending);
    if patch == LightingSettings::default() {
        None
    } else {
        Some(patch)
    }
}

/// Remember the settings the frame is actually using.
pub fn publish_settings(text: &str) {
    lock().live = text.to_string();
}

/// The live settings, or the last request when the frame has not published yet.
pub fn settings_text() -> String {
    let state = lock();
    if !state.live.is_empty() {
        state.live.clone()
    } else if !state.posted.is_empty() {
        state.posted.clone()
    } else {
        "sun: unknown\n".to_string()
    }
}

/// True when a client is waiting for a picture. The frame draws one readback.
pub fn take_shot_request() -> bool {
    let mut state = lock();
    let wanted = state.shot_wanted;
    state.shot_wanted = false;
    wanted
}

/// Store the PNG the client is waiting for and wake it.
pub fn finish_shot(png: &[u8]) {
    let mut state = lock();
    state.shot = png.to_vec();
    state.shot_serial = state.shot_serial.wrapping_add(1);
    WAKE.notify_all();
}

/// Ask the frame for a picture and wait. The PNG is also written to `shot_path`.
pub fn wait_shot(timeout: Duration) -> Result<std::path::PathBuf, String> {
    let before = {
        let mut state = lock();
        state.shot_wanted = true;
        state.shot_serial
    };
    WAKE.notify_all();
    let deadline = Instant::now() + timeout;
    let png = {
        let mut state = lock();
        loop {
            if state.shot_serial != before && !state.shot.is_empty() {
                break state.shot.clone();
            }
            let now = Instant::now();
            if now >= deadline {
                return Err("the picture did not arrive".into());
            }
            let (guard, _) = WAKE
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .unwrap_or_else(|err| err.into_inner());
            state = guard;
        }
    };
    let path = shot_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, &png).map_err(|err| err.to_string())?;
    Ok(path)
}

/// Latest PNG, if a shot has finished.
pub fn shot_bytes() -> Vec<u8> {
    lock().shot.clone()
}

/// Where `wait_shot` writes the PNG.
pub fn shot_path() -> std::path::PathBuf {
    discover::discovery_dir().join("shot.png")
}

fn merge(into: &mut LightingSettings, patch: &LightingSettings) {
    if patch.sun_frozen.is_some() {
        into.sun_frozen = patch.sun_frozen;
    }
    if patch.sky_on.is_some() {
        into.sky_on = patch.sky_on;
    }
    if patch.boxes_still.is_some() {
        into.boxes_still = patch.boxes_still;
    }
    if patch.dynamic.is_some() {
        into.dynamic = patch.dynamic;
    }
    if patch.day.is_some() {
        into.day = patch.day;
    }
}

fn format_settings(patch: &LightingSettings) -> String {
    let mut out = String::new();
    if let Some(value) = patch.sun_frozen {
        out.push_str(if value { "sun: freeze\n" } else { "sun: run\n" });
    }
    if let Some(value) = patch.sky_on {
        out.push_str(if value { "sky: on\n" } else { "sky: off\n" });
    }
    if let Some(value) = patch.boxes_still {
        out.push_str(if value { "boxes: still\n" } else { "boxes: move\n" });
    }
    if let Some(value) = patch.dynamic {
        out.push_str(&format!("dynamic: {value}\n"));
    }
    if let Some(value) = patch.day {
        out.push_str(&format!("day: {value}\n"));
    }
    out
}

fn parse_settings(text: &str) -> Result<LightingSettings, String> {
    let mut patch = LightingSettings::default();
    for raw in text.split(|c| c == '\n' || c == '&' || c == ';') {
        let line = raw.trim().trim_start_matches('?');
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("use key=value, got {line}"));
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "sun" => patch.sun_frozen = Some(flag(value, "freeze", "run")?),
            "sky" => patch.sky_on = Some(flag(value, "on", "off")?),
            "boxes" => patch.boxes_still = Some(flag(value, "still", "move")?),
            "dynamic" => {
                let n = value
                    .parse::<u32>()
                    .map_err(|_| format!("dynamic is 0 to 100, got {value}"))?;
                if n > 100 {
                    return Err(format!("dynamic is 0 to 100, got {value}"));
                }
                patch.dynamic = Some(n);
            }
            "day" => patch.day = Some(number(key, value, 0.0, 1.0)?),
            _ => return Err(format!("unknown setting {key}")),
        }
    }
    Ok(patch)
}

fn number(key: &str, value: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let n = value
        .parse::<f32>()
        .map_err(|_| format!("{key} is a number, got {value}"))?;
    if !n.is_finite() || n < lo || n > hi {
        return Err(format!("{key} is {lo} to {hi}, got {value}"));
    }
    Ok(n)
}

fn flag(value: &str, yes: &str, no: &str) -> Result<bool, String> {
    if value == yes || value == "1" {
        Ok(true)
    } else if value == no || value == "0" {
        Ok(false)
    } else {
        Err(format!("use {yes} or {no}, got {value}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_post_reaches_the_frame() {
        let text = post_settings("sun=freeze&boxes=still&dynamic=0&day=0.25").expect("parse");
        assert!(text.contains("sun: freeze"), "{text}");
        assert!(text.contains("boxes: still"), "{text}");
        let patch = take_settings().expect("pending");
        assert_eq!(patch.sun_frozen, Some(true));
        assert_eq!(patch.boxes_still, Some(true));
        assert_eq!(patch.dynamic, Some(0));
        assert_eq!(patch.day, Some(0.25));
        assert!(take_settings().is_none());
    }

    #[test]
    fn a_bad_setting_names_the_key() {
        let err = post_settings("shell=1").unwrap_err();
        assert!(err.contains("unknown setting shell"), "{err}");
    }
}
