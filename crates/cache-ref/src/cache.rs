//! The world cache under test: cells on a lattice, each an SH-L1 of incoming
//! radiance fed one way by rays from surface points in the cell. Stage 2b adds
//! in-cell position data; stage 2c splits cells one level.
//!
//! Every frame each live cell casts `rays_per_cell` uniform-hemisphere rays
//! from surface points it owns. A hit returns `albedo / pi * (direct + cache)`
//! where `cache` is the previous frame's resolved cache read at the hit
//! (read previous, write next). The frame mean is merged into history capped at
//! `history_frames * rays_per_cell` samples.

use std::collections::HashMap;
use std::f64::consts::PI;

use crate::geom::{fl, uniform_hemi, EPS, V3};
use crate::lattice::{Lattice, Layout, LookupCost};
use crate::rng::Rng;
use crate::scene::Scene;
use crate::sh::{self, Encoding, Y0};

/// How ray origins are picked inside a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// Uniform over the cell's surface pixels.
    Uniform,
    /// Must-fail F2: `q ∝ 1 + 8 hardness` (short-ray occlusion), unweighted.
    SkewUnweighted,
    /// F2 control: the same `q` with Hájek `1/q` weights.
    SkewWeighted,
}

/// Must-fail F3: texels hold an EMA of what they showed and feed it back into
/// the cell resolve. Texels are not cleared on a lamp change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexelFeedback {
    /// Texel EMA keep factor per frame.
    pub alpha: f64,
    /// Share of the cell's L0 replaced by the texel mean each resolve.
    pub beta: f64,
}

/// How a split seeds its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// Children start empty.
    Zero,
    /// Children copy the parent cell.
    Copy,
    /// Children take the parent's 2b model at their own centroid.
    Model,
}

/// Stage 2c: one subdivision level with hysteresis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subdiv {
    /// Split when the camera is nearer than this (metres).
    pub split_dist: f64,
    /// Merge only beyond `split_dist * hysteresis`.
    pub hysteresis: f64,
    /// Also split on data: `|g| h / L0 > tau` with at least `n_split` samples.
    /// `None` = camera trigger only.
    pub tau: Option<f64>,
    pub n_split: f64,
    /// Merge needs the data below `tau / 2` for this many frames.
    pub merge_frames: usize,
    /// Frames a cell keeps its state before it may change again.
    pub min_dwell: usize,
    pub seed: Seed,
    /// Sample count children are seeded with (low confidence).
    pub seed_n: f64,
    /// Cross-fade: child weight `n / (n + fade_k)` against the parent.
    pub fade_k: f64,
    /// Children below `boost_min_n` samples cast `boost_rays` extra rays.
    pub boost_min_n: f64,
    pub boost_rays: usize,
}

impl Default for Subdiv {
    fn default() -> Self {
        Self {
            split_dist: 1.5,
            hysteresis: 1.3,
            tau: Some(0.5),
            n_split: 128.0,
            merge_frames: 30,
            min_dwell: 30,
            seed: Seed::Model,
            seed_n: 8.0,
            fade_k: 32.0,
            boost_min_n: 64.0,
            boost_rays: 16,
        }
    }
}

/// Every knob of a cache. Nothing in the cache is hard-coded beyond this.
#[derive(Clone, Copy, Debug, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent experiment toggles that the comparisons combine freely"
)]
pub struct CacheConfig {
    pub layout: Layout,
    /// Cubic-equivalent cell size (metres); BCC keeps the same point density.
    pub cell_size: f64,
    pub rays_per_cell: usize,
    /// History cap in frames.
    pub history_frames: f64,
    /// History kept on a lamp change, in frames.
    pub clear_keep_frames: f64,
    /// Frames after a lamp change during which the history cap stays at
    /// `clear_keep_frames`, so stale bounces flush instead of being averaged
    /// back in (stand-in for the stage 1 convergence fix).
    pub recover_frames: usize,
    /// Key on the sign of the normal along its dominant axis (2b).
    pub side_bit: bool,
    /// With `side_bit`: key on the dominant axis too (6 classes, 3 bits), so a
    /// wall face and the floor beside it never share a cell.
    pub axis_key: bool,
    /// Gradient shrink by its own measured noise (Wiener factor
    /// `|g|^2 / (|g|^2 + var(g))`) on top of the count shrink. One extra scalar.
    pub wiener: bool,
    /// F2 skew strength: `q ∝ 1 + skew * hardness`.
    pub skew: f64,
    /// Centroid, clamped gradient and mean hit distance (2b).
    pub position: bool,
    /// Gradient shrink: `n / (n + shrink_n0)`.
    pub shrink_n0: f64,
    /// Irradiance normalisation: scale reads by the cell's measured
    /// cosine-weighted irradiance over what its SH-L1 gives at the same
    /// normals. Corrects L1's hemisphere bias; one extra scalar per cell.
    pub irr_norm: bool,
    /// Read-side visibility weight: drop a neighbour cell when the segment
    /// from the reader to the cell's centroid is blocked (one short
    /// occlusion ray per neighbour; falls back to all neighbours when every
    /// one is blocked). Needs `position`. ADR 0039 risk "thin-wall leaks".
    pub vis: bool,
    pub encoding: Encoding,
    pub selection: Selection,
    pub texel: Option<TexelFeedback>,
    /// Pool capacity in cells (`None` = unlimited). Cells nearest the camera
    /// win.
    pub pool_cells: Option<usize>,
    pub subdiv: Option<Subdiv>,
}

impl CacheConfig {
    /// Stage 2 as on master: plain SH-L1, fp16, no side bit.
    #[must_use]
    pub fn plain(cell_size: f64, rays_per_cell: usize) -> Self {
        Self {
            layout: Layout::Cubic,
            cell_size,
            rays_per_cell,
            history_frames: 32.0,
            clear_keep_frames: 2.0,
            recover_frames: 24,
            side_bit: false,
            axis_key: false,
            wiener: false,
            skew: 8.0,
            position: false,
            shrink_n0: 64.0,
            irr_norm: false,
            vis: false,
            encoding: Encoding::F16,
            selection: Selection::Uniform,
            texel: None,
            pool_cells: None,
            subdiv: None,
        }
    }

    /// Stage 2b: side bit and position data, compact encoding.
    #[must_use]
    pub fn stage_2b(cell_size: f64, rays_per_cell: usize) -> Self {
        Self {
            side_bit: true,
            position: true,
            encoding: Encoding::Compact,
            ..Self::plain(cell_size, rays_per_cell)
        }
    }

    /// Estimated engine bytes per cell: payload plus 12 bytes of key, count
    /// and age. An estimate (ADR 0039 addendum A).
    #[must_use]
    pub fn engine_bytes_per_cell(&self) -> u32 {
        let norm = if self.irr_norm {
            norm_bytes(self.encoding)
        } else {
            0
        };
        let gvar = if self.wiener && self.position {
            norm_bytes(self.encoding)
        } else {
            0
        };
        self.encoding.engine_payload_bytes(self.position) + norm + gvar + 12
    }
}

/// Bytes of one extra per-cell scalar at `e`.
fn norm_bytes(e: Encoding) -> u32 {
    match e {
        Encoding::Exact => 4,
        Encoding::F16 => 2,
        Encoding::Compact => 1,
    }
}

/// Cell key: side bit, level-0 point, and for a child its level-1 point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub side: u8,
    pub k: [i32; 3],
    pub child: Option<[i32; 3]>,
}

/// One resolved cell.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cell {
    pub alive: bool,
    /// Samples in history.
    pub n: f64,
    /// SH-L1 radiance.
    pub sh: [f64; 4],
    /// 2b: ray-start centroid.
    pub mu: V3,
    /// 2b: gradient of `sh[0]` in space.
    pub grad: V3,
    /// 2b: mean of `1 / hit distance`.
    pub inv_r: f64,
    /// 2b Wiener shrink: running variance of the per-frame gradient.
    pub gvar: f64,
    /// Irradiance normalisation: mean measured irradiance at the pool normals
    /// and the mean (unnormalised) pool normal.
    pub ecos: f64,
    pub nbar: V3,
    /// 2c: this level-0 cell is split into children.
    pub split: bool,
    pub dwell: usize,
    pub calm: usize,
    /// Lattice point position (anchor for compact centroid offsets).
    pub anchor: V3,
}

#[derive(Clone, Copy, Debug, Default)]
struct Acc {
    w: f64,
    sh: [f64; 4],
    x: V3,
    xx: V3,
    xy: V3,
    inv_d: f64,
    ecos: f64,
    nsum: V3,
    rays: usize,
}

/// Counters for one run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub rays: u64,
    pub catch_up_rays: u64,
    pub splits: u64,
    pub merges: u64,
    /// Splits or merges of a cell that had already changed state.
    pub toggles: u64,
    pub peak_cells: usize,
    pub dropped_cells: usize,
}

/// The cache plus the scene it runs on.
pub struct Sim<'a> {
    pub scene: &'a Scene,
    pub cfg: CacheConfig,
    pub lat: Lattice,
    pub cells: Vec<Cell>,
    pub keys: Vec<Key>,
    index: HashMap<Key, usize>,
    pools: Vec<Vec<usize>>,
    /// Cumulative selection probability per pool entry (skewed selection).
    cdf: Vec<Vec<f64>>,
    /// Hájek weights per pool entry.
    wts: Vec<Vec<f64>>,
    hardness: Vec<f64>,
    texel: Vec<f64>,
    children: HashMap<usize, Vec<usize>>,
    changed_once: Vec<bool>,
    free: Vec<usize>,
    pub frame: usize,
    lamps_were_on: bool,
    /// Frames since the last lamp change (`usize::MAX` = never).
    since_change: usize,
    seed: u64,
    threads: usize,
    pub counters: Counters,
}

/// Key class of a normal: 0 without the side bit, else the sign along the
/// dominant axis (1 bit), or axis and sign (6 classes) with `axis_key`.
#[must_use]
pub fn side_of(n: V3, cfg: &CacheConfig) -> u8 {
    if !cfg.side_bit {
        return 0;
    }
    let ax = n.dominant_axis();
    let pos = u8::from(n.axis(ax) > 0.0);
    if cfg.axis_key {
        let a = match ax {
            0 => 0,
            1 => 2,
            _ => 4,
        };
        a + pos
    } else {
        pos
    }
}

impl<'a> Sim<'a> {
    /// Allocate the level-0 cells that own surface pixels.
    #[must_use]
    pub fn new(scene: &'a Scene, cfg: CacheConfig, seed: u64, threads: usize) -> Self {
        let lat = Lattice::with_density_of_cubic(cfg.layout, cfg.cell_size);
        let mut groups: HashMap<Key, Vec<usize>> = HashMap::new();
        for (i, px) in scene.pixels.iter().enumerate() {
            if px.valid {
                let key = Key {
                    side: side_of(px.n, &cfg),
                    k: lat.nearest(px.p),
                    child: None,
                };
                groups.entry(key).or_default().push(i);
            }
        }
        let mut entries: Vec<(Key, Vec<usize>)> = groups.into_iter().collect();
        // Deterministic order, nearest the camera first (pool capacity keeps those).
        entries.sort_by(|a, b| {
            let da = lat.position(a.0.k).sub(scene.camera).length();
            let db = lat.position(b.0.k).sub(scene.camera).length();
            da.total_cmp(&db)
                .then(a.0.k.cmp(&b.0.k))
                .then(a.0.side.cmp(&b.0.side))
        });
        let mut sim = Self {
            scene,
            cfg,
            lat,
            cells: Vec::new(),
            keys: Vec::new(),
            index: HashMap::new(),
            pools: Vec::new(),
            cdf: Vec::new(),
            wts: Vec::new(),
            hardness: hardness(scene),
            texel: vec![0.0; scene.pixels.len()],
            children: HashMap::new(),
            changed_once: Vec::new(),
            free: Vec::new(),
            frame: 0,
            lamps_were_on: scene.lamps_on(0),
            since_change: usize::MAX,
            seed,
            threads: threads.max(1),
            counters: Counters::default(),
        };
        let cap = cfg.pool_cells.unwrap_or(usize::MAX);
        let total = entries.len();
        for (key, pool) in entries.into_iter().take(cap) {
            let anchor = lat.position(key.k);
            sim.insert(
                key,
                pool,
                Cell {
                    alive: true,
                    anchor,
                    ..Cell::default()
                },
            );
        }
        sim.counters.dropped_cells = total.saturating_sub(cap);
        sim.counters.peak_cells = sim.live_cells();
        sim
    }

    fn insert(&mut self, key: Key, pool: Vec<usize>, cell: Cell) -> usize {
        let (cdf, wts) = self.selection_tables(&pool);
        let idx = if let Some(i) = self.free.pop() {
            self.cells[i] = cell;
            self.keys[i] = key;
            self.pools[i] = pool;
            self.cdf[i] = cdf;
            self.wts[i] = wts;
            self.changed_once[i] = false;
            i
        } else {
            self.cells.push(cell);
            self.keys.push(key);
            self.pools.push(pool);
            self.cdf.push(cdf);
            self.wts.push(wts);
            self.changed_once.push(false);
            self.cells.len() - 1
        };
        self.index.insert(key, idx);
        idx
    }

    fn remove(&mut self, idx: usize) {
        self.index.remove(&self.keys[idx]);
        self.cells[idx].alive = false;
        self.pools[idx].clear();
        self.free.push(idx);
    }

    fn selection_tables(&self, pool: &[usize]) -> (Vec<f64>, Vec<f64>) {
        match self.cfg.selection {
            Selection::Uniform => (Vec::new(), Vec::new()),
            Selection::SkewUnweighted | Selection::SkewWeighted => {
                let q: Vec<f64> = pool
                    .iter()
                    .map(|&i| 1.0 + self.cfg.skew * self.hardness[i])
                    .collect();
                let total: f64 = q.iter().sum();
                let m = fl(pool.len());
                let mut acc = 0.0;
                let cdf = q
                    .iter()
                    .map(|v| {
                        acc += v / total;
                        acc
                    })
                    .collect();
                let wts = q.iter().map(|v| 1.0 / (v / total * m)).collect();
                (cdf, wts)
            }
        }
    }

    /// Live cells (children count, split parents do not cast).
    #[must_use]
    pub fn live_cells(&self) -> usize {
        self.cells.iter().filter(|c| c.alive).count()
    }

    fn spacing(&self, key: &Key) -> f64 {
        if key.child.is_some() {
            self.lat.child().cubic_equivalent()
        } else {
            self.lat.cubic_equivalent()
        }
    }

    /// Indirect irradiance the cell model gives at `p`, normal `n`.
    fn eval(&self, idx: usize, p: V3, n: V3) -> f64 {
        let c = &self.cells[idx];
        let mut e = sh::irradiance(c.sh, n);
        if self.cfg.irr_norm {
            let den = sh::irradiance_lin(c.sh, c.nbar);
            if den > 1e-12 && c.ecos > 0.0 {
                e *= sh::norm_ratio(c.ecos / den, self.cfg.encoding);
            }
        }
        if !self.cfg.position || c.sh[0] <= 0.0 {
            return e;
        }
        let h = self.spacing(&self.keys[idx]);
        let g = self.grad_of(c, h);
        let ratio = (1.0 + g.dot(p.sub(c.mu)) / c.sh[0]).clamp(0.0, 3.0);
        e * ratio
    }

    /// Value of a level-0 cell at `p`, through its children when split.
    fn eval_top(&self, idx: usize, p: V3, n: V3) -> f64 {
        let parent = self.eval(idx, p, n);
        if !self.cells[idx].split {
            return parent;
        }
        let Some(sd) = self.cfg.subdiv else {
            return parent;
        };
        let key = self.keys[idx];
        let (pts, cnt) = self.lat.child().stencil(p);
        let (mut acc, mut wsum) = (0.0, 0.0);
        for (k, w) in &pts[..cnt] {
            let ck = Key {
                child: Some(*k),
                ..key
            };
            if let Some(&ci) = self.index.get(&ck) {
                let c = &self.cells[ci];
                let f = c.n / (c.n + sd.fade_k);
                acc += w * (f * self.eval(ci, p, n) + (1.0 - f) * parent);
                wsum += w;
            }
        }
        if wsum > 1e-6 {
            acc / wsum
        } else {
            parent
        }
    }

    /// Interpolated cache irradiance at `p`, normal `n`.
    #[must_use]
    pub fn read(&self, p: V3, n: V3) -> f64 {
        let side = side_of(n, &self.cfg);
        let (pts, cnt) = self.lat.stencil(p);
        let (mut acc, mut wsum) = (0.0, 0.0);
        let (mut all, mut wall) = (0.0, 0.0);
        let o = p.add(n.scale(EPS * 4.0));
        for (k, w) in &pts[..cnt] {
            let key = Key {
                side,
                k: *k,
                child: None,
            };
            if let Some(&i) = self.index.get(&key) {
                let c = &self.cells[i];
                if c.n > 0.0 {
                    let v = w * self.eval_top(i, p, n);
                    all += v;
                    wall += w;
                    if self.cfg.vis && self.cfg.position && self.blocked(o, c) {
                        continue;
                    }
                    acc += v;
                    wsum += w;
                }
            }
        }
        if wsum > 1e-6 {
            acc / wsum
        } else if wall > 1e-6 {
            all / wall
        } else {
            0.0
        }
    }

    /// Whether the segment from `o` to the cell's centroid (lifted off the
    /// surface along the cell's mean normal) is occluded.
    fn blocked(&self, o: V3, c: &Cell) -> bool {
        let nl = c.nbar.length();
        let lift = if nl > 1e-9 {
            c.nbar.scale(EPS * 4.0 / nl)
        } else {
            V3::default()
        };
        let t = c.mu.add(lift).sub(o);
        let dist = t.length();
        if dist < 1e-6 {
            return false;
        }
        self.scene
            .world
            .trace(o, t.scale(1.0 / dist), dist)
            .is_some()
    }

    /// The two stage-2 seed representations from exactly the same cell state.
    /// `(scalar axis irradiance, SH irradiance at the texel's true normal)`.
    #[must_use]
    pub fn seed_values(&self, p: V3, n: V3) -> (f64, f64) {
        let side = side_of(n, &self.cfg);
        let (pts, cnt) = self.lat.stencil(p);
        let (mut scalar, mut directional, mut wsum) = (0.0, 0.0, 0.0);
        for (k, w) in &pts[..cnt] {
            let key = Key {
                side,
                k: *k,
                child: None,
            };
            if let Some(&i) = self.index.get(&key) {
                let c = &self.cells[i];
                if c.n > 0.0 {
                    scalar += w * PI * crate::sh::Y0 * c.sh[0];
                    directional += w * self.eval(i, p, n);
                    wsum += w;
                }
            }
        }
        if wsum > 1e-6 {
            (scalar / wsum, directional / wsum)
        } else {
            (0.0, 0.0)
        }
    }

    /// Lookup cost proxy per read at level 0.
    #[must_use]
    pub fn lookup_cost(&self) -> LookupCost {
        self.lat.lookup_cost()
    }

    /// What every pixel shows (invalid pixels 0).
    #[must_use]
    pub fn display(&self) -> Vec<f64> {
        let px = &self.scene.pixels;
        let mut out = vec![0.0; px.len()];
        let chunk = px.len().div_ceil(self.threads).max(1);
        std::thread::scope(|s| {
            for (ci, slice) in out.chunks_mut(chunk).enumerate() {
                s.spawn(move || {
                    for (k, v) in slice.iter_mut().enumerate() {
                        let p = &px[ci * chunk + k];
                        if p.valid {
                            *v = self.read(p.p, p.n);
                        }
                    }
                });
            }
        });
        out
    }

    fn casting(&self, idx: usize) -> bool {
        let c = &self.cells[idx];
        c.alive && !c.split && !self.pools[idx].is_empty()
    }

    fn trace_cell(&self, idx: usize, rng: &mut Rng) -> Acc {
        let mut acc = Acc::default();
        let pool = &self.pools[idx];
        let mut rays = self.cfg.rays_per_cell;
        if let Some(sd) = self.cfg.subdiv {
            if self.keys[idx].child.is_some() && self.cells[idx].n < sd.boost_min_n {
                rays += sd.boost_rays;
            }
        }
        let lamps = self.scene.lamps_on(self.frame);
        let rho = self.scene.world.albedo;
        let s = self.scene.pixel_size;
        for _ in 0..rays {
            let (j, w) = match self.cfg.selection {
                Selection::Uniform => (rng.uniform_usize(pool.len()), 1.0),
                Selection::SkewUnweighted | Selection::SkewWeighted => {
                    let u = rng.next_f64();
                    let cdf = &self.cdf[idx];
                    let j = cdf.partition_point(|&c| c < u).min(pool.len() - 1);
                    let w = if self.cfg.selection == Selection::SkewWeighted {
                        self.wts[idx][j]
                    } else {
                        1.0
                    };
                    (j, w)
                }
            };
            let px = &self.scene.pixels[pool[j]];
            // Jitter inside the pixel, in the surface plane.
            let mut p = px.p;
            let ax = px.n.dominant_axis();
            for a in [(ax + 1) % 3, (ax + 2) % 3] {
                p.set_axis(a, p.axis(a) + (rng.next_f64() - 0.5) * s);
            }
            let d = uniform_hemi(px.n, rng);
            let o = p.add(px.n.scale(EPS * 4.0));
            let (radiance, t) = match self.scene.world.trace(o, d, 1e9) {
                Some(hit) if hit.n.dot(d) < 0.0 => {
                    let e = self.scene.world.direct(hit.p, hit.n, lamps) + self.read(hit.p, hit.n);
                    (rho / PI * e, hit.t)
                }
                Some(hit) => (0.0, hit.t),
                None => (0.0, 1e3),
            };
            let b = sh::basis(d);
            for (a, bb) in acc.sh.iter_mut().zip(b) {
                *a += w * 2.0 * PI * radiance * bb;
            }
            let s0 = 2.0 * PI * radiance * Y0;
            acc.w += w;
            acc.x = acc.x.add(p.scale(w));
            acc.xx = acc.xx.add(p.mul(p).scale(w));
            acc.xy = acc.xy.add(p.scale(w * s0));
            acc.inv_d += w / t.max(0.01);
            acc.ecos += w * 2.0 * PI * radiance * d.dot(px.n).max(0.0);
            acc.nsum = acc.nsum.add(px.n.scale(w));
            acc.rays += 1;
        }
        acc
    }

    fn resolve(&mut self, idx: usize, acc: &Acc) {
        if acc.rays == 0 || acc.w <= 0.0 {
            return;
        }
        let frames = if self.since_change < self.cfg.recover_frames {
            self.cfg.clear_keep_frames
        } else {
            self.cfg.history_frames
        };
        let cap = frames * fl(self.cfg.rays_per_cell);
        let h = self.spacing(&self.keys[idx]);
        let enc = self.cfg.encoding;
        let position = self.cfg.position;
        let c = &mut self.cells[idx];
        let inv = 1.0 / acc.w;
        let fsh = acc.sh.map(|v| v * inv);
        let xbar = acc.x.scale(inv);
        let ybar = fsh[0];
        let ridge = (0.05 * h).powi(2);
        let mut g = V3::default();
        for a in 0..3 {
            let var = (acc.xx.axis(a) * inv - xbar.axis(a).powi(2)).max(0.0);
            let cov = acc.xy.axis(a) * inv - xbar.axis(a) * ybar;
            g.set_axis(a, cov / (var + ridge));
        }
        let nf = fl(acc.rays);
        let prev = c.n.min(cap);
        let total = prev + nf;
        let a = nf / total;
        for (s, f) in c.sh.iter_mut().zip(fsh) {
            *s += a * (f - *s);
        }
        let ecos = acc.ecos * inv;
        let nbar = acc.nsum.scale(inv);
        if prev <= 0.0 {
            c.ecos = ecos;
            c.nbar = nbar;
        } else {
            c.ecos += a * (ecos - c.ecos);
            c.nbar = c.nbar.add(nbar.sub(c.nbar).scale(a));
        }
        if prev <= 0.0 {
            c.mu = xbar;
            c.grad = g;
            c.gvar = g.dot(g);
            c.inv_r = acc.inv_d * inv;
        } else {
            c.mu = c.mu.add(xbar.sub(c.mu).scale(a));
            let dg = g.sub(c.grad);
            c.gvar += a * (dg.dot(dg) - c.gvar);
            c.grad = c.grad.add(dg.scale(a));
            c.inv_r += a * (acc.inv_d * inv - c.inv_r);
        }
        c.n = total;
        encode(c, enc, position, h);
    }

    /// Advance one frame.
    pub fn step(&mut self) {
        let lamps = self.scene.lamps_on(self.frame);
        if lamps != self.lamps_were_on {
            let keep = self.cfg.clear_keep_frames * fl(self.cfg.rays_per_cell);
            for c in &mut self.cells {
                c.n = c.n.min(keep);
            }
            self.lamps_were_on = lamps;
            self.since_change = 0;
        }
        let live: Vec<usize> = (0..self.cells.len()).filter(|&i| self.casting(i)).collect();
        let chunk = 32;
        let groups: Vec<&[usize]> = live.chunks(chunk).collect();
        let mut accs: Vec<Vec<Acc>> = vec![Vec::new(); groups.len()];
        let threads = self.threads;
        let frame_seed = self.seed
            ^ (u64::try_from(self.frame)
                .unwrap_or(0)
                .wrapping_mul(0xa076_1d64_78bd_642f));
        {
            let this = &*self;
            let mut lanes: Vec<Vec<(usize, &mut Vec<Acc>)>> =
                (0..threads).map(|_| Vec::new()).collect();
            for (gi, slot) in accs.iter_mut().enumerate() {
                lanes[gi % threads].push((gi, slot));
            }
            let groups = &groups;
            std::thread::scope(|s| {
                for lane in lanes {
                    s.spawn(move || {
                        for (gi, slot) in lane {
                            let mut rng = Rng::new(
                                frame_seed
                                    ^ u64::try_from(gi + 1)
                                        .unwrap_or(0)
                                        .wrapping_mul(0xe703_7ed1_a0b4_28db),
                            );
                            *slot = groups[gi]
                                .iter()
                                .map(|&i| this.trace_cell(i, &mut rng))
                                .collect();
                        }
                    });
                }
            });
        }
        for (g, a) in groups.iter().zip(&accs) {
            for (&i, acc) in g.iter().zip(a) {
                self.counters.rays += u64::try_from(acc.rays).unwrap_or(0);
                let extra = acc.rays.saturating_sub(self.cfg.rays_per_cell);
                self.counters.catch_up_rays += u64::try_from(extra).unwrap_or(0);
                self.resolve(i, acc);
            }
        }
        if self.cfg.subdiv.is_some() {
            self.aggregate_parents();
            self.subdivide();
        }
        if let Some(tf) = self.cfg.texel {
            self.texel_feedback(tf);
        }
        self.counters.peak_cells = self.counters.peak_cells.max(self.live_cells());
        self.frame += 1;
        self.since_change = self.since_change.saturating_add(1);
    }

    fn texel_feedback(&mut self, tf: TexelFeedback) {
        let shown = self.display();
        for (t, s) in self.texel.iter_mut().zip(&shown) {
            *t = tf.alpha * *t + (1.0 - tf.alpha) * s;
        }
        for i in 0..self.cells.len() {
            if !self.casting(i) {
                continue;
            }
            let pool = &self.pools[i];
            let m = fl(pool.len());
            let t: f64 = pool.iter().map(|&j| self.texel[j]).sum::<f64>() / m;
            let e: f64 = pool.iter().map(|&j| shown[j]).sum::<f64>() / m;
            let c = &mut self.cells[i];
            c.sh[0] = (c.sh[0] + tf.beta * (t - e) / (PI * Y0)).max(0.0);
        }
    }

    /// Split parents hold the count-weighted sum of their children.
    fn aggregate_parents(&mut self) {
        let parents: Vec<(usize, Vec<usize>)> =
            self.children.iter().map(|(p, c)| (*p, c.clone())).collect();
        for (p, kids) in parents {
            let n: f64 = kids.iter().map(|&k| self.cells[k].n).sum();
            if n <= 0.0 {
                continue;
            }
            let mut sh_acc = [0.0; 4];
            let (mut mu, mut grad, mut inv_r) = (V3::default(), V3::default(), 0.0);
            let (mut ecos, mut nbar, mut gvar) = (0.0, V3::default(), 0.0);
            for &k in &kids {
                let c = &self.cells[k];
                let w = c.n / n;
                for (s, v) in sh_acc.iter_mut().zip(c.sh) {
                    *s += w * v;
                }
                mu = mu.add(c.mu.scale(w));
                grad = grad.add(c.grad.scale(w));
                inv_r += w * c.inv_r;
                gvar += w * c.gvar;
                ecos += w * c.ecos;
                nbar = nbar.add(c.nbar.scale(w));
            }
            let cap = self.cfg.history_frames * fl(self.cfg.rays_per_cell);
            let c = &mut self.cells[p];
            c.sh = sh_acc;
            c.mu = mu;
            c.grad = grad;
            c.inv_r = inv_r;
            c.gvar = gvar;
            c.ecos = ecos;
            c.nbar = nbar;
            c.n = n.min(cap);
        }
    }

    fn data_ratio(&self, idx: usize) -> f64 {
        let c = &self.cells[idx];
        if c.sh[0] <= 0.0 {
            return 0.0;
        }
        let h = self.spacing(&self.keys[idx]);
        self.grad_of(c, h).length() * h / c.sh[0]
    }

    fn subdivide(&mut self) {
        let Some(sd) = self.cfg.subdiv else {
            return;
        };
        let cam = self.scene.camera_at(self.frame);
        let tops: Vec<usize> = (0..self.cells.len())
            .filter(|&i| self.cells[i].alive && self.keys[i].child.is_none())
            .collect();
        for i in tops {
            let dist = self.cells[i].anchor.sub(cam).length();
            let ratio = self.data_ratio(i);
            let c = self.cells[i];
            self.cells[i].dwell += 1;
            if c.dwell < sd.min_dwell && self.changed_once[i] {
                continue;
            }
            if c.split {
                let far = dist > sd.split_dist * sd.hysteresis;
                let quiet = sd.tau.is_none_or(|t| ratio < 0.5 * t);
                self.cells[i].calm = if quiet { c.calm + 1 } else { 0 };
                let settled = sd.tau.is_none() || self.cells[i].calm >= sd.merge_frames;
                if far && settled {
                    self.merge(i);
                }
            } else {
                let near = dist < sd.split_dist;
                let data = sd.tau.is_some_and(|t| ratio > t && c.n >= sd.n_split);
                if near || data {
                    self.split(i, sd);
                }
            }
        }
    }

    fn split(&mut self, i: usize, sd: Subdiv) {
        let child_lat = self.lat.child();
        let key = self.keys[i];
        let mut groups: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
        for &j in &self.pools[i] {
            groups
                .entry(child_lat.nearest(self.scene.pixels[j].p))
                .or_default()
                .push(j);
        }
        if let Some(cap) = self.cfg.pool_cells {
            if self.live_cells() + groups.len() > cap {
                return;
            }
        }
        let mut entries: Vec<([i32; 3], Vec<usize>)> = groups.into_iter().collect();
        entries.sort_by_key(|e| e.0);
        let parent = self.cells[i];
        let mut kids = Vec::with_capacity(entries.len());
        for (ck, pool) in entries {
            let centroid = pool
                .iter()
                .fold(V3::default(), |a, &j| a.add(self.scene.pixels[j].p))
                .scale(1.0 / fl(pool.len()));
            let mut cell = Cell {
                alive: true,
                anchor: child_lat.position(ck),
                mu: centroid,
                ..Cell::default()
            };
            match sd.seed {
                Seed::Zero => {}
                Seed::Copy | Seed::Model => {
                    cell.sh = parent.sh;
                    cell.grad = parent.grad;
                    cell.inv_r = parent.inv_r;
                    cell.gvar = parent.gvar;
                    cell.ecos = parent.ecos;
                    cell.nbar = parent.nbar;
                    cell.n = parent.n.min(sd.seed_n);
                    if sd.seed == Seed::Model && self.cfg.position && parent.sh[0] > 0.0 {
                        let h = self.lat.cubic_equivalent();
                        let g = self.grad_of(&parent, h);
                        let r =
                            (1.0 + g.dot(centroid.sub(parent.mu)) / parent.sh[0]).clamp(0.0, 3.0);
                        cell.sh = parent.sh.map(|v| v * r);
                    }
                }
            }
            let ckey = Key {
                child: Some(ck),
                ..key
            };
            kids.push(self.insert(ckey, pool, cell));
        }
        if self.changed_once[i] {
            self.counters.toggles += 1;
        }
        self.changed_once[i] = true;
        self.counters.splits += 1;
        let c = &mut self.cells[i];
        c.split = true;
        c.dwell = 0;
        c.calm = 0;
        self.children.insert(i, kids);
    }

    fn merge(&mut self, i: usize) {
        // The parent already holds the children's sum (aggregate_parents).
        if let Some(kids) = self.children.remove(&i) {
            for k in kids {
                self.remove(k);
            }
        }
        self.counters.toggles += 1;
        self.counters.merges += 1;
        let c = &mut self.cells[i];
        c.split = false;
        c.dwell = 0;
        c.calm = 0;
    }

    /// Gradient a read uses: count shrink, optional Wiener shrink, clamp.
    fn grad_of(&self, c: &Cell, h: f64) -> V3 {
        let g = clamped_grad(c, h, self.cfg.shrink_n0);
        if !self.cfg.wiener || c.n <= 0.0 {
            return g;
        }
        // Variance of the history mean: per-frame variance over frames held.
        let frames = c.n / fl(self.cfg.rays_per_cell.max(1));
        let var = c.gvar / frames.max(1.0);
        let g2 = c.grad.dot(c.grad);
        if g2 + var <= 0.0 {
            return g;
        }
        g.scale(g2 / (g2 + var))
    }

    /// Split level-0 cells right now.
    #[must_use]
    pub fn split_cells(&self) -> usize {
        self.cells.iter().filter(|c| c.alive && c.split).count()
    }
}

/// Gradient after shrink and the `L0 / max(R, h/2)` clamp.
fn clamped_grad(c: &Cell, h: f64, n0: f64) -> V3 {
    let g = c.grad.scale(c.n / (c.n + n0));
    let r = if c.inv_r > 0.0 { 1.0 / c.inv_r } else { h };
    let limit = c.sh[0].max(0.0) / r.max(0.5 * h);
    let len = g.length();
    if len > limit && len > 0.0 {
        g.scale(limit / len)
    } else {
        g
    }
}

/// Round a cell to its stored form.
fn encode(c: &mut Cell, enc: Encoding, position: bool, h: f64) {
    match enc {
        Encoding::Exact => {}
        Encoding::F16 => {
            c.sh = c.sh.map(sh::f16);
            if position {
                c.mu = V3::new(sh::f16(c.mu.x), sh::f16(c.mu.y), sh::f16(c.mu.z));
                c.grad = V3::new(sh::f16(c.grad.x), sh::f16(c.grad.y), sh::f16(c.grad.z));
                c.inv_r = sh::f16(c.inv_r);
            }
        }
        Encoding::Compact => {
            let l0 = sh::round_sig(c.sh[0].max(0.0), 9);
            let r3 = 3f64.sqrt();
            for i in 1..4 {
                c.sh[i] = if l0 > 0.0 {
                    sh::snorm8(c.sh[i] / l0, r3) * l0
                } else {
                    0.0
                };
            }
            c.sh[0] = l0;
            if position {
                // Gradient stored after the clamp, as g (h/2) / L0 in [-1, 1].
                let r = if c.inv_r > 0.0 { 1.0 / c.inv_r } else { h };
                let r = sh::unorm8(r.max(1e-3).log2(), -10.0, 6.0).exp2();
                c.inv_r = 1.0 / r;
                let limit = l0 / r.max(0.5 * h);
                let len = c.grad.length();
                if len > limit && len > 0.0 {
                    c.grad = c.grad.scale(limit / len);
                }
                let s = 0.5 * h / l0.max(1e-12);
                let q = |v: f64| {
                    if l0 > 0.0 {
                        sh::snorm8(v * s, 1.0) / s
                    } else {
                        0.0
                    }
                };
                c.grad = V3::new(q(c.grad.x), q(c.grad.y), q(c.grad.z));
                if c.gvar > 0.0 {
                    c.gvar = sh::unorm8(c.gvar.log2(), -60.0, 20.0).exp2();
                }
                let off = c.mu.sub(c.anchor);
                let qo = |v: f64| sh::unorm8(v, -h, h);
                c.mu = c.anchor.add(V3::new(qo(off.x), qo(off.y), qo(off.z)));
            }
        }
    }
}

/// Short-ray occlusion per pixel (fraction of 16 rays hitting within 0.3 m).
#[must_use]
pub fn hardness(scene: &Scene) -> Vec<f64> {
    let mut rng = Rng::new(0x5eed);
    let dirs: Vec<(f64, f64)> = (0..16).map(|_| (rng.next_f64(), rng.next_f64())).collect();
    scene
        .pixels
        .iter()
        .map(|px| {
            if !px.valid {
                return 0.0;
            }
            let (t, b) = crate::geom::basis(px.n);
            let o = px.p.add(px.n.scale(EPS * 4.0));
            let hits = dirs
                .iter()
                .filter(|(u1, u2)| {
                    let r = u1.sqrt();
                    let phi = std::f64::consts::TAU * u2;
                    let d = t
                        .scale(r * phi.cos())
                        .add(b.scale(r * phi.sin()))
                        .add(px.n.scale((1.0 - u1).sqrt()));
                    scene.world.trace(o, d, 0.3).is_some()
                })
                .count();
            fl(hits) / 16.0
        })
        .collect()
}
