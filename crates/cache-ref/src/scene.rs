//! Test scenes: box rooms, point lamps, and the receiver pixels on every
//! surface that faces into the room.

use crate::geom::{fl, floor_usize, Aabb, Lamp, World, V3};

/// One axis-aligned surface, rasterised into square pixels.
#[derive(Clone, Debug)]
pub struct Patch {
    /// Axis of the normal (0 = x, 1 = y, 2 = z).
    pub axis: usize,
    /// Sign of the normal along `axis`.
    pub sign: f64,
    /// Position of the surface along `axis`.
    pub coord: f64,
    /// The two in-plane axes.
    pub u_axis: usize,
    pub v_axis: usize,
    /// Lower corner in the plane.
    pub u0: f64,
    pub v0: f64,
    /// Pixel counts.
    pub w: usize,
    pub h: usize,
    /// Index of this patch's first pixel in [`Scene::pixels`].
    pub first: usize,
}

/// A receiver pixel: centre, normal, and whether it is open to the room.
#[derive(Clone, Copy, Debug)]
pub struct Pixel {
    pub p: V3,
    pub n: V3,
    pub valid: bool,
    pub patch: usize,
}

/// What a scene does over time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timeline {
    /// Lamps always on.
    Static,
    /// Lamps on for frames `0..off_frame`, then off.
    LampOff { off_frame: usize },
    /// Lamps off for frames `0..on_frame`, then on (lag and fill timing).
    LampOn { on_frame: usize },
    /// Camera moves from `from` to `to` over `frames` frames, then holds.
    /// Camera held at `from` for `start` frames (cache warm-up), then moved
    /// to `to` over `frames` frames, then held.
    Dolly {
        from: V3,
        to: V3,
        start: usize,
        frames: usize,
    },
}

/// A scene: geometry, lamps, receivers, regions, timeline and camera.
#[derive(Clone, Debug)]
pub struct Scene {
    pub name: &'static str,
    pub world: World,
    pub interior: Aabb,
    pub patches: Vec<Patch>,
    pub pixels: Vec<Pixel>,
    pub pixel_size: f64,
    /// Pixels in the leak region (dark side of a thin wall), may be empty.
    pub leak: Vec<usize>,
    pub timeline: Timeline,
    pub camera: V3,
}

/// Names accepted by [`Scene::by_name`].
pub const SCENES: [&str; 7] = [
    "contact",
    "near-wall",
    "thin-wall",
    "lamp-off",
    "lamp-on",
    "shadow-edge",
    "dolly",
];

const W: f64 = 4.0;
const H: f64 = 2.6;
const T: f64 = 0.2;

fn room() -> (Vec<Aabb>, Aabb) {
    let interior = Aabb::new(V3::new(0.0, 0.0, 0.0), V3::new(W, H, W));
    let boxes = vec![
        Aabb::new(V3::new(-T, -T, -T), V3::new(W + T, 0.0, W + T)),
        Aabb::new(V3::new(-T, H, -T), V3::new(W + T, H + T, W + T)),
        Aabb::new(V3::new(-T, -T, -T), V3::new(0.0, H + T, W + T)),
        Aabb::new(V3::new(W, -T, -T), V3::new(W + T, H + T, W + T)),
        Aabb::new(V3::new(-T, -T, -T), V3::new(W + T, H + T, 0.0)),
        Aabb::new(V3::new(-T, -T, W), V3::new(W + T, H + T, W + T)),
    ];
    (boxes, interior)
}

fn lamp(x: f64, y: f64, z: f64, intensity: f64) -> Lamp {
    Lamp {
        pos: V3::new(x, y, z),
        intensity,
    }
}

impl Scene {
    /// Build a named scene with receiver pixels `pixel_size` metres wide.
    #[must_use]
    pub fn by_name(name: &str, pixel_size: f64) -> Option<Self> {
        let (mut boxes, interior) = room();
        let contact_box = Aabb::new(V3::new(1.2, 0.0, 1.2), V3::new(2.0, 0.6, 2.0));
        let mut timeline = Timeline::Static;
        let mut camera = V3::new(3.5, 1.6, 3.5);
        let (name, lamps): (&'static str, Vec<Lamp>) = match name {
            "contact" => {
                boxes.push(contact_box);
                ("contact", vec![lamp(2.9, 2.2, 2.9, 6.0)])
            }
            "near-wall" => ("near-wall", vec![lamp(0.3, 1.3, 2.0, 3.0)]),
            "thin-wall" => {
                // 5 cm wall at x = 2, a door at z in [2.8, 3.6], y < 2.0.
                let (x0, x1) = (1.975, 2.025);
                boxes.push(Aabb::new(V3::new(x0, 0.0, 0.0), V3::new(x1, H, 2.8)));
                boxes.push(Aabb::new(V3::new(x0, 0.0, 3.6), V3::new(x1, H, W)));
                boxes.push(Aabb::new(V3::new(x0, 2.0, 2.8), V3::new(x1, H, 3.6)));
                camera = V3::new(3.5, 1.6, 1.0);
                ("thin-wall", vec![lamp(1.0, 1.8, 1.4, 6.0)])
            }
            "lamp-off" => {
                boxes.push(contact_box);
                timeline = Timeline::LampOff { off_frame: 300 };
                ("lamp-off", vec![lamp(2.9, 2.2, 2.9, 6.0)])
            }
            "lamp-on" => {
                boxes.push(contact_box);
                timeline = Timeline::LampOn { on_frame: 300 };
                ("lamp-on", vec![lamp(2.9, 2.2, 2.9, 6.0)])
            }
            "shadow-edge" => {
                boxes.push(Aabb::new(V3::new(1.0, 1.2, 1.0), V3::new(2.0, 1.25, 3.0)));
                ("shadow-edge", vec![lamp(1.6, 2.4, 2.0, 6.0)])
            }
            "dolly" => {
                boxes.push(contact_box);
                timeline = Timeline::Dolly {
                    from: V3::new(3.8, 2.0, 3.8),
                    to: V3::new(2.3, 0.9, 2.3),
                    start: 160,
                    frames: 300,
                };
                camera = V3::new(3.8, 2.0, 3.8);
                ("dolly", vec![lamp(2.9, 2.2, 2.9, 6.0)])
            }
            _ => return None,
        };
        let world = World {
            boxes,
            lamps,
            albedo: 0.6,
        };
        let mut scene = Self {
            name,
            world,
            interior,
            patches: Vec::new(),
            pixels: Vec::new(),
            pixel_size,
            leak: Vec::new(),
            timeline,
            camera,
        };
        scene.rasterise();
        if name == "thin-wall" {
            // The dark face of the wall and the floor strip within 0.5 m of it.
            scene.leak = scene
                .pixels
                .iter()
                .enumerate()
                .filter(|(_, px)| {
                    px.valid
                        && px.p.z < 2.8
                        && ((px.n.x > 0.5 && (px.p.x - 2.025).abs() < 1e-6)
                            || (px.n.y > 0.5 && px.p.x > 2.025 && px.p.x < 2.525))
                })
                .map(|(i, _)| i)
                .collect();
        }
        Some(scene)
    }

    /// Lamps on at `frame`.
    #[must_use]
    pub fn lamps_on(&self, frame: usize) -> bool {
        match self.timeline {
            Timeline::LampOff { off_frame } => frame < off_frame,
            Timeline::LampOn { on_frame } => frame >= on_frame,
            Timeline::Static | Timeline::Dolly { .. } => true,
        }
    }

    /// Camera at `frame`.
    #[must_use]
    pub fn camera_at(&self, frame: usize) -> V3 {
        match self.timeline {
            Timeline::Dolly {
                from,
                to,
                start,
                frames,
            } => {
                let t = (fl(frame.saturating_sub(start)) / fl(frames.max(1))).min(1.0);
                from.add(to.sub(from).scale(t))
            }
            Timeline::Static | Timeline::LampOff { .. } | Timeline::LampOn { .. } => self.camera,
        }
    }

    fn rasterise(&mut self) {
        let lo = self.interior.min;
        let hi = self.interior.max;
        let eps = 1e-6;
        let boxes = self.world.boxes.clone();
        for b in &boxes {
            for axis in 0..3 {
                for (coord, sign) in [(b.min.axis(axis), -1.0), (b.max.axis(axis), 1.0)] {
                    if coord < lo.axis(axis) - eps || coord > hi.axis(axis) + eps {
                        continue;
                    }
                    if (coord - lo.axis(axis)).abs() < eps && sign < 0.0 {
                        continue;
                    }
                    if (coord - hi.axis(axis)).abs() < eps && sign > 0.0 {
                        continue;
                    }
                    let (ua, va) = ((axis + 1) % 3, (axis + 2) % 3);
                    let u0 = b.min.axis(ua).max(lo.axis(ua));
                    let u1 = b.max.axis(ua).min(hi.axis(ua));
                    let v0 = b.min.axis(va).max(lo.axis(va));
                    let v1 = b.max.axis(va).min(hi.axis(va));
                    if u1 - u0 < eps || v1 - v0 < eps {
                        continue;
                    }
                    self.add_patch(axis, sign, coord, (ua, va), (u0, u1), (v0, v1));
                }
            }
        }
    }

    fn add_patch(
        &mut self,
        axis: usize,
        sign: f64,
        coord: f64,
        (ua, va): (usize, usize),
        (u0, u1): (f64, f64),
        (v0, v1): (f64, f64),
    ) {
        let s = self.pixel_size;
        let w = floor_usize((u1 - u0) / s + 0.5).max(1);
        let h = floor_usize((v1 - v0) / s + 0.5).max(1);
        let (su, sv) = ((u1 - u0) / fl(w), (v1 - v0) / fl(h));
        let patch = self.patches.len();
        let first = self.pixels.len();
        let n = V3::unit_axis(axis, sign);
        for j in 0..h {
            for i in 0..w {
                let mut p = V3::default();
                p.set_axis(axis, coord);
                p.set_axis(ua, u0 + (fl(i) + 0.5) * su);
                p.set_axis(va, v0 + (fl(j) + 0.5) * sv);
                let probe = p.add(n.scale(1e-3));
                let valid = self.world.boxes.iter().all(|b| !b.contains(probe, 0.0));
                self.pixels.push(Pixel { p, n, valid, patch });
            }
        }
        self.patches.push(Patch {
            axis,
            sign,
            coord,
            u_axis: ua,
            v_axis: va,
            u0,
            v0,
            w,
            h,
            first,
        });
    }

    /// Valid pixel count.
    #[must_use]
    pub fn valid_pixels(&self) -> usize {
        self.pixels.iter().filter(|p| p.valid).count()
    }

    /// Open surface area in square metres.
    #[must_use]
    pub fn surface_area(&self) -> f64 {
        self.patches
            .iter()
            .map(|pa| {
                let su = self.pixel_size;
                let valid = (pa.first..pa.first + pa.w * pa.h)
                    .filter(|&i| self.pixels[i].valid)
                    .count();
                fl(valid) * su * su
            })
            .sum()
    }
}
