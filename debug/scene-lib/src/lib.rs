#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_panics_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![expect(clippy::struct_excessive_bools, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::too_many_lines, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::unwrap_used, reason = "TODO(lint-2026-10-14): pre-existing. docs/LINT_RULES.md")]

//! Shared scene files for the per-system debug binaries.
//!
//! Each file in `debug/scenes/` is one named scene. A binary loads the directory,
//! offers the names at startup and from its menu, and builds its own world from
//! the record. The files stay data: this crate does not open a window or a device.

use std::path::{Path, PathBuf};

/// Where a checkpoint stands, in metres, and where it looks.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    pub name: String,
    pub eye: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// A directional sun. `direction` is the way the rays travel.
#[derive(Clone, Debug, PartialEq)]
pub struct Sun {
    pub direction: [f32; 3],
    pub color: [f32; 3],
}

/// A point lamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Lamp {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

/// The stress building, driven by `genos-stress`'s stage.
#[derive(Clone, Debug, PartialEq)]
pub struct StressSpec {
    pub scale: String,
    pub lights: usize,
    pub dynamic: u32,
    pub layout: String,
    pub time: f32,
    pub freeze_sun: bool,
    pub still_boxes: bool,
    pub still_lamps: bool,
    pub sky: bool,
}

/// What the file asks the binary to build.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Stress(StressSpec),
    /// A `.rhai` scene, path relative to the repository root.
    Rhai { path: String },
    /// A mesh-workbench workspace directory, relative to the repository root
    /// unless it is absolute, on a ground plane under the lamps the file names.
    Mesh {
        workspace: String,
        sun: Option<Sun>,
        sky: Option<[f32; 3]>,
        lamps: Vec<Lamp>,
    },
    /// A glTF file, path relative to the repository root, plus the lamps the
    /// file names. The analytic floor is not part of the model.
    Gltf {
        path: String,
        sun: Option<Sun>,
        sky: Option<[f32; 3]>,
        lamps: Vec<Lamp>,
    },
}

/// What `--check` demands. The named checkpoint is the view. `min_mean` is that
/// picture's luma floor (0–255). Scenes do not share one floor.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub checkpoint: String,
    pub min_mean: f32,
}

/// One scene file.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub review: String,
    pub checkpoints: Vec<Checkpoint>,
    pub check: Check,
    pub kind: Kind,
    /// The file's `hdr KEY VALUE` lines, in file order. Each pair is a key and
    /// a value `genos_scene::HdrSettings::set_key` takes. The program lays them
    /// over the lighting preset's HDR defaults.
    pub hdr: Vec<(String, String)>,
    /// The file the scene was read from. `None` for text that was never a file.
    pub path: Option<PathBuf>,
}

impl Entry {
    pub fn checkpoint(&self, name: &str) -> Option<&Checkpoint> {
        self.checkpoints.iter().find(|c| c.name == name)
    }
}

/// The scene a binary opens when no scene was named: the lighting stress
/// building.
pub const DEFAULT_SCENE: &str = "lighting-stress";

/// The name a scene file is stored under. `stress` is the old name of the
/// lighting stress scene: it still opens that scene and logs a warning once.
pub fn canonical(name: &str) -> &str {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if name == "stress" {
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("scene name stress is deprecated; use {DEFAULT_SCENE}");
        }
        return DEFAULT_SCENE;
    }
    name
}

/// `debug/scene-lib/../../` is the repository root.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `debug/scenes/`, next to this crate.
pub fn scenes_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scenes")
}

/// Every `*.scene` file in `dir`, in file-name order.
pub fn load_dir(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut paths = Vec::new();
    for item in std::fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))? {
        let item = item.map_err(|err| err.to_string())?;
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) == Some("scene") {
            paths.push(path);
        }
    }
    paths.sort();
    paths.iter().map(|path| parse_path(path)).collect()
}

/// Parse one scene file. The file name is only used in errors.
pub fn parse_path(path: &Path) -> Result<Entry, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    let mut entry = parse_str(&text).map_err(|err| format!("{}: {err}", path.display()))?;
    entry.path = Some(path.to_path_buf());
    Ok(entry)
}

/// `(key, value)` of an `hdr KEY VALUE` line, already trimmed. `None` for any
/// other line. A comment is not an hdr line.
fn hdr_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("hdr")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    Some(rest.split_once(char::is_whitespace).map_or((rest, ""), |(key, value)| (key, value.trim())))
}

/// The scene text with its look replaced: every `hdr` line is removed and the
/// look of `settings` (`genos_scene::HDR_LOOK_KEYS`) is appended, one
/// `hdr KEY VALUE` line per key. Every other line and comment stays byte for
/// byte. The second value is the lines that were appended.
pub fn with_hdr_look(text: &str, settings: &genos_scene::HdrSettings) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len() + 512);
    for line in text.split_inclusive('\n') {
        if hdr_line(line).is_none() {
            out.push_str(line);
        }
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    let lines: Vec<String> = settings
        .look_pairs()
        .into_iter()
        .map(|(key, value)| format!("hdr {key} {value}"))
        .collect();
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    (out, lines)
}

/// Write the look of `settings` into the scene file at `path`
/// ([`with_hdr_look`]). The new text goes to a temporary file beside `path`,
/// which is then renamed over it, so a reader never sees half a file. Returns
/// the lines written.
pub fn save_hdr_look(path: &Path, settings: &genos_scene::HdrSettings) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let (next, lines) = with_hdr_look(&text, settings);
    // The file must still be a scene before it replaces the one on disk.
    parse_str(&next).map_err(|err| format!("{}: {err}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: not a file name", path.display()))?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, next).map_err(|err| format!("{}: {err}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {err}", path.display())
    })?;
    Ok(lines)
}

/// Parse the scene text. See `docs/systems/debugging.md` for the lines.
pub fn parse_str(text: &str) -> Result<Entry, String> {
    let mut name = None;
    let mut kind = None;
    let mut review = String::new();
    let mut checkpoints = Vec::new();
    let mut scale = "small".to_string();
    let mut lights = 5usize;
    let mut dynamic = 50u32;
    let mut layout = "spread".to_string();
    let mut time = 0.25f32;
    let mut freeze_sun = false;
    let mut still_boxes = false;
    let mut still_lamps = false;
    let mut sky_on = true;
    let mut rhai = None;
    let mut gltf = None;
    let mut workspace = None;
    let mut sun = None;
    let mut sky = None;
    let mut lamps = Vec::new();
    let mut check = None;
    let mut hdr = Vec::new();
    // Each pair is tried on a scratch value, so a bad key or a bad value is
    // refused here, with its line.
    let mut hdr_scratch = genos_scene::HdrSettings::legacy_bytes();

    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap();
        let rest: Vec<&str> = parts.collect();
        let at = |n: usize| -> Result<f32, String> {
            rest.get(n)
                .ok_or_else(|| format!("line {}: {key} is short", number + 1))?
                .parse()
                .map_err(|_| format!("line {}: bad number in {key}", number + 1))
        };
        match key {
            "name" => name = Some(join(&rest)),
            "kind" => kind = Some(join(&rest)),
            "review" => {
                if !review.is_empty() {
                    review.push(' ');
                }
                review.push_str(&join(&rest));
            }
            "scale" => scale = join(&rest),
            "lights" => lights = at(0)? as usize,
            "dynamic" => dynamic = at(0)? as u32,
            "layout" => layout = join(&rest),
            "time" => time = at(0)?,
            "freeze-sun" => freeze_sun = true,
            "still-boxes" => still_boxes = true,
            "still-lamps" => still_lamps = true,
            "no-sky" => sky_on = false,
            "rhai" => rhai = Some(join(&rest)),
            "gltf" => gltf = Some(join(&rest)),
            "workspace" => workspace = Some(join(&rest)),
            "checkpoint" => checkpoints.push(parse_checkpoint(number + 1, &rest)?),
            "check" => {
                if check.is_some() {
                    return Err(format!("line {}: check is already set", number + 1));
                }
                check = Some(parse_check(number + 1, &rest)?);
            }
            "sun" => {
                sun = Some(Sun {
                    direction: [at(0)?, at(1)?, at(2)?],
                    color: [at(3)?, at(4)?, at(5)?],
                });
            }
            "sky" => sky = Some([at(0)?, at(1)?, at(2)?]),
            "lamp" => lamps.push(Lamp {
                position: [at(0)?, at(1)?, at(2)?],
                color: [at(3)?, at(4)?, at(5)?],
            }),
            "hdr" => {
                let [key, value] = rest[..] else {
                    return Err(format!("line {}: hdr is 'hdr KEY VALUE'", number + 1));
                };
                hdr_scratch
                    .set_key(key, value)
                    .map_err(|err| format!("line {}: {err}", number + 1))?;
                hdr.push((key.to_string(), value.to_string()));
            }
            other => return Err(format!("line {}: unknown key {other}", number + 1)),
        }
    }

    let name = name.ok_or("scene has no name")?;
    let kind = match kind.as_deref() {
        Some("stress") => Kind::Stress(StressSpec {
            scale,
            lights,
            dynamic,
            layout,
            time,
            freeze_sun,
            still_boxes,
            still_lamps,
            sky: sky_on,
        }),
        Some("rhai") => Kind::Rhai {
            path: rhai.ok_or("rhai scene needs a rhai path")?,
        },
        Some("gltf") => Kind::Gltf {
            path: gltf.ok_or("gltf scene needs a gltf path")?,
            sun,
            sky,
            lamps,
        },
        Some("mesh") => Kind::Mesh {
            workspace: workspace.ok_or("mesh scene needs a workspace path")?,
            sun,
            sky,
            lamps,
        },
        Some(other) => return Err(format!("unknown kind {other}")),
        None => return Err("scene has no kind".into()),
    };
    if checkpoints.is_empty() {
        return Err(format!("{name} has no checkpoint"));
    }
    let check = check.ok_or_else(|| format!("{name} has no check"))?;
    if !checkpoints.iter().any(|c| c.name == check.checkpoint) {
        return Err(format!(
            "{name} check names unknown checkpoint {}",
            check.checkpoint
        ));
    }
    Ok(Entry {
        name,
        review,
        checkpoints,
        check,
        kind,
        hdr,
        path: None,
    })
}

/// `check CHECKPOINT mean MIN`. MIN is the luma floor for that checkpoint.
fn parse_check(line: usize, rest: &[&str]) -> Result<Check, String> {
    if rest.len() != 3 || rest[1] != "mean" {
        return Err(format!(
            "line {line}: check is 'check CHECKPOINT mean MIN'"
        ));
    }
    let min_mean = num(line, rest[2])?;
    if min_mean < 0.0 {
        return Err(format!("line {line}: check mean is negative"));
    }
    Ok(Check {
        checkpoint: rest[0].to_string(),
        min_mean,
    })
}

fn join(parts: &[&str]) -> String {
    parts.join(" ")
}

fn parse_checkpoint(line: usize, rest: &[&str]) -> Result<Checkpoint, String> {
    if rest.len() < 5 {
        return Err(format!("line {line}: checkpoint is short"));
    }
    let name = rest[0].to_string();
    let eye = [
        num(line, rest[1])?,
        num(line, rest[2])?,
        num(line, rest[3])?,
    ];
    let (yaw, pitch) = match rest[4] {
        "yaw" => {
            let yaw = num(line, rest.get(5).copied().unwrap_or(""))?;
            let pitch = if rest.get(6) == Some(&"pitch") {
                num(line, rest.get(7).copied().unwrap_or(""))?
            } else {
                0.0
            };
            (yaw, pitch)
        }
        "look" => {
            let target = [
                num(line, rest.get(5).copied().unwrap_or(""))?,
                num(line, rest.get(6).copied().unwrap_or(""))?,
                num(line, rest.get(7).copied().unwrap_or(""))?,
            ];
            look_yaw_pitch(eye, target)
        }
        _ => return Err(format!("line {line}: checkpoint needs yaw or look")),
    };
    Ok(Checkpoint {
        name,
        eye,
        yaw,
        pitch,
    })
}

fn num(line: usize, text: &str) -> Result<f32, String> {
    text.parse()
        .map_err(|_| format!("line {line}: bad number {text}"))
}

/// yaw = atan2(dx, -dz), pitch = asin(dy / len). Yaw 0 looks down -Z.
pub fn look_yaw_pitch(eye: [f32; 3], target: [f32; 3]) -> (f32, f32) {
    let d = [
        target[0] - eye[0],
        target[1] - eye[1],
        target[2] - eye[2],
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1.0e-8);
    let pitch = (d[1] / len).clamp(-1.0, 1.0).asin();
    let yaw = d[0].atan2(-d[2]);
    (yaw, pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stress_file_keeps_its_checkpoints_and_a_look_at_pose() {
        let entry = parse_str(
            "name stress\nkind stress\nfreeze-sun\nreview Hall bounce.\n\
             checkpoint hall 14.5 1.7 23.5 yaw -0.3554 pitch 0.08\n\
             checkpoint corner 3.5 1.6 3.5 look 0.2 0.4 0.2\n\
             check hall mean 40\n",
        )
        .unwrap();
        assert_eq!(entry.name, "stress");
        let Kind::Stress(spec) = &entry.kind else {
            panic!("kind");
        };
        assert!(spec.freeze_sun);
        assert_eq!(spec.lights, 5);
        assert_eq!(entry.checkpoints[0].name, "hall");
        assert!((entry.checkpoints[0].pitch - 0.08).abs() < 1e-6);
        let (yaw, pitch) = look_yaw_pitch([3.5, 1.6, 3.5], [0.2, 0.4, 0.2]);
        assert!((entry.checkpoints[1].yaw - yaw).abs() < 1e-5);
        assert!((entry.checkpoints[1].pitch - pitch).abs() < 1e-5);
        assert_eq!(entry.check.checkpoint, "hall");
        assert!((entry.check.min_mean - 40.0).abs() < 1e-6);
    }

    #[test]
    fn a_scene_needs_a_check_that_names_one_of_its_checkpoints() {
        let missing = parse_str(
            "name room\nkind rhai\nrhai examples/stress/scenes/room.rhai\n\
             checkpoint open 0 1.7 8 look 0 1 0\n",
        );
        assert!(missing.unwrap_err().contains("has no check"));
        let unknown = parse_str(
            "name room\nkind rhai\nrhai examples/stress/scenes/room.rhai\n\
             checkpoint open 0 1.7 8 look 0 1 0\n\
             check corner mean 8\n",
        );
        assert!(unknown.unwrap_err().contains("unknown checkpoint corner"));
    }

    #[test]
    fn the_old_stress_name_opens_the_lighting_stress_scene() {
        assert_eq!(canonical("stress"), "lighting-stress");
        assert_eq!(canonical("sponza"), "sponza");
        assert_eq!(canonical(DEFAULT_SCENE), DEFAULT_SCENE);
    }

    #[test]
    fn a_mesh_file_names_its_workspace() {
        let entry = parse_str(
            "name m\nkind mesh\nworkspace some/dir\nsky 0.1 0.2 0.3\n\
             checkpoint front 0 1 4 look 0 0 0\ncheck front mean 8\n",
        )
        .unwrap();
        let Kind::Mesh { workspace, sun, sky, lamps } = &entry.kind else {
            panic!("kind");
        };
        assert_eq!(workspace, "some/dir");
        assert_eq!((sun.is_none(), *sky, lamps.len()), (true, Some([0.1, 0.2, 0.3]), 0));
    }

    const ROOM: &str = "name room\nkind rhai\nrhai examples/stress/scenes/room.rhai\n\
                        checkpoint open 0 1.7 8 look 0 1 0\ncheck open mean 8\n";

    #[test]
    fn hdr_lines_are_kept_in_order_and_a_bad_one_names_its_line() {
        let entry = parse_str(&format!(
            "{ROOM}hdr compensation -1\nhdr bloom_intensity 0.02\nhdr exposure_mode auto\n"
        ))
        .unwrap();
        let pairs: Vec<(&str, &str)> = entry.hdr.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(
            pairs,
            [("compensation", "-1"), ("bloom_intensity", "0.02"), ("exposure_mode", "auto")]
        );
        assert!(entry.path.is_none());
        assert_eq!(parse_str(ROOM).unwrap().hdr, Vec::new());

        let key = parse_str(&format!("{ROOM}hdr glow 1\n")).unwrap_err();
        assert_eq!(key, "line 6: unknown hdr setting glow");
        let value = parse_str(&format!("{ROOM}hdr compensation -1\nhdr ev100 bright\n")).unwrap_err();
        assert_eq!(value, "line 7: ev100 needs a number, got bright");
        let short = parse_str(&format!("{ROOM}hdr bloom\n")).unwrap_err();
        assert_eq!(short, "line 6: hdr is 'hdr KEY VALUE'");
    }

    #[test]
    fn saving_a_look_replaces_the_hdr_lines_and_nothing_else() {
        let before = "# A room.\nname room\nkind rhai\nrhai examples/stress/scenes/room.rhai\n\
                      hdr compensation 3\n  # hdr is set below\n\
                      checkpoint open 0 1.7 8   look 0 1 0\n\thdr bloom off\n\
                      check open mean 8\n\n# hdrs\n";
        let kept = "# A room.\nname room\nkind rhai\nrhai examples/stress/scenes/room.rhai\n\
                    \x20 # hdr is set below\n\
                    checkpoint open 0 1.7 8   look 0 1 0\n\
                    check open mean 8\n\n# hdrs\n";
        let dir = std::env::temp_dir().join(format!("genos-scene-lib-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("room.scene");
        std::fs::write(&path, before).unwrap();

        let mut look = genos_scene::HdrSettings::for_preset(genos_scene::LightingPreset::High);
        look.compensation = -1.5;
        look.bloom_intensity = 0.02;
        look.transition = 2.0;
        let lines = save_hdr_look(&path, &look).unwrap();
        assert_eq!(lines.len(), genos_scene::HDR_LOOK_KEYS.len());
        assert!(lines.contains(&"hdr compensation -1.5".to_string()));
        assert!(lines.contains(&"hdr bloom on".to_string()));
        assert!(lines.contains(&"hdr transition 2".to_string()));
        assert!(!lines.iter().any(|line| {
            line.starts_with("hdr output") || line.starts_with("hdr histogram_bins") || line.starts_with("hdr legacy_bytes")
        }));

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, format!("{kept}{}\n", lines.join("\n")));
        // No temporary file is left beside it.
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|item| item.unwrap().file_name()).collect();
        assert_eq!(left, ["room.scene"]);

        // The file's lines over the preset give the look back.
        let entry = parse_path(&path).unwrap();
        assert_eq!(entry.path.as_deref(), Some(path.as_path()));
        let mut read = genos_scene::HdrSettings::for_preset(genos_scene::LightingPreset::High);
        for (key, value) in &entry.hdr {
            read.set_key(key, value).unwrap();
        }
        assert_eq!(read, look);

        // A second save changes nothing: the hdr lines do not pile up.
        save_hdr_look(&path, &look).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after);

        // A file with no newline at its end gets one before the look.
        std::fs::write(&path, ROOM.trim_end()).unwrap();
        let lines = save_hdr_look(&path, &look).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{ROOM}{}\n", lines.join("\n")));
        std::fs::remove_dir_all(&dir).unwrap();

        let missing = save_hdr_look(&dir.join("gone.scene"), &look).unwrap_err();
        assert!(missing.contains("gone.scene"), "{missing}");
    }

    #[test]
    fn the_shipped_directory_loads() {
        let entries = load_dir(&scenes_dir()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"sponza"), "{names:?}");
        assert!(names.contains(&DEFAULT_SCENE), "{names:?}");
        assert!(!names.contains(&"stress"), "the old name is an alias, not a file");
        assert!(names.contains(&"hell"));
        let sponza = entries.iter().find(|e| e.name == "sponza").unwrap();
        assert!(sponza.checkpoint("atrium").is_some());
        assert!(sponza.checkpoint("dark-corner").is_some());
        let bounce = entries.iter().find(|e| e.name == "bounce_corner").unwrap();
        assert_eq!(bounce.check.checkpoint, "room-a");
        assert!(bounce.checkpoint("corner").is_some());
        for entry in &entries {
            assert!(
                entry.checkpoint(&entry.check.checkpoint).is_some(),
                "{} check {}",
                entry.name,
                entry.check.checkpoint
            );
        }
    }
}
