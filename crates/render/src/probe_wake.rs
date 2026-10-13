// Probe wake events. The GPU schedules the rings; this queue only remembers a
// change for a few frames and uploads its centre. Lamps are not in the geometry
// change list (kept hits are shaded again on their own schedule).

use crate::pack::SCENE_TAIL;

pub(crate) const WAKE_EVENT_CAP: usize = 8;
const WAKE_AGE: u32 = 8;
const WAKE_BIG_COLOR: f32 = 0.75;

#[derive(Clone, Copy, Debug)]
pub(crate) struct WakeConfig {
    pub mode: u32,
    pub tolerance: f32,
    pub budget: u32,
    pub max_radius: f32,
}

/// `GENOS_LIGHTING_WAKE=rings` selects the ring scheduler. Legacy is the default
/// and leaves the check-ray path alone. The other variables are the tolerance,
/// the verification budget and the furthest ring, in metres.
pub(crate) fn lighting_wake_config() -> WakeConfig {
    let mode = match std::env::var("GENOS_LIGHTING_WAKE").ok().as_deref() {
        Some("rings") | Some("1") => 1,
        _ => 0,
    };
    let tolerance = std::env::var("GENOS_LIGHTING_WAKE_TOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.08_f32)
        .clamp(1.0e-4, 1.0);
    let budget = std::env::var("GENOS_LIGHTING_WAKE_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let max_radius = std::env::var("GENOS_LIGHTING_WAKE_RADIUS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8.0);
    WakeConfig {
        mode,
        tolerance,
        budget,
        max_radius,
    }
}

#[derive(Clone, Copy, Debug)]
struct LampRec {
    pos: [f32; 3],
    directional: bool,
    color: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WakeEvent {
    pub center: [f32; 3],
    /// Negative: one full wake. Positive: ring tests start at this radius.
    pub radius: f32,
    pub serial: u32,
    pub age: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct WakeQueue {
    live: [WakeEvent; WAKE_EVENT_CAP],
    n: usize,
    next_serial: u32,
    tol_scale: f32,
}

impl WakeQueue {
    pub(crate) fn new() -> Self {
        Self {
            live: [WakeEvent {
                center: [0.0; 3],
                radius: 0.0,
                serial: 0,
                age: 0,
            }; WAKE_EVENT_CAP],
            n: 0,
            next_serial: 1,
            tol_scale: 1.0,
        }
    }

    pub(crate) fn tolerance(&self, setting: f32) -> f32 {
        setting * self.tol_scale
    }

    /// A miss tightens the ring tolerance. Hits with no miss relax it back
    /// toward the setting. The floor is a quarter of the setting.
    pub(crate) fn adapt(&mut self, hits: u32, misses: u32) {
        if misses > 0 {
            self.tol_scale = (self.tol_scale * 0.8).max(0.25);
        } else if hits > 0 {
            self.tol_scale = (self.tol_scale * 1.02).min(1.0);
        }
    }

    pub(crate) fn begin_frame(&mut self) {
        let mut n = 0;
        for i in 0..self.n {
            if self.live[i].age > 1 {
                self.live[n] = self.live[i];
                self.live[n].age -= 1;
                n += 1;
            }
        }
        self.n = n;
    }

    /// Fold this frame's lamp diff and geometry spheres into the queue.
    /// `changes == None` is more geometry than the change list holds: one full wake.
    pub(crate) fn note(&mut self, old: &[u8], new: &[u8], changes: Option<&[[f32; 4]]>) {
        if old.is_empty() || new.is_empty() {
            return;
        }
        let (a, b) = (lamps_of(old), lamps_of(new));
        if a.len() != b.len() {
            for lamp in a.iter().chain(b.iter()) {
                self.push(lamp.pos, -1.0, 1);
            }
        } else {
            for (prev, next) in a.iter().zip(b.iter()) {
                self.note_lamp(prev, next);
            }
        }
        match changes {
            None => self.push([0.0; 3], -1.0, 1),
            Some(list) => {
                for sphere in list {
                    self.push([sphere[0], sphere[1], sphere[2]], sphere[3].max(0.5), WAKE_AGE);
                }
            }
        }
    }

    fn note_lamp(&mut self, prev: &LampRec, next: &LampRec) {
        let dist = length(sub(next.pos, prev.pos));
        let (ad, sum) = color_delta(prev.color, next.color);
        let rel = ad / sum.max(1.0e-3);
        let moved = dist > 1.0e-3;
        let recolored = rel > 0.02;
        if !moved && !recolored && prev.directional == next.directional {
            return;
        }
        if prev.directional != next.directional || prev.directional || rel > WAKE_BIG_COLOR {
            self.push(next.pos, -1.0e6, 1);
            return;
        }
        let radius = dist.max(0.5);
        self.push(prev.pos, radius, WAKE_AGE);
        if moved {
            self.push(next.pos, radius, WAKE_AGE);
        }
    }

    fn push(&mut self, center: [f32; 3], radius: f32, age: u32) {
        let big = radius < 0.0;
        for i in 0..self.n {
            let e = &mut self.live[i];
            let same_kind = (e.radius < 0.0) == big;
            if same_kind && length(sub(e.center, center)) < 0.05 {
                e.center = center;
                e.radius = radius;
                e.age = e.age.max(age);
                return;
            }
        }
        if self.n == WAKE_EVENT_CAP {
            return;
        }
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        self.live[self.n] = WakeEvent {
            center,
            radius,
            serial,
            age,
        };
        self.n += 1;
    }

    /// A moved mesh, in metres. Same ring as an occluder sphere.
    pub(crate) fn note_sphere(&mut self, center: [f32; 3], radius: f32) {
        self.push(center, radius.max(0.5), WAKE_AGE);
    }

    pub(crate) fn len(&self) -> usize {
        self.n
    }

    pub(crate) fn events(&self) -> &[WakeEvent] {
        &self.live[..self.n]
    }

    /// Eight events, two vec4 each: centre and radius, then the serial in the
    /// first float's bits. Empty slots stay zero.
    pub(crate) fn pack(&self) -> [f32; WAKE_EVENT_CAP * 8] {
        let mut out = [0.0f32; WAKE_EVENT_CAP * 8];
        for (i, e) in self.events().iter().enumerate() {
            let b = i * 8;
            out[b] = e.center[0];
            out[b + 1] = e.center[1];
            out[b + 2] = e.center[2];
            out[b + 3] = e.radius;
            out[b + 4] = f32::from_bits(e.serial);
            out[b + 5] = e.age as f32;
        }
        out
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn color_delta(a: [f32; 3], b: [f32; 3]) -> (f32, f32) {
    let mut ad = 0.0;
    let mut sum = 0.0;
    for i in 0..3 {
        ad += (a[i] - b[i]).abs();
        sum += a[i] + b[i];
    }
    (ad, sum)
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    bytes
        .get(at..at + 4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(0.0)
}

fn lamps_of(bytes: &[u8]) -> Vec<LampRec> {
    let count = u32::from_le_bytes(bytes.get(0..4).unwrap_or(&[0; 4]).try_into().unwrap_or([0; 4])) as usize;
    (0..count)
        .filter_map(|i| {
            let at = SCENE_TAIL + i * 32;
            if bytes.len() < at + 32 {
                return None;
            }
            Some(LampRec {
                pos: [f32_at(bytes, at), f32_at(bytes, at + 4), f32_at(bytes, at + 8)],
                directional: f32_at(bytes, at + 12) > 0.5,
                color: [
                    f32_at(bytes, at + 16),
                    f32_at(bytes, at + 20),
                    f32_at(bytes, at + 24),
                ],
            })
        })
        .collect()
}

#[cfg(test)]
fn scene_with(lamps: &[LampRec]) -> Vec<u8> {
    let mut bytes = vec![0u8; SCENE_TAIL + lamps.len() * 32];
    bytes[..4].copy_from_slice(&(lamps.len() as u32).to_le_bytes());
    for (i, lamp) in lamps.iter().enumerate() {
        let at = SCENE_TAIL + i * 32;
        let mut put = |off: usize, v: f32| {
            bytes[at + off..at + off + 4].copy_from_slice(&v.to_le_bytes());
        };
        put(0, lamp.pos[0]);
        put(4, lamp.pos[1]);
        put(8, lamp.pos[2]);
        put(12, if lamp.directional { 1.0 } else { 0.0 });
        put(16, lamp.color[0]);
        put(20, lamp.color[1]);
        put(24, lamp.color[2]);
    }
    bytes
}

#[cfg(test)]
mod wake_tests {
    use super::*;

    fn point(pos: [f32; 3], color: [f32; 3]) -> LampRec {
        LampRec {
            pos,
            directional: false,
            color,
        }
    }

    #[test]
    fn a_still_scene_queues_nothing() {
        let mut q = WakeQueue::new();
        let bytes = scene_with(&[point([1.0, 2.0, 3.0], [1.0, 1.0, 1.0])]);
        q.note(&bytes, &bytes, Some(&[]));
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn a_small_move_rings_both_centres_and_a_doubled_colour_is_not_big() {
        let mut q = WakeQueue::new();
        let old = scene_with(&[point([0.0, 1.0, 0.0], [1.0, 1.0, 1.0])]);
        let moved = scene_with(&[point([0.6, 1.0, 0.0], [1.0, 1.0, 1.0])]);
        q.note(&old, &moved, Some(&[]));
        assert_eq!(q.len(), 2, "{:?}", q.events());
        for e in q.events() {
            assert!(e.radius >= 0.6, "radius {}", e.radius);
            assert!(e.radius > 0.0);
        }
        let mut color = WakeQueue::new();
        let doubled = scene_with(&[point([0.0, 1.0, 0.0], [2.0, 2.0, 2.0])]);
        color.note(&old, &doubled, Some(&[]));
        assert_eq!(color.len(), 1);
        assert!(color.events()[0].radius > 0.0, "2× colour is a ring, not a full wake");
    }

    #[test]
    fn a_sun_or_a_large_colour_change_wakes_once() {
        let old = scene_with(&[LampRec {
            pos: [0.0, 5.0, 0.0],
            directional: true,
            color: [1.0, 1.0, 0.9],
        }]);
        let moved = scene_with(&[LampRec {
            pos: [0.0, 5.0, 1.0],
            directional: true,
            color: [1.0, 1.0, 0.9],
        }]);
        let mut q = WakeQueue::new();
        q.note(&old, &moved, Some(&[]));
        assert_eq!(q.len(), 1);
        assert!(q.events()[0].radius < 0.0);
        assert_eq!(q.events()[0].age, 1);
        q.begin_frame();
        assert_eq!(q.len(), 0, "a full wake is one frame");

        let dim = scene_with(&[point([0.0, 1.0, 0.0], [1.0, 0.0, 0.0])]);
        let gone = scene_with(&[point([0.0, 1.0, 0.0], [0.0, 0.0, 0.0])]);
        let mut big = WakeQueue::new();
        big.note(&dim, &gone, Some(&[]));
        assert!(big.events()[0].radius < 0.0);
    }

    #[test]
    fn geometry_and_an_overflow_become_events_and_a_miss_tightens_tolerance() {
        let bytes = scene_with(&[point([0.0, 1.0, 0.0], [1.0, 1.0, 1.0])]);
        let mut q = WakeQueue::new();
        q.note(&bytes, &bytes, Some(&[[4.0, 0.0, 1.0, 1.5]]));
        assert_eq!(q.len(), 1);
        assert!((q.events()[0].radius - 1.5).abs() < 1.0e-5);
        let mut overflow = WakeQueue::new();
        overflow.note(&bytes, &bytes, None);
        assert!(overflow.events()[0].radius < 0.0);

        let mut tol = WakeQueue::new();
        assert!((tol.tolerance(0.08) - 0.08).abs() < 1.0e-6);
        tol.adapt(0, 1);
        assert!((tol.tolerance(0.08) - 0.064).abs() < 1.0e-5);
        for _ in 0..20 {
            tol.adapt(0, 1);
        }
        assert!((tol.tol_scale - 0.25).abs() < 1.0e-5);
        tol.adapt(4, 0);
        assert!(tol.tol_scale > 0.25 && tol.tol_scale <= 1.0);
    }
}
