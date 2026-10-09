//! The latest lighting report from the running picture.
//!
//! The renderer writes it. `GET /lighting` and the `lighting_status` tool read it.

use std::sync::Mutex;

static REPORT: Mutex<String> = Mutex::new(String::new());

/// Replace the report the endpoint serves.
pub fn publish(text: &str) {
    *REPORT.lock().unwrap_or_else(|err| err.into_inner()) = text.to_string();
}

/// The latest report, or a line that says none has arrived.
pub fn current() -> String {
    let text = REPORT.lock().unwrap_or_else(|err| err.into_inner()).clone();
    if text.is_empty() {
        "stable: unknown\npending: unknown\n".to_string()
    } else {
        text
    }
}
