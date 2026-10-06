//! Omarchy theme colors through the shipped load path and `lighting_frame`.

use std::path::{Path, PathBuf};

use genos_scene::Camera;
use genos_ui::id;
use genos_ui::omarchy::{omarchy_current_dir, omarchy_palette, on_omarchy, Mode};
use genos_ui::{lighting_frame, Frame, Look, Pointer, Shown, State};

const VIEW: [f32; 2] = [1280.0, 720.0];

#[test]
fn an_absent_current_directory_keeps_the_built_in_looks() {
    let current = fixture("absent").join("missing-current");
    assert!(
        !on_omarchy(&current),
        "a missing directory counted as Omarchy"
    );
    assert!(omarchy_palette(&current).is_none());

    let mut state = State::default();
    state.omarchy_current = Some(current);
    let camera = Camera::opening();
    let idle = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(0.0, 0.0, false),
    );
    assert_eq!(shown(&idle, id::PANEL).look, builtin_panel_idle());
    assert_eq!(shown(&idle, id::X).look, builtin_control_idle());

    let (x, y) = center(&shown(&idle, id::X));
    let hover = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(x, y, false),
    );
    assert_eq!(shown(&hover, id::X).look, builtin_control_hover());
    let press = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(x, y, true),
    );
    assert_eq!(shown(&press, id::X).look, builtin_control_pressed());
}

#[test]
fn theme_colors_paint_the_panel_and_a_legacy_alias_does_not_win() {
    let root = fixture("colors");
    let current = root.join("current");
    std::fs::create_dir_all(&current).unwrap();
    install_theme(&current, "fixture", FIXTURE);
    assert!(on_omarchy(&current));

    let palette = omarchy_palette(&current).expect("fixture palette");
    assert_eq!(palette.mode, Mode::Dark);
    assert_eq!(palette.background, hex("111111"));
    assert_eq!(palette.foreground, hex("333333"));
    assert_eq!(palette.accent, hex("555555"));

    let camera = Camera::opening();
    let mut state = State::default();
    state.omarchy_current = Some(current);
    let idle = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(0.0, 0.0, false),
    );
    let panel = shown(&idle, id::PANEL);
    let (x, y) = (panel.rect.x + 1.0, panel.rect.y + 1.0);
    let hover = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(x, y, false),
    );
    let press = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(x, y, true),
    );
    let colors = paint_colors([&idle, &hover, &press]);
    assert!(
        colors.contains(&hex("111111")),
        "background missing: {colors:?}"
    );
    assert!(
        colors.contains(&hex("333333")),
        "foreground missing: {colors:?}"
    );
    assert!(
        colors.contains(&hex("555555")),
        "accent missing: {colors:?}"
    );
    assert!(!colors.contains(&hex("222222")), "bg overrode background");
    assert!(!colors.contains(&hex("444444")), "fg overrode foreground");
    assert!(
        !colors.contains(&hex("666666")),
        "dark_background became a panel color"
    );
    assert!(
        !colors.contains(&hex("777777")),
        "dark_bg overrode dark_background"
    );

    let button = shown(&idle, id::X);
    let (bx, by) = center(&button);
    let button_hover = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(bx, by, false),
    );
    let button_press = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(bx, by, true),
    );
    assert_eq!(shown(&button_hover, id::X).look.border, hex("555555"));
    assert_eq!(shown(&button_press, id::X).look.background, hex("555555"));
    assert_eq!(shown(&idle, id::X).look.background, hex("111111"));
    assert_eq!(shown(&idle, id::X).look.border, hex("333333"));
}

#[test]
fn mode_is_light_from_the_key_or_from_light_mode_and_dark_otherwise() {
    let root = fixture("mode");
    let current = root.join("current");
    std::fs::create_dir_all(&current).unwrap();

    install_theme(&current, "dark", &colors("111111", "333333", "555555"));
    assert_eq!(omarchy_palette(&current).unwrap().mode, Mode::Dark);

    install_theme(
        &current,
        "keyed",
        &format!(
            "{}\nmode = \"light\"\n",
            colors("111111", "333333", "555555")
        ),
    );
    assert_eq!(omarchy_palette(&current).unwrap().mode, Mode::Light);

    install_theme(&current, "marker", &colors("111111", "333333", "555555"));
    std::fs::write(current.join("theme").join("light.mode"), "").unwrap();
    assert_eq!(omarchy_palette(&current).unwrap().mode, Mode::Light);

    install_theme(
        &current,
        "marker-over-dark",
        &format!(
            "{}\nmode = \"dark\"\n",
            colors("111111", "333333", "555555")
        ),
    );
    std::fs::write(current.join("theme").join("light.mode"), "").unwrap();
    assert_eq!(omarchy_palette(&current).unwrap().mode, Mode::Light);
    assert_ne!(omarchy_palette(&current).unwrap().mode, Mode::Dark);
}

#[test]
fn a_theme_swap_updates_the_next_frame_after_a_missing_file() {
    let root = fixture("swap");
    let current = root.join("current");
    std::fs::create_dir_all(&current).unwrap();
    install_theme(&current, "a", &colors("100000", "200000", "300000"));

    let camera = Camera::opening();
    let mut state = State::default();
    state.omarchy_current = Some(current.clone());
    let first = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(0.0, 0.0, false),
    );
    assert!(
        paint_colors([&first]).contains(&hex("100000")),
        "theme A did not paint"
    );
    assert!(!paint_colors([&first]).contains(&hex("400000")));

    std::fs::remove_dir_all(current.join("theme")).unwrap();
    let gap = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(0.0, 0.0, false),
    );
    assert!(
        paint_colors([&gap]).contains(&hex("100000")),
        "the gap dropped the previous palette"
    );
    assert!(!paint_colors([&gap]).contains(&hex("400000")));

    install_theme(&current, "b", &colors("400000", "500000", "600000"));
    let next = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(0.0, 0.0, false),
    );
    let painted = paint_colors([&next]);
    assert!(
        painted.contains(&hex("400000")),
        "theme B did not paint: {painted:?}"
    );
    assert!(
        painted.contains(&hex("500000")),
        "theme B foreground missing"
    );
    assert!(
        !painted.contains(&hex("100000")),
        "theme A stayed after the swap"
    );

    std::fs::write(
        current.join("theme").join("colors.toml"),
        colors("700000", "800000", "900000"),
    )
    .unwrap();
    let restored = lighting_frame(
        &mut state,
        VIEW,
        &camera,
        Some([0.0, 7.0, 0.0]),
        pointer(0.0, 0.0, false),
    );
    let painted = paint_colors([&restored]);
    assert!(
        painted.contains(&hex("700000")),
        "the restored file did not paint"
    );
    assert!(
        !painted.contains(&hex("400000")),
        "theme B stayed after the rewrite"
    );
    assert!(
        !painted.contains(&hex("100000")),
        "theme A stayed after the rewrite"
    );
}

#[test]
fn the_host_palette_background_matches_colors_toml() {
    let current = omarchy_current_dir();
    if !on_omarchy(&current) {
        assert!(omarchy_palette(&current).is_none());
        eprintln!("host is not Omarchy at {}", current.display());
        return;
    }
    let path = current.join("theme").join("colors.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("host has no readable colors.toml at {}", path.display());
        return;
    };
    let expected = file_background(&text).expect("background key");
    let palette = omarchy_palette(&current).expect("host palette");
    assert_eq!(palette.background, expected);
    eprintln!(
        "host palette background {expected:?} matches {}",
        path.display()
    );
}

const FIXTURE: &str = r##"
background = "#111111"
bg = "#222222"
foreground = "#333333"
fg = "#444444"
accent = "#555555"
dark_background = "#666666"
dark_bg = "#777777"
"##;

fn colors(background: &str, foreground: &str, accent: &str) -> String {
    format!(
        "background = \"#{background}\"\nforeground = \"#{foreground}\"\naccent = \"#{accent}\"\n"
    )
}

fn install_theme(current: &Path, name: &str, body: &str) {
    let next = current.join("next-theme");
    let _ = std::fs::remove_dir_all(&next);
    std::fs::create_dir_all(&next).unwrap();
    std::fs::write(next.join("colors.toml"), body).unwrap();
    let theme = current.join("theme");
    let _ = std::fs::remove_dir_all(&theme);
    std::fs::rename(&next, &theme).unwrap();
    std::fs::write(current.join("theme.name"), name).unwrap();
}

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genos-ui-omarchy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn pointer(x: f32, y: f32, down: bool) -> Pointer {
    Pointer { x, y, down }
}

fn shown(frame: &Frame, id: u32) -> Shown {
    frame
        .shown
        .iter()
        .cloned()
        .find(|item| item.id == id)
        .expect("panel item")
}

fn center(item: &Shown) -> (f32, f32) {
    (
        item.rect.x + item.rect.w * 0.5,
        item.rect.y + item.rect.h * 0.5,
    )
}

fn paint_colors<'a>(frames: impl IntoIterator<Item = &'a Frame>) -> Vec<[f32; 3]> {
    frames
        .into_iter()
        .flat_map(|frame| frame.paints.iter().map(|paint| paint.color))
        .collect()
}

fn hex(digits: &str) -> [f32; 3] {
    let packed = u32::from_str_radix(digits, 16).unwrap();
    [
        ((packed >> 16) & 0xff) as f32 / 255.0,
        ((packed >> 8) & 0xff) as f32 / 255.0,
        (packed & 0xff) as f32 / 255.0,
    ]
}

fn builtin_panel_idle() -> Look {
    Look {
        background: [0.08, 0.09, 0.11],
        border: [0.42, 0.46, 0.52],
    }
}

fn builtin_control_idle() -> Look {
    Look {
        background: [0.16, 0.17, 0.20],
        border: [0.35, 0.40, 0.45],
    }
}

fn builtin_control_hover() -> Look {
    Look {
        background: [0.28, 0.32, 0.38],
        border: [0.55, 0.65, 0.75],
    }
}

fn builtin_control_pressed() -> Look {
    Look {
        background: [0.42, 0.55, 0.32],
        border: [0.75, 0.90, 0.45],
    }
}

fn file_background(text: &str) -> Option<[f32; 3]> {
    let mut canonical = None;
    let mut alias = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(value) = keyed(line, "background") {
            canonical = hex_value(value);
        } else if let Some(value) = keyed(line, "bg") {
            alias = hex_value(value);
        }
    }
    canonical.or(alias)
}

fn keyed<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    Some(quoted(rest))
}

fn quoted(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return value;
    }
    let quote = bytes[0];
    if quote != b'"' && quote != b'\'' {
        return value.split_whitespace().next().unwrap_or(value);
    }
    match value[1..].find(quote as char) {
        Some(index) => &value[1..index + 1],
        None => &value[1..],
    }
}

fn hex_value(value: &str) -> Option<[f32; 3]> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    Some(hex(digits))
}
