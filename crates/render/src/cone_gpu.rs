// The cone-traced cascades on the GPU (experimental, lighting key
// `cone_cascades`). Included into gpu.rs, as lighting_gpu.rs is: the passes
// use the lighting descriptor set, its layout and its push constant. The
// field (`cone_field.rs`) is built on a thread when the world or its glTF
// model changes or the eye leaves its clip volumes, uploaded over a few
// frames, and then every frame the passes in `shaders/cone_*.comp` run before
// compose, the light pass (`cone_light.comp`) first.

/// Words a coarse texel holds on the GPU (`cone_common.glsl` CONE_TEXEL): the
/// six ways' and the lamps' light and area, occupancy, seat, albedo, each
/// way's surface along its axis, each way's sun share (kept by the light
/// pass; 0 here), and each way's front along its axis.
const CONE_TEXEL_WORDS: usize = 26;
/// Bytes of the field uploaded each frame.
const CONE_UPLOAD_STEP: usize = 64 << 20;
/// The first interval's reach in cascade-0 probe spacings.
const CONE_T0: f32 = 2.5;
/// Push flags: the merge and the shade weigh each probe by what passes to it.
const CONE_SEE_MERGE: u32 = 1;
const CONE_SEE_SHADE: u32 = 2;
/// Push flag, knockout only: seat probes on their pixels' mean
/// ([`crate::gate::CONE_FIELD_SEAT`]).
const CONE_PIXEL_SEAT: u32 = 4;
/// Push flag: a metal's specular light is on (`cone_specular.glsl`
/// CONE_SPECULAR_FLAG; key `cone_cascades.specular`).
const CONE_SPECULAR_FLAG: u32 = 64;
/// Header word of the specular list's offset (`cone_specular.glsl`
/// CONE_SPECULAR_LAYOUT), and of its count, cleared each frame
/// (CONE_SPECULAR_COUNT).
const CONE_SPECULAR_LAYOUT_AT: u64 = 104;
const CONE_SPECULAR_COUNT_AT: u64 = 53;
/// Knockout only: the share of the screen's probes the tables are sized for
/// ([`crate::gate::CONE_TABLE_CAP`]).
const CONE_KNOCKOUT_SHARE: u64 = 16;
/// Work-buffer layout, as `cone_common.glsl` has it.
const CONE_HEADER_WORDS: u64 = 640;
/// The pixel pick's record in the header (`cone_common.glsl` CONE_PICK).
const CONE_PICK_AT: u64 = 256;
const CONE_PICK_HEAD: usize = 16;
const CONE_PICK_LEVEL: usize = 104;
/// The own probe's first cones at each level (`cone_gather.glsl`), two words
/// a cone, after the pick's levels.
const CONE_PICK_DIRS_AT: usize = 480;
const CONE_PICK_DIRS: usize = 8;
/// The own probe's next-cascade probes at each level, four words each.
const CONE_PICK_UP_AT: usize = 512;
const CONE_PICK_PROBE: usize = 12;
/// Push flag bits of the debug view (`cone_common.glsl` CONE_VIEW_SHIFT).
const CONE_VIEW_SHIFT: u32 = 4;
const CONE_HASH_WORDS: u64 = 10;
const CONE_PROBE_WORDS: u64 = 32;
const CONE_CONE_WORDS: u64 = 4;
/// Cascade 0's probes for each `probe_px` square of the screen. The voxel
/// layer with each pixel's eight neighbours measured 2.8 in the hall (floors
/// seen at a grazing angle add to it), so 6 leaves room; gate B fails if it
/// does not.
const CONE_PROBE_SLACK: u64 = 6;
/// Each cascade holds this share of the probes of the one below (measured
/// 1/3.1 with the neighbours spread: probes on a surface, at twice the spacing).
const CONE_CASCADE_SHARE: u64 = 3;
const CONE_MIN_CAP: u64 = 64;
/// Cascade-0 room for the world layer's probes (`cone_world.comp`): one a
/// surface face at 2 m within 8 m of the eye and 4 m within 16 m;
/// Sponza has about 7,500 such faces on screen or off.
const CONE_WORLD_CAP: u64 = 16_384;
/// Header words cleared each frame; the layout follows them.
const CONE_CLEARED: u64 = 64;
const CONE_LAYOUT_AT: u64 = 64;
const CONE_PIPES: usize = 19;
const CONE_PIPE_NAMES: [&str; CONE_PIPES] = [
    "cone-insert",
    "cone-compact",
    "cone-chain",
    "cone-trace",
    "cone-weights",
    "cone-merge",
    "cone-shade",
    "cone-lamps",
    "cone-spread",
    "cone-light",
    "cone-world",
    "cone-sun",
    "cone-resolve",
    "cone-fill",
    "cone-targets",
    "cone-lamp",
    "cone-map-list",
    "cone-map-trace",
    "cone-specular",
];
/// GPU timestamps a frame's cone passes write: one before the first pass and
/// one after each stage in [`CONE_STAGE_NAMES`].
const CONE_STAGES: usize = 19;
const CONE_STAMPS: u32 = 20;
/// The stages `cone_pass_ms` times, in frame order. `trace0`..`trace4` are
/// the cascades' traces; `lamp_rays` is each cascade-0 probe's rays at its
/// lamp targets; `shade` gathers the probes' light at one pixel of
/// each square, `resolve` spreads it to every pixel, `fill` gathers for the
/// pixels with no match; `sun` is the shadow-edge pixels' own sun rays;
/// `specular` is the metal pixels' specular cones.
pub(crate) const CONE_STAGE_NAMES: [&str; CONE_STAGES] =
    ["light", "lamps", "insert", "world", "compact", "chain", "trace0", "trace1", "trace2", "trace3", "trace4", "weights", "merge", "lamp_rays", "shade", "resolve", "fill", "sun", "specular"];
/// Header words of last frame's tables (`cone_common.glsl` CONE_PREV_LAYOUT).
const CONE_PREV_LAYOUT_AT: u64 = 112;
/// Header words of the merged cones' offsets, one a cascade
/// (`cone_common.glsl` CONE_MERGED_LAYOUT): the other half's cone tables.
const CONE_MERGED_LAYOUT_AT: u64 = 137;
/// The most frames a probe keeps its cones before it traces them again. The
/// period is the bounce's history over the frame's time, so the cones lag
/// the bounce by about what it already lags: four frames when three fit in
/// the history, three when two do, and two otherwise.
const CONE_KEEP_FRAMES: u32 = 4;
/// Lamp maps (`cone_common.glsl`, Lamp maps): the header's counters (cleared
/// each frame) and head words, the most lamps mapped, a lamp's words (its
/// head, 5456 tiles of 5 words, 349,184 texels of 2 words), a lamp's tiles,
/// the tile list's room, and a tile's texels.
const CONE_MAP_COUNT_AT: u64 = 160;
const CONE_MAP_COUNTERS: u64 = 16;
const CONE_MAP_HEAD_AT: u64 = 176;
const CONE_MAP_LAMPS: u64 = 64;
const CONE_MAP_TILES: u64 = 5456;
const CONE_MAP_LAMP_WORDS: u64 = 8 + CONE_MAP_TILES * 5 + 698_368;
const CONE_MAP_LIST: u64 = 8192;
const CONE_MAP_TILE_TEXELS: u64 = 64;
const CONE_MAP_LEVELS: usize = 5;
/// Header words of the gather grid's and the fill list's offsets
/// (`cone_common.glsl` CONE_GATHER_LAYOUT).
const CONE_GATHER_LAYOUT_AT: u64 = 96;
/// Header words of last frame's eye and whether its probes are in the work
/// buffer (`cone_common.glsl` CONE_PREV_EYE), for the light pass.
const CONE_PREV_EYE_AT: u64 = 92;
/// The bounce's short history (Anthony, 25 Nov 2026): each frame a voxel's
/// light moves `1 - exp(-dt / CONE_BOUNCE_SECONDS)` of the way to what last
/// frame's probes and this frame's suns give it, so a bounce settles in a few
/// tenths of a second whatever the frame rate. The direct light does not wait.
const CONE_BOUNCE_SECONDS: f32 = 0.1;
/// Threads the light pass runs a fine brick and a coarse brick.
const CONE_BRICK_VOXELS: u64 = 512;
const CONE_PADDED_TEXELS: u64 = 1000;
/// Points on each lamp's sphere (`cone_lamps.comp`), and the field levels it
/// is written at: the most lamp-hash entries one lamp can fill.
const CONE_LAMP_POINTS: u64 = 256;
const CONE_LAMP_ENTRIES: u64 = CONE_LAMP_POINTS * 8;
const CONE_LAMP_WORDS: u64 = 9;
/// Lamps a probe's target list holds (`cone_common.glsl` CONE_TARGETS), and
/// the header words of the lists' offsets, one a cascade.
const CONE_TARGETS: u64 = 16;
const CONE_TARGET_LAYOUT_AT: u64 = 98;
/// Words a gathered pixel holds: its light, and the lamps it carries
/// (`cone_common.glsl` CONE_GATHER_WORDS). The grid has room for every pixel.
const CONE_GATHER_WORDS: u64 = 5;

fn cone_dirs(theta: u32, c: u32) -> u64 {
    let t = u64::from(theta) << c;
    2 * t * t
}
/// Cascade sizes and word offsets in the work buffer, from the voxel layer's
/// cap: so many probes for the screen, each hash at most half full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ConeLayout {
    hash: [u64; 5],
    cap: [u64; 5],
    hash_at: [u64; 5],
    probes_at: [u64; 5],
    cones_at: [u64; 5],
    dirs: [u64; 5],
    count: u32,
    /// The lamp hash: entries (a power of two) and word offset.
    lamp_hash: u64,
    lamp_at: u64,
    /// The sun list (`cone_sun.comp`): a word a screen pixel.
    sun_at: u64,
    /// The gather grid (`cone_shade.comp`) and the fill list
    /// (`cone_fill.comp`): a word a screen pixel each.
    gather_at: u64,
    fill_at: u64,
    /// The specular list (`cone_specular.comp`): a word a screen pixel.
    specular_at: u64,
    /// Each cascade's lamp target lists ([`CONE_TARGETS`] words a probe).
    targets_at: [u64; 5],
    /// What cascade 0's probes' rays saw of their targets.
    seen_at: u64,
    words: u64,
}

/// `world` is cascade 0's room for the world layer's probes
/// ([`CONE_WORLD_CAP`]; none under the table-cap knockout).
fn cone_layout(theta: u32, count: u32, pixels: u64, probe_px: u32, lamps: u64, screen: u64, world: u64) -> ConeLayout {
    let px = u64::from(probe_px.max(1));
    let base = pixels.div_ceil(px * px).max(1) * CONE_PROBE_SLACK;
    let count = count.min(5);
    let mut l = ConeLayout { count, ..ConeLayout::default() };
    let mut at = CONE_HEADER_WORDS;
    let mut cap = base + world;
    for c in 0..count as usize {
        let k = u32::try_from(c).unwrap_or(0);
        l.cap[c] = cap.max(CONE_MIN_CAP);
        l.hash[c] = (2 * l.cap[c]).next_power_of_two();
        l.dirs[c] = cone_dirs(theta, k);
        l.hash_at[c] = at;
        at += l.hash[c] * CONE_HASH_WORDS;
        l.probes_at[c] = at;
        at += l.cap[c] * CONE_PROBE_WORDS;
        l.cones_at[c] = at;
        at += l.cap[c] * l.dirs[c] * CONE_CONE_WORDS;
        cap = cap.div_ceil(CONE_CASCADE_SHARE);
    }
    l.lamp_hash = (2 * lamps.max(1) * CONE_LAMP_ENTRIES).next_power_of_two();
    l.lamp_at = at;
    at += l.lamp_hash * CONE_LAMP_WORDS;
    l.sun_at = at;
    at += screen;
    l.gather_at = at;
    at += screen * CONE_GATHER_WORDS;
    l.fill_at = at;
    at += screen;
    l.specular_at = at;
    at += screen;
    for c in 0..count as usize {
        l.targets_at[c] = at;
        at += l.cap[c] * CONE_TARGETS;
    }
    l.seen_at = at;
    at += l.cap[0] * CONE_TARGETS;
    l.words = at;
    l
}

impl ConeLayout {
    /// The same layout `by` words further on: the other of the work buffer's
    /// two halves.
    fn shifted(&self, by: u64) -> Self {
        let mut l = *self;
        for at in l.hash_at.iter_mut().chain(&mut l.probes_at).chain(&mut l.cones_at).chain(&mut l.targets_at) {
            *at += by;
        }
        for at in [&mut l.lamp_at, &mut l.sun_at, &mut l.gather_at, &mut l.fill_at, &mut l.specular_at, &mut l.seen_at] {
            *at += by;
        }
        l
    }

    /// The words one half holds (everything past the header).
    fn half_words(&self) -> u64 {
        self.words - CONE_HEADER_WORDS
    }

    /// Last frame's tables as the shaders read them (`cone_common.glsl`
    /// CONE_PREV_LAYOUT): for each cascade its hash size, hash, cap, cones
    /// and target lists.
    fn prev_header(&self) -> Vec<u32> {
        let w = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        (0..5).flat_map(|c| [w(self.hash[c]), w(self.hash_at[c]), w(self.cap[c]), w(self.cones_at[c]), w(self.targets_at[c])]).collect()
    }

    /// The header words the shaders read it from (`cone_common.glsl`).
    fn header(&self) -> Vec<u32> {
        let w = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        (0..5)
            .flat_map(|c| [w(self.hash[c]), w(self.cap[c]), w(self.hash_at[c]), w(self.probes_at[c]), w(self.cones_at[c])])
            .chain([w(self.lamp_hash), w(self.lamp_at), w(self.sun_at)])
            .collect()
    }
}

/// The renderer's cone-cascade state.
struct ConeGpu {
    pipes: [Handle; CONE_PIPES],
    table: Buffer,
    fine: Buffer,
    records: Buffer,
    coarse: Buffer,
    work: Buffer,
    /// The world, model, lamps and lamp radius the field was built for.
    key: u64,
    building: Option<(u64, [f32; 3], std::thread::JoinHandle<ConeBuilt>)>,
    /// The eye the resident field's clip volumes are about.
    clip_eye: [f32; 3],
    /// The resident field has its fine levels (not the coarse levels alone).
    has_fine: bool,
    /// Bytes still to upload: (buffer 0..3, bytes, cursor).
    pending: Vec<(usize, Vec<u8>, usize)>,
    /// A field built while another is resident (the clip volumes about a
    /// moved eye): its four buffers fill here while the resident field stays
    /// bound, and take its place before a frame once they are full.
    next: Vec<Buffer>,
    next_full: bool,
    /// Fine and coarse bricks of the field in use, and of the one filling.
    bricks: (u64, u64),
    next_bricks: (u64, u64),
    /// This frame's eye, and last frame's when its probes are still in the
    /// work buffer (the light pass reads their irradiance).
    eye: [f32; 3],
    prev_eye: Option<[f32; 3]>,
    /// When the light pass last ran (its blend steps by real frame time), and
    /// whether the field in use is new (its voxels take their light at once).
    lit_at: Option<std::time::Instant>,
    fresh: bool,
    /// Frames recorded: which share of the world layer is traced this frame.
    frames: u32,
    ready: bool,
    /// Which half of the work buffer this frame's cascades are in (the other
    /// holds last frame's, whose cones a probe keeps until its turn), and
    /// the layout the last recorded frame used.
    half: u64,
    last_layout: ConeLayout,
    /// The work buffer's shape (theta, cascades, pixels, probe_px, lamps) and layout.
    work_shape: (u32, u32, u64, u32, u64, u64),
    /// The lamp maps' word offset in the work buffer, the lamps mapped, and
    /// whether the maps are still to be cleared (a new buffer).
    maps_at: u64,
    maps_lamps: u64,
    maps_new: bool,
    /// The field's number: up one with each new field.
    map_field: u32,
    map_started: Option<std::time::Instant>,
    /// Knockout only ([`crate::gate::CONE_LAMP_FOLLOW`]): the lamps were
    /// written once and are kept.
    lamps_frozen: bool,
    /// Lights in the scene, suns included: the lamp pass's lamp count.
    scene_lights: u32,
    layout: ConeLayout,
    /// The last build's sizes, for the debug tools.
    report: String,
    /// Each solid's pose last frame, and the solids that moved within
    /// [`CONE_MOVER_SECONDS`]: the field leaves them out.
    solid_poses: Vec<[f32; 5]>,
    movers: std::collections::BTreeMap<usize, std::time::Instant>,
    /// The solids left out of the field in use, and of the one building.
    field_movers: std::collections::BTreeSet<usize>,
    building_movers: std::collections::BTreeSet<usize>,
    /// The eye's velocity (metres a second, smoothed), when it was last
    /// read, and how long the last field took from the start of its build
    /// until it was resident: a walker's next field is built about where the
    /// eye will be when it arrives.
    eye_velocity: [f32; 3],
    eye_seen: Option<([f32; 3], std::time::Instant)>,
    build_started: Option<std::time::Instant>,
    build_seconds: f32,
    /// The pixel the shade records what it reads for ([`Gpu::cone_pick`]).
    pick: Option<[u32; 2]>,
    /// The suns' directions the faces' sun shares were marched for, and the
    /// frames of the re-march still to run after they changed.
    sun_key: u64,
    sun_turns: u32,
    /// The passes' timestamp pool ([`CONE_STAMPS`] a flight), and the flight
    /// the last recorded frame stamped.
    time_pool: Handle,
    time_flight: Option<usize>,
}

impl ConeGpu {
    fn new() -> Self {
        Self {
            pipes: [std::ptr::null_mut(); CONE_PIPES],
            table: Buffer::empty(),
            fine: Buffer::empty(),
            records: Buffer::empty(),
            coarse: Buffer::empty(),
            work: Buffer::empty(),
            key: 0,
            building: None,
            clip_eye: [0.0; 3],
            has_fine: false,
            pending: Vec::new(),
            next: Vec::new(),
            next_full: false,
            bricks: (0, 0),
            next_bricks: (0, 0),
            eye: [0.0; 3],
            prev_eye: None,
            lit_at: None,
            fresh: true,
            frames: 0,
            ready: false,
            half: 0,
            last_layout: ConeLayout::default(),
            work_shape: (0, 0, 0, 0, 0, 0),
            maps_at: 0,
            maps_lamps: 0,
            maps_new: false,
            map_field: 1,
            map_started: None,
            lamps_frozen: false,
            scene_lights: 0,
            layout: ConeLayout::default(),
            report: String::new(),
            solid_poses: Vec::new(),
            movers: std::collections::BTreeMap::new(),
            field_movers: std::collections::BTreeSet::new(),
            building_movers: std::collections::BTreeSet::new(),
            eye_velocity: [0.0; 3],
            eye_seen: None,
            build_started: None,
            build_seconds: CONE_BUILD_SECONDS,
            pick: None,
            sun_key: 0,
            sun_turns: 0,
            time_pool: std::ptr::null_mut(),
            time_flight: None,
        }
    }
}

/// What a cone-cascade field depends on: the world and its glTF model. The
/// lamps are written each frame (`cone_lamps.comp`), so a lamp that moves or
/// changes does not rebuild it.
fn cone_world_key(world: &World) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    world.identity().hash(&mut h);
    world.gltf_surface.as_ref().map(|g| g.key()).hash(&mut h);
    h.finish()
}

/// The scene's point lamps (a sun is not a sphere in the field).
fn cone_point_lamps(scene: &genos_scene::Scene) -> u64 {
    (0..scene.light_len())
        .filter(|&i| {
            let d = scene.light(i).direction;
            d.x == 0.0 && d.y == 0.0 && d.z == 0.0
        })
        .count() as u64
}

/// The share of the faces whose sun share the light pass marches again each
/// frame after a sun turned (`cone_light.comp` CONE_SUN_SHARE): the re-march
/// runs over this many frames.
const CONE_SUN_SHARE: u32 = 8;

/// What the faces' sun shares depend on besides the field: each sun's
/// direction and whether it shines.
fn cone_sun_key(scene: &genos_scene::Scene) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for i in 0..scene.light_len() {
        let light = scene.light(i);
        let d = light.direction;
        if d.x != 0.0 || d.y != 0.0 || d.z != 0.0 {
            [d.x.to_bits(), d.y.to_bits(), d.z.to_bits()].hash(&mut h);
            light.color.iter().any(|c| *c > 0.0).hash(&mut h);
        }
    }
    h.finish()
}

fn cone_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The field's four buffers as bytes: the table (head, levels and clip
/// volumes first), the
/// fine headers, the fine records, and the coarse texels
/// ([`CONE_TEXEL_WORDS`] each).
fn cone_field_bytes(field: &crate::cone_field::ConeField) -> [Vec<u8>; 4] {
    let mut table: Vec<u32> = vec![field.atlas[0], field.atlas[1], field.atlas[2], field.coarse_bricks];
    for level in &field.levels {
        table.extend(level.origin.map(|v| u32::from_ne_bytes(v.to_ne_bytes())));
        table.push(level.offset);
        table.extend(level.dims);
        table.push(0);
    }
    for level in &field.levels {
        table.extend(level.clip.map(|v| u32::from_ne_bytes(v.to_ne_bytes())));
    }
    // info: where the brick origins start in `table`, and the brick counts.
    let entries = u32::try_from(field.table.len()).unwrap_or(u32::MAX);
    let fine_count = u32::try_from(field.fine_origins.len()).unwrap_or(u32::MAX);
    let coarse_count = u32::try_from(field.coarse_origins.len()).unwrap_or(u32::MAX);
    table.extend([entries, entries + 4 * fine_count, fine_count, coarse_count]);
    table.extend_from_slice(&field.table);
    for origin in field.fine_origins.iter().chain(&field.coarse_origins) {
        table.extend(origin.map(|v| u32::from_ne_bytes(v.to_ne_bytes())));
    }
    let texels = field.occupancy.len();
    let w = CONE_TEXEL_WORDS;
    let mut coarse = vec![0u32; texels * w];
    for i in 0..texels {
        for d in 0..7 {
            let f = &field.faces[d][i * 4..i * 4 + 4];
            coarse[i * w + 2 * d] = u32::from(f[0]) | (u32::from(f[1]) << 16);
            coarse[i * w + 2 * d + 1] = u32::from(f[2]) | (u32::from(f[3]) << 16);
        }
        coarse[i * w + 14] = u32::from(field.occupancy[i] != 0);
        coarse[i * w + 15] = field.coarse_seats[i];
        let a = &field.coarse_albedo[i * 3..i * 3 + 3];
        coarse[i * w + 16] = u32::from(a[0]) | (u32::from(a[1]) << 16);
        coarse[i * w + 17] = u32::from(a[2]);
        let f = &field.coarse_planes[i * 6..i * 6 + 6];
        for k in 0..3 {
            coarse[i * w + 18 + k] = u32::from(f[2 * k]) | (u32::from(f[2 * k + 1]) << 16);
        }
        let f = &field.coarse_fronts[i * 6..i * 6 + 6];
        for k in 0..3 {
            coarse[i * w + 23 + k] = u32::from(f[2 * k]) | (u32::from(f[2 * k + 1]) << 16);
        }
    }
    [cone_bytes(&table), cone_bytes(&field.fine_bricks), cone_bytes(&field.fine_voxels), cone_bytes(&coarse)]
}

/// The field for a scene around the eye, built off the frame thread: its
/// surfaces, the fine levels in clip volumes about `eye`. The lamps are not
/// in it; they are written each frame.
/// Seconds a solid stays a mover after it last moved.
const CONE_MOVER_SECONDS: f32 = 2.0;

/// The scene without the solids `drop` names.
fn cone_without_solids(scene: &genos_scene::Scene, drop: &std::collections::BTreeSet<usize>) -> genos_scene::Scene {
    if drop.is_empty() {
        return scene.clone();
    }
    let walls = scene.walls().cloned().collect();
    let solids = scene.solids().cloned().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, s)| s).collect();
    scene.replace_parts(walls, solids, scene.lights_cloned())
}

fn cone_build(scene: &genos_scene::Scene, gltf: Option<&crate::gltf_surface::GltfSurface>, eye: [f32; 3], fine: bool) -> ConeBuilt {
    let started = std::time::Instant::now();
    let field = crate::cone_field::FieldVoxels::around_eye(scene, gltf, eye, fine).brick();
    let bytes = cone_field_bytes(&field);
    let report = format!(
        "{} points, {} voxels, {} fine bricks, {} fine records, {} coarse bricks, {} MiB, built in {:.2} s",
        field.points,
        field.voxels,
        field.fine_bricks.len() / crate::cone_field::FINE_HEADER,
        field.fine_voxels.len() / crate::cone_field::FINE_RECORD,
        field.coarse_bricks,
        bytes.iter().map(Vec::len).sum::<usize>() >> 20,
        started.elapsed().as_secs_f64()
    );
    let bricks = (u64::try_from(field.fine_origins.len()).unwrap_or(u64::MAX), u64::try_from(field.coarse_origins.len()).unwrap_or(u64::MAX));
    ConeBuilt { bytes, report: if fine { report } else { format!("coarse levels only: {report}") }, fine, bricks }
}

/// A built field as the GPU takes it, packed on the build thread.
struct ConeBuilt {
    bytes: [Vec<u8>; 4],
    report: String,
    /// False for the coarse levels alone ([`cone_build`]).
    fine: bool,
    /// Fine and coarse bricks: the light pass's threads.
    bricks: (u64, u64),
}

/// A field's build and upload time before one has been measured (seconds).
const CONE_BUILD_SECONDS: f32 = 1.5;
/// The furthest ahead of the eye a field is centred (metres): the finest
/// clip volume's width, so a walker who stops is still at its edge.
const CONE_LEAD_MOST: f32 = 8.0;
/// A field is in use from when it arrives until the next does, a build's
/// time later: its middle is where the eye is halfway through that.
const CONE_LEAD_BUILDS: f32 = 1.5;
/// A step over this in one frame is a jump of the camera, not a walk: it
/// says nothing of where the eye goes next.
const CONE_JUMP: f32 = 1.0;
/// The share of a frame's measured velocity taken into the smoothed one a
/// tenth of a second (by frame time).
const CONE_VELOCITY_SECONDS: f32 = 0.1;

/// Where the eye will be in the middle of the time a field whose build
/// starts now is in use: the eye moved on by its velocity over
/// [`CONE_LEAD_BUILDS`] of the last build's time, at most [`CONE_LEAD_MOST`]
/// an axis. A walker outran the field: built about where
/// the eye was, it arrived 1.4 s later with the eye metres past its middle,
/// near surfaces read coarser probes until the next one, and the light on
/// them changed with each field (Anthony's walk, 26 Nov 2026).
fn cone_eye_ahead(eye: [f32; 3], velocity: [f32; 3], seconds: f32) -> [f32; 3] {
    std::array::from_fn(|a| eye[a] + (velocity[a] * seconds * CONE_LEAD_BUILDS).clamp(-CONE_LEAD_MOST, CONE_LEAD_MOST))
}

/// The pixel that gathers for the square of `scale` pixels holding `pixel`:
/// the square's middle (`cone_common.glsl` `cone_gather_pixel`).
fn cone_gather_pixel(pixel: [u32; 2], scale: u32, dims: [u32; 2]) -> [u32; 2] {
    std::array::from_fn(|a| ((pixel[a] / scale) * scale + scale / 2).min(dims[a].saturating_sub(1)))
}

/// Whether the eye has moved far enough from the clip volumes' centre that
/// they are built again around it.
fn cone_clip_stale(built: [f32; 3], eye: [f32; 3]) -> bool {
    (0..3).any(|a| (built[a] - eye[a]).abs() > crate::cone_field::CLIP_MOVE)
}

/// Cone mode runs only the cascades: the old pipeline's direct pass, shadows,
/// GI, ambient fill, denoiser and compose reuse are off, and compose takes the
/// cascades' light as the surface's emitted light. The settings the user wrote stay as they are.
fn cone_effective(settings: &mut genos_scene::LightingSettings) {
    if !settings.cone.enabled {
        return;
    }
    settings.direct.enabled = false;
    settings.restir_di = false;
    settings.gi.enabled = false;
    settings.world_cache.enabled = false;
    settings.bounce.count = 0;
    settings.ambient.mode = genos_scene::AmbientMode::Off;
    settings.emissive = true;
    // Nothing carries over between frames in this mode: the denoiser's
    // history and compose reuse would hold an old frame's light.
    settings.denoiser.enabled = false;
    settings.compose_reuse = false;
}

impl Gpu {
    /// Build, upload and size the cone field for this world when cone mode is on.
    /// Note which solids moved since last frame; the solids that moved
    /// within [`CONE_MOVER_SECONDS`].
    fn note_cone_movers(&mut self, scene: &genos_scene::Scene) -> std::collections::BTreeSet<usize> {
        let now = std::time::Instant::now();
        let poses: Vec<[f32; 5]> = scene.solids().map(|s| [s.position.x, s.position.y, s.position.z, s.yaw, s.size + s.height]).collect();
        if poses.len() == self.cone.solid_poses.len() {
            for (i, (a, b)) in poses.iter().zip(&self.cone.solid_poses).enumerate() {
                if a != b {
                    self.cone.movers.insert(i, now);
                }
            }
        } else {
            self.cone.movers.clear();
        }
        self.cone.solid_poses = poses;
        self.cone.movers.retain(|_, at| now.duration_since(*at).as_secs_f32() < CONE_MOVER_SECONDS);
        self.cone.movers.keys().copied().collect()
    }

    fn sync_cone(&mut self, world: &World, eye: [f32; 3]) -> Result<(), String> {
        let cone = self.lighting.settings.cone;
        if !cone.enabled {
            return Ok(());
        }
        self.cone_swap_next()?;
        self.cone.eye = eye;
        let key = cone_world_key(world);
        // One build at a time: each holds a field, and a thread a frame once
        // ran the machine out of memory in a minute.
        // The fine levels are clip volumes about the eye: a new world, or an
        // eye that has left the middle of the volumes, builds them again
        // (the field in use stays until the new one is resident).
        // A new world first gets its coarse levels alone, which build in a
        // fraction of the time, so cone mode lights at once; the fine levels
        // follow in the next build.
        let new_world = self.cone.key != key;
        // A moving solid is left out of the field: the field is built on a
        // thread now and then, and held a mover where it stood then, so the
        // pixels on the mover started their cones inside its old copy and
        // went black (Anthony, 25 Nov 2026: the moving green box). A solid
        // that starts to move builds the field again without it.
        let movers = self.note_cone_movers(&world.scene);
        let mover_held = !movers.is_subset(&self.cone.field_movers);
        // The eye's velocity, by frame time; a jump of the camera is not one.
        let now = std::time::Instant::now();
        if let Some((last, at)) = self.cone.eye_seen {
            let dt = now.duration_since(at).as_secs_f32();
            let step: [f32; 3] = std::array::from_fn(|a| eye[a] - last[a]);
            if dt > 0.0 {
                let jumped = step.iter().any(|s| s.abs() > CONE_JUMP);
                let share = 1.0 - (-dt / CONE_VELOCITY_SECONDS).exp();
                for a in 0..3 {
                    let measured = if jumped { 0.0 } else { step[a] / dt };
                    self.cone.eye_velocity[a] += (measured - self.cone.eye_velocity[a]) * if jumped { 1.0 } else { share };
                }
            }
        }
        self.cone.eye_seen = Some((eye, now));
        // The field is built about where the eye will be when it arrives,
        // and built again when that place leaves the middle of the volumes.
        let ahead = cone_eye_ahead(eye, self.cone.eye_velocity, self.cone.build_seconds);
        let stale = new_world || mover_held || !self.cone.has_fine || cone_clip_stale(self.cone.clip_eye, ahead);
        let eye = ahead;
        if stale && self.cone.building.is_none() {
            self.cone.build_started = Some(now);
            let scene = cone_without_solids(&world.scene, &movers);
            self.cone.building_movers = movers;
            let gltf = world.gltf_surface.clone();
            let fine = !new_world;
            let handle = std::thread::Builder::new()
                .name("cone-field".into())
                .spawn(move || cone_build(&scene, gltf.as_deref(), eye, fine))
                .map_err(|e| format!("cone field thread: {e}"))?;
            self.cone.building = Some((key, eye, handle));
        }
        if self.cone.building.as_ref().is_some_and(|(_, _, h)| h.is_finished()) {
            if let Some((k, at, handle)) = self.cone.building.take() {
                let built = handle.join().map_err(|_| "cone field thread panicked".to_string())?;
                let fine = built.fine;
                self.cone_take_field(k, built)?;
                self.cone.clip_eye = at;
                self.cone.has_fine = fine;
                self.cone.field_movers = std::mem::take(&mut self.cone.building_movers);
            }
        }
        let pixels = u64::from(self.lighting.dims[0]) * u64::from(self.lighting.dims[1]);
        // Room for at least 16 lamps, so lamps added in play rarely resize it.
        let lamps = cone_point_lamps(&world.scene).max(16).next_power_of_two();
        self.cone.scene_lights = u32::try_from(world.scene.light_len()).unwrap_or(u32::MAX);
        let sun_key = cone_sun_key(&world.scene);
        if sun_key != self.cone.sun_key {
            self.cone.sun_key = sun_key;
            self.cone.sun_turns = CONE_SUN_SHARE;
        }
        // A map for each of the scene's lights (a sun's is never asked for).
        let mapped = if cone.lamp_map_tiles == 0 { 0 } else { (world.scene.light_len() as u64).max(16).next_power_of_two().min(CONE_MAP_LAMPS) };
        let shape = (cone.theta, cone.count, pixels, cone.probe_px, lamps, mapped);
        if pixels > 0 && (self.cone.work_shape != shape || self.cone.work.buffer.is_null()) {
            // Knockout only: tables for a share of the screen and no room for
            // the world layer (its room alone held a small scene's probes,
            // and the knocked-out gate stayed green).
            let guarded = crate::gate::guard(crate::gate::CONE_TABLE_CAP);
            let sized = if guarded { pixels } else { pixels / CONE_KNOCKOUT_SHARE };
            let layout = cone_layout(cone.theta, cone.count, sized, cone.probe_px, lamps, pixels, if guarded { CONE_WORLD_CAP } else { 0 });
            // Two halves: this frame's cascades and last frame's.
            let maps_at = CONE_HEADER_WORDS + 2 * layout.half_words();
            let words = maps_at + CONE_MAP_LIST + mapped * CONE_MAP_LAMP_WORDS;
            self.idle_device();
            let mut old = std::mem::replace(&mut self.cone.work, Buffer::empty());
            self.destroy_buffer(&mut old);
            let share = self.light_families();
            self.cone.work = self.make_buffer_queues(words * 4, 0x20 | 0x2, Memory::Device, &share)?;
            self.cone.work_shape = shape;
            self.cone.maps_at = maps_at;
            self.cone.maps_lamps = mapped;
            self.cone.maps_new = true;
            self.cone.layout = layout;
            self.cone.prev_eye = None;
            if self.lighting.ready {
                self.write_lighting_sets()?;
            }
        }
        if self.cone.time_pool.is_null() && self.timestamp_period > 0.0 {
            self.cone.time_pool = self.make_query_pool_n(2 * CONE_STAMPS)?;
        }
        if self.cone.pipes[0].is_null() && self.lighting.ready {
            let codes: [&[u8]; CONE_PIPES] = [CONE_INSERT_SPV, CONE_COMPACT_SPV, CONE_CHAIN_SPV, CONE_TRACE_SPV, CONE_WEIGHTS_SPV, CONE_MERGE_SPV, CONE_SHADE_SPV, CONE_LAMPS_SPV, CONE_SPREAD_SPV, CONE_LIGHT_SPV, CONE_WORLD_SPV, CONE_SUN_SPV, CONE_RESOLVE_SPV, CONE_FILL_SPV, CONE_TARGETS_SPV, CONE_LAMP_SPV, CONE_MAP_LIST_SPV, CONE_MAP_TRACE_SPV, CONE_SPECULAR_SPV];
            for (k, code) in codes.iter().enumerate() {
                self.cone.pipes[k] = self.make_compute_pipe(code, self.lighting.layout, CONE_PIPE_NAMES[k])?;
            }
        }
        Ok(())
    }

    /// A built field: new buffers, and its bytes queued for upload.
    fn cone_take_field(&mut self, key: u64, built: ConeBuilt) -> Result<(), String> {
        let ConeBuilt { bytes, report, bricks, .. } = built;
        self.cone.report = report;
        let share = self.light_families();
        if self.cone.ready && self.cone.next.is_empty() {
            // A field is resident and lighting: fill the new one beside it.
            for b in &bytes {
                let size = u64::try_from(b.len().max(16)).unwrap_or(16);
                let made = self.make_buffer_queues(size, 0x20 | 0x2, Memory::Device, &share)?;
                self.cone.next.push(made);
            }
            self.cone.pending = bytes.into_iter().enumerate().map(|(k, b)| (k, b, 0)).collect();
            self.cone.next_full = false;
            self.cone.next_bricks = bricks;
            self.cone.key = key;
            return Ok(());
        }
        self.idle_device();
        for (k, b) in bytes.iter().enumerate() {
            let target = match k {
                0 => &mut self.cone.table,
                1 => &mut self.cone.fine,
                2 => &mut self.cone.records,
                _ => &mut self.cone.coarse,
            };
            let mut old = std::mem::replace(target, Buffer::empty());
            self.destroy_buffer(&mut old);
            let size = u64::try_from(b.len().max(16)).unwrap_or(16);
            let made = self.make_buffer_queues(size, 0x20 | 0x2, Memory::Device, &share)?;
            match k {
                0 => self.cone.table = made,
                1 => self.cone.fine = made,
                2 => self.cone.records = made,
                _ => self.cone.coarse = made,
            }
        }
        self.cone.pending = bytes.into_iter().enumerate().map(|(k, b)| (k, b, 0)).collect();
        self.cone.bricks = bricks;
        self.cone.fresh = true;
        self.cone.ready = false;
        self.cone.lamps_frozen = false;
        self.cone.key = key;
        if self.lighting.ready {
            self.write_lighting_sets()?;
        }
        Ok(())
    }

    /// Record the next part of the field's upload into this frame.
    fn cone_upload(&mut self) -> Result<(), String> {
        if self.cone.pending.is_empty() {
            return Ok(());
        }
        let mut budget = CONE_UPLOAD_STEP;
        let cmd = self.cmd;
        let mut pending = std::mem::take(&mut self.cone.pending);
        for (k, bytes, cursor) in &mut pending {
            if budget == 0 {
                break;
            }
            let end = (*cursor + budget).min(bytes.len());
            if end > *cursor {
                let dst = match (self.cone.next.get(*k), *k) {
                    (Some(next), _) => next.buffer,
                    (None, 0) => self.cone.table.buffer,
                    (None, 1) => self.cone.fine.buffer,
                    (None, 2) => self.cone.records.buffer,
                    (None, _) => self.cone.coarse.buffer,
                };
                self.write_device(cmd, dst, u64::try_from(*cursor).unwrap_or(0), &bytes[*cursor..end])?;
                budget -= end - *cursor;
                *cursor = end;
            }
        }
        pending.retain(|(_, bytes, cursor)| *cursor < bytes.len());
        self.cone.pending = pending;
        if self.cone.pending.is_empty() {
            self.memory_barrier(0x1000, 0x800, 0x1000, 0x20);
            if self.cone.next.is_empty() {
                self.cone.ready = true;
            } else {
                self.cone.next_full = true;
            }
            if let Some(started) = self.cone.build_started.take() {
                self.cone.build_seconds = started.elapsed().as_secs_f32().clamp(0.1, 5.0);
            }
        }
        Ok(())
    }

    /// Put a filled next field in the resident one's place, before a frame
    /// is recorded (the lighting sets are written again).
    fn cone_swap_next(&mut self) -> Result<(), String> {
        if !self.cone.next_full {
            return Ok(());
        }
        self.idle_device();
        let mut next = std::mem::take(&mut self.cone.next);
        for (k, made) in next.drain(..).enumerate() {
            let target = match k {
                0 => &mut self.cone.table,
                1 => &mut self.cone.fine,
                2 => &mut self.cone.records,
                _ => &mut self.cone.coarse,
            };
            let mut old = std::mem::replace(target, made);
            self.destroy_buffer(&mut old);
        }
        self.cone.next_full = false;
        self.cone.bricks = self.cone.next_bricks;
        self.cone.fresh = true;
        self.cone.lamps_frozen = false;
        if self.lighting.ready {
            self.write_lighting_sets()?;
        }
        Ok(())
    }

    fn cone_dispatch(&self, pipe: usize, push: [u32; 12], groups: (u32, u32)) {
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, self.cone.pipes[pipe]);
            (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, LIGHTING_PC_BYTES, push.as_ptr().cast::<c_void>());
            (self.fns.cmd_dispatch)(self.cmd, groups.0.max(1), groups.1.max(1), 1);
        }
    }

    /// Write cone timestamp `index` of this frame (0 resets the frame's set).
    fn cone_stamp(&self, index: u32) {
        if self.cone.time_pool.is_null() {
            return;
        }
        let first = u32::try_from(self.flight).unwrap_or(0) * CONE_STAMPS;
        unsafe {
            if index == 0 {
                (self.fns.cmd_reset_query)(self.cmd, self.cone.time_pool, first, CONE_STAMPS);
            }
            // After every earlier command has finished (the bottom of the pipe).
            (self.fns.cmd_write_timestamp)(self.cmd, 0x2000, self.cone.time_pool, first + index);
        }
    }

    /// Milliseconds each stage of the last recorded frame's cone passes took
    /// on the GPU, by [`CONE_STAGE_NAMES`]. Waits for that frame. Empty when
    /// cone mode has not run or the device has no timestamps.
    pub(crate) fn cone_pass_ms(&mut self) -> Vec<(&'static str, f64)> {
        let Some(flight) = self.cone.time_flight else {
            return Vec::new();
        };
        if self.cone.time_pool.is_null() {
            return Vec::new();
        }
        self.idle_device();
        let mut raw = [0u64; CONE_STAGES + 1];
        let got = unsafe {
            (self.fns.get_query_results)(
                self.device,
                self.cone.time_pool,
                u32::try_from(flight).unwrap_or(0) * CONE_STAMPS,
                CONE_STAMPS,
                std::mem::size_of_val(&raw),
                raw.as_mut_ptr().cast::<c_void>(),
                8,
                0x1,
            )
        };
        if got != 0 {
            return Vec::new();
        }
        CONE_STAGE_NAMES
            .iter()
            .enumerate()
            .map(|(k, name)| {
                // A stage is well under the 4 s of ticks a u32 holds.
                let ticks = u32::try_from(tick_delta(raw[k], raw[k + 1], self.timestamp_bits)).unwrap_or(u32::MAX);
                (*name, f64::from(ticks) * f64::from(self.timestamp_period) * 1.0e-6)
            })
            .collect()
    }

    /// The cascades for this frame, before compose: probes, chains, cones,
    /// merge and shade. Does nothing until the field is resident.
    fn record_cone(&mut self) {
        let cone = self.lighting.settings.cone;
        if !cone.enabled || !self.cone.ready || self.cone.pipes[0].is_null() || self.cone.work.buffer.is_null() || self.cone.work_shape.0 != cone.theta || self.cone.work_shape.1 != cone.count || self.cone.work_shape.3 != cone.probe_px {
            self.cone.prev_eye = None;
            self.cone.time_flight = None;
            return;
        }
        self.cone.time_flight = Some(self.flight);
        self.cone_stamp(0);
        let (w, h) = (self.lighting.dims[0], self.lighting.dims[1]);
        let (theta, count) = (cone.theta, cone.count);
        let seat = if crate::gate::guard(crate::gate::CONE_FIELD_SEAT) { 0 } else { CONE_PIXEL_SEAT };
        let flags = CONE_SEE_MERGE | CONE_SEE_SHADE | seat;
        let radius = cone.lamp_radius.to_bits();
        let finest = cone.finest_probe_level();
        let view = match cone.view {
            genos_scene::ConeView::Light => 0,
            genos_scene::ConeView::Coverage => 1,
            genos_scene::ConeView::Dark => 2,
            genos_scene::ConeView::Specular => 3,
        };
        // Knockout only ([`crate::gate::CONE_SPECULAR`]): no specular light,
        // as before a metal had any.
        let specular = cone.specular && crate::gate::guard(crate::gate::CONE_SPECULAR);
        let flags = flags | (view << CONE_VIEW_SHIFT) | if specular { CONE_SPECULAR_FLAG } else { 0 };
        let push = |c: u32| [w, h, finest, 0, c, theta, count, cone.probe_px, flags, CONE_T0.to_bits(), radius, 0];
        // Knockout only: the lamps written once and kept, as when the field held them.
        let write_lamps = crate::gate::guard(crate::gate::CONE_LAMP_FOLLOW) || !self.cone.lamps_frozen;
        // This frame's half of the work buffer, and last frame's.
        let base = self.cone.layout;
        let layout = base.shifted(self.cone.half * base.half_words());
        let before = base.shifted((1 - self.cone.half) * base.half_words());
        self.cone.half = 1 - self.cone.half;
        self.cone.last_layout = layout;
        let header = layout.header();
        let prev_header = before.prev_header();
        let line = |n: u64| -> (u32, u32) {
            let groups = n.div_ceil(64);
            let x = groups.min(65_535);
            (u32::try_from(x).unwrap_or(1), u32::try_from(groups.div_ceil(x.max(1))).unwrap_or(1))
        };
        // The light pass first: it lights the field's voxels from this
        // frame's lamps and last frame's probes, which are still in the work
        // buffer, keyed from last frame's eye.
        self.memory_barrier(0x800, 0x1000, 0x20 | 0x40, 0x1000);
        let prev = self.cone.prev_eye.map_or([0u32; 4], |e| [e[0].to_bits(), e[1].to_bits(), e[2].to_bits(), 1]);
        unsafe {
            (self.fns.cmd_update_buffer)(self.cmd, self.cone.work.buffer, CONE_PREV_EYE_AT * 4, 16, prev.as_ptr().cast::<c_void>());
        }
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
        let (fine_bricks, coarse_bricks) = self.cone.bricks;
        let now = std::time::Instant::now();
        // The lamp maps' head: where they are, the time, the frame's and
        // the field's numbers, the frame's budget of tiles.
        let maps_at = self.cone.maps_at;
        let maps_lamps = self.cone.maps_lamps;
        if self.cone.fresh {
            self.cone.map_field = self.cone.map_field.wrapping_add(1).max(1);
        }
        let map_ms = now.duration_since(*self.cone.map_started.get_or_insert(now)).as_millis();
        let map_tiles = cone.lamp_map_tiles;
        let map_head = [
            u32::try_from(maps_at + CONE_MAP_LIST).unwrap_or(0),
            u32::try_from(maps_lamps).unwrap_or(0),
            u32::try_from(map_ms & 0x7fff_ffff).unwrap_or(0),
            self.cone.frames.wrapping_add(1).max(1),
            self.cone.map_field,
            map_tiles,
            u32::try_from(maps_at).unwrap_or(0),
            u32::try_from(CONE_MAP_LIST).unwrap_or(0),
            cone.lamp_radius.to_bits(),
        ];
        let dt = self.cone.lit_at.map_or(1.0, |t| now.duration_since(t).as_secs_f32().min(1.0));
        let fresh = self.cone.fresh;
        // A probe traces its cones one frame in `period` and keeps them the
        // others (cone_trace.comp): every frame while a field is new or no
        // last frame is in the buffer. (The light pass stays every frame: a
        // voxel lit one frame in a few fell out of step with the world
        // layer's turns, some never met their probe, and the picture after
        // a turn away and back differed in 110,000 pixels.) At least every second frame even when
        // frames are long: with a period of one a slow frame did all the
        // work and stayed slow.
        let period: u32 = if fresh || prev[3] != 1 {
            1
        } else if dt * 3.0 <= CONE_BOUNCE_SECONDS {
            CONE_KEEP_FRAMES
        } else if dt * 2.0 <= CONE_BOUNCE_SECONDS {
            3
        } else {
            2
        };
        let toward = if fresh { 1.0 } else { 1.0 - (-dt / CONE_BOUNCE_SECONDS).exp() };
        self.cone.lit_at = Some(now);
        self.cone.fresh = false;
        let mut light_push = push(0);
        light_push[11] = toward.to_bits();
        // The frame's count for the sun shares' turns; bit 31 marches every
        // face (a new field), bit 30 this frame's share of them (a sun
        // turned within the last CONE_SUN_SHARE frames).
        let turning = self.cone.sun_turns > 0;
        self.cone.sun_turns = if fresh { 0 } else { self.cone.sun_turns.saturating_sub(1) };
        light_push[4] = (self.cone.frames & 0x3fff_ffff) | if fresh { 0x8000_0000 } else { 0 } | if turning { 0x4000_0000 } else { 0 };
        self.cone_dispatch(9, light_push, line(fine_bricks * CONE_BRICK_VOXELS + coarse_bricks * CONE_PADDED_TEXELS));
        self.cone_stamp(1);
        // The clears below are transfers: order them after every compute read
        // and write of the work buffer before them, the light pass's included;
        // the field's new light is read by the passes after them.
        self.memory_barrier(0x800, 0x1000 | 0x800, 0x20 | 0x40, 0x1000 | 0x20);
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, 0, CONE_CLEARED * 4, 0);
            (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, CONE_MAP_COUNT_AT * 4, CONE_MAP_COUNTERS * 4, 0);
            if maps_lamps > 0 && self.cone.maps_new {
                (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, maps_at * 4, (CONE_MAP_LIST + maps_lamps * CONE_MAP_LAMP_WORDS) * 4, 0);
            }
            (self.fns.cmd_update_buffer)(self.cmd, self.cone.work.buffer, CONE_MAP_HEAD_AT * 4, 36, map_head.as_ptr().cast::<c_void>());
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.cone.work.buffer,
                CONE_LAYOUT_AT * 4,
                (header.len() * 4) as u64,
                header.as_ptr().cast::<c_void>(),
            );
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.cone.work.buffer,
                CONE_PREV_LAYOUT_AT * 4,
                (prev_header.len() * 4) as u64,
                prev_header.as_ptr().cast::<c_void>(),
            );
            let merged = before.cones_at.map(|at| u32::try_from(at).unwrap_or(u32::MAX));
            (self.fns.cmd_update_buffer)(self.cmd, self.cone.work.buffer, CONE_MERGED_LAYOUT_AT * 4, 20, merged.as_ptr().cast::<c_void>());
            let gather = [u32::try_from(layout.gather_at).unwrap_or(u32::MAX), u32::try_from(layout.fill_at).unwrap_or(u32::MAX)];
            (self.fns.cmd_update_buffer)(self.cmd, self.cone.work.buffer, CONE_GATHER_LAYOUT_AT * 4, 8, gather.as_ptr().cast::<c_void>());
            let specular_at = [u32::try_from(layout.specular_at).unwrap_or(u32::MAX)];
            (self.fns.cmd_update_buffer)(self.cmd, self.cone.work.buffer, CONE_SPECULAR_LAYOUT_AT * 4, 4, specular_at.as_ptr().cast::<c_void>());
            let mut targets = [0u32; 6];
            for (word, at) in targets.iter_mut().zip(layout.targets_at.iter().chain(std::iter::once(&layout.seen_at))) {
                *word = u32::try_from(*at).unwrap_or(u32::MAX);
            }
            (self.fns.cmd_update_buffer)(self.cmd, self.cone.work.buffer, CONE_TARGET_LAYOUT_AT * 4, 24, targets.as_ptr().cast::<c_void>());
            (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, layout.seen_at * 4, layout.cap[0] * CONE_TARGETS * 4, 0);
            for c in 0..count as usize {
                (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, layout.hash_at[c] * 4, layout.hash[c] * CONE_HASH_WORDS * 4, 0);
                (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, layout.targets_at[c] * 4, layout.cap[c] * CONE_TARGETS * 4, 0);
            }

        }
        let compute = |gpu: &Self| gpu.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
        let pixels = (w.div_ceil(8), h.div_ceil(8));
        // (The lamps are not written into the field's hash: the cascade-0
        // probes take them from the light grid, cone_lamp.comp.)
        self.cone_stamp(2);
        // Cascade 0 from the pixels, then each cascade from the one below it
        // (its compact first, so its probes are numbered).
        self.cone_dispatch(0, push(0), pixels);
        compute(self);
        self.cone_stamp(3);
        // The world layer: probes on the surfaces around the eye no pixel covers.
        let mut world_push = push(0);
        world_push[11] = self.cone.frames;
        // The last frame before a new field takes over: every world probe,
        // not this frame's share. The new field's voxels take their light at
        // once from this frame's probes, and a face with none took the sun
        // alone: three quarters of the faces off screen lost their bounce
        // for that frame and the picture dipped (gate G).
        world_push[4] = u32::from(self.cone.next_full);
        self.cone.frames = self.cone.frames.wrapping_add(1);
        self.cone_dispatch(10, world_push, line(coarse_bricks * CONE_PADDED_TEXELS));
        compute(self);
        self.cone_stamp(4);
        for c in 0..count {
            if c > 0 {
                self.cone_dispatch(8, push(c), line(layout.cap[c as usize - 1]));
                compute(self);
            }
            self.cone_dispatch(1, push(c), line(layout.hash[c as usize]));
            compute(self);
        }
        self.cone_stamp(5);
        for c in 1..count {
            self.cone_dispatch(2, push(c), line(layout.cap[c as usize]));
            compute(self);
        }
        self.cone_stamp(6);
        for c in 0..5 {
            if c < count {
                let mut trace_push = push(c);
                trace_push[3] = self.cone.frames;
                trace_push[11] = period;
                self.cone_dispatch(3, trace_push, line(layout.cap[c as usize] * layout.dirs[c as usize]));
                compute(self);
            }
            self.cone_stamp(7 + c);
        }
        for c in 0..count.saturating_sub(1) {
            self.cone_dispatch(4, push(c), line(layout.cap[c as usize]));
        }
        compute(self);
        self.cone_stamp(12);
        // Every cascade's merged records, from the coarsest down (its own
        // are its merged ones), into the other half of the buffer: the
        // trace has kept what it keeps of last frame's records there.
        for c in (0..count).rev() {
            self.cone_dispatch(5, push(c), line(layout.cap[c as usize] * layout.dirs[c as usize]));
            compute(self);
        }
        self.cone_stamp(13);
        // Each cascade-0 probe's rays at its lamp targets. Knockout only
        // ([`crate::gate::CONE_LAMP_FOLLOW`]): once, then never again.
        if write_lamps {
            self.cone_dispatch(15, push(0), line(layout.cap[0]));
            compute(self);
            if maps_lamps > 0 {
                // The lamps that moved, the tiles asked for, the tiles to
                // trace, their rays, then each probe's own share.
                let mut map_push = push(0);
                self.cone_dispatch(16, map_push, line(maps_lamps));
                compute(self);
                for mode in 1..4 {
                    map_push[3] = mode;
                    self.cone_dispatch(16, map_push, line(maps_lamps * CONE_MAP_TILES));
                    compute(self);
                }
                let most = (u64::from(map_tiles) * 2 + maps_lamps * 16).min(CONE_MAP_LIST);
                self.cone_dispatch(17, push(0), line(most * CONE_MAP_TILE_TEXELS));
                compute(self);
                let mut seen_push = push(0);
                seen_push[3] = 1;
                self.cone_dispatch(15, seen_push, line(layout.cap[0]));
                compute(self);
                self.cone.maps_new = false;
            }
            self.cone.lamps_frozen = true;
        }
        self.cone_stamp(14);
        // The gather at one pixel of each square of `scale`, the resolve to
        // every pixel, the fill for the pixels with no match, then the
        // shadow-edge pixels' own sun rays.
        let scale = cone.shade_scale.clamp(1, 4);
        let pick = self.cone.pick.filter(|p| p[0] < w && p[1] < h).map_or(0, |p| {
            let [x, y] = cone_gather_pixel(p, scale, [w, h]);
            y * w + x + 1
        });
        let mut shade_push = push(0);
        shade_push[3] = pick;
        shade_push[11] = scale;
        self.cone_dispatch(6, shade_push, (w.div_ceil(scale).div_ceil(8), h.div_ceil(scale).div_ceil(8)));
        compute(self);
        self.cone_stamp(15);
        let mut spread_push = push(0);
        spread_push[11] = scale;
        self.cone_dispatch(12, spread_push, pixels);
        compute(self);
        self.cone_stamp(16);
        self.cone_dispatch(13, spread_push, line(u64::from(w) * u64::from(h)));
        compute(self);
        self.cone_stamp(17);
        self.cone_dispatch(11, push(0), line(u64::from(w) * u64::from(h)));
        compute(self);
        self.cone_stamp(18);
        // The metal pixels the resolve listed: one specular cone each. With
        // the specular light off none is listed and the pass is not run.
        if specular {
            self.cone_dispatch(18, push(0), line(u64::from(w) * u64::from(h)));
            compute(self);
        }
        self.cone_stamp(19);
        self.cone.prev_eye = Some(self.cone.eye);
    }

    /// Last frame's counts for each cascade. Waits for that frame. Empty
    /// when cone mode has not run.
    pub(crate) fn cone_stats(&mut self) -> Result<Vec<crate::ConeCascadeStats>, String> {
        let layout = self.cone.layout;
        let count = layout.count;
        if self.cone.work.buffer.is_null() || count == 0 {
            return Ok(Vec::new());
        }
        self.idle_device();
        let raw = self.copy_device_range(self.cone.work.buffer, 0, 64 * 4)?;
        let w = u32_words(&raw);
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        Ok((0..count)
            .map(|c| {
                let i = c as usize;
                let cap = u32::try_from(layout.cap[i]).unwrap_or(u32::MAX);
                let probes = at(i);
                crate::ConeCascadeStats {
                    probes,
                    cap,
                    dropped_compact: at(8 + i),
                    dropped_insert: at(32 + i),
                    cones_traced: at(16 + i),
                    cones_closed: at(24 + i),
                    cones_kept: at(56 + i),
                    seat_fallbacks: at(40 + i),
                }
            })
            .collect())
    }

    /// What the pixel at `x`, `y` read in the last frame's shade: its levels
    /// and, for each, the eight probes about it. The first call for a pixel
    /// sets it; the record is there after the next frame (`None` until then).
    pub(crate) fn cone_pick(&mut self, x: u32, y: u32) -> Result<Option<crate::ConePick>, String> {
        self.cone.pick = Some([x, y]);
        if self.cone.work.buffer.is_null() {
            return Ok(None);
        }
        self.idle_device();
        let words = CONE_PICK_HEAD + 2 * CONE_PICK_LEVEL;
        let all = CONE_PICK_UP_AT - usize::try_from(CONE_PICK_AT).unwrap_or(0) + 64;
        let up_at = CONE_PICK_UP_AT - usize::try_from(CONE_PICK_AT).unwrap_or(0);
        let w = u32_words(&self.copy_device_range(self.cone.work.buffer, CONE_PICK_AT * 4, u64::try_from(all * 4).unwrap_or(0))?);
        let dirs_at = CONE_PICK_DIRS_AT - usize::try_from(CONE_PICK_AT).unwrap_or(0);
        // The pick is of the pixel that gathers for this one's square.
        let dims = [self.lighting.dims[0], self.lighting.dims[1]];
        let [x, y] = cone_gather_pixel([x, y], self.lighting.settings.cone.shade_scale.clamp(1, 4), dims);
        let pixel = y * dims[0] + x + 1;
        if w.len() < words || w[0] != pixel {
            return Ok(None);
        }
        let f = |i: usize| f32::from_bits(w[i]);
        let f3 = |i: usize| [f(i), f(i + 1), f(i + 2)];
        let levels = (0..2)
            .map(|l| CONE_PICK_HEAD + l * CONE_PICK_LEVEL)
            .filter(|&at| w[at + 6] != 0)
            .map(|at| crate::ConePickLevel {
                own_up: (0..8)
                    .map(|j| up_at + (at - CONE_PICK_HEAD) / CONE_PICK_LEVEL * 32 + 4 * j)
                    .filter(|&i| f32::from_bits(w[i + 3]) >= 0.0)
                    .map(|i| ([f32::from_bits(w[i]), f32::from_bits(w[i + 1]), f32::from_bits(w[i + 2])], f32::from_bits(w[i + 3])))
                    .collect(),
                own_dirs: (0..CONE_PICK_DIRS)
                    .map(|k| dirs_at + (at - CONE_PICK_HEAD) / CONE_PICK_LEVEL * 2 * CONE_PICK_DIRS + 2 * k)
                    .map(|i| {
                        let (luma, open) = crate::cone_field::unpack_half2_pub(w[i]);
                        let way = w[i + 1].to_le_bytes().map(|b| f32::from(i8::from_le_bytes([b])) / 127.0);
                        crate::ConePickCone { way: [way[0], way[1], way[2]], luma, open }
                    })
                    .collect(),
                lod: w[at],
                kept: w[at + 1],
                own: [w[at + 2].cast_signed(), w[at + 3].cast_signed(), w[at + 4].cast_signed()],
                probes: (0..8)
                    .map(|k| at + 8 + k * CONE_PICK_PROBE)
                    .map(|r| crate::ConePickProbe {
                        state: match (w[r] & 1 != 0, w[r] & 2 != 0) {
                            (false, _) => crate::ConePickState::Absent,
                            (true, false) => crate::ConePickState::Unseated,
                            (true, true) => crate::ConePickState::Seated,
                        },
                        own: w[r] & 4 != 0,
                        surface: w[r] & 8 != 0,
                        probe: w[r + 1],
                        weight: f(r + 2),
                        passed: f(r + 3),
                        dark_dirs: w[r + 4],
                        lit_dirs: w[r + 9],
                        mean_luma: f(r + 5),
                        mean_open: f(r + 10),
                        outward_luma: f(r + 11),
                        seat: f3(r + 6),
                    })
                    .collect(),
            })
            .collect();
        Ok(Some(crate::ConePick {
            lo: w[1],
            hi: w[2],
            hi_weight: f(3),
            position: f3(4),
            normal: f3(7),
            face: w[10],
            field_sun: f(11),
            irradiance: f3(12),
            levels,
        }))
    }

    /// Lamp points that found no lamp-hash slot last frame. Waits for that frame.
    pub(crate) fn cone_lamp_drops(&mut self) -> Result<u32, String> {
        if self.cone.work.buffer.is_null() {
            return Ok(0);
        }
        self.idle_device();
        let raw = self.copy_device_range(self.cone.work.buffer, 48 * 4, 4)?;
        Ok(u32_words(&raw).first().copied().unwrap_or(0))
    }

    /// The lamp maps' tiles last frame, a level (coarsest first): those
    /// asked for whose lamp had moved, and those listed for tracing. Waits
    /// for that frame.
    pub(crate) fn cone_map_tiles(&mut self) -> Result<[[u32; CONE_MAP_LEVELS]; 2], String> {
        let mut out = [[0u32; CONE_MAP_LEVELS]; 2];
        if self.cone.work.buffer.is_null() {
            return Ok(out);
        }
        self.idle_device();
        let raw = self.copy_device_range(self.cone.work.buffer, (CONE_MAP_COUNT_AT + 1) * 4, 2 * CONE_MAP_LEVELS as u64 * 4)?;
        for (i, word) in u32_words(&raw).iter().enumerate().take(2 * CONE_MAP_LEVELS) {
            out[i / CONE_MAP_LEVELS][i % CONE_MAP_LEVELS] = *word;
        }
        Ok(out)
    }

    /// Pixels the resolve listed for the specular pass last frame (pixels
    /// with metal under the rough band). Waits for that frame.
    pub(crate) fn cone_specular_pixels(&mut self) -> Result<u32, String> {
        if self.cone.work.buffer.is_null() {
            return Ok(0);
        }
        self.idle_device();
        let raw = self.copy_device_range(self.cone.work.buffer, CONE_SPECULAR_COUNT_AT * 4, 4)?;
        Ok(u32_words(&raw).first().copied().unwrap_or(0))
    }

    /// Cascade `c`'s probes as last frame wrote them. Waits for that frame.
    pub(crate) fn cone_probes(&mut self, c: u32) -> Result<Vec<crate::ConeProbe>, String> {
        let layout = self.cone.layout;
        if self.cone.work.buffer.is_null() || c >= layout.count {
            return Ok(Vec::new());
        }
        self.idle_device();
        let head = u32_words(&self.copy_device_range(self.cone.work.buffer, 0, 8 * 4)?);
        let kept = u64::from(head.get(c as usize).copied().unwrap_or(0)).min(layout.cap[c as usize]);
        if kept == 0 {
            return Ok(Vec::new());
        }
        let at = self.cone.last_layout.probes_at[c as usize];
        let raw = self.copy_device_range(self.cone.work.buffer, at * 4, kept * CONE_PROBE_WORDS * 4)?;
        let w = u32_words(&raw);
        let stride = usize::try_from(CONE_PROBE_WORDS).unwrap_or(32);
        Ok(w.chunks_exact(stride)
            .map(|r| crate::ConeProbe {
                key: r[0],
                face: (r[0] >> 24) & 7,
                lod: r[1],
                cell: [r[4].cast_signed(), r[5].cast_signed(), r[6].cast_signed()],
                pixels: r[7],
                seat: [f32::from_bits(r[8]), f32::from_bits(r[9]), f32::from_bits(r[10])],
            })
            .collect())
    }

    fn destroy_cone(&mut self) {
        for pipe in &mut self.cone.pipes {
            if !pipe.is_null() {
                unsafe { (self.fns.destroy_pipeline)(self.device, *pipe, std::ptr::null()) };
                *pipe = std::ptr::null_mut();
            }
        }
        for k in 0..5 {
            let mut old = std::mem::replace(
                match k {
                    0 => &mut self.cone.table,
                    1 => &mut self.cone.fine,
                    2 => &mut self.cone.records,
                    3 => &mut self.cone.coarse,
                    _ => &mut self.cone.work,
                },
                Buffer::empty(),
            );
            self.destroy_buffer(&mut old);
        }
        let mut next = std::mem::take(&mut self.cone.next);
        for buffer in &mut next {
            self.destroy_buffer(buffer);
        }
        if !self.cone.time_pool.is_null() {
            unsafe { (self.fns.destroy_query_pool)(self.device, self.cone.time_pool, std::ptr::null()) };
            self.cone.time_pool = std::ptr::null_mut();
            self.cone.time_flight = None;
        }
        self.cone.next_full = false;
        self.cone.ready = false;
        self.cone.key = 0;
        self.cone.has_fine = false;
        self.cone.pending.clear();
    }
}
