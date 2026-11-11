//! Reconnection weight and the five share gates.

use genos_math::Vec3;

use crate::deposit::{add_own, add_shared, mean_luma, Deposit, Faults};

pub const T_MIN: f32 = 2.0;
pub const W_CAP: f32 = 2.0;
const COS_FLOOR: f32 = 0.05;
const PLANE: f32 = 0.02;
const NORMAL_MIN: f32 = 0.99;
const SHARE_CAP: u32 = 32;
/// Same clamp as `lighting_sc_set_seed`. A seed below this is not stored.
const SEED_CONF_MIN: f32 = 0.05;
const SEED_CONF: f32 = 0.25;
const SHARE_RECEIVERS: u32 = 3;

/// F4 knobs. High's starting values match the shader params and `FillSettings::high`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShareLimits {
    /// In-tile sharing. Off leaves every neighbour unseeded.
    pub enabled: bool,
    pub t_min: f32,
    pub w_cap: f32,
    pub cos_floor: f32,
    pub plane: f32,
    pub normal_min: f32,
    pub share_cap: u32,
    /// Low confidence passed to the seed channel. It is not an own sample.
    pub confidence: f32,
    /// Distinct receivers one sample may seed. The tile holds the rest.
    pub receivers: u32,
}

impl ShareLimits {
    #[must_use]
    pub fn high() -> Self {
        Self {
            enabled: true,
            t_min: T_MIN,
            w_cap: W_CAP,
            cos_floor: COS_FLOOR,
            plane: PLANE,
            normal_min: NORMAL_MIN,
            share_cap: SHARE_CAP,
            confidence: SEED_CONF,
            receivers: SHARE_RECEIVERS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Plane,
    Normal,
    Axis,
    NearHit,
    OwnCount,
    Stamp,
}

#[derive(Clone, Copy, Debug)]
pub struct ShareCell {
    pub pos: Vec3,
    pub normal: Vec3,
    pub depth: f32,
    pub own_count: u32,
    pub stamp: u32,
    pub mean_frame: u32,
    pub firefly: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ShareHit {
    pub point: Vec3,
    pub normal: Vec3,
    pub radiance: Vec3,
    pub t: f32,
}

/// Six axes. Opposite faces do not match.
#[must_use]
pub fn axis_id(n: Vec3) -> u8 {
    let ax = n.x.abs();
    let ay = n.y.abs();
    let az = n.z.abs();
    if ax >= ay && ax >= az {
        u8::from(n.x < 0.0)
    } else if ay >= az {
        2 + u8::from(n.y < 0.0)
    } else {
        4 + u8::from(n.z < 0.0)
    }
}

/// The reconnection weight, or the first gate that rejected it.
///
/// # Errors
///
/// Returns the rejecting gate. A passed share returns its weight.
pub fn share_weight(
    origin: &ShareCell,
    hit: &ShareHit,
    recv: &ShareCell,
    faults: &Faults,
    path_frame: u32,
) -> Result<f32, Reject> {
    share_weight_at(origin, hit, recv, *faults, path_frame, &ShareLimits::high())
}

fn share_weight_at(
    origin: &ShareCell,
    hit: &ShareHit,
    recv: &ShareCell,
    faults: Faults,
    path_frame: u32,
    limits: &ShareLimits,
) -> Result<f32, Reject> {
    if !faults.gate1_off {
        let gap = recv.pos - origin.pos;
        let depth = origin.depth.max(recv.depth).max(1.0e-3);
        if origin.normal.dot(gap).abs() >= limits.plane * depth {
            return Err(Reject::Plane);
        }
        if origin.normal.dot(recv.normal) < limits.normal_min {
            return Err(Reject::Normal);
        }
        if axis_id(origin.normal) != axis_id(recv.normal) {
            return Err(Reject::Axis);
        }
    }
    if !faults.t_min_off && hit.t < limits.t_min {
        return Err(Reject::NearHit);
    }
    let scale = own_scale(recv.own_count, limits.share_cap);
    if scale <= 0.0 {
        return Err(Reject::OwnCount);
    }
    if recv.stamp > recv.mean_frame && path_frame <= recv.stamp {
        return Err(Reject::Stamp);
    }
    let raw = jacobian(origin, hit, recv, limits.cos_floor) * scale;
    let w = if faults.unclamped {
        raw
    } else {
        raw.min(limits.w_cap)
    };
    if !w.is_finite() || w <= 0.0 {
        return Err(Reject::Plane);
    }
    Ok(w)
}

fn own_scale(own: u32, cap: u32) -> f32 {
    let cap = cap.max(1).min(u32::from(u16::MAX));
    let own_f = f32::from(u16::try_from(own.min(u32::from(u16::MAX))).unwrap_or(u16::MAX));
    let cap_f = f32::from(u16::try_from(cap).unwrap_or(1));
    (1.0 - own_f / cap_f).max(0.0)
}

fn jacobian(origin: &ShareCell, hit: &ShareHit, recv: &ShareCell, cos_floor: f32) -> f32 {
    let to_o = hit.point - origin.pos;
    let to_j = hit.point - recv.pos;
    let len_j = to_j.length();
    if to_o.length() <= 0.0 || len_j <= 0.0 {
        return 0.0;
    }
    let dir_o = to_o.normalize();
    let dir_j = to_j.normalize();
    let cos_theta_o = clamp_cos(origin.normal.dot(dir_o), cos_floor);
    let cos_theta_j = clamp_cos(recv.normal.dot(dir_j), cos_floor);
    let cos_phi_o = clamp_cos(hit.normal.dot(-dir_o), cos_floor);
    let cos_phi_j = clamp_cos(hit.normal.dot(-dir_j), cos_floor);
    let t2 = hit.t.max(1.0e-3) * hit.t.max(1.0e-3);
    let d2 = len_j * len_j;
    (cos_theta_j / cos_theta_o) * ((cos_phi_j / d2) / (cos_phi_o / t2))
}

fn clamp_cos(v: f32, floor: f32) -> f32 {
    let floor = if floor > 0.0 { floor } else { COS_FLOOR };
    if v <= floor { floor } else { v }
}

#[derive(Clone, Copy, Debug)]
pub struct Leak {
    pub other: f32,
    pub source: f32,
}

#[must_use]
pub fn leak_ok(leak: Leak) -> bool {
    leak.other <= 0.02 * leak.source.max(1.0e-6)
}

#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub origin: ShareCell,
    pub other: ShareCell,
}

fn cell(pos: Vec3, normal: Vec3) -> ShareCell {
    ShareCell {
        pos,
        normal,
        depth: 3.0,
        own_count: 0,
        stamp: 0,
        mean_frame: 0,
        firefly: 1.0e6,
    }
}

/// A thin wall: opposite normals. Gate 1 holds the other side at zero.
#[must_use]
pub fn thin_wall(gate1_off: bool) -> Leak {
    let origin = cell(Vec3::ZERO, Vec3::Z);
    let other = cell(Vec3::new(0.0, 0.02, 0.0), -Vec3::Z);
    let hit = ShareHit {
        point: Vec3::new(0.0, 0.0, 3.0),
        normal: -Vec3::Z,
        radiance: Vec3::new(1.0, 1.0, 1.0),
        t: 3.0,
    };
    energies(&origin, &other, &hit, gate1_off)
}

/// A pillar corner: normals a quarter turn apart.
#[must_use]
pub fn pillar(gate1_off: bool) -> Leak {
    let origin = cell(Vec3::ZERO, Vec3::Z);
    let other = cell(Vec3::new(0.05, 0.0, 0.05), Vec3::X);
    let hit = ShareHit {
        point: Vec3::new(0.0, 0.0, 3.0),
        normal: -Vec3::Z,
        radiance: Vec3::new(1.0, 1.0, 1.0),
        t: 3.0,
    };
    energies(&origin, &other, &hit, gate1_off)
}

/// Wall meeting floor.
#[must_use]
pub fn wall_base(gate1_off: bool) -> Leak {
    let origin = cell(Vec3::ZERO, Vec3::Y);
    let other = cell(Vec3::new(0.0, 0.02, 0.02), Vec3::Z);
    let hit = ShareHit {
        point: Vec3::new(0.0, 3.0, 0.0),
        normal: -Vec3::Y,
        radiance: Vec3::new(1.0, 1.0, 1.0),
        t: 3.0,
    };
    energies(&origin, &other, &hit, gate1_off)
}

fn energies(origin: &ShareCell, other: &ShareCell, hit: &ShareHit, gate1_off: bool) -> Leak {
    let faults = Faults {
        gate1_off,
        ..Faults::default()
    };
    let mut src = Deposit::default();
    add_own(&mut src, hit.radiance);
    let mut dst = Deposit::default();
    if let Ok(w) = share_weight(origin, hit, other, &faults, 1) {
        deposit_shared(&mut dst, hit.radiance, w, other.firefly, faults);
    }
    Leak {
        source: mean_luma(&src),
        other: mean_luma(&dst),
    }
}

fn luma_of(rgb: Vec3) -> f32 {
    0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z
}

fn firefly(rgb: Vec3, cap: f32) -> Vec3 {
    let luma = luma_of(rgb);
    if luma > cap && luma > 0.0 {
        rgb * (cap / luma)
    } else {
        rgb
    }
}

/// Gate 4. The contribution is what is deposited, after the weight clamp.
fn deposit_shared(dst: &mut Deposit, radiance: Vec3, weight: f32, cap: f32, faults: Faults) {
    let contrib = radiance * weight;
    let capped = if faults.firefly_off {
        contrib
    } else {
        firefly(contrib, cap)
    };
    add_shared(dst, capped, 1.0, faults);
}

/// `0.1 + short mean + sigma * deviation`, the running upper estimate.
fn running_upper(short_mean: f32, deviation: f32, sigma: f32) -> f32 {
    0.1 + short_mean + sigma * deviation
}

/// The unclamped fault produces a weight above `W_CAP`. The real path clamps it.
#[must_use]
pub fn unclamped_exceeds_cap() -> bool {
    // A hit at 2 m whose normal faces the neighbour more than the origin.
    // The raw weight is above 2; the real path clamps it.
    let origin = cell(Vec3::ZERO, Vec3::Y);
    let recv = cell(Vec3::new(-0.213_771_8, 0.0, 0.172_413_2), Vec3::Y);
    let hit = ShareHit {
        point: Vec3::new(-1.204_053, 0.980_968_8, 1.260_142),
        normal: Vec3::new(0.330_765_2, -0.609_517_2, 0.720_474_3),
        radiance: Vec3::new(1.0, 1.0, 1.0),
        t: 2.0,
    };
    let raw = Faults {
        unclamped: true,
        ..Faults::default()
    };
    let Ok(heavy) = share_weight(&origin, &hit, &recv, &raw, 1) else {
        return false;
    };
    if heavy <= W_CAP {
        return false;
    }
    let held = share_weight(&origin, &hit, &recv, &Faults::default(), 1);
    matches!(held, Ok(w) if (w - W_CAP).abs() < 1.0e-4)
}

/// Radiance of the ceiling at `hit`. It varies across the plane, so a
/// constant weight is not the receiver's integral.
fn ceiling_radiance(hit: Vec3) -> f32 {
    (0.2 + 0.8 * hit.x).max(0.05)
}

/// Cosine-weighted irradiance of the ceiling, `pi` times the mean radiance.
fn receiver_integral(recv: Vec3, samples: u16) -> f32 {
    let mut sum = 0.0_f32;
    for index in 0..samples {
        let dir = cosine_dir(index, samples);
        let t = 3.0 / dir.y.max(0.05);
        sum += ceiling_radiance(recv + dir * t);
    }
    std::f32::consts::PI * sum / f32::from(samples)
}

/// Relative error of the shared estimator against a brute-force integral at
/// the receiver. `constant_weight` replaces the Jacobian with 1.
fn plane_error(constant_weight: bool) -> f32 {
    let samples = 4096_u16;
    let origin = cell(Vec3::ZERO, Vec3::Y);
    let recv = cell(Vec3::new(0.2, 0.0, 0.0), Vec3::Y);
    let mut shared_w = 0.0_f32;
    let mut shared_l = 0.0_f32;
    for index in 0..samples {
        let dir = cosine_dir(index, samples);
        let t = 3.0 / dir.y.max(0.05);
        if t < T_MIN {
            continue;
        }
        let hit_at = origin.pos + dir * t;
        let hit = ShareHit {
            point: hit_at,
            normal: -Vec3::Y,
            radiance: Vec3::new(1.0, 1.0, 1.0),
            t,
        };
        let faults = Faults::default();
        if let Ok(raw) = share_weight(&origin, &hit, &recv, &faults, 1) {
            let weight = if constant_weight { 1.0 } else { raw };
            let radiance = ceiling_radiance(hit_at);
            shared_w += weight;
            shared_l += weight * radiance;
        }
    }
    let shared = if shared_w > 0.0 {
        std::f32::consts::PI * shared_l / shared_w
    } else {
        0.0
    };
    let truth = receiver_integral(recv.pos, 8192);
    (shared - truth).abs() / truth
}

/// Relative error of the Jacobian estimator against the receiver integral.
#[must_use]
pub fn plane_rel_error() -> f32 {
    plane_error(false)
}

/// The same integral with every accepted weight forced to 1.
#[must_use]
pub fn constant_weight_plane_error() -> f32 {
    plane_error(true)
}

/// Gate 4, and the reweight clamp at 2.
///
/// A bright hit whose raw weight exceeds 2 is deposited at weight 2, and that
/// contribution's luma is held to `0.1 + short mean + sigma * deviation`.
/// Removing either bound fails this check.
#[must_use]
pub fn firefly_bound_holds() -> bool {
    let cap = running_upper(0.2, 0.05, 1.0);
    let origin = cell(Vec3::ZERO, Vec3::Y);
    let mut recv = cell(Vec3::new(-0.213_771_8, 0.0, 0.172_413_2), Vec3::Y);
    recv.firefly = cap;
    let hit = ShareHit {
        point: Vec3::new(-1.204_053, 0.980_968_8, 1.260_142),
        normal: Vec3::new(0.330_765_2, -0.609_517_2, 0.720_474_3),
        radiance: Vec3::new(8.0, 8.0, 8.0),
        t: 2.0,
    };
    let raw = Faults {
        unclamped: true,
        ..Faults::default()
    };
    let Ok(heavy) = share_weight(&origin, &hit, &recv, &raw, 1) else {
        return false;
    };
    let Ok(held_w) = share_weight(&origin, &hit, &recv, &Faults::default(), 1) else {
        return false;
    };
    if heavy <= W_CAP || (held_w - W_CAP).abs() > 1.0e-4 {
        return false;
    }
    let mut held = Deposit::default();
    deposit_shared(&mut held, hit.radiance, held_w, cap, Faults::default());
    let mut open = Deposit::default();
    deposit_shared(
        &mut open,
        hit.radiance,
        held_w,
        cap,
        Faults {
            firefly_off: true,
            ..Faults::default()
        },
    );
    let held_luma = mean_luma(&held);
    let open_luma = mean_luma(&open);
    held_luma <= cap + 1.0e-3 && open_luma > cap + 1.0
}

fn cosine_dir(index: u16, count: u16) -> Vec3 {
    let unit = (f32::from(index) + 0.5) / f32::from(count);
    let spin = (f32::from(index) * 0.754_877_7).fract();
    let radius = unit.sqrt();
    let phi = 2.0 * std::f32::consts::PI * spin;
    Vec3::new(radius * phi.cos(), (1.0 - unit).sqrt().max(0.05), radius * phi.sin()).normalize()
}

#[must_use]
pub fn near_hit_stays_home(faults: &Faults) -> bool {
    let origin = cell(Vec3::ZERO, Vec3::Y);
    let recv = cell(Vec3::new(0.2, 0.0, 0.0), Vec3::Y);
    let hit = ShareHit {
        point: Vec3::new(0.0, 0.5, 0.0),
        normal: -Vec3::Y,
        radiance: Vec3::new(1.0, 1.0, 1.0),
        t: 0.5,
    };
    share_weight(&origin, &hit, &recv, faults, 1).is_err()
}

/// One other surface in the pick's screen tile.
#[derive(Clone, Copy, Debug)]
pub struct TileNeighbour {
    pub id: u32,
    pub cell: ShareCell,
    /// False when the surface is outside this tile. F4a does not seed it.
    pub in_tile: bool,
}

/// A shared sample stored on the existing low-confidence channel.
///
/// There is no count field. The receiver's own ray count is not this value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShareSeed {
    pub id: u32,
    pub rgb: Vec3,
    pub confidence: f32,
}

/// What one traced sample does to the rest of its tile.
#[derive(Clone, Copy, Debug)]
pub struct TileShare {
    pub seeds: [ShareSeed; 8],
    pub count: u32,
    /// Rays this sample consumed. Sharing does not change it.
    pub rays: u32,
    /// The origin cell's own paths after the share. Sharing does not change it.
    pub origin_own: u32,
}

/// Claim word for one sample. Higher weight wins. The same weight keeps the
/// lower ray index. Zero means no claim, so a real ray never packs to zero.
#[must_use]
pub fn share_claim(weight: f32, w_cap: f32, ray: u32) -> u32 {
    let cap = if w_cap > 0.0 { w_cap } else { 1.0 };
    let q = q10((weight / cap).clamp(0.0, 1.0));
    let pack = (q << 22) | ((!ray) & 0x003F_FFFF);
    if pack == 0 { 1 } else { pack }
}

/// The winning claim. An empty list wins nothing.
#[must_use]
pub fn share_elect(claims: &[u32]) -> u32 {
    let mut best = 0u32;
    for claim in claims {
        if *claim > best {
            best = *claim;
        }
    }
    best
}

fn q10(t: f32) -> u32 {
    let scaled = t.clamp(0.0, 1.0) * 1023.0;
    let mut lo = 0u32;
    let mut hi = 1024u32;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        let mid_f = f32::from(u16::try_from(mid).unwrap_or(1023));
        if mid_f <= scaled {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

fn seed_confidence(v: f32) -> f32 {
    if !v.is_finite() {
        return SEED_CONF;
    }
    v.clamp(SEED_CONF_MIN, 1.0)
}

/// One traced sample and the other surfaces in its tile.
pub struct TileSample<'a> {
    pub origin_id: u32,
    pub origin: &'a ShareCell,
    pub hit: &'a ShareHit,
    pub irradiance: Vec3,
    pub neighbours: &'a [TileNeighbour],
    pub limits: &'a ShareLimits,
    pub faults: Faults,
    pub path_frame: u32,
    pub rays: u32,
}

/// In-tile share (stage F4a). Each neighbour that passes the gates gets one
/// low-confidence seed. The origin's own count and the ray budget stay put.
/// A neighbour outside the tile gets nothing. Ring sharing is not this function.
#[must_use]
pub fn share_in_tile(sample: &TileSample<'_>) -> TileShare {
    let mut out = TileShare {
        seeds: [ShareSeed {
            id: 0,
            rgb: Vec3::ZERO,
            confidence: 0.0,
        }; 8],
        count: 0,
        rays: sample.rays,
        origin_own: sample.origin.own_count,
    };
    if !sample.limits.enabled || !sample.irradiance.x.is_finite() {
        return out;
    }
    let room = usize::try_from(sample.limits.receivers.min(8)).unwrap_or(0);
    for neighbour in sample.neighbours {
        if out.count >= u32::try_from(room).unwrap_or(0) {
            break;
        }
        if !neighbour.in_tile || neighbour.id == sample.origin_id {
            continue;
        }
        let Ok(w) = share_weight_at(
            sample.origin,
            sample.hit,
            &neighbour.cell,
            sample.faults,
            sample.path_frame,
            sample.limits,
        ) else {
            continue;
        };
        let contrib = sample.irradiance * w;
        let capped = if sample.faults.firefly_off {
            contrib
        } else {
            firefly(contrib, neighbour.cell.firefly)
        };
        if !luma_of(capped).is_finite() || luma_of(capped) <= 0.0 {
            continue;
        }
        let at = usize::try_from(out.count).unwrap_or(0);
        if at >= out.seeds.len() {
            break;
        }
        out.seeds[at] = ShareSeed {
            id: neighbour.id,
            rgb: capped,
            confidence: seed_confidence(sample.limits.confidence),
        };
        out.count += 1;
    }
    out
}

/// The wall gets no seed, the open fault does, and a coplanar neighbour does
/// not spend another ray.
#[must_use]
pub fn tile_share_holds() -> bool {
    let limits = ShareLimits::high();
    let origin = cell(Vec3::ZERO, Vec3::Y);
    let hit = ShareHit {
        point: Vec3::new(0.0, 3.0, 0.0),
        normal: -Vec3::Y,
        radiance: Vec3::new(1.0, 1.0, 1.0),
        t: 3.0,
    };
    let irr = Vec3::new(0.4, 0.4, 0.4);
    let mut other = cell(Vec3::new(0.0, 0.02, 0.0), -Vec3::Y);
    other.firefly = 1.0e6;
    let wall = [TileNeighbour {
        id: 2,
        cell: other,
        in_tile: true,
    }];
    let blocked = share_in_tile(&TileSample {
        origin_id: 1,
        origin: &origin,
        hit: &hit,
        irradiance: irr,
        neighbours: &wall,
        limits: &limits,
        faults: Faults::default(),
        path_frame: 1,
        rays: 100_000,
    });
    let leaked = share_in_tile(&TileSample {
        origin_id: 1,
        origin: &origin,
        hit: &hit,
        irradiance: irr,
        neighbours: &wall,
        limits: &limits,
        faults: Faults {
            gate1_off: true,
            ..Faults::default()
        },
        path_frame: 1,
        rays: 100_000,
    });
    let mut flat = cell(Vec3::new(0.2, 0.0, 0.0), Vec3::Y);
    flat.firefly = 1.0e6;
    let plane = [TileNeighbour {
        id: 3,
        cell: flat,
        in_tile: true,
    }];
    let shared = share_in_tile(&TileSample {
        origin_id: 1,
        origin: &origin,
        hit: &hit,
        irradiance: irr,
        neighbours: &plane,
        limits: &limits,
        faults: Faults::default(),
        path_frame: 1,
        rays: 100_000,
    });
    blocked.count == 0
        && blocked.rays == 100_000
        && leaked.count == 1
        && !leak_ok(Leak {
            source: luma_of(irr),
            other: luma_of(leaked.seeds[0].rgb),
        })
        && shared.count == 1
        && shared.rays == 100_000
        && shared.origin_own == origin.own_count
        && shared.seeds[0].confidence < 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unoccluded_plane_stays_within_one_percent_past_t_min() {
        let err = plane_rel_error();
        assert!(err < 0.01, "{err}");
    }

    #[test]
    fn sharing_on_holds_the_wall_and_gate_1_off_fails_the_leak_check() {
        for (on, off) in [
            (thin_wall(false), thin_wall(true)),
            (pillar(false), pillar(true)),
            (wall_base(false), wall_base(true)),
        ] {
            assert!(leak_ok(on), "other {} source {}", on.other, on.source);
            assert!(on.other <= 1.0e-6);
            assert!(!leak_ok(off), "fault other {} source {}", off.other, off.source);
        }
    }

    #[test]
    fn t_min_off_shares_a_near_hit_and_the_clamp_holds_a_heavy_weight() {
        assert!(near_hit_stays_home(&Faults::default()));
        let open = Faults {
            t_min_off: true,
            ..Faults::default()
        };
        assert!(!near_hit_stays_home(&open));
        assert!(unclamped_exceeds_cap(), "no sample exceeded the clamp");
    }

    #[test]
    fn a_full_own_count_and_a_stale_stamp_share_nothing() {
        let origin = cell(Vec3::ZERO, Vec3::Y);
        let mut recv = cell(Vec3::new(0.2, 0.0, 0.0), Vec3::Y);
        recv.own_count = 32;
        let hit = ShareHit {
            point: Vec3::new(0.0, 3.0, 0.0),
            normal: -Vec3::Y,
            radiance: Vec3::Y,
            t: 3.0,
        };
        assert_eq!(
            share_weight(&origin, &hit, &recv, &Faults::default(), 10),
            Err(Reject::OwnCount)
        );
        recv.own_count = 0;
        recv.stamp = 20;
        recv.mean_frame = 10;
        assert_eq!(
            share_weight(&origin, &hit, &recv, &Faults::default(), 20),
            Err(Reject::Stamp)
        );
        assert!(share_weight(&origin, &hit, &recv, &Faults::default(), 21).is_ok());
    }

    #[test]
    fn an_unoccluded_plane_tracks_the_receiver_and_a_constant_weight_does_not() {
        let err = plane_rel_error();
        let broken = constant_weight_plane_error();
        assert!(err < 0.01, "jacobian error {err}");
        assert!(
            broken >= 0.01,
            "constant weight error {broken} still under 1%"
        );
    }

    #[test]
    fn the_firefly_bound_holds_the_contribution_and_the_weight_at_two() {
        assert!(firefly_bound_holds());
    }

    fn plane_hit() -> (ShareCell, ShareHit, Vec3) {
        (
            cell(Vec3::ZERO, Vec3::Y),
            ShareHit {
                point: Vec3::new(0.0, 3.0, 0.0),
                normal: -Vec3::Y,
                radiance: Vec3::new(1.0, 1.0, 1.0),
                t: 3.0,
            },
            Vec3::new(0.4, 0.4, 0.4),
        )
    }

    fn neighbour(id: u32, pos: Vec3, normal: Vec3, in_tile: bool) -> TileNeighbour {
        let mut cell = cell(pos, normal);
        cell.firefly = 1.0e6;
        TileNeighbour { id, cell, in_tile }
    }

    fn run(
        origin: &ShareCell,
        hit: &ShareHit,
        irr: Vec3,
        neighbours: &[TileNeighbour],
        limits: &ShareLimits,
        faults: Faults,
        frame_rays: (u32, u32),
    ) -> TileShare {
        share_in_tile(&TileSample {
            origin_id: 1,
            origin,
            hit,
            irradiance: irr,
            neighbours,
            limits,
            faults,
            path_frame: frame_rays.0,
            rays: frame_rays.1,
        })
    }

    #[test]
    fn an_in_tile_neighbour_is_a_low_confidence_seed_and_the_budget_stays() {
        let (origin, hit, irr) = plane_hit();
        let mut origin = origin;
        origin.own_count = 7;
        let limits = ShareLimits::high();
        let neighbours = [neighbour(3, Vec3::new(0.2, 0.0, 0.0), Vec3::Y, true)];
        let got = run(
            &origin,
            &hit,
            irr,
            &neighbours,
            &limits,
            Faults::default(),
            (1, 100_000),
        );
        assert_eq!(got.count, 1);
        assert_eq!(got.rays, 100_000, "sharing must not spend another ray");
        assert_eq!(got.origin_own, 7, "a shared sample must not raise own_count");
        assert!((got.seeds[0].confidence - SEED_CONF).abs() < 1.0e-4);
        assert!(got.seeds[0].confidence < 1.0);
        let luma = luma_of(got.seeds[0].rgb);
        assert!(luma > 0.0);
        assert!(luma <= luma_of(irr) * W_CAP + 1.0e-3);
    }

    #[test]
    fn a_gated_neighbour_gets_no_seed_and_the_fault_does() {
        assert!(tile_share_holds());
        let (origin, hit, irr) = plane_hit();
        let limits = ShareLimits::high();
        let outside = [neighbour(4, Vec3::new(0.2, 0.0, 0.0), Vec3::Y, false)];
        let skipped = run(&origin, &hit, irr, &outside, &limits, Faults::default(), (1, 8));
        assert_eq!(skipped.count, 0);
        assert_eq!(skipped.rays, 8);
        let mut off = limits;
        off.enabled = false;
        let same = [neighbour(3, Vec3::new(0.2, 0.0, 0.0), Vec3::Y, true)];
        let closed = run(&origin, &hit, irr, &same, &off, Faults::default(), (1, 8));
        assert_eq!(closed.count, 0);
        let mut full = neighbour(5, Vec3::new(0.2, 0.0, 0.0), Vec3::Y, true);
        full.cell.own_count = SHARE_CAP;
        let expired = run(&origin, &hit, irr, &[full], &limits, Faults::default(), (1, 8));
        assert_eq!(expired.count, 0);
        let mut stamped = neighbour(6, Vec3::new(0.2, 0.0, 0.0), Vec3::Y, true);
        stamped.cell.stamp = 20;
        stamped.cell.mean_frame = 10;
        let stale = run(&origin, &hit, irr, &[stamped], &limits, Faults::default(), (20, 8));
        assert_eq!(stale.count, 0);
        let fresh = run(&origin, &hit, irr, &[stamped], &limits, Faults::default(), (21, 8));
        assert_eq!(fresh.count, 1);
        let near_hit = ShareHit { t: 0.5, ..hit };
        let near = run(
            &origin,
            &near_hit,
            irr,
            &same,
            &limits,
            Faults::default(),
            (1, 8),
        );
        assert_eq!(near.count, 0);
    }

    #[test]
    fn the_receiver_cap_drops_the_fourth_cell_and_the_firefly_holds_the_seed() {
        let (origin, hit, irr) = plane_hit();
        let mut limits = ShareLimits::high();
        limits.receivers = 3;
        let neighbours = [
            neighbour(2, Vec3::new(0.1, 0.0, 0.0), Vec3::Y, true),
            neighbour(3, Vec3::new(0.2, 0.0, 0.0), Vec3::Y, true),
            neighbour(4, Vec3::new(-0.1, 0.0, 0.0), Vec3::Y, true),
            neighbour(5, Vec3::new(0.0, 0.0, 0.2), Vec3::Y, true),
        ];
        let got = run(
            &origin,
            &hit,
            irr,
            &neighbours,
            &limits,
            Faults::default(),
            (1, 4),
        );
        assert_eq!(got.count, 3);
        assert_eq!(got.rays, 4);
        assert_ne!(got.seeds[0].id, 5);
        assert_ne!(got.seeds[1].id, 5);
        assert_ne!(got.seeds[2].id, 5);
        let cap = running_upper(0.2, 0.0, 8.0);
        let mut dim = neighbour(3, Vec3::new(0.2, 0.0, 0.0), Vec3::Y, true);
        dim.cell.firefly = cap;
        let bright = Vec3::new(8.0, 8.0, 8.0);
        let held = run(&origin, &hit, bright, &[dim], &limits, Faults::default(), (1, 1));
        let open = run(
            &origin,
            &hit,
            bright,
            &[dim],
            &limits,
            Faults {
                firefly_off: true,
                ..Faults::default()
            },
            (1, 1),
        );
        assert!(luma_of(held.seeds[0].rgb) <= cap + 1.0e-3);
        assert!(luma_of(open.seeds[0].rgb) > cap + 1.0);
    }

    #[test]
    fn the_higher_weight_claims_the_seed_and_a_tie_keeps_the_lower_ray() {
        let heavy = share_claim(2.0, 2.0, 9);
        let light = share_claim(1.0, 2.0, 1);
        assert!(heavy > light);
        assert_eq!(share_elect(&[light, heavy, 0]), heavy);
        let early = share_claim(1.0, 2.0, 1);
        let later = share_claim(1.0, 2.0, 5);
        assert!(early > later);
        assert_ne!(share_claim(1.0, 2.0, 0), 0);
        assert_ne!(share_claim(0.0, 2.0, 3), 0);
    }
}
