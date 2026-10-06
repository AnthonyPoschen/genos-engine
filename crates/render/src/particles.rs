//! Seeded particles and the smoke a fire gives off.
//!
//! The step does not touch Vulkan. An emitter holds the options the step and the
//! card builder read. Fire emission is one light in the ground field. A card with
//! density is one fog card. A card without density is one quad per point.

use crate::mesh;
use crate::world::{Bounds, DrawKind, Object, ParticleFrame, ParticleImage, World};
use genos_load::{MemorySource, Texture, TextureMap};
use genos_scene::Scene;

/// Steps in the camera proof. Two cold runs of this many steps match.
pub const PROOF_STEPS: u32 = 120;
pub const STEP_DT: f32 = 1.0 / 60.0;

const MAX_SLOTS: usize = 40;
const SEED: u32 = 0xA341_316C;

/// XZ falloff of the fog in-scatter fire term. `shaders/scene.frag` uses the same number.
pub const FIRE_XZ_FALLOFF: f32 = 0.65;
/// Vertical falloff of the CPU fire helper. The GPU flame is a cascade lamp.
pub const FIRE_Y_FALLOFF: f32 = 0.08;
/// A small fire lights a few meters of floor when the scene lamp is off.
pub const FIRE_STRENGTH: f32 = 1.7;
pub const FIRE_COLOR: [f32; 3] = [1.0, 0.36, 0.06];
/// Extinction coefficient. `shaders/scene.frag` uses this density on each puff.
pub const SMOKE_DENSITY: f32 = 1.7;
pub const SMOKE_RADIUS: f32 = 0.42;
/// Gray single-scatter albedo. `shaders/scene.frag` uses the same value.
pub const SMOKE_ALBEDO: f32 = 0.62;
pub const FIELD_SCATTER: f32 = 0.35;
pub const LAMP_FALLOFF: f32 = 0.08;
const MAX_PUFFS: usize = 8;

const EMBER_PNG: &[u8] = include_bytes!("../assets/ember.png");
const EMBER_MAP: &[u8] = include_bytes!("../assets/ember.map");
const SMOKE_PNG: &[u8] = include_bytes!("../assets/smoke.png");
const SMOKE_MAP: &[u8] = include_bytes!("../assets/smoke.map");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Stationary fire. The age advances the texture-map frame. The position stays.
    Flame,
    Fire,
    Smoke,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveParticle {
    pub kind: Kind,
    pub position: [f32; 3],
    pub age: f32,
    pub life: f32,
}

/// Options the step reads at birth and the card builder reads at draw time.
///
/// Birth uses `origin`, `spawn_radius`, `rise`, `drift`, `life`, `interval`, and
/// `capacity`. The card uses `albedo`, `emission`, `lit`, `face_camera`, `angle`,
/// `size`, and `image`. One option does not change the other group's result.
#[derive(Clone, Debug)]
pub struct Emitter {
    pub enabled: bool,
    pub origin: [f32; 3],
    pub spawn_radius: f32,
    pub face_camera: bool,
    pub angle: f32,
    pub lit: bool,
    pub albedo: [f32; 3],
    pub emission: [f32; 3],
    pub size: f32,
    pub life: f32,
    pub rise: f32,
    pub drift: f32,
    pub interval: f32,
    pub capacity: usize,
    pub image: Option<ParticleImage>,
}

impl Emitter {
    pub fn at(origin: [f32; 3]) -> Self {
        Self {
            enabled: true,
            origin,
            spawn_radius: 0.0,
            face_camera: true,
            angle: 0.0,
            lit: false,
            albedo: [1.0, 1.0, 1.0],
            emission: [0.0, 0.0, 0.0],
            size: 0.2,
            life: 1.0,
            rise: 0.0,
            drift: 0.0,
            interval: 0.1,
            capacity: 8,
            image: None,
        }
    }
}

/// One particle card. `normal` points at the eye when the card faces the camera.
#[derive(Clone, Copy, Debug)]
pub struct Card {
    pub normal: [f32; 3],
    pub corners: [[f32; 3]; 4],
    /// Frame-local coordinates. `(0, 0)` is the top-left of the frame. Negative means no image.
    pub uv: [[f32; 2]; 4],
    pub albedo: [f32; 3],
    pub lit: bool,
    pub frame: ParticleFrame,
    pub textured: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireLight {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub strength: f32,
}

impl FireLight {
    pub fn off() -> Self {
        Self {
            position: [0.0; 3],
            color: [0.0; 3],
            strength: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puff {
    pub center: [f32; 3],
    pub radius: f32,
    pub density: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct FogLamp {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogHit {
    pub optical: f32,
    pub transmittance: f32,
    pub inscatter: [f32; 3],
}

struct Slot {
    alive: bool,
    kind: Kind,
    position: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    life: f32,
}

impl Slot {
    fn empty() -> Self {
        Self {
            alive: false,
            kind: Kind::Fire,
            position: [0.0; 3],
            velocity: [0.0; 3],
            age: 0.0,
            life: 0.0,
        }
    }
}

pub struct Simulation {
    ember: Emitter,
    smoke: Option<Emitter>,
    /// Stationary fire. `None` for a fresh emitter that has no base flame.
    flame: Option<Emitter>,
    slots: [Slot; MAX_SLOTS],
    rng: u32,
    emit_left: f32,
    born: u32,
    retired: u32,
}

impl Simulation {
    /// One emitter and no smoke trail.
    pub fn fresh(emitter: Emitter) -> Self {
        Self {
            ember: emitter,
            smoke: None,
            flame: None,
            slots: std::array::from_fn(|_| Slot::empty()),
            rng: SEED,
            emit_left: 0.0,
            born: 0,
            retired: 0,
        }
    }

    /// Embers on the top of the red solid, plus the smoke those embers leave.
    /// No red solid keeps the simulation empty.
    pub fn from_scene(scene: &Scene) -> Self {
        let Some(solid) = scene.solid_by_color([1.0, 0.0, 0.0]) else {
            let mut emitter = Emitter::at([0.0, 0.0, 0.0]);
            emitter.enabled = false;
            return Self::fresh(emitter);
        };
        let flame_size = 1.2;
        // The card is centered on this point, so the light sits in the flame, not on the box top.
        let origin = [
            solid.position.x,
            solid.height + flame_size * 0.5,
            solid.position.z,
        ];
        let mut flame = Emitter::at(origin);
        flame.face_camera = true;
        flame.lit = false;
        flame.albedo = [1.0, 0.78, 0.32];
        flame.emission = FIRE_COLOR;
        flame.size = flame_size;
        flame.rise = 0.0;
        flame.drift = 0.0;
        flame.life = 1.0e6;
        flame.capacity = 3;
        flame.image = bundled_image(EMBER_PNG, EMBER_MAP, "burn");
        let mut ember = Emitter::at(origin);
        ember.spawn_radius = (solid.size * 0.16).max(0.05);
        ember.face_camera = true;
        ember.lit = false;
        ember.albedo = [1.0, 0.55, 0.12];
        ember.emission = [0.0, 0.0, 0.0];
        ember.size = 0.42;
        ember.life = 0.9;
        ember.rise = 1.55;
        ember.drift = 0.22;
        ember.interval = 0.06;
        ember.capacity = 14;
        ember.image = bundled_image(EMBER_PNG, EMBER_MAP, "burn");
        let mut smoke = Emitter::at(origin);
        smoke.lit = true;
        smoke.face_camera = true;
        smoke.albedo = [1.0, 1.0, 1.0];
        smoke.emission = [0.0, 0.0, 0.0];
        smoke.size = 1.05;
        smoke.life = 2.2;
        smoke.rise = 0.55;
        smoke.capacity = 18;
        smoke.image = bundled_image(SMOKE_PNG, SMOKE_MAP, "trail");
        let mut sim = Self {
            ember,
            smoke: Some(smoke),
            flame: Some(flame),
            slots: std::array::from_fn(|_| Slot::empty()),
            rng: SEED,
            emit_left: 0.0,
            born: 0,
            retired: 0,
        };
        sim.place_flames();
        sim
    }

    pub fn born(&self) -> u32 {
        self.born
    }

    pub fn retired(&self) -> u32 {
        self.retired
    }

    pub fn live(&self) -> Vec<LiveParticle> {
        self.slots
            .iter()
            .filter(|slot| slot.alive)
            .map(|slot| LiveParticle {
                kind: slot.kind,
                position: slot.position,
                age: slot.age,
                life: slot.life,
            })
            .collect()
    }

    /// Move, retire, and emit. `dt` is seconds. The same seed and the same calls match.
    pub fn step(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        let mut dying = [0usize; MAX_SLOTS];
        let mut dying_count = 0;
        for index in 0..MAX_SLOTS {
            if !self.slots[index].alive {
                continue;
            }
            self.slots[index].age += dt;
            for axis in 0..3 {
                self.slots[index].position[axis] += self.slots[index].velocity[axis] * dt;
            }
            let kind = self.slots[index].kind;
            let age = self.slots[index].age;
            let life = self.slots[index].life;
            if kind == Kind::Flame {
                continue;
            } else if kind == Kind::Fire && age >= life {
                dying[dying_count] = index;
                dying_count += 1;
            } else if kind == Kind::Smoke && age >= life {
                self.retire(index);
            }
        }
        for slot in dying.iter().take(dying_count) {
            self.spawn_smoke_from(*slot);
        }
        if !self.ember.enabled || self.ember.capacity == 0 {
            return;
        }
        self.emit_left -= dt;
        let mut guard = 0;
        let limit = self.ember.capacity.min(MAX_SLOTS);
        while self.emit_left <= 0.0 && guard < limit {
            guard += 1;
            if !self.spawn_fire() {
                break;
            }
            self.emit_left += self.ember.interval.max(1.0e-4);
        }
    }

    /// Replace particle draws. `emission` feeds the field. `smoke` keeps the trail.
    pub fn apply(&self, world: &mut World, emission: bool, smoke: bool) {
        world
            .objects
            .retain(|object| !matches!(object.kind, DrawKind::Particles { .. }));
        self.push_kind(world, Kind::Flame, emission);
        self.push_kind(world, Kind::Fire, emission);
        if smoke {
            self.push_kind(world, Kind::Smoke, false);
        }
    }

    /// One camera frame: step, then submit the embers and the trail.
    pub fn advance(&mut self, world: &mut World, dt: f32) {
        self.step(dt);
        self.apply(world, true, true);
    }

    fn push_kind(&self, world: &mut World, kind: Kind, emission_on: bool) {
        let emitter = match kind {
            Kind::Flame => match &self.flame {
                Some(emitter) => emitter,
                None => return,
            },
            Kind::Fire => &self.ember,
            Kind::Smoke => match &self.smoke {
                Some(emitter) => emitter,
                None => return,
            },
        };
        let mut points = Vec::new();
        let mut ages = Vec::new();
        for slot in &self.slots {
            if slot.alive && slot.kind == kind {
                points.push(slot.position);
                ages.push(slot.age);
            }
        }
        if points.is_empty() {
            return;
        }
        let emission = if emission_on {
            emitter.emission
        } else {
            [0.0, 0.0, 0.0]
        };
        world.objects.push(Object {
            hidden: false,
            affects_light: false,
            bounds: bounds_of(&points, emitter.size),
            kind: DrawKind::Particles {
                points,
                ages,
                color: emitter.albedo,
                size: emitter.size,
                emission,
                density: 0.0,
                lit: emitter.lit,
                face_camera: emitter.face_camera,
                angle: emitter.angle,
                image: emitter.image.clone(),
            },
        });
    }

    fn place_flames(&mut self) {
        let Some(flame) = self.flame.as_ref() else {
            return;
        };
        let origin = flame.origin;
        let life = flame.life.max(1.0);
        let offsets = [[0.0, 0.0], [0.1, 0.04], [-0.08, -0.06]];
        let phases = [0.0_f32, 0.18, 0.34];
        for index in 0..offsets.len() {
            let Some(slot) = self.free_slot() else {
                return;
            };
            self.slots[slot] = Slot {
                alive: true,
                kind: Kind::Flame,
                position: [
                    origin[0] + offsets[index][0],
                    origin[1],
                    origin[2] + offsets[index][1],
                ],
                velocity: [0.0, 0.0, 0.0],
                age: phases[index],
                life,
            };
            self.born += 1;
        }
    }

    fn count(&self, kind: Kind) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.alive && slot.kind == kind)
            .count()
    }

    fn spawn_fire(&mut self) -> bool {
        if self.count(Kind::Fire) >= self.ember.capacity {
            return false;
        }
        let Some(index) = self.free_slot() else {
            return false;
        };
        let (ox, oz) = disk(&mut self.rng, self.ember.spawn_radius);
        let drift = self.ember.drift;
        let vx = (unit(&mut self.rng) - 0.5) * 2.0 * drift;
        let vz = (unit(&mut self.rng) - 0.5) * 2.0 * drift;
        self.slots[index] = Slot {
            alive: true,
            kind: Kind::Fire,
            position: [
                self.ember.origin[0] + ox,
                self.ember.origin[1],
                self.ember.origin[2] + oz,
            ],
            velocity: [vx, self.ember.rise, vz],
            age: 0.0,
            life: self.ember.life.max(1.0e-3),
        };
        self.born += 1;
        true
    }

    fn spawn_smoke_from(&mut self, fire_index: usize) {
        let position = self.slots[fire_index].position;
        let parent_v = self.slots[fire_index].velocity;
        self.retire(fire_index);
        let Some(smoke) = self.smoke.as_ref() else {
            return;
        };
        let enabled = smoke.enabled;
        let capacity = smoke.capacity;
        let rise = smoke.rise;
        let life = smoke.life.max(1.0e-3);
        if !enabled || self.count(Kind::Smoke) >= capacity {
            return;
        }
        let Some(index) = self.free_slot() else {
            return;
        };
        self.slots[index] = Slot {
            alive: true,
            kind: Kind::Smoke,
            position,
            velocity: [parent_v[0] * 0.35, rise, parent_v[2] * 0.35],
            age: 0.0,
            life,
        };
        self.born += 1;
    }

    fn retire(&mut self, index: usize) {
        if self.slots[index].alive {
            self.slots[index].alive = false;
            self.retired += 1;
        }
    }

    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|slot| !slot.alive)
    }
}

/// Copy one loaded texture into particle content. The whole image is one frame.
pub fn image_from_texture(texture: &Texture) -> ParticleImage {
    ParticleImage {
        width: texture.image.width,
        height: texture.image.height,
        pixels: texture.image.pixels.clone(),
        frames: vec![ParticleFrame {
            x: 0,
            y: 0,
            width: texture.image.width,
            height: texture.image.height,
        }],
        frame_seconds: 1.0e6,
    }
}

/// Copy one clip from a loaded texture map. `None` means the clip name is absent.
pub fn image_from_map(map: &TextureMap, clip: &str) -> Option<ParticleImage> {
    let clip = map.clips.iter().find(|item| item.name == clip)?;
    if clip.frames.is_empty() || !(clip.frame_seconds > 0.0) {
        return None;
    }
    let mut frames = Vec::new();
    for name in &clip.frames {
        let frame = map.frames.iter().find(|frame| frame.name == *name)?;
        frames.push(ParticleFrame {
            x: frame.rect.x,
            y: frame.rect.y,
            width: frame.rect.width,
            height: frame.rect.height,
        });
    }
    Some(ParticleImage {
        width: map.image.width,
        height: map.image.height,
        pixels: map.image.pixels.clone(),
        frames,
        frame_seconds: clip.frame_seconds,
    })
}

/// Frame rectangle at `age` seconds. The index matches `TextureMap::sample`.
pub fn frame_at(image: &ParticleImage, age: f32) -> ParticleFrame {
    if image.frames.is_empty() {
        return ParticleFrame {
            x: 0,
            y: 0,
            width: image.width,
            height: image.height,
        };
    }
    if !(image.frame_seconds > 0.0) {
        return image.frames[0];
    }
    let ticks = if age.is_finite() && age > 0.0 {
        age / image.frame_seconds
    } else {
        0.0
    };
    if !ticks.is_finite() {
        return image.frames[0];
    }
    let index = (ticks.floor() as u64) % (image.frames.len() as u64);
    image.frames[index as usize]
}

/// Card corners and the normal. A camera-facing normal points at `eye`.
pub fn card_quad(
    center: [f32; 3],
    eye: [f32; 3],
    size: f32,
    face_camera: bool,
    angle: f32,
) -> ([f32; 3], [[f32; 3]; 4]) {
    let (right, up, normal) = card_basis(center, eye, face_camera, angle);
    let half = size.max(0.02) * 0.5;
    let corner = |rx: f32, uy: f32| {
        [
            center[0] + right[0] * rx + up[0] * uy,
            center[1] + right[1] * rx + up[1] * uy,
            center[2] + right[2] * rx + up[2] * uy,
        ]
    };
    (
        normal,
        [
            corner(-half, -half),
            corner(half, -half),
            corner(half, half),
            corner(-half, half),
        ],
    )
}

/// Cards for one object. Fog density does not build a card here.
pub fn object_cards(object: &Object, eye: [f32; 3]) -> Vec<Card> {
    let DrawKind::Particles {
        points,
        ages,
        color,
        size,
        density,
        lit,
        face_camera,
        angle,
        image,
        ..
    } = &object.kind
    else {
        return Vec::new();
    };
    if *density > 0.0 {
        return Vec::new();
    }
    let mut cards = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let age = ages.get(index).copied().unwrap_or(0.0);
        let (normal, corners) = card_quad(*point, eye, *size, *face_camera, *angle);
        let (frame, textured, uv) = match image {
            Some(image) => (
                frame_at(image, age),
                true,
                [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            ),
            None => (
                ParticleFrame {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                false,
                [[-1.0, -1.0]; 4],
            ),
        };
        cards.push(Card {
            normal,
            corners,
            uv,
            albedo: *color,
            lit: *lit,
            frame,
            textured,
        });
    }
    cards
}

/// `albedo * (direct + bounce)` with the same gain the fragment shader uses.
/// Albedo zero stays black. The result is not a built-in flame color.
pub fn shade_lit(albedo: [f32; 3], direct: f32, bounce: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let level = normal[1].abs() > 0.5;
    mesh::compose(albedo, direct, bounce, level)
}

fn bundled_image(png: &[u8], map: &[u8], clip: &str) -> Option<ParticleImage> {
    let decoded =
        genos_load::load_texture_map(&MemorySource::new(png), &MemorySource::new(map)).ok()?;
    image_from_map(&decoded, clip)
}

fn card_basis(
    center: [f32; 3],
    eye: [f32; 3],
    face_camera: bool,
    angle: f32,
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let mut forward = [center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]];
    let flen = length3(forward).max(1.0e-4);
    forward = scale3(forward, 1.0 / flen);
    let mut right = [-forward[2], 0.0, forward[0]];
    let rlen = length3(right);
    if rlen < 1.0e-4 {
        right = [1.0, 0.0, 0.0];
    } else {
        right = scale3(right, 1.0 / rlen);
    }
    let mut up = cross(forward, right);
    if !face_camera {
        forward = yaw(forward, angle);
        right = yaw(right, angle);
        up = yaw(up, angle);
    }
    let normal = scale3(forward, -1.0);
    (right, up, normal)
}

fn yaw(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [c * v[0] + s * v[2], v[1], -s * v[0] + c * v[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn bounds_of(points: &[[f32; 3]], pad: f32) -> Bounds {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for point in points {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Bounds {
        center: [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ],
        half: [
            (max[0] - min[0]) * 0.5 + pad,
            (max[1] - min[1]) * 0.5 + pad,
            (max[2] - min[2]) * 0.5 + pad,
        ],
    }
}

fn disk(state: &mut u32, radius: f32) -> (f32, f32) {
    if radius <= 0.0 {
        return (0.0, 0.0);
    }
    let theta = unit(state) * std::f32::consts::TAU;
    let reach = radius * unit(state).sqrt();
    (reach * theta.cos(), reach * theta.sin())
}

/// Fire and fog the draw uploads. Emitting points share one fire term.
/// Puffs are nearest-first and capped.
pub fn medium(world: &World, eye: [f32; 3]) -> (FireLight, Vec<Puff>) {
    let mut sum_pos = [0.0; 3];
    let mut sum_color = [0.0; 3];
    let mut weight = 0.0;
    let mut puffs = Vec::new();
    for object in &world.objects {
        let DrawKind::Particles {
            points,
            emission,
            size,
            density,
            ..
        } = &object.kind
        else {
            continue;
        };
        let peak = emission[0].max(emission[1]).max(emission[2]);
        if peak > 0.0 {
            for point in points {
                for axis in 0..3 {
                    sum_pos[axis] += point[axis] * peak;
                    sum_color[axis] += emission[axis] * peak;
                }
                weight += peak;
            }
        }
        if *density > 0.0 {
            for point in points {
                puffs.push(Puff {
                    center: *point,
                    radius: size.max(0.05),
                    density: *density,
                });
            }
        }
    }
    let fire = if weight <= 0.0 {
        FireLight::off()
    } else {
        let color = [
            sum_color[0] / weight,
            sum_color[1] / weight,
            sum_color[2] / weight,
        ];
        let peak = color[0].max(color[1]).max(color[2]);
        FireLight {
            position: [
                sum_pos[0] / weight,
                sum_pos[1] / weight,
                sum_pos[2] / weight,
            ],
            color,
            strength: FIRE_STRENGTH * peak,
        }
    };
    puffs.sort_by(|a, b| {
        dist2(eye, a.center)
            .partial_cmp(&dist2(eye, b.center))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    puffs.truncate(MAX_PUFFS);
    (fire, puffs)
}

/// Colored fire at a ground probe. Zero strength leaves the probe unchanged.
pub fn fire_radiance(xz: [f32; 2], fire: FireLight) -> [f32; 3] {
    if fire.strength <= 0.0 {
        return [0.0; 3];
    }
    let dx = xz[0] - fire.position[0];
    let dz = xz[1] - fire.position[2];
    let dist2 = dx * dx + dz * dz;
    let dy = fire.position[1];
    let fall = fire.strength / (1.0 + dist2 * FIRE_XZ_FALLOFF + dy * dy * FIRE_Y_FALLOFF);
    [
        fire.color[0] * fall,
        fire.color[1] * fall,
        fire.color[2] * fall,
    ]
}

/// Optical depth and single-scatter light along one view ray.
///
/// `shaders/scene.frag` integrates the same chord, the same extinction, and the same
/// lamp, fire, and field terms. Empty puffs leave the ray clear.
pub fn fog_along(
    eye: [f32; 3],
    dir: [f32; 3],
    puffs: &[Puff],
    lamps: &[FogLamp],
    fire: FireLight,
    field: [f32; 3],
) -> FogHit {
    let mut dir = dir;
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1.0e-6 {
        return FogHit {
            optical: 0.0,
            transmittance: 1.0,
            inscatter: [0.0; 3],
        };
    }
    dir = [dir[0] / len, dir[1] / len, dir[2] / len];
    let mut order: Vec<usize> = (0..puffs.len()).collect();
    order.sort_by(|&a, &b| {
        let ta = ray_t(eye, dir, puffs[a].center);
        let tb = ray_t(eye, dir, puffs[b].center);
        ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut optical = 0.0;
    let mut trans = 1.0;
    let mut inscatter = [0.0; 3];
    for index in order {
        let puff = puffs[index];
        if puff.density <= 0.0 || puff.radius <= 0.0 {
            continue;
        }
        let chord = sphere_chord(eye, dir, puff.center, puff.radius);
        if chord <= 0.0 {
            continue;
        }
        let slice = puff.density * chord;
        let absorb = 1.0 - (-slice).exp();
        let mid = sphere_mid(eye, dir, puff.center);
        let light = scatter_light(mid, lamps, fire, field);
        for axis in 0..3 {
            inscatter[axis] += trans * absorb * light[axis] * SMOKE_ALBEDO;
        }
        trans *= (-slice).exp();
        optical += slice;
    }
    FogHit {
        optical,
        transmittance: trans,
        inscatter,
    }
}

/// `background * transmittance + inscatter`.
pub fn compose_fog(background: [f32; 3], fog: FogHit) -> [f32; 3] {
    [
        background[0] * fog.transmittance + fog.inscatter[0],
        background[1] * fog.transmittance + fog.inscatter[1],
        background[2] * fog.transmittance + fog.inscatter[2],
    ]
}

fn scatter_light(p: [f32; 3], lamps: &[FogLamp], fire: FireLight, field: [f32; 3]) -> [f32; 3] {
    let mut sum = [0.0; 3];
    for lamp in lamps {
        let dx = lamp.position[0] - p[0];
        let dy = lamp.position[1] - p[1];
        let dz = lamp.position[2] - p[2];
        let dist2 = dx * dx + dy * dy + dz * dz;
        let fall = 1.0 / (1.0 + dist2 * LAMP_FALLOFF);
        for axis in 0..3 {
            sum[axis] += lamp.color[axis] * fall;
        }
    }
    if fire.strength > 0.0 {
        let dx = fire.position[0] - p[0];
        let dy = fire.position[1] - p[1];
        let dz = fire.position[2] - p[2];
        let dist2 = dx * dx + dy * dy + dz * dz;
        let fall = fire.strength / (1.0 + dist2 * FIRE_XZ_FALLOFF);
        for axis in 0..3 {
            sum[axis] += fire.color[axis] * fall;
        }
    }
    for axis in 0..3 {
        sum[axis] += field[axis] * FIELD_SCATTER;
    }
    sum
}

fn sphere_chord(origin: [f32; 3], dir: [f32; 3], center: [f32; 3], radius: f32) -> f32 {
    let oc = [
        origin[0] - center[0],
        origin[1] - center[1],
        origin[2] - center[2],
    ];
    let b = oc[0] * dir[0] + oc[1] * dir[1] + oc[2] * dir[2];
    let c = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return 0.0;
    }
    let s = disc.sqrt();
    let mut t0 = -b - s;
    let t1 = -b + s;
    if t1 < 0.0 {
        return 0.0;
    }
    if t0 < 0.0 {
        t0 = 0.0;
    }
    (t1 - t0).max(0.0)
}

fn sphere_mid(origin: [f32; 3], dir: [f32; 3], center: [f32; 3]) -> [f32; 3] {
    let t = ray_t(origin, dir, center).max(0.0);
    [
        origin[0] + dir[0] * t,
        origin[1] + dir[1] * t,
        origin[2] + dir[2] * t,
    ]
}

fn ray_t(origin: [f32; 3], dir: [f32; 3], point: [f32; 3]) -> f32 {
    (point[0] - origin[0]) * dir[0]
        + (point[1] - origin[1]) * dir[1]
        + (point[2] - origin[2]) * dir[2]
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

fn length3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn scale3(v: [f32; 3], scale: f32) -> [f32; 3] {
    [v[0] * scale, v[1] * scale, v[2] * scale]
}

fn next_u32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

fn unit(state: &mut u32) -> f32 {
    (next_u32(state) >> 8) as f32 * (1.0 / 16_777_216.0)
}
