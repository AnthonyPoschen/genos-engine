//! Debug menu visibility, page, and folds.
//!
//! The menu is shown until `--menu hidden`, the hide control, or `visible`
//! is set to hidden. `tab` is the page title. `collapsed` lists folded
//! section keys. `--menu` wins. `GENOS_MENU` applies only when that argument
//! was omitted, and then a set variable wins every frame.

use crate::EngineSettings;

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
        }
    }
}

impl MenuSettings {
    /// `--menu shown` or `--menu hidden`. `--menu=hidden` is the same flag.
    /// A bare `--menu` is an error.
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
    pub fn cli() -> Result<MenuCli, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::cli_from(&args)
    }

    /// The value the menu uses.
    ///
    /// A CLI slot wins for `visible`. When that slot is empty, `GENOS_MENU` wins.
    /// Otherwise the field is used. `tab` and `collapsed` stay on the struct.
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
    pub fn resolve(&self, cli_visible: Option<bool>, env_visible: Option<&str>) -> Self {
        let mut settings = self.clone();
        if let Some(on) = cli_visible {
            settings.visible = on;
        } else if let Some(on) = env_flag(env_visible) {
            settings.visible = on;
        }
        settings
    }

    pub fn to_text(&self) -> String {
        format!(
            "visible={}\ntab={}\ncollapsed={}\n",
            if self.visible { "shown" } else { "hidden" },
            self.tab,
            self.collapsed.join(",")
        )
    }

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
        assert!(settings.tab.is_empty());
        assert!(settings.collapsed.is_empty());
        let text = EngineSettings::to_text(&settings);
        assert_eq!(text, "visible=shown\ntab=\ncollapsed=\n");
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
        assert!(settings.collapsed.is_empty());
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
        assert!(!crate::load_str(&{
            let mut off = std::fs::read_to_string(path).unwrap();
            off.push_str("\nmenu(\"visible=maybe\")\n");
            off
        })
        .is_ok());
    }
}
