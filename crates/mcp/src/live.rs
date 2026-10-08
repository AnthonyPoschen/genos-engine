//! The scene and camera the frame draws and steps.
//!
//! Edits use stable handles. A collider change rebuilds the room the capsule
//! walks through. The floor handle is always `floor`.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use genos_scene::{
    look_direction, Camera, Floor, Light, Scene, Shape, Solid, Vec3, Wall, PITCH_LIMIT,
};

use crate::json::{self, Value};

pub const SCENE_URI: &str = "genos://scene";

#[derive(Clone)]
pub struct Host {
    shared: std::sync::Arc<Shared>,
}

struct Shared {
    inner: Mutex<Inner>,
    changed: Condvar,
}

struct Inner {
    scene: Scene,
    camera: Camera,
    handles: Handles,
    scene_revision: u64,
    published: u64,
    sessions: HashMap<String, bool>,
}

struct Handles {
    next: u64,
    walls: Vec<u64>,
    solids: Vec<u64>,
    lights: Vec<u64>,
}

#[derive(Clone, Copy)]
enum Slot {
    Floor,
    Wall(usize),
    Solid(usize),
    Light(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Floor,
    Wall,
    Solid,
    Light,
}

#[derive(Default)]
struct Patch {
    position: Option<Vec3>,
    half_x: Option<f32>,
    half_z: Option<f32>,
    height: Option<f32>,
    size: Option<f32>,
    shape: Option<Shape>,
    color: Option<[f32; 3]>,
    absorption: Option<f32>,
    look: Option<Vec3>,
    yaw: Option<f32>,
    pitch: Option<f32>,
}

pub enum ToolResult {
    Unknown,
    Done(Result<String, String>),
}

impl Host {
    pub fn new(scene: Scene, camera: Camera) -> Self {
        Self {
            shared: std::sync::Arc::new(Shared {
                inner: Mutex::new(Inner {
                    handles: Handles::index(&scene),
                    scene,
                    camera,
                    scene_revision: 1,
                    published: 0,
                    sessions: HashMap::new(),
                }),
                changed: Condvar::new(),
            }),
        }
    }

    /// Run one frame against the live scene and camera.
    ///
    /// The closure holds the scene lock. Do not call back into the host.
    /// A change to the scene or the camera pose publishes one resource update.
    pub fn with_frame<R>(&self, body: impl FnOnce(&mut Scene, &mut Camera) -> R) -> R {
        self.edit(move |inner| {
            let scene_before = inner.scene.clone();
            let pose_before = pose(&inner.camera);
            let result = body(&mut inner.scene, &mut inner.camera);
            let scene_changed = inner.scene != scene_before;
            let pose_changed = pose(&inner.camera) != pose_before;
            if scene_changed {
                inner.scene_revision = inner.scene_revision.wrapping_add(1);
            }
            if scene_changed || pose_changed {
                inner.published = inner.published.wrapping_add(1);
            }
            result
        })
    }

    /// Scene the frame copies into the render world.
    pub fn drawn_scene(&self) -> Scene {
        self.lock().scene.clone()
    }

    pub fn camera(&self) -> Camera {
        self.lock().camera.clone()
    }

    pub fn scene_revision(&self) -> u64 {
        self.lock().scene_revision
    }

    pub(crate) fn open_session(&self) -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = format!(
            "{:x}-{:x}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        self.lock().sessions.insert(id.clone(), false);
        id
    }

    pub(crate) fn close_session(&self, id: &str) {
        self.lock().sessions.remove(id);
    }

    pub(crate) fn session_open(&self, id: &str) -> bool {
        self.lock().sessions.contains_key(id)
    }

    pub(crate) fn published(&self) -> u64 {
        self.lock().published
    }

    pub(crate) fn wake(&self) {
        self.shared.changed.notify_all();
    }

    /// Wait until a subscribed session should hear a resource update.
    /// Returns `None` when `shutdown` is set.
    pub(crate) fn wait_published(
        &self,
        session: &str,
        seen: u64,
        shutdown: &std::sync::atomic::AtomicBool,
    ) -> Option<u64> {
        let mut inner = self.lock();
        loop {
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                return None;
            }
            let subscribed = inner.sessions.get(session).copied().unwrap_or(false);
            if subscribed && inner.published != seen {
                return Some(inner.published);
            }
            let (guard, _) = self
                .shared
                .changed
                .wait_timeout(inner, Duration::from_millis(400))
                .unwrap_or_else(|err| err.into_inner());
            inner = guard;
        }
    }

    pub(crate) fn subscribe(&self, session: &str, uri: &str) -> Result<(), String> {
        if uri != SCENE_URI {
            return Err(format!("unknown resource {uri}"));
        }
        let mut inner = self.lock();
        let Some(flag) = inner.sessions.get_mut(session) else {
            return Err("session is gone".into());
        };
        *flag = true;
        Ok(())
    }

    pub(crate) fn unsubscribe(&self, session: &str, uri: &str) -> Result<(), String> {
        if uri != SCENE_URI {
            return Err(format!("unknown resource {uri}"));
        }
        let mut inner = self.lock();
        let Some(flag) = inner.sessions.get_mut(session) else {
            return Err("session is gone".into());
        };
        *flag = false;
        Ok(())
    }

    pub(crate) fn scene_text(&self) -> String {
        let inner = self.lock();
        json::encode(&document(&inner))
    }

    pub(crate) fn call_tool(&self, name: &str, arguments: &Value) -> ToolResult {
        let known = matches!(
            name,
            "read_scene" | "set_object" | "spawn_object" | "remove_object" | "set_camera"
        );
        if !known {
            return ToolResult::Unknown;
        }
        let outcome = self.edit(|inner| match name {
            "read_scene" => Ok(json::encode(&document(inner))),
            "set_object" => set_object(inner, arguments).map(|()| json::encode(&document(inner))),
            "spawn_object" => spawn_object(inner, arguments).map(|handle| {
                json::encode(&json::object([
                    ("handle", json::string(handle)),
                    ("scene", document(inner)),
                ]))
            }),
            "remove_object" => {
                remove_object(inner, arguments).map(|()| json::encode(&document(inner)))
            }
            "set_camera" => set_camera(inner, arguments).map(|()| json::encode(&document(inner))),
            _ => Err("unknown tool".into()),
        });
        ToolResult::Done(outcome)
    }

    fn edit<R>(&self, body: impl FnOnce(&mut Inner) -> R) -> R {
        let mut inner = self.lock();
        let before = inner.published;
        let result = body(&mut inner);
        let notify = inner.published != before;
        drop(inner);
        if notify {
            self.shared.changed.notify_all();
        }
        result
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.shared
            .inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}

impl Handles {
    fn index(scene: &Scene) -> Self {
        let mut handles = Self {
            next: 1,
            walls: Vec::new(),
            solids: Vec::new(),
            lights: Vec::new(),
        };
        for _ in &scene.walls {
            let id = handles.alloc();
            handles.walls.push(id);
        }
        for _ in &scene.solids {
            let id = handles.alloc();
            handles.solids.push(id);
        }
        for _ in &scene.lights {
            let id = handles.alloc();
            handles.lights.push(id);
        }
        handles
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }
}

fn pose(camera: &Camera) -> (Vec3, u32, u32) {
    (
        camera.position,
        camera.yaw.to_bits(),
        camera.pitch.to_bits(),
    )
}

fn document(inner: &Inner) -> Value {
    let look = look_direction(inner.camera.yaw, inner.camera.pitch);
    json::object([
        ("floor", floor_value(&inner.scene.floor)),
        (
            "walls",
            json::array(
                inner
                    .scene
                    .walls
                    .iter()
                    .zip(&inner.handles.walls)
                    .map(|(wall, id)| wall_value(*id, wall))
                    .collect(),
            ),
        ),
        (
            "solids",
            json::array(
                inner
                    .scene
                    .solids
                    .iter()
                    .zip(&inner.handles.solids)
                    .map(|(solid, id)| solid_value(*id, solid))
                    .collect(),
            ),
        ),
        (
            "lights",
            json::array(
                inner
                    .scene
                    .lights
                    .iter()
                    .zip(&inner.handles.lights)
                    .map(|(light, id)| light_value(*id, light))
                    .collect(),
            ),
        ),
        (
            "camera",
            json::object([
                ("position", vec3_value(inner.camera.position)),
                ("yaw", json::float(inner.camera.yaw as f64)),
                ("pitch", json::float(inner.camera.pitch as f64)),
                ("look", vec3_value(look)),
            ]),
        ),
    ])
}

fn floor_value(floor: &Floor) -> Value {
    json::object([
        ("handle", json::string("floor")),
        ("position", vec3_value(floor.position)),
        ("half_x", json::float(floor.half_x as f64)),
        ("half_z", json::float(floor.half_z as f64)),
        ("color", color_value(floor.color)),
    ])
}

fn wall_value(id: u64, wall: &Wall) -> Value {
    json::object([
        ("handle", json::string(format!("wall:{id}"))),
        ("position", vec3_value(wall.position)),
        ("half_x", json::float(wall.half_x as f64)),
        ("half_z", json::float(wall.half_z as f64)),
        ("height", json::float(wall.height as f64)),
        ("color", color_value(wall.color)),
        ("absorption", json::float(wall.absorption as f64)),
    ])
}

fn solid_value(id: u64, solid: &Solid) -> Value {
    json::object([
        ("handle", json::string(format!("solid:{id}"))),
        ("shape", json::string(shape_name(solid.shape))),
        ("position", vec3_value(solid.position)),
        ("size", json::float(solid.size as f64)),
        ("height", json::float(solid.height as f64)),
        ("color", color_value(solid.color)),
        ("absorption", json::float(solid.absorption as f64)),
    ])
}

fn light_value(id: u64, light: &Light) -> Value {
    json::object([
        ("handle", json::string(format!("light:{id}"))),
        ("position", vec3_value(light.position)),
        ("color", color_value(light.color)),
    ])
}

fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Square => "square",
        Shape::Circle => "circle",
    }
}

fn vec3_value(value: Vec3) -> Value {
    json::array(vec![
        json::float(value.x as f64),
        json::float(value.y as f64),
        json::float(value.z as f64),
    ])
}

fn color_value(color: [f32; 3]) -> Value {
    json::array(vec![
        json::float(color[0] as f64),
        json::float(color[1] as f64),
        json::float(color[2] as f64),
    ])
}

fn set_object(inner: &mut Inner, arguments: &Value) -> Result<(), String> {
    let handle = required_str(arguments, "handle")?;
    let patch = parse_patch(&without(arguments, &["handle"])?)?;
    let slot = find(inner, handle).ok_or_else(|| format!("unknown handle {handle}"))?;
    reject_unused(slot.kind(), &patch)?;
    let before = inner.scene.clone();
    if let Slot::Solid(index) = slot {
        if patch.position.is_some()
            || patch.shape.is_some()
            || patch.size.is_some()
            || patch.height.is_some()
        {
            inner.camera.release_codimation(index);
        }
    }
    apply_slot(inner, slot, &patch);
    if inner.scene == before {
        return Ok(());
    }
    if geometry_changed(&before, &inner.scene) {
        inner.camera.attach_scene(&inner.scene);
    }
    touch_scene(inner);
    Ok(())
}

fn spawn_object(inner: &mut Inner, arguments: &Value) -> Result<String, String> {
    let kind = required_str(arguments, "kind")?;
    let patch = parse_patch(&without(arguments, &["kind"])?)?;
    let handle = match kind {
        "wall" => {
            reject_unused(Kind::Wall, &patch)?;
            let mut wall = Wall {
                base: 0.0,
                position: Vec3::ZERO,
                half_x: 0.5,
                half_z: 0.5,
                height: 2.6,
                color: [1.0, 1.0, 1.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            };
            apply_wall(&mut wall, &patch);
            let id = inner.handles.alloc();
            inner.scene.walls.push(wall);
            inner.handles.walls.push(id);
            inner.camera.attach_scene(&inner.scene);
            format!("wall:{id}")
        }
        "solid" => {
            reject_unused(Kind::Solid, &patch)?;
            let mut solid = Solid {
                yaw: 0.0,
                shape: Shape::Square,
                position: Vec3::ZERO,
                size: 1.0,
                height: 1.2,
                color: [1.0, 1.0, 1.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            };
            apply_solid(&mut solid, &patch);
            let id = inner.handles.alloc();
            inner.scene.solids.push(solid);
            inner.handles.solids.push(id);
            inner.camera.attach_scene(&inner.scene);
            format!("solid:{id}")
        }
        "light" => {
            reject_unused(Kind::Light, &patch)?;
            let mut light = Light {
                position: Vec3::new(0.0, 2.0, 0.0),
                color: [1.0, 1.0, 1.0],

                direction: Vec3::ZERO,
            };
            apply_light(&mut light, &patch);
            let id = inner.handles.alloc();
            inner.scene.lights.push(light);
            inner.handles.lights.push(id);
            format!("light:{id}")
        }
        _ => return Err("kind must be wall, solid, or light".into()),
    };
    touch_scene(inner);
    Ok(handle)
}

fn remove_object(inner: &mut Inner, arguments: &Value) -> Result<(), String> {
    require_only(arguments, "handle")?;
    let handle = required_str(arguments, "handle")?;
    let slot = find(inner, handle).ok_or_else(|| format!("unknown handle {handle}"))?;
    match slot {
        Slot::Floor => return Err("the floor stays".into()),
        Slot::Wall(index) => {
            inner.scene.walls.remove(index);
            inner.handles.walls.remove(index);
        }
        Slot::Solid(index) => {
            inner.camera.solid_removed(index);
            inner.scene.solids.remove(index);
            inner.handles.solids.remove(index);
        }
        Slot::Light(index) => {
            inner.scene.lights.remove(index);
            inner.handles.lights.remove(index);
        }
    }
    if !matches!(slot, Slot::Light(_)) {
        inner.camera.attach_scene(&inner.scene);
    }
    touch_scene(inner);
    Ok(())
}

fn set_camera(inner: &mut Inner, arguments: &Value) -> Result<(), String> {
    let patch = parse_patch(arguments)?;
    reject_camera_only(&patch)?;
    let mut position = inner.camera.position;
    let mut yaw = inner.camera.yaw;
    let mut pitch = inner.camera.pitch;
    if let Some(look) = patch.look {
        let (next_yaw, next_pitch) = yaw_pitch(look);
        yaw = next_yaw;
        pitch = next_pitch;
    }
    if let Some(value) = patch.yaw {
        yaw = value;
    }
    if let Some(value) = patch.pitch {
        pitch = value;
    }
    if let Some(value) = patch.position {
        position = value;
    }
    pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
    if position == inner.camera.position && yaw == inner.camera.yaw && pitch == inner.camera.pitch {
        return Ok(());
    }
    inner.camera.set_pose(position, yaw, pitch);
    inner.published = inner.published.wrapping_add(1);
    Ok(())
}

fn touch_scene(inner: &mut Inner) {
    inner.scene_revision = inner.scene_revision.wrapping_add(1);
    inner.published = inner.published.wrapping_add(1);
}

fn yaw_pitch(look: Vec3) -> (f32, f32) {
    let length = look.length();
    if length < 1.0e-8 {
        return (0.0, 0.0);
    }
    let dir = look / length;
    let pitch = dir.y.clamp(-1.0, 1.0).asin();
    let yaw = dir.x.atan2(-dir.z);
    (yaw, pitch)
}

fn find(inner: &Inner, handle: &str) -> Option<Slot> {
    if handle == "floor" {
        return Some(Slot::Floor);
    }
    let (kind, id) = handle.split_once(':')?;
    let id: u64 = id.parse().ok()?;
    match kind {
        "wall" => inner
            .handles
            .walls
            .iter()
            .position(|item| *item == id)
            .map(Slot::Wall),
        "solid" => inner
            .handles
            .solids
            .iter()
            .position(|item| *item == id)
            .map(Slot::Solid),
        "light" => inner
            .handles
            .lights
            .iter()
            .position(|item| *item == id)
            .map(Slot::Light),
        _ => None,
    }
}

impl Slot {
    fn kind(self) -> Kind {
        match self {
            Slot::Floor => Kind::Floor,
            Slot::Wall(_) => Kind::Wall,
            Slot::Solid(_) => Kind::Solid,
            Slot::Light(_) => Kind::Light,
        }
    }
}

fn apply_slot(inner: &mut Inner, slot: Slot, patch: &Patch) {
    match slot {
        Slot::Floor => apply_floor(&mut inner.scene.floor, patch),
        Slot::Wall(index) => apply_wall(&mut inner.scene.walls[index], patch),
        Slot::Solid(index) => apply_solid(&mut inner.scene.solids[index], patch),
        Slot::Light(index) => apply_light(&mut inner.scene.lights[index], patch),
    }
}

fn apply_floor(floor: &mut Floor, patch: &Patch) {
    if let Some(position) = patch.position {
        floor.position = position;
    }
    if let Some(half_x) = patch.half_x {
        floor.half_x = half_x;
    }
    if let Some(half_z) = patch.half_z {
        floor.half_z = half_z;
    }
    if let Some(color) = patch.color {
        floor.color = color;
    }
}

fn apply_wall(wall: &mut Wall, patch: &Patch) {
    if let Some(position) = patch.position {
        wall.position = position;
    }
    if let Some(half_x) = patch.half_x {
        wall.half_x = half_x;
    }
    if let Some(half_z) = patch.half_z {
        wall.half_z = half_z;
    }
    if let Some(height) = patch.height {
        wall.height = height;
    }
    if let Some(color) = patch.color {
        wall.color = color;
    }
    if let Some(absorption) = patch.absorption {
        wall.absorption = absorption;
    }
}

fn apply_solid(solid: &mut Solid, patch: &Patch) {
    if let Some(position) = patch.position {
        solid.position = position;
    }
    if let Some(size) = patch.size {
        solid.size = size;
    }
    if let Some(height) = patch.height {
        solid.height = height;
    }
    if let Some(shape) = patch.shape {
        solid.shape = shape;
    }
    if let Some(color) = patch.color {
        solid.color = color;
    }
    if let Some(absorption) = patch.absorption {
        solid.absorption = absorption;
    }
}

fn apply_light(light: &mut Light, patch: &Patch) {
    if let Some(position) = patch.position {
        light.position = position;
    }
    if let Some(color) = patch.color {
        light.color = color;
    }
}

fn geometry_changed(before: &Scene, after: &Scene) -> bool {
    if before.floor.position.x != after.floor.position.x
        || before.floor.position.z != after.floor.position.z
        || before.floor.half_x != after.floor.half_x
        || before.floor.half_z != after.floor.half_z
    {
        return true;
    }
    if before.walls.len() != after.walls.len() || before.solids.len() != after.solids.len() {
        return true;
    }
    for (left, right) in before.walls.iter().zip(&after.walls) {
        if left.position.x != right.position.x
            || left.position.z != right.position.z
            || left.half_x != right.half_x
            || left.half_z != right.half_z
            || left.height != right.height
        {
            return true;
        }
    }
    for (left, right) in before.solids.iter().zip(&after.solids) {
        if left.shape != right.shape
            || left.position.x != right.position.x
            || left.position.y != right.position.y
            || left.position.z != right.position.z
            || left.size != right.size
            || left.height != right.height
        {
            return true;
        }
    }
    false
}

fn reject_unused(kind: Kind, patch: &Patch) -> Result<(), String> {
    let mut bad = Vec::new();
    let (half, height, size, shape, absorption, pose) = match kind {
        Kind::Floor => (true, false, false, false, false, false),
        Kind::Wall => (true, true, false, false, true, false),
        Kind::Solid => (false, true, true, true, true, false),
        Kind::Light => (false, false, false, false, false, false),
    };
    if !half && (patch.half_x.is_some() || patch.half_z.is_some()) {
        bad.push("half_x");
    }
    if !height && patch.height.is_some() {
        bad.push("height");
    }
    if !size && patch.size.is_some() {
        bad.push("size");
    }
    if !shape && patch.shape.is_some() {
        bad.push("shape");
    }
    if !absorption && patch.absorption.is_some() {
        bad.push("absorption");
    }
    if !pose && (patch.yaw.is_some() || patch.pitch.is_some() || patch.look.is_some()) {
        bad.push("yaw");
    }
    if kind == Kind::Light && (patch.half_x.is_some() || patch.half_z.is_some()) {
        // already recorded
    }
    if !bad.is_empty() {
        return Err(format!("this object has no {}", bad.join(" ")));
    }
    Ok(())
}

fn reject_camera_only(patch: &Patch) -> Result<(), String> {
    if patch.half_x.is_some()
        || patch.half_z.is_some()
        || patch.height.is_some()
        || patch.size.is_some()
        || patch.shape.is_some()
        || patch.color.is_some()
        || patch.absorption.is_some()
    {
        return Err("set_camera takes position, yaw, pitch, or look".into());
    }
    Ok(())
}

fn parse_patch(value: &Value) -> Result<Patch, String> {
    let Some(pairs) = value.as_object_pairs() else {
        return Err("arguments must be an object".into());
    };
    let mut patch = Patch::default();
    for (key, item) in pairs {
        match key.as_str() {
            "position" => patch.position = Some(parse_vec3(item)?),
            "look" => patch.look = Some(parse_vec3(item)?),
            "half_x" => patch.half_x = Some(parse_non_negative(item, "half_x")?),
            "half_z" => patch.half_z = Some(parse_non_negative(item, "half_z")?),
            "height" => patch.height = Some(parse_non_negative(item, "height")?),
            "size" => patch.size = Some(parse_non_negative(item, "size")?),
            "yaw" => patch.yaw = Some(parse_finite(item, "yaw")?),
            "pitch" => patch.pitch = Some(parse_finite(item, "pitch")?),
            "absorption" => patch.absorption = Some(parse_finite(item, "absorption")?),
            "shape" => {
                let name = item.as_str().ok_or("shape must be a string")?;
                patch.shape = Some(match name {
                    "square" => Shape::Square,
                    "circle" => Shape::Circle,
                    _ => return Err("shape must be square or circle".into()),
                });
            }
            "color" => patch.color = Some(parse_color(item)?),
            other => return Err(format!("unknown field {other}")),
        }
    }
    Ok(patch)
}

fn parse_vec3(value: &Value) -> Result<Vec3, String> {
    let Some(items) = value.as_array() else {
        return Err("position must be three numbers".into());
    };
    if items.len() != 3 {
        return Err("position must be three numbers".into());
    }
    Ok(Vec3::new(
        parse_finite(&items[0], "x")?,
        parse_finite(&items[1], "y")?,
        parse_finite(&items[2], "z")?,
    ))
}

fn parse_color(value: &Value) -> Result<[f32; 3], String> {
    let Some(items) = value.as_array() else {
        return Err("color must be three numbers".into());
    };
    if items.len() != 3 {
        return Err("color must be three numbers".into());
    }
    Ok([
        parse_finite(&items[0], "color")?,
        parse_finite(&items[1], "color")?,
        parse_finite(&items[2], "color")?,
    ])
}

fn parse_finite(value: &Value, name: &str) -> Result<f32, String> {
    let number = value
        .as_f64()
        .ok_or_else(|| format!("{name} must be a number"))? as f32;
    if !number.is_finite() {
        return Err(format!("{name} must be a finite number"));
    }
    Ok(number)
}

fn parse_non_negative(value: &Value, name: &str) -> Result<f32, String> {
    let number = parse_finite(value, name)?;
    if number < 0.0 {
        return Err(format!("{name} must be zero or greater"));
    }
    Ok(number)
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn without(value: &Value, skip: &[&str]) -> Result<Value, String> {
    let Some(pairs) = value.as_object_pairs() else {
        return Err("arguments must be an object".into());
    };
    let mut kept = Vec::new();
    for (key, item) in pairs {
        if skip.contains(&key.as_str()) {
            continue;
        }
        kept.push((key.clone(), item.clone()));
    }
    Ok(Value::Object(kept))
}

fn require_only(value: &Value, key: &str) -> Result<(), String> {
    let Some(pairs) = value.as_object_pairs() else {
        return Err("arguments must be an object".into());
    };
    for (name, _) in pairs {
        if name != key {
            return Err(format!("unknown field {name}"));
        }
    }
    Ok(())
}

trait ObjectPairs {
    fn as_object_pairs(&self) -> Option<&[(String, Value)]>;
}

impl ObjectPairs for Value {
    fn as_object_pairs(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(pairs) => Some(pairs),
            _ => None,
        }
    }
}
