//! The stress building: one 25 m section tiled into a grid, its moving boxes, its lamp
//! slots and the sun. Everything here is scene content built on the public scene types;
//! the engine sees an ordinary [`Scene`].
//!
//! # One section (local metres, u along +X, v along +Z, north is -Z)
//!
//! ```text
//!  v=0    +--------+------------------+--------+   roof slab at 3.00-3.25 m with
//!         | room A |  hall (north)    | room C |   four holes: the light-well slot
//!         | bounce |                  | windows|   (u 0.6-7.4, v 8.3-10.2), the hall
//!  v=8    +-[ 6 m opening ]-+  [skylight] +--door--+   skylight (u 10.5-15.5, v 10-14),
//!  v=8-10.5 light well      |  pillars,   |corridor|   the corridor slot (u 17.5-24.5,
//!  v=10.5 +==screen wall====+  green box  | slot   |   v 11.5-12.3) and the sunroom
//!         | hall (west)        orbiting   +--door--+   skylight (u 20-23, v 19.5-22).
//!  v=16   |~~partition 1.6 m~~|           | room B |
//!         | hall (south), red box slides  | sunroom|
//!  v=25   +-------------------------------+--------+
//!         u=0     u=8                    u=17     u=25
//! ```
//!
//! Room A has no window and no roof hole. Its only opening is 6 m wide in its south
//! wall and faces a light well: a 2.5 m strip under its own roof slot, closed to the
//! south by a full-height screen wall and open to the hall at its east end. The sun
//! always stands in the north (as seen from Sydney), so its rays always travel
//! south and can never pass a south-facing opening: the sun lands in the light well
//! and on the screen, and room A sees only that bounce. No lamp slot has a straight
//! line to the opening either (checked when the slots are made), so room A is lit by
//! bounce alone, from the sun by day and from the hall lamps at night. Its red box,
//! blue cylinder and green wall panel are the colour-bleed checks. The hall gets the sun through the skylight and (on the
//! building edge) through its windows and big doorways; room C only through small
//! windows; room B through a wide window and its own skylight.
//!
//! Sections tile on a `cols × rows` grid. A section edge on the building perimeter is
//! an outside wall with windows and big doorways; a shared edge is an inside wall with
//! wide openings so the sections join into one open building.

use genos_scene::{Floor, Light, Scene, Shape, Solid, Vec3, Wall};

/// Side of one square section in metres.
pub const SECTION: f32 = 25.0;
/// Floor to the roof underside.
pub const ROOM_HEIGHT: f32 = 3.0;
const ROOF_THICK: f32 = 0.25;
const WALL_THICK: f32 = 0.3;
/// Open ground around the building so the sun has something to land on outside.
const YARD: f32 = 8.0;
/// Lamp height under the 3 m roof.
const LAMP_HEIGHT: f32 = 2.6;
/// Lamp slots generated per section. A section holds at most this many lamps.
pub const LOCAL_SLOTS: usize = 128;
/// Sum of all lamp colours at power 1. Five lamps of 0.2 light one section without
/// washing out the sun patches; more lamps share the same total so the picture stays
/// comparable between counts.
const LAMP_TOTAL: f32 = 1.0;
/// Sun colour at full height. A sun of 1 lights like a unit lamp 7 m away.
const SUN_COLOR: [f32; 3] = [2.0, 1.9, 1.7];
/// Height of the sun (sine of its elevation) over which it fades in from the horizon.
/// A scene stand-in for the long air path at sunrise and sunset; it has no cost.
const SUN_FADE: f32 = 0.1;

/// A gap in a wall: `from..to` along the wall, open between `sill` and `head`.
#[derive(Clone, Copy, Debug)]
struct Opening {
    from: f32,
    to: f32,
    sill: f32,
    head: f32,
}

const fn window(from: f32, to: f32) -> Opening {
    Opening {
        from,
        to,
        sill: 0.9,
        head: 2.2,
    }
}
const fn high_window(from: f32, to: f32) -> Opening {
    Opening {
        from,
        to,
        sill: 1.7,
        head: 2.6,
    }
}
const fn wide_window(from: f32, to: f32) -> Opening {
    Opening {
        from,
        to,
        sill: 0.5,
        head: 2.6,
    }
}
const fn door(from: f32, to: f32) -> Opening {
    Opening {
        from,
        to,
        sill: 0.0,
        head: 2.3,
    }
}
const fn big_door(from: f32, to: f32) -> Opening {
    Opening {
        from,
        to,
        sill: 0.0,
        head: 2.7,
    }
}

// Room A spans u 0-8 and v 0-8 and keeps both of its outside walls blank.
const WEST_OUTSIDE: [Opening; 3] = [window(10.0, 14.0), window(18.0, 19.0), big_door(21.0, 24.0)];
const WEST_INSIDE: [Opening; 2] = [big_door(10.0, 14.0), big_door(19.0, 23.0)];
const NORTH_OUTSIDE: [Opening; 3] = [
    high_window(11.0, 12.0),
    window(13.0, 16.0),
    window(19.0, 22.0),
];
const NORTH_INSIDE: [Opening; 2] = [big_door(10.0, 16.0), door(19.0, 22.0)];
const EAST_OUTSIDE: [Opening; 3] = [
    window(3.0, 5.0),
    big_door(10.0, 13.5),
    wide_window(17.5, 23.5),
];
const SOUTH_OUTSIDE: [Opening; 3] = [
    window(3.0, 7.0),
    big_door(10.0, 14.0),
    wide_window(18.0, 24.0),
];

/// Roof holes, local `[u0, v0, u1, v1]`.
const ROOF_HOLES: [[f32; 4]; 4] = [
    [0.6, 8.3, 7.4, 10.2],
    [10.5, 10.0, 15.5, 14.0],
    [17.5, 11.5, 24.5, 12.3],
    [20.0, 19.5, 23.0, 22.0],
];

/// Room A's opening in its south wall at v = 8: the bounce-only check.
const ROOM_A_OPENING: [f32; 2] = [1.0, 7.0];
const ROOM_A_SOUTH: f32 = 8.0;
/// The light well's screen wall: v, and its east end.
const SCREEN_V: f32 = 10.5;
const SCREEN_END: f32 = 9.0;

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

/// Where a test camera stands. Coordinates are in section 0, which every scale has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// South end of the hall looking north past the red box, the skylight and the
    /// pillars. The benchmark viewpoint.
    Hall,
    /// Inside room A looking out of its opening at the light well, past the red box,
    /// the blue cylinder and the green panel.
    RoomA,
}

impl View {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "hall" => Some(Self::Hall),
            "rooma" | "room-a" | "roomA" | "bounce" => Some(Self::RoomA),
            _ => None,
        }
    }

    /// Ground `x`, `z`, yaw and pitch for [`genos_scene::Camera::new`].
    pub fn pose(self) -> (f32, f32, f32, f32) {
        match self {
            Self::Hall => (14.5, 23.5, (-6.5f32).atan2(17.5), 0.08),
            Self::RoomA => (7.2, 0.9, (-4.7f32).atan2(-7.1), -0.12),
        }
    }
}

/// Path of a moving box.
#[derive(Clone, Copy, Debug)]
enum Path {
    /// Eased back and forth between two points.
    Slide { from: Vec3, to: Vec3 },
    /// Circle about `center`.
    Orbit { center: Vec3, radius: f32 },
}

#[derive(Clone, Copy, Debug)]
struct Mover {
    solid: usize,
    path: Path,
    period: f32,
    /// Seconds added to the clock, so sections do not move in step.
    phase: f32,
    /// Turn rate in radians per second.
    spin: f32,
}

impl Mover {
    fn at(&self, time: f32) -> (Vec3, f32) {
        let t = time + self.phase;
        let turn = std::f32::consts::TAU * t / self.period;
        let position = match self.path {
            Path::Slide { from, to } => {
                let s = 0.5 - 0.5 * turn.cos();
                from + (to - from) * s
            }
            Path::Orbit { center, radius } => {
                center + Vec3::new(turn.cos(), 0.0, turn.sin()) * radius
            }
        };
        (position, self.spin * t)
    }
}

/// How a dynamic lamp moves. Every motion passes through the lamp's home at time 0,
/// and a static lamp sits at its home, so switching a lamp between the two keeps it
/// in place.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Motion {
    /// Circle of `radius` through the home, `rate` radians per second.
    Orbit { radius: f32, rate: f32 },
    /// Swing along unit `axis` by `amp` metres either side of the home.
    Sway { axis: Vec3, amp: f32, rate: f32 },
}

impl Motion {
    fn offset(self, time: f32) -> Vec3 {
        match self {
            Self::Orbit { radius, rate } => {
                let a = rate * time;
                Vec3::new((a.cos() - 1.0) * radius, 0.0, a.sin() * radius)
            }
            Self::Sway { axis, amp, rate } => axis * (amp * (rate * time).sin()),
        }
    }

    /// Offsets along one full loop, to check the path stays clear.
    fn samples(self) -> impl Iterator<Item = Vec3> {
        let rate = match self {
            Self::Orbit { rate, .. } | Self::Sway { rate, .. } => rate.abs().max(1.0e-3),
        };
        let period = std::f32::consts::TAU / rate;
        (0..32).map(move |i| self.offset(period * i as f32 / 32.0))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LampSlot {
    pub home: Vec3,
    motion: Motion,
    tint: [f32; 3],
}

impl LampSlot {
    pub fn at(&self, time: f32) -> Vec3 {
        self.home + self.motion.offset(time)
    }
}

/// Where the lamps go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Lamp `k` in section `k mod sections`: the lamps spread over the building.
    Spread,
    /// Every lamp in section 0, where the benchmark camera stands. The view sees the
    /// same lamps at every scale, so a scale delta is the cost of size alone.
    First,
}

impl Layout {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "spread" => Some(Self::Spread),
            "first" | "local" => Some(Self::First),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Spread => "spread",
            Self::First => "first",
        }
    }
}

/// Lamp counts for one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightMix {
    /// Lamps in the whole building.
    pub count: usize,
    /// Share of those lamps that move, in percent.
    pub dynamic_pct: u32,
    pub layout: Layout,
}

impl LightMix {
    /// Moving lamps, rounded to the nearest whole lamp.
    pub fn dynamic(self) -> usize {
        ((self.count as f32 * self.dynamic_pct.min(100) as f32 / 100.0).round() as usize)
            .min(self.count)
    }
}

/// Which of `count` lamps move when `dynamic` of them do. The order is fixed by the
/// lamp index alone (golden-ratio spacing), so raising the share only adds movers and
/// a lamp keeps its slot whatever the share.
pub fn moving_set(count: usize, dynamic: usize) -> Vec<bool> {
    let mut order: Vec<usize> = (0..count).collect();
    let key = |k: usize| (k as f64 * 0.618_033_988_749_895).fract();
    order.sort_by(|a, b| key(*a).total_cmp(&key(*b)));
    let mut moving = vec![false; count];
    for &k in order.iter().take(dynamic) {
        moving[k] = true;
    }
    moving
}

/// Deterministic generator: SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

pub struct Building {
    /// Walls, still solids and the moving boxes at time 0. No lamps.
    pub scene: Scene,
    movers: Vec<Mover>,
    /// Lamp slots in building order for [`Layout::Spread`]: slot `k` is lamp `k`.
    spread: Vec<LampSlot>,
    /// Section 0's slots, for [`Layout::First`].
    first: Vec<LampSlot>,
}

impl Building {
    pub fn new(cols: u32, rows: u32, seed: u64) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let mut rng = Rng(seed);
        let local = local_slots(&mut rng);
        let mut walls = Vec::new();
        let mut solids = Vec::new();
        let mut movers = Vec::new();
        let mut boxes = Vec::new();
        for j in 0..rows {
            for i in 0..cols {
                let origin = Vec3::new(i as f32 * SECTION, 0.0, j as f32 * SECTION);
                let edges = Edges {
                    west: if i == 0 {
                        &WEST_OUTSIDE[..]
                    } else {
                        &WEST_INSIDE[..]
                    },
                    north: if j == 0 {
                        &NORTH_OUTSIDE[..]
                    } else {
                        &NORTH_INSIDE[..]
                    },
                    east: (i + 1 == cols).then_some(&EAST_OUTSIDE[..]),
                    south: (j + 1 == rows).then_some(&SOUTH_OUTSIDE[..]),
                };
                for mut wall in section_walls(&edges) {
                    wall.position += origin;
                    walls.push(wall);
                }
                for mut solid in section_solids() {
                    solid.position += origin;
                    solids.push(solid);
                }
                for (mut solid, mut mover) in section_movers() {
                    solid.position += origin;
                    mover.path = match mover.path {
                        Path::Slide { from, to } => Path::Slide {
                            from: from + origin,
                            to: to + origin,
                        },
                        Path::Orbit { center, radius } => Path::Orbit {
                            center: center + origin,
                            radius,
                        },
                    };
                    mover.phase = rng.range(0.0, mover.period);
                    boxes.push((solid, mover));
                }
            }
        }
        let still_count = solids.len();
        for (index, (solid, mut mover)) in boxes.into_iter().enumerate() {
            mover.solid = still_count + index;
            solids.push(solid);
            movers.push(mover);
        }
        let sections = (cols * rows) as usize;
        let mut slots = Vec::with_capacity(sections * LOCAL_SLOTS);
        for k in 0..sections * LOCAL_SLOTS {
            let section = k % sections;
            let nth = k / sections;
            let slot = local[(section + nth) % LOCAL_SLOTS];
            let origin = Vec3::new(
                (section as u32 % cols) as f32 * SECTION,
                0.0,
                (section as u32 / cols) as f32 * SECTION,
            );
            slots.push(LampSlot {
                home: slot.home + origin,
                ..slot
            });
        }
        let width = cols as f32 * SECTION;
        let depth = rows as f32 * SECTION;
        let floor = Floor {
            position: Vec3::new(width * 0.5, 0.0, depth * 0.5),
            half_x: width * 0.5 + YARD,
            half_z: depth * 0.5 + YARD,
            color: WHITE,
        };
        let scene = Scene {
            floor,
            walls,
            solids,
            lights: Vec::new(),
            ceiling: None,
        };
        let mut building = Self {
            scene,
            movers,
            spread: slots,
            first: local,
        };
        let mut solids = std::mem::take(&mut building.scene.solids);
        building.place_movers(&mut solids, 0.0);
        building.scene.solids = solids;
        building
    }

    fn slots(&self, layout: Layout) -> &[LampSlot] {
        match layout {
            Layout::Spread => &self.spread,
            Layout::First => &self.first,
        }
    }

    /// Most lamps this building has slots for in `layout`.
    pub fn max_lamps(&self, layout: Layout) -> usize {
        self.slots(layout).len()
    }

    pub fn movers(&self) -> usize {
        self.movers.len()
    }

    /// Put the moving boxes where they are at `time` seconds.
    pub fn place_movers(&self, solids: &mut [Solid], time: f32) {
        for mover in &self.movers {
            if let Some(solid) = solids.get_mut(mover.solid) {
                let (position, yaw) = mover.at(time);
                solid.position = position;
                solid.yaw = yaw;
            }
        }
    }

    /// The first `mix.count` lamps at `time`. Moving lamps follow their paths; the
    /// others sit at home. Each lamp gets an equal share of `power` × [`LAMP_TOTAL`].
    pub fn lamps(&self, mix: LightMix, time: f32, power: f32) -> Vec<Light> {
        let slots = self.slots(mix.layout);
        let count = mix.count.min(slots.len());
        let moving = moving_set(count, mix.dynamic().min(count));
        let share = if count > 0 {
            power * LAMP_TOTAL / count as f32
        } else {
            0.0
        };
        slots[..count]
            .iter()
            .zip(moving)
            .map(|(slot, moves)| Light {
                position: if moves { slot.at(time) } else { slot.home },
                color: slot.tint.map(|c| c * share),
                direction: Vec3::ZERO,
            })
            .collect()
    }
}

/// Fraction of a day to the light it gives. 0 is sunrise in the east (+X), 0.25 is
/// noon, 0.5 is sunset in the west; the arc leans north (-Z), as seen from Sydney.
/// The night half has no sun.
pub fn sun(day: f32) -> Option<Light> {
    let angle = std::f32::consts::TAU * day.rem_euclid(1.0);
    let up = angle.sin();
    if up <= 0.0 {
        return None;
    }
    let toward = Vec3::new(angle.cos(), up, -0.45).normalize();
    let fade = (up / SUN_FADE).min(1.0);
    Some(Light {
        position: Vec3::new(0.0, 7.0, 0.0),
        color: SUN_COLOR.map(|c| c * fade),
        direction: toward * -1.0,
    })
}

/// Clock time for a day fraction, sunrise at 06:00.
pub fn clock(day: f32) -> (u32, u32) {
    let minutes = ((day.rem_euclid(1.0) * 24.0 * 60.0) as u32 + 6 * 60) % (24 * 60);
    (minutes / 60, minutes % 60)
}

struct Edges<'a> {
    west: &'a [Opening],
    north: &'a [Opening],
    east: Option<&'a [Opening]>,
    south: Option<&'a [Opening]>,
}

/// One box of wall along X (`along_x`) or Z at `line`, spanning `a..b`, from `base` to `top`.
fn piece(along_x: bool, line: f32, a: f32, b: f32, base: f32, top: f32, thick: f32) -> Wall {
    let mid = (a + b) * 0.5;
    let half = (b - a).abs() * 0.5;
    Wall {
        position: if along_x {
            Vec3::new(mid, 0.0, line)
        } else {
            Vec3::new(line, 0.0, mid)
        },
        half_x: if along_x { half } else { thick * 0.5 },
        half_z: if along_x { thick * 0.5 } else { half },
        height: top - base,
        base,
        color: WHITE,
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

/// A full-height wall from `from` to `to` with its openings: piers between them, a sill
/// under each window and a lintel over each opening.
fn wall_run(
    walls: &mut Vec<Wall>,
    along_x: bool,
    line: f32,
    from: f32,
    to: f32,
    openings: &[Opening],
) {
    let mut at = from;
    for gap in openings {
        if gap.from > at {
            walls.push(piece(
                along_x,
                line,
                at,
                gap.from,
                0.0,
                ROOM_HEIGHT,
                WALL_THICK,
            ));
        }
        if gap.sill > 0.0 {
            walls.push(piece(
                along_x, line, gap.from, gap.to, 0.0, gap.sill, WALL_THICK,
            ));
        }
        if gap.head < ROOM_HEIGHT {
            walls.push(piece(
                along_x,
                line,
                gap.from,
                gap.to,
                gap.head,
                ROOM_HEIGHT,
                WALL_THICK,
            ));
        }
        at = gap.to;
    }
    if to > at {
        walls.push(piece(along_x, line, at, to, 0.0, ROOM_HEIGHT, WALL_THICK));
    }
}

/// Walls of one section in local coordinates: its west and north edges, the east and
/// south edges on the building perimeter, the rooms, the baffle, the low partition,
/// the coloured panel, the furniture and the roof.
fn section_walls(edges: &Edges) -> Vec<Wall> {
    let mut walls = Vec::new();
    wall_run(&mut walls, false, 0.0, 0.0, SECTION, edges.west);
    wall_run(&mut walls, true, 0.0, 0.0, SECTION, edges.north);
    if let Some(east) = edges.east {
        wall_run(&mut walls, false, SECTION, 0.0, SECTION, east);
    }
    if let Some(south) = edges.south {
        wall_run(&mut walls, true, SECTION, 0.0, SECTION, south);
    }
    walls.extend(inner_walls());
    for [u0, v0, u1, v1] in roof_rects() {
        let mut slab = piece(
            true,
            (v0 + v1) * 0.5,
            u0,
            u1,
            ROOM_HEIGHT,
            ROOM_HEIGHT + ROOF_THICK,
            v1 - v0,
        );
        slab.color = WHITE;
        walls.push(slab);
    }
    walls
}

/// Room walls, baffle, partition and wall-shaped furniture. These are the same in every
/// section, so the lamp slot checks use them.
fn inner_walls() -> Vec<Wall> {
    let mut walls = Vec::new();
    // Room A: blank east wall, one wide opening south onto the light well.
    wall_run(&mut walls, false, 8.0, 0.0, 8.0, &[]);
    let opening = Opening {
        from: ROOM_A_OPENING[0],
        to: ROOM_A_OPENING[1],
        sill: 0.0,
        head: 2.4,
    };
    wall_run(&mut walls, true, ROOM_A_SOUTH, 0.0, 8.0, &[opening]);
    // The light well's screen wall.
    wall_run(&mut walls, true, SCREEN_V, 0.0, SCREEN_END, &[]);
    // Room C: doors west and south.
    wall_run(&mut walls, false, 17.0, 0.0, 8.0, &[door(2.0, 3.4)]);
    wall_run(&mut walls, true, 8.0, 17.0, SECTION, &[door(20.0, 21.6)]);
    // Room B: a door north and a wide opening west.
    wall_run(&mut walls, true, 16.0, 17.0, SECTION, &[door(19.0, 20.4)]);
    wall_run(
        &mut walls,
        false,
        17.0,
        16.0,
        SECTION,
        &[big_door(19.0, 23.0)],
    );
    // Low partition in the hall: light passes over it.
    walls.push(piece(true, 17.0, 3.0, 10.0, 0.0, 1.6, 0.2));
    // Green panel on room A's west wall, a shelf in room C, two hall benches.
    let mut panel = piece(false, 0.21, 2.5, 5.5, 0.0, 2.2, 0.12);
    panel.color = [0.15, 0.75, 0.2];
    walls.push(panel);
    let mut shelf = piece(true, 0.45, 19.5, 23.5, 0.0, 1.8, 0.5);
    shelf.color = [0.85, 0.8, 0.7];
    walls.push(shelf);
    for (line, a, b) in [(12.5, 2.0, 5.0), (22.0, 4.0, 7.0)] {
        let mut bench = piece(true, line, a, b, 0.0, 0.45, 0.6);
        bench.color = [0.55, 0.5, 0.45];
        walls.push(bench);
    }
    walls
}

/// The roof over `0..SECTION` square minus [`ROOF_HOLES`], as non-overlapping rectangles.
fn roof_rects() -> Vec<[f32; 4]> {
    let mut xs = vec![0.0, SECTION];
    for hole in ROOF_HOLES {
        xs.push(hole[0]);
        xs.push(hole[2]);
    }
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    let mut columns: Vec<(f32, f32, Vec<[f32; 2]>)> = Vec::new();
    for pair in xs.windows(2) {
        let (x0, x1) = (pair[0], pair[1]);
        let mut cover: Vec<[f32; 2]> = ROOF_HOLES
            .iter()
            .filter(|h| h[0] <= x0 && h[2] >= x1)
            .map(|h| [h[1], h[3]])
            .collect();
        cover.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut spans = Vec::new();
        let mut z = 0.0;
        for [z0, z1] in cover {
            if z0 > z {
                spans.push([z, z0]);
            }
            z = z.max(z1);
        }
        if z < SECTION {
            spans.push([z, SECTION]);
        }
        match columns.last_mut() {
            Some(last) if last.2 == spans && last.1 == x0 => last.1 = x1,
            _ => columns.push((x0, x1, spans)),
        }
    }
    columns
        .into_iter()
        .flat_map(|(x0, x1, spans)| spans.into_iter().map(move |[z0, z1]| [x0, z0, x1, z1]))
        .collect()
}

fn solid(shape: Shape, u: f32, v: f32, size: f32, height: f32, color: [f32; 3]) -> Solid {
    Solid {
        shape,
        position: Vec3::new(u, 0.0, v),
        size,
        height,
        yaw: 0.0,
        color,
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

const PILLARS: [[f32; 2]; 4] = [[9.8, 9.3], [16.2, 9.3], [9.8, 14.7], [16.2, 14.7]];
const PILLAR_SIZE: f32 = 0.45;

/// Pillars and still furniture of one section.
fn section_solids() -> Vec<Solid> {
    let grey = [0.75, 0.75, 0.75];
    let mut solids: Vec<Solid> = PILLARS
        .iter()
        .map(|[u, v]| solid(Shape::Square, *u, *v, PILLAR_SIZE, ROOM_HEIGHT, grey))
        .collect();
    solids.extend([
        // Room A bleed checks.
        solid(Shape::Square, 2.0, 2.5, 1.4, 0.75, [0.9, 0.9, 0.9]),
        solid(Shape::Square, 5.5, 4.0, 1.0, 1.0, [0.85, 0.1, 0.1]),
        solid(Shape::Circle, 3.0, 5.5, 0.8, 1.2, [0.1, 0.25, 0.85]),
        // Room C crate, room B planter and table.
        solid(Shape::Square, 23.3, 6.3, 0.9, 0.9, [0.9, 0.75, 0.1]),
        solid(Shape::Circle, 24.0, 17.0, 1.2, 0.6, [0.3, 0.6, 0.25]),
        solid(Shape::Square, 19.5, 23.5, 1.0, 0.5, [0.6, 0.45, 0.3]),
    ]);
    solids
}

/// The three coloured boxes that move in every section.
fn section_movers() -> [(Solid, Mover); 3] {
    let mover = |path, period, spin| Mover {
        solid: 0,
        path,
        period,
        phase: 0.0,
        spin,
    };
    [
        (
            solid(Shape::Square, 10.5, 19.5, 1.0, 1.0, [0.9, 0.15, 0.1]),
            mover(
                Path::Slide {
                    from: Vec3::new(10.5, 0.0, 19.5),
                    to: Vec3::new(15.5, 0.0, 19.5),
                },
                8.0,
                0.6,
            ),
        ),
        (
            solid(Shape::Square, 15.0, 12.0, 0.9, 0.9, [0.15, 0.8, 0.2]),
            mover(
                Path::Orbit {
                    center: Vec3::new(13.0, 0.0, 12.0),
                    radius: 2.0,
                },
                12.0,
                -0.8,
            ),
        ),
        (
            solid(Shape::Square, 22.0, 17.5, 0.8, 0.8, [0.15, 0.3, 0.9]),
            mover(
                Path::Slide {
                    from: Vec3::new(22.0, 0.0, 17.5),
                    to: Vec3::new(22.0, 0.0, 23.5),
                },
                10.0,
                1.0,
            ),
        ),
    ]
}

/// Footprint `[u0, v0, u1, v1]` of a wall that reaches above the lamps.
fn tall_rects() -> Vec<[f32; 4]> {
    inner_walls()
        .iter()
        .filter(|w| w.base <= 0.0 && w.height >= LAMP_HEIGHT)
        .map(|w| {
            [
                w.position.x - w.half_x,
                w.position.z - w.half_z,
                w.position.x + w.half_x,
                w.position.z + w.half_z,
            ]
        })
        .collect()
}

/// True when the segment `a`-`b` crosses the rectangle (slab test in 2D).
fn segment_hits(a: [f32; 2], b: [f32; 2], r: [f32; 4]) -> bool {
    let lo = [r[0], r[1]];
    let hi = [r[2], r[3]];
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    for k in 0..2 {
        let d = b[k] - a[k];
        if d.abs() < 1.0e-6 {
            if a[k] < lo[k] || a[k] > hi[k] {
                return false;
            }
        } else {
            let (mut ta, mut tb) = ((lo[k] - a[k]) / d, (hi[k] - a[k]) / d);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// A lamp may stand at local `(u, v)`: inside the section, clear of tall walls and
/// pillars, out of room A and with no straight line to room A's opening, so room A
/// stays bounce-only whatever the lamp count. Lintels count as open, which only errs
/// safe.
fn lamp_spot_ok(u: f32, v: f32, tall: &[[f32; 4]]) -> bool {
    const MARGIN: f32 = 0.6;
    const CLEAR: f32 = 0.35;
    if !(MARGIN..=SECTION - MARGIN).contains(&u) || !(MARGIN..=SECTION - MARGIN).contains(&v) {
        return false;
    }
    if u < 8.0 + MARGIN && v < ROOM_A_SOUTH + MARGIN {
        return false;
    }
    let near =
        |r: &[f32; 4]| u > r[0] - CLEAR && u < r[2] + CLEAR && v > r[1] - CLEAR && v < r[3] + CLEAR;
    if tall.iter().any(near) {
        return false;
    }
    if PILLARS
        .iter()
        .any(|[pu, pv]| (u - pu).abs() < 0.5 && (v - pv).abs() < 0.5)
    {
        return false;
    }
    !sees_room_a(u, v, tall)
}

fn sees_room_a(u: f32, v: f32, tall: &[[f32; 4]]) -> bool {
    let [from, to] = ROOM_A_OPENING;
    (0..=12).any(|i| {
        let at = from + 0.05 + (to - from - 0.1) * i as f32 / 12.0;
        !tall
            .iter()
            .any(|r| segment_hits([u, v], [at, ROOM_A_SOUTH], *r))
    })
}

fn path_ok(home: Vec3, motion: Motion, tall: &[[f32; 4]]) -> bool {
    motion
        .samples()
        .all(|o| lamp_spot_ok(home.x + o.x, home.z + o.z, tall))
}

/// Lamp slots of one section in local coordinates. The first five are placed by hand:
/// hall west, room C, room B, hall south, hall north. The rest are random spots that pass [`lamp_spot_ok`] along their whole path.
fn local_slots(rng: &mut Rng) -> Vec<LampSlot> {
    let tall = tall_rects();
    let warm = [1.0, 0.9, 0.75];
    let cool = [0.8, 0.88, 1.0];
    let x = Vec3::new(1.0, 0.0, 0.0);
    let z = Vec3::new(0.0, 0.0, 1.0);
    let at = |u: f32, v: f32| Vec3::new(u, LAMP_HEIGHT, v);
    let mut slots = vec![
        LampSlot {
            home: at(4.5, 13.5),
            motion: Motion::Sway {
                axis: z,
                amp: 2.0,
                rate: 0.7,
            },
            tint: warm,
        },
        LampSlot {
            home: at(21.0, 4.0),
            motion: Motion::Orbit {
                radius: 1.5,
                rate: 0.9,
            },
            tint: cool,
        },
        LampSlot {
            home: at(20.5, 21.0),
            motion: Motion::Sway {
                axis: x,
                amp: 1.5,
                rate: 0.6,
            },
            tint: warm,
        },
        LampSlot {
            home: at(12.5, 21.5),
            motion: Motion::Orbit {
                radius: 1.5,
                rate: -0.8,
            },
            tint: warm,
        },
        LampSlot {
            home: at(12.5, 4.5),
            motion: Motion::Sway {
                axis: x,
                amp: 2.0,
                rate: 0.5,
            },
            tint: cool,
        },
    ];
    debug_assert!(slots.iter().all(|s| path_ok(s.home, s.motion, &tall)));
    while slots.len() < LOCAL_SLOTS {
        let home = at(rng.range(0.6, SECTION - 0.6), rng.range(0.6, SECTION - 0.6));
        let rate = rng.range(0.4, 1.2) * if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        let motion = if rng.unit() < 0.5 {
            Motion::Orbit {
                radius: rng.range(0.8, 1.6),
                rate,
            }
        } else {
            let axis = if rng.unit() < 0.5 { x } else { z };
            Motion::Sway {
                axis,
                amp: rng.range(1.0, 2.5),
                rate,
            }
        };
        if !path_ok(home, motion, &tall) {
            continue;
        }
        let tint = if slots.len() % 3 == 1 { cool } else { warm };
        slots.push(LampSlot { home, motion, tint });
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lamp_slot_keeps_room_a_bounce_only() {
        let tall = tall_rects();
        let slots = local_slots(&mut Rng(1));
        assert_eq!(slots.len(), LOCAL_SLOTS);
        for slot in &slots {
            for offset in slot.motion.samples() {
                let p = slot.home + offset;
                assert!(!sees_room_a(p.x, p.z, &tall), "lamp at {p:?} sees room A");
            }
        }
    }

    #[test]
    fn the_screen_and_room_walls_hide_room_a_from_the_hall() {
        let tall = tall_rects();
        assert!(!sees_room_a(12.5, 4.5, &tall));
        assert!(!sees_room_a(4.5, 13.5, &tall));
        // From inside the light well the opening is in plain view.
        assert!(sees_room_a(4.0, 9.5, &tall));
        assert!(sees_room_a(12.0, 9.5, &tall));
    }

    #[test]
    fn raising_the_dynamic_share_only_adds_movers() {
        for count in [5usize, 25, 50, 100] {
            let mut last = vec![false; count];
            for pct in [0u32, 25, 50, 75, 100] {
                let mix = LightMix {
                    count,
                    dynamic_pct: pct,
                    layout: Layout::Spread,
                };
                let now = moving_set(count, mix.dynamic());
                assert_eq!(now.iter().filter(|m| **m).count(), mix.dynamic());
                assert!(last.iter().zip(&now).all(|(was, is)| !was || *is));
                last = now;
            }
        }
    }

    #[test]
    fn a_lamp_keeps_its_home_when_it_stops_moving() {
        let building = Building::new(4, 4, 1);
        for layout in [Layout::Spread, Layout::First] {
            let mix = |dynamic_pct| LightMix {
                count: 25,
                dynamic_pct,
                layout,
            };
            let still = building.lamps(mix(0), 7.3, 1.0);
            let moving = building.lamps(mix(100), 0.0, 1.0);
            for (a, b) in still.iter().zip(&moving) {
                assert!((a.position - b.position).length() < 1.0e-4);
            }
        }
    }

    #[test]
    fn the_first_layout_puts_the_same_lamps_in_section_zero_at_every_scale() {
        let mix = LightMix {
            count: 50,
            dynamic_pct: 50,
            layout: Layout::First,
        };
        let small = Building::new(1, 1, 1).lamps(mix, 3.0, 1.0);
        let big = Building::new(4, 4, 1).lamps(mix, 3.0, 1.0);
        assert_eq!(small, big);
    }

    #[test]
    fn the_roof_covers_the_section_except_the_holes() {
        let rects = roof_rects();
        let area: f32 = rects.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum();
        let holes: f32 = ROOF_HOLES
            .iter()
            .map(|h| (h[2] - h[0]) * (h[3] - h[1]))
            .sum();
        assert!((area + holes - SECTION * SECTION).abs() < 1.0e-2);
    }

    #[test]
    fn sun_rays_always_travel_south_so_room_a_never_sees_the_sun() {
        for step in 0..1000 {
            if let Some(sun) = sun(step as f32 / 1000.0) {
                assert!(sun.direction.z > 0.0);
            }
        }
    }

    #[test]
    fn the_sun_sets_at_half_a_day() {
        assert!(sun(0.25).is_some());
        assert!(sun(0.75).is_none());
        let noon = sun(0.25).unwrap();
        assert!(noon.direction.y < -0.8);
        assert_eq!(clock(0.25), (12, 0));
    }

    #[test]
    fn big_tiles_sixteen_sections_without_overlapping_movers() {
        let building = Building::new(4, 4, 1);
        assert_eq!(building.movers(), 48);
        assert!(building.max_lamps(Layout::Spread) >= 100);
    }
}
