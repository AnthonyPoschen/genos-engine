//! Omarchy theme colors for the default UI palette.
//!
//! The signal is the `current` directory, not `/etc/os-release`.
//! Omarchy deletes `current/theme` and then moves the next theme into that path.
//! A read opens `colors.toml`, copies the bytes, and closes the file.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Light or dark, from `colors.toml` or from a `light.mode` marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// Theme colors the lighting panel paints. Channels are `sRGB byte / 255`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: [f32; 3],
    pub foreground: [f32; 3],
    pub accent: [f32; 3],
    pub mode: Mode,
}

/// `$XDG_STATE_HOME/omarchy/current`, or `~/.local/state/omarchy/current`.
pub fn omarchy_current_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_STATE_HOME");
    let home = std::env::var_os("HOME");
    omarchy_current_from(xdg.as_deref(), home.as_deref())
}

/// The process is on Omarchy when this directory exists.
pub fn on_omarchy(current: &Path) -> bool {
    current.is_dir()
}

/// Read `current/theme/colors.toml`. `None` means the file is missing or unusable.
///
/// A `theme/light.mode` file forces [`Mode::Light`].
pub fn omarchy_palette(current: &Path) -> Option<Palette> {
    if !on_omarchy(current) {
        return None;
    }
    let theme = current.join("theme");
    let bytes = std::fs::read(theme.join("colors.toml")).ok()?;
    let mut palette = parse_palette(&bytes)?;
    if theme.join("light.mode").is_file() {
        palette.mode = Mode::Light;
    }
    Some(palette)
}

fn omarchy_current_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    if let Some(xdg) = xdg {
        if !xdg.is_empty() {
            return Path::new(xdg).join("omarchy").join("current");
        }
    }
    Path::new(home.unwrap_or(OsStr::new("")))
        .join(".local")
        .join("state")
        .join("omarchy")
        .join("current")
}

fn parse_palette(bytes: &[u8]) -> Option<Palette> {
    let text = std::str::from_utf8(bytes).ok()?;
    let raw = parse_pairs(text);
    let background = color(&raw, "background", "bg")?;
    let foreground = color(&raw, "foreground", "fg")?;
    let accent = parse_hex(raw.get("accent")?)?;
    let mode = if raw.get("mode") == Some("light") {
        Mode::Light
    } else {
        Mode::Dark
    };
    Some(Palette {
        background,
        foreground,
        accent,
        mode,
    })
}

struct Pairs {
    pairs: Vec<(String, String)>,
}

impl Pairs {
    fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

fn color(raw: &Pairs, canonical: &str, alias: &str) -> Option<[f32; 3]> {
    let value = if raw.get(canonical).is_some() {
        raw.get(canonical)?
    } else {
        raw.get(alias)?
    };
    parse_hex(value)
}

fn parse_pairs(text: &str) -> Pairs {
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = strip_comment(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.starts_with('[') {
            continue;
        }
        pairs.push((key.to_string(), unquote(value.trim()).to_string()));
    }
    Pairs { pairs }
}

fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut quote = b'"';
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'\\' {
                index = index.saturating_add(2).min(bytes.len());
                continue;
            }
            if byte == quote {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' || byte == b'\'' {
            in_string = true;
            quote = byte;
            index += 1;
            continue;
        }
        if byte == b'#' {
            return &line[..index];
        }
        index += 1;
    }
    line
}

fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let quote = bytes[0];
        if (quote == b'"' || quote == b'\'') && bytes[bytes.len() - 1] == quote {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn parse_hex(value: &str) -> Option<[f32; 3]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let packed = u32::from_str_radix(hex, 16).ok()?;
    Some([
        ((packed >> 16) & 0xff) as f32 / 255.0,
        ((packed >> 8) & 0xff) as f32 / 255.0,
        (packed & 0xff) as f32 / 255.0,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_keys_win_and_aliases_fill_gaps() {
        let both = br##"
            # comment, and a hex marker that is not a color key
            bg = "#111111"
            background = "#010000" # canonical wins
            fg = "#222222"
            foreground = "#020000"
            dark_bg = "#333333"
            dark_background = "#030000"
            darker_bg = "#444444"
            darker_background = "#040000"
            lighter_background = "#050000"
            lighter_bg = "#555555"
            dark_fg = "#666666"
            dark_foreground = "#060000"
            light_foreground = "#070000"
            light_fg = "#777777"
            bright_fg = "#888888"
            bright_foreground = "#080000"
            accent = "#ff8000"
            mode = "dark"
        "##;
        let raw = parse_pairs(std::str::from_utf8(both).unwrap());
        let cases = [
            ("background", "bg", "#010000", "#111111"),
            ("foreground", "fg", "#020000", "#222222"),
            ("dark_background", "dark_bg", "#030000", "#333333"),
            ("darker_background", "darker_bg", "#040000", "#444444"),
            ("lighter_background", "lighter_bg", "#050000", "#555555"),
            ("dark_foreground", "dark_fg", "#060000", "#666666"),
            ("light_foreground", "light_fg", "#070000", "#777777"),
            ("bright_foreground", "bright_fg", "#080000", "#888888"),
        ];
        for (canonical, alias, want, other) in cases {
            assert_eq!(
                color(&raw, canonical, alias),
                Some(hex(want)),
                "{canonical}"
            );
            assert_ne!(
                color(&raw, canonical, alias),
                Some(hex(other)),
                "{canonical}"
            );
        }
        let palette = parse_palette(both).expect("palette");
        assert_eq!(palette.background, hex("#010000"));
        assert_eq!(palette.foreground, hex("#020000"));
        assert_eq!(palette.accent, [1.0, 128.0 / 255.0, 0.0]);
        assert_eq!(palette.mode, Mode::Dark);

        let alias_only = b"bg = \"#0a0b0c\"\nfg = '#010203'\naccent = \"#ffffff\"\n";
        let palette = parse_palette(alias_only).expect("alias palette");
        assert_eq!(palette.background, hex("#0a0b0c"));
        assert_eq!(palette.foreground, hex("#010203"));
        assert_eq!(palette.accent, [1.0, 1.0, 1.0]);
        assert_eq!(palette.mode, Mode::Dark);
    }

    #[test]
    fn mode_light_is_exact_and_a_broken_file_is_absent() {
        let light = b"background = \"#000000\"\nforeground = \"#111111\"\naccent = \"#222222\"\nmode = \"light\"\n";
        assert_eq!(parse_palette(light).unwrap().mode, Mode::Light);
        let named = b"mode = \"Light\"\nbackground = \"#000000\"\nforeground = \"#111111\"\naccent = \"#222222\"\n";
        assert_eq!(parse_palette(named).unwrap().mode, Mode::Dark);
        assert!(parse_palette(b"background = \"#000000\"\nforeground = \"#111111\"\n").is_none());
        assert!(parse_palette(
            b"background = \"#12345\"\nforeground = \"#111111\"\naccent = \"#222222\"\n"
        )
        .is_none());
        assert!(parse_palette(b"\xff\xfe not toml").is_none());
    }

    #[test]
    fn the_state_directory_follows_xdg_then_home() {
        assert_eq!(
            omarchy_current_from(
                Some(OsStr::new("/var/state")),
                Some(OsStr::new("/home/dev"))
            ),
            PathBuf::from("/var/state/omarchy/current")
        );
        assert_eq!(
            omarchy_current_from(Some(OsStr::new("")), Some(OsStr::new("/home/dev"))),
            PathBuf::from("/home/dev/.local/state/omarchy/current")
        );
        assert_eq!(
            omarchy_current_from(None, Some(OsStr::new("/home/dev"))),
            PathBuf::from("/home/dev/.local/state/omarchy/current")
        );
    }

    fn hex(value: &str) -> [f32; 3] {
        let digits = value.trim_start_matches('#');
        let packed = u32::from_str_radix(digits, 16).unwrap();
        [
            ((packed >> 16) & 0xff) as f32 / 255.0,
            ((packed >> 8) & 0xff) as f32 / 255.0,
            (packed & 0xff) as f32 / 255.0,
        ]
    }
}
