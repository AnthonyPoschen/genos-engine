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
}

impl Entry {
    pub fn checkpoint(&self, name: &str) -> Option<&Checkpoint> {
        self.checkpoints.iter().find(|c| c.name == name)
    }
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
    parse_str(&text).map_err(|err| format!("{}: {err}", path.display()))
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
    let mut sun = None;
    let mut sky = None;
    let mut lamps = Vec::new();
    let mut check = None;

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
    fn the_shipped_directory_loads() {
        let entries = load_dir(&scenes_dir()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"sponza"), "{names:?}");
        assert!(names.contains(&"stress"));
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
