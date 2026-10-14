//! Debug menu visibility, page, folds, and focus policy.
//!
//! The menu is shown until `--menu hidden`, the hide control, or `visible`
//! is set to hidden. `tab` is the page title. `collapsed` lists folded
//! section keys. `wheel` and `keys` are each `hover`, `modal`, `click`, or
//! `passive`. The debug menu default is hover for the wheel and click for
//! keys. `--menu` wins for `visible`. `GENOS_MENU` applies only when that
//! argument was omitted, and then a set variable wins every frame.

#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

use crate::EngineSettings;

/// How one panel claims one kind of input. The game picks this per panel and can
/// change it while the frame loop is running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusPolicy {
    /// While the panel is shown it owns this input. A click outside does not escape.
    Modal,
    /// This input arrives only while the pointer is over the panel.
    #[default]
    Hover,
    /// A click on the panel takes this input. A click outside, Escape, or grave releases it.
    ClickToFocus,
    /// This input never arrives.
    Passive,
}

impl FocusPolicy {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modal => "modal",
            Self::Hover => "hover",
            Self::ClickToFocus => "click",
            Self::Passive => "passive",
        }
    }
}

/// `hover`, `modal`, `click`, or `passive`. `exclusive` is modal.
///
/// # Errors
///
/// Returns an error when `value` is not one of those words.
pub fn parse_focus_policy(value: &str) -> Result<FocusPolicy, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "modal" | "exclusive" => Ok(FocusPolicy::Modal),
        "hover" => Ok(FocusPolicy::Hover),
        "click" | "clicktofocus" | "click_to_focus" => Ok(FocusPolicy::ClickToFocus),
        "passive" => Ok(FocusPolicy::Passive),
        _ => Err(format!(
            "focus is hover, modal, click, or passive, got {value}"
        )),
    }
}

/// Whether the debug menu is on screen, which page is open, and which folds are shut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuSettings {
    /// Shown draws the panel. Hidden leaves the corner tab.
    pub visible: bool,
    /// Page title. Empty leaves the first page.
    pub tab: String,
    /// Section keys that are folded. Empty leaves each fold at its layout default
    /// until a later write names them.
    pub collapsed: Vec<String>,
    /// How the wheel reaches the menu. The debug menu default is hover.
    pub wheel: FocusPolicy,
    /// How keys reach the menu. The debug menu default is click-to-focus.
    pub keys: FocusPolicy,
}

/// `--menu` from one argument list. Other arguments are ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuCli {
    pub visible: Option<bool>,
}

impl Default for MenuSettings {
    fn default() -> Self {
        Self {
            visible: true,
            tab: String::new(),
            collapsed: Vec::new(),
            wheel: FocusPolicy::Hover,
            keys: FocusPolicy::ClickToFocus,
        }
    }
}

impl MenuSettings {
    /// `--menu shown` or `--menu hidden`. `--menu=hidden` is the same flag.
    /// A bare `--menu` is an error.
    ///
    /// # Errors
    ///
    /// Returns an error when `--menu` has no value, or that value is not shown or hidden.
    pub fn cli_from(args: &[String]) -> Result<MenuCli, String> {
        let mut visible = None;
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if let Some(value) = arg.strip_prefix("--menu=") {
                visible = Some(parse_menu_visible(value)?);
                index += 1;
                continue;
            }
            if arg == "--menu" {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                    .ok_or("--menu needs shown or hidden")?;
                visible = Some(parse_menu_visible(value)?);
                index += 2;
                continue;
            }
            index += 1;
        }
        Ok(MenuCli { visible })
    }

    /// The process arguments. A bad flag is an error so the binary can exit at startup.
    ///
    /// # Errors
    ///
    /// Returns an error when `--menu` is present and its value is not shown or hidden.
    pub fn cli() -> Result<MenuCli, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::cli_from(&args)
    }

    /// The value the menu uses.
    ///
    /// A CLI slot wins for `visible`. When that slot is empty, `GENOS_MENU` wins.
    /// Otherwise the field is used. `tab` and `collapsed` stay on the struct.
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): pass this settings value in from the caller. docs/LINT_RULES.md"
    )]
    #[must_use]
    pub fn effective(&self) -> Self {
        let cli = Self::cli().unwrap_or(MenuCli { visible: None });
        let env_visible = if cli.visible.is_none() {
            std::env::var("GENOS_MENU").ok()
        } else {
            None
        };
        self.resolve(cli.visible, env_visible.as_deref())
    }

    /// Same rule as [`Self::effective`], with the CLI and environment supplied.
    /// An environment string that is empty or not a value is ignored.
    #[must_use]
    pub fn resolve(&self, cli_visible: Option<bool>, env_visible: Option<&str>) -> Self {
        let mut settings = self.clone();
        if let Some(on) = cli_visible {
            settings.visible = on;
        } else if let Some(on) = env_flag(env_visible) {
            settings.visible = on;
        }
        settings
    }

    /// Text form: `visible`, `tab`, `collapsed`, `wheel`, and `keys`.
    #[must_use]
    pub fn to_text(&self) -> String {
        format!(
            "visible={}\ntab={}\ncollapsed={}\nwheel={}\nkeys={}\n",
            if self.visible { "shown" } else { "hidden" },
            self.tab,
            self.collapsed.join(","),
            self.wheel.as_str(),
            self.keys.as_str()
        )
    }

    /// Parse the text form.
    ///
    /// # Errors
    ///
    /// Returns an error when a line is not `key=value`, or a value is not valid for its key.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::default();
        for raw in text.split(['\n', ';']) {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("use key=value, got {line}"));
            };
            settings.set_key(key, value)?;
        }
        Ok(settings)
    }

    /// Set one key.
    ///
    /// # Errors
    ///
    /// Returns an error when `key` is unknown, or `value` is not valid for that key.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim();
        let value = value.trim();
        match key {
            "visible" | "menu" => {
                self.visible = parse_menu_visible(value)?;
                Ok(())
            }
            "tab" => {
                self.tab = value.to_string();
                Ok(())
            }
            "collapsed" => {
                self.collapsed = split_keys(value);
                Ok(())
            }
            "fold" => {
                if !value.is_empty() && !self.collapsed.iter().any(|name| name == value) {
                    self.collapsed.push(value.to_string());
                }
                Ok(())
            }
            "open" => {
                self.collapsed.retain(|name| name != value);
                Ok(())
            }
            "wheel" | "focus_wheel" => {
                self.wheel = parse_focus_policy(value)?;
                Ok(())
            }
            "keys" | "focus_keys" => {
                self.keys = parse_focus_policy(value)?;
                Ok(())
            }
            _ => Err(format!("unknown menu setting {key}")),
        }
    }
}

impl EngineSettings for MenuSettings {
    fn to_text(&self) -> String {
        MenuSettings::to_text(self)
    }

    fn parse(text: &str) -> Result<Self, String> {
        MenuSettings::parse(text)
    }

    fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        MenuSettings::set_key(self, key, value)
    }
}

/// `shown` and `hidden` are the menu words. `on` and `off` are accepted too.
///
/// # Errors
///
/// Returns an error when `value` is not a shown or hidden word.
pub fn parse_menu_visible(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" | "shown" | "show" => Ok(true),
        "0" | "false" | "off" | "no" | "hidden" | "hide" => Ok(false),
        _ => Err(format!("visible is shown or hidden, got {value}")),
    }
}

fn split_keys(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn env_flag(value: Option<&str>) -> Option<bool> {
    let text = value.map(str::trim).filter(|text| !text.is_empty())?;
    parse_menu_visible(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_shown_on_the_first_page() {
        let settings = MenuSettings::default();
        assert!(settings.visible);
        assert_eq!(settings.tab, "");
        assert_eq!(settings.collapsed, Vec::<String>::new());
        let text = EngineSettings::to_text(&settings);
        assert_eq!(
            text,
            "visible=shown\ntab=\ncollapsed=\nwheel=hover\nkeys=click\n"
        );
        assert_eq!(settings.wheel, FocusPolicy::Hover);
        assert_eq!(settings.keys, FocusPolicy::ClickToFocus);
        let parsed = <MenuSettings as EngineSettings>::parse(&text).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn visibility_page_and_folds_round_trip() {
        let mut settings = MenuSettings::default();
        EngineSettings::set_key(&mut settings, "visible", "hidden").unwrap();
        EngineSettings::set_key(&mut settings, "tab", "GI").unwrap();
        EngineSettings::set_key(&mut settings, "collapsed", "world, gi").unwrap();
        assert!(!settings.visible);
        assert_eq!(settings.tab, "GI");
        assert_eq!(
            settings.collapsed,
            vec!["world".to_string(), "gi".to_string()]
        );
        let parsed = MenuSettings::parse(&settings.to_text()).unwrap();
        assert_eq!(parsed, settings);
        settings.set_key("menu", "shown").unwrap();
        settings.set_key("fold", "direct").unwrap();
        settings.set_key("fold", "direct").unwrap();
        settings.set_key("open", "world").unwrap();
        assert!(settings.visible);
        assert_eq!(
            settings.collapsed,
            vec!["gi".to_string(), "direct".to_string()]
        );
        settings.set_key("collapsed", "").unwrap();
        assert_eq!(settings.collapsed, Vec::<String>::new());
        settings.set_key("wheel", "passive").unwrap();
        settings.set_key("focus_keys", "modal").unwrap();
        assert_eq!(settings.wheel, FocusPolicy::Passive);
        assert_eq!(settings.keys, FocusPolicy::Modal);
        let parsed = MenuSettings::parse(&settings.to_text()).unwrap();
        assert_eq!(parsed.wheel, FocusPolicy::Passive);
        assert_eq!(parsed.keys, FocusPolicy::Modal);
        assert!(settings.set_key("wheel", "sideways").is_err());
        assert!(settings.set_key("home", "1").is_err());
        assert!(settings.set_key("visible", "maybe").is_err());
        assert!(MenuSettings::parse("visible\n").is_err());
    }

    #[test]
    fn cli_wins_over_the_env_mirror_and_the_field() {
        let stored = MenuSettings {
            visible: true,
            tab: "Info".into(),
            collapsed: vec!["world".into()],
            ..MenuSettings::default()
        };
        let locked = stored.resolve(Some(false), Some("shown"));
        assert!(!locked.visible);
        assert_eq!(locked.tab, "Info");
        assert_eq!(locked.collapsed, vec!["world".to_string()]);
        let from_env = stored.resolve(None, Some("hidden"));
        assert!(!from_env.visible);
        let ignored = stored.resolve(None, Some("maybe"));
        assert_eq!(ignored, stored);
        let shown = stored.resolve(Some(true), Some("hidden"));
        assert!(shown.visible);
    }

    #[test]
    fn cli_flags_parse_from_an_argument_list() {
        let hidden = MenuSettings::cli_from(&[
            "--scene".into(),
            "stress".into(),
            "--menu".into(),
            "hidden".into(),
        ])
        .unwrap();
        assert_eq!(hidden.visible, Some(false));
        let shown = MenuSettings::cli_from(&["--menu=shown".into()]).unwrap();
        assert_eq!(shown.visible, Some(true));
        let on = MenuSettings::cli_from(&["--menu".into(), "off".into()]).unwrap();
        assert_eq!(on.visible, Some(false));
        assert!(MenuSettings::cli_from(&["--menu".into()]).is_err());
        assert!(MenuSettings::cli_from(&["--menu".into(), "maybe".into()]).is_err());
        assert!(MenuSettings::cli_from(&["--menu=sideways".into()]).is_err());
    }

    #[test]
    fn a_script_sets_the_menu() {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/camera/scene.rhai"
        ));
        let mut source = std::fs::read_to_string(path).unwrap();
        source.push_str("\nmenu(\"visible=hidden; tab=GI; collapsed=world\")\n");
        let scene = crate::load_str(&source).unwrap();
        assert!(!scene.menu.visible);
        assert_eq!(scene.menu.tab, "GI");
        assert_eq!(scene.menu.collapsed, vec!["world".to_string()]);
        assert!(crate::load_str(&{
            let mut off = std::fs::read_to_string(path).unwrap();
            off.push_str("\nmenu(\"visible=maybe\")\n");
            off
        })
        .is_err());
    }
}
