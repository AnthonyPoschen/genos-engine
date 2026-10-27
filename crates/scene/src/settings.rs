#![expect(
    clippy::missing_errors_doc,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! The text contract for an engine feature's settings.
//!
//! A game, another system, the MCP catalogue, and the debug menu all read and
//! write the same value. [`crate::LightingSettings`] is the reference
//! implementor. The rule is `docs/engine/settings-principles.md`.

/// Serializable settings for one engine feature.
///
/// `to_text` and `parse` round-trip the whole value. `set_key` overlays one
/// knob and leaves the rest. Unknown keys fail.
pub trait EngineSettings: Clone {
    fn to_text(&self) -> String;

    fn parse(text: &str) -> Result<Self, String>;

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String>;
}

impl EngineSettings for crate::LightingSettings {
    fn to_text(&self) -> String {
        crate::LightingSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        crate::LightingSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        crate::LightingSettings::set_key(self, key, value)
    }
}
