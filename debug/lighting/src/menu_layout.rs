//! The debug menu is a table. Tabs and folds come from the lines, not from code.
//!
//! A line is `tab | section key | title | open or closed | setting`.
//! A tab of `-` sits outside the tab strip. Blank lines and `#` comments are skipped.

/// One setting in the menu table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuLine {
    /// `None` sits outside the tab strip.
    pub tab: Option<String>,
    pub section: String,
    pub title: String,
    pub collapsed: bool,
    pub setting: String,
}

/// A fold the menu can remember. `key` is the name in `collapsed.txt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuFold {
    pub id: u32,
    pub key: String,
    pub collapsed: bool,
}

/// Read `GENOS_MENU_LAYOUT`, then `debug/lighting/menu.txt` in the repo, then the
/// copy compiled into the binary.
pub fn load_menu_layout() -> Result<Vec<MenuLine>, String> {
    let text = if let Ok(path) = std::env::var("GENOS_MENU_LAYOUT") {
        std::fs::read_to_string(&path).map_err(|err| format!("GENOS_MENU_LAYOUT {path}: {err}"))?
    } else {
        let path = genos_scene_lib::repo_root().join("debug/lighting/menu.txt");
        if path.is_file() {
            std::fs::read_to_string(&path).map_err(|err| err.to_string())?
        } else {
            include_str!("../menu.txt").to_string()
        }
    };
    parse_menu_layout(&text)
}

pub fn parse_menu_layout(text: &str) -> Result<Vec<MenuLine>, String> {
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('|').map(str::trim).collect();
        if parts.len() != 5 {
            return Err(format!(
                "menu layout line {}: expected tab | section | title | open or closed | setting",
                index + 1
            ));
        }
        let collapsed = match parts[3] {
            "open" => false,
            "closed" => true,
            other => {
                return Err(format!(
                    "menu layout line {}: {other} is not open or closed",
                    index + 1
                ))
            }
        };
        if parts[1].is_empty() || parts[2].is_empty() || parts[4].is_empty() {
            return Err(format!("menu layout line {}: missing a field", index + 1));
        }
        let tab = if parts[0] == "-" || parts[0].is_empty() {
            None
        } else {
            Some(parts[0].to_string())
        };
        lines.push(MenuLine {
            tab,
            section: parts[1].to_string(),
            title: parts[2].to_string(),
            collapsed,
            setting: parts[4].to_string(),
        });
    }
    if lines.is_empty() {
        return Err("menu layout is empty".into());
    }
    Ok(lines)
}

/// One fold per section key, in first-seen order. The id stays stable for the
/// keys the fold file already uses.
pub fn menu_folds(lines: &[MenuLine]) -> Vec<MenuFold> {
    let mut folds = Vec::new();
    for line in lines {
        if folds.iter().any(|fold: &MenuFold| fold.key == line.section) {
            continue;
        }
        folds.push(MenuFold {
            id: section_id(&line.section),
            key: line.section.clone(),
            collapsed: line.collapsed,
        });
    }
    folds
}

pub fn section_id(key: &str) -> u32 {
    match key {
        "scene" => 5100,
        "pipeline" => 5101,
        "direct" => 5102,
        "gi" => 5103,
        "denoise" => 5104,
        "shadows" => 5105,
        "perf" => 5106,
        "info" => 5107,
        other => 12000 + (hash(other) % 2000),
    }
}

pub fn tab_id(name: &str) -> u32 {
    9000 + (hash(name) % 500)
}

pub fn strip_id(index: usize) -> u32 {
    7000 + index as u32
}

fn hash(text: &str) -> u32 {
    let mut value = 2166136261u32;
    for byte in text.bytes() {
        value ^= u32::from(byte);
        value = value.wrapping_mul(16777619);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_places_a_setting_outside_or_in_a_tab() {
        let lines = parse_menu_layout(
            "
            # outside
            - | info | Info | closed | review
            Direct | direct | Direct/Lamps | open | lighting_direct
            ",
        )
        .unwrap();
        assert_eq!(lines[0].tab, None);
        assert!(lines[0].collapsed);
        assert_eq!(lines[0].setting, "review");
        assert_eq!(lines[1].tab.as_deref(), Some("Direct"));
        assert!(!lines[1].collapsed);
        assert_eq!(lines[1].section, "direct");
        let folds = menu_folds(&lines);
        assert_eq!(folds[0].key, "info");
        assert_eq!(folds[0].id, 5107);
        assert!(folds[0].collapsed);
        assert!(!folds[1].collapsed);
        assert!(parse_menu_layout("- | info | Info | maybe | review").is_err());
    }
}
