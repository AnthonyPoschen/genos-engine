//! The session file: the scene a program was showing, where the camera
//! stood and the scene's settings values, written while the program runs
//! and read when it next opens. Text, one `key=value` a line under a
//! `[section]` line; a key the program no longer has is skipped, so a file
//! from an older build restores what still fits.

use std::path::{Path, PathBuf};

use genos_scene::{Camera, EngineSettings, Scene, Vec3};

/// The first line of the file.
const HEAD: &str = "# genos session";

/// What a session file holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Session {
    pub scene: String,
    /// Position, yaw and pitch.
    pub camera: Option<[f32; 5]>,
    /// How fast a still scene's suns turn (knob `sun_speed`).
    pub sun_speed: Option<f64>,
    /// Each settings value's lines, by its section name.
    sections: Vec<(String, Vec<(String, String)>)>,
}

/// `<out>/session/<program>.txt`.
pub(crate) fn path(out: &Path, program: &str) -> PathBuf {
    out.join("session").join(format!("{program}.txt"))
}

fn lines_of(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|line| line.split_once('=')).map(|(key, value)| (key.trim().to_string(), value.trim().to_string())).collect()
}

/// The keys of a section that are not restored: where files are written is
/// the launch's (`--out`), and the menu's tab and folds have files of their own.
fn skipped(section: &str, key: &str) -> bool {
    matches!((section, key), ("debug", "out") | ("menu", "tab" | "collapsed"))
}

fn overlay<T: EngineSettings>(value: &mut T, section: &str, lines: &[(String, String)]) {
    for (key, text) in lines {
        if !skipped(section, key) {
            // A key or a value this build does not have: left as it is.
            let _ = value.set_key(key, text);
        }
    }
}

impl Session {
    /// The session as it is now.
    pub(crate) fn of(name: &str, scene: &Scene, camera: &Camera, sun_speed: f64) -> Self {
        let sections = [
            ("lighting", scene.lighting.to_text()),
            ("hdr", scene.hdr.to_text()),
            ("fog", scene.fog.to_text()),
            ("particles", scene.particles.to_text()),
            ("reflections", scene.reflections.to_text()),
            ("debug", scene.debug.to_text()),
            ("fps_counter", scene.fps_counter.to_text()),
            ("menu", scene.menu.to_text()),
        ];
        Self {
            scene: name.to_string(),
            camera: Some([camera.position.x, camera.position.y, camera.position.z, camera.yaw, camera.pitch]),
            sun_speed: Some(sun_speed),
            sections: sections.into_iter().map(|(section, text)| (section.to_string(), lines_of(&text))).collect(),
        }
    }

    pub(crate) fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = format!("{HEAD}\n[program]\nscene={}\n", self.scene);
        if let Some(c) = self.camera {
            let _ = writeln!(out, "camera={} {} {} {} {}", c[0], c[1], c[2], c[3], c[4]);
        }
        if let Some(speed) = self.sun_speed {
            let _ = writeln!(out, "sun_speed={speed}");
        }
        for (section, lines) in &self.sections {
            let _ = writeln!(out, "[{section}]");
            for (key, value) in lines {
                let _ = writeln!(out, "{key}={value}");
            }
        }
        out
    }

    /// The session a file's text holds; none when it names no scene.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut session = Self { scene: String::new(), camera: None, sun_speed: None, sections: Vec::new() };
        let mut program = false;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(section) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
                program = section == "program";
                if !program {
                    session.sections.push((section.to_string(), Vec::new()));
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if !program {
                if let Some((_, lines)) = session.sections.last_mut() {
                    lines.push((key.to_string(), value.to_string()));
                }
                continue;
            }
            match key {
                "scene" => session.scene = value.to_string(),
                "camera" => {
                    let parts: Vec<f32> = value.split_whitespace().filter_map(|part| part.parse().ok()).collect();
                    session.camera = <[f32; 5]>::try_from(parts).ok().filter(|c| c.iter().all(|v| v.is_finite()));
                }
                "sun_speed" => session.sun_speed = value.parse().ok().filter(|v: &f64| v.is_finite()),
                _ => {}
            }
        }
        (!session.scene.is_empty()).then_some(session)
    }

    /// Put the saved settings on `scene`, key by key. `lighting` false
    /// leaves the lighting settings as the launch's arguments made them.
    pub(crate) fn restore(&self, scene: &mut Scene, lighting: bool) {
        for (section, lines) in &self.sections {
            match section.as_str() {
                "lighting" if lighting => overlay(&mut scene.lighting, section, lines),
                "hdr" => overlay(&mut scene.hdr, section, lines),
                "fog" => overlay(&mut scene.fog, section, lines),
                "particles" => overlay(&mut scene.particles, section, lines),
                "reflections" => overlay(&mut scene.reflections, section, lines),
                "debug" => overlay(&mut scene.debug, section, lines),
                "fps_counter" => overlay(&mut scene.fps_counter, section, lines),
                "menu" => overlay(&mut scene.menu, section, lines),
                _ => {}
            }
        }
    }

    /// Stand the camera where it was saved.
    pub(crate) fn place(&self, camera: &mut Camera) {
        if let Some(c) = self.camera {
            camera.set_pose(Vec3::new(c[0], c[1], c[2]), c[3], c[4]);
        }
    }
}

pub(crate) fn load(out: &Path, program: &str) -> Option<Session> {
    Session::parse(&std::fs::read_to_string(path(out, program)).ok()?)
}

pub(crate) fn save(out: &Path, program: &str, text: &str) -> Result<(), String> {
    let path = path(out, program);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, text).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Scene {
        Scene::empty(genos_scene::Floor { position: Vec3::new(0.0, 0.0, 0.0), half_x: 1.0, half_z: 1.0, color: [0.5; 3] })
    }

    /// What a person changed comes back on a scene opened fresh, and a line
    /// the build does not know is passed over.
    #[test]
    fn a_saved_session_restores_the_changed_settings_and_the_camera() {
        let mut scene = blank();
        scene.lighting.set_key("cone_cascades", "on").expect("key");
        scene.lighting.set_key("cone_cascades.lamp_map_tiles", "512").expect("key");
        scene.debug.set_key("light_gizmos", "on").expect("key");
        let mut camera = Camera::new(1.0, 2.0, 3.0);
        camera.set_pose(Vec3::new(4.0, 5.0, 6.0), 0.5, -0.25);
        let text = Session::of("sponza", &scene, &camera, 2.0).to_text().replace("[hdr]", "[hdr]\nno_such_key=7");
        let session = Session::parse(&text).expect("a session");
        assert_eq!(session.scene, "sponza");
        assert_eq!(session.sun_speed, Some(2.0));
        let mut opened = blank();
        let mut stood = Camera::new(0.0, 0.0, 0.0);
        session.restore(&mut opened, true);
        session.place(&mut stood);
        assert_eq!(opened.lighting.to_text(), scene.lighting.to_text());
        assert_eq!(opened.debug.to_text(), scene.debug.to_text());
        assert_eq!(opened.hdr.to_text(), scene.hdr.to_text());
        assert_eq!((stood.position.x, stood.position.y, stood.position.z, stood.yaw, stood.pitch), (4.0, 5.0, 6.0, 0.5, -0.25));
        // The launch's lighting arguments win when it gave any.
        let mut argued = blank();
        session.restore(&mut argued, false);
        assert_eq!(argued.lighting.to_text(), blank().lighting.to_text());
        assert!(Session::parse("# nothing\n").is_none());
    }
}
