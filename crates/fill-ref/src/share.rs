//! Reconnection weight and the five share gates.

use genos_math::Vec3;

use crate::deposit::{add_own, add_shared, mean_luma, Deposit, Faults};

pub const T_MIN: f32 = 2.0;
pub const W_CAP: f32 = 2.0;
const COS_FLOOR: f32 = 0.05;
const PLANE: f32 = 0.02;
const NORMAL_MIN: f32 = 0.99;
const SHARE_CAP: u32 = 32;

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
    if !faults.gate1_off {
        let gap = recv.pos - origin.pos;
        if origin.normal.dot(gap).abs() >= PLANE * origin.depth.max(recv.depth).max(1.0e-3) {
            return Err(Reject::Plane);
        }
        if origin.normal.dot(recv.normal) < NORMAL_MIN {
            return Err(Reject::Normal);
        }
        if axis_id(origin.normal) != axis_id(recv.normal) {
            return Err(Reject::Axis);
        }
    }
    if !faults.t_min_off && hit.t < T_MIN {
        return Err(Reject::NearHit);
    }
    let scale = own_scale(recv.own_count);
    if scale <= 0.0 {
        return Err(Reject::OwnCount);
    }
    if recv.stamp > recv.mean_frame && path_frame <= recv.stamp {
        return Err(Reject::Stamp);
    }
    let raw = jacobian(origin, hit, recv) * scale;
    let w = if faults.unclamped { raw } else { raw.min(W_CAP) };
    if !w.is_finite() || w <= 0.0 {
        return Err(Reject::Plane);
    }
    Ok(w)
}

fn own_scale(own: u32) -> f32 {
    let own_f = f32::from(u16::try_from(own.min(u32::from(u16::MAX))).unwrap_or(u16::MAX));
    (1.0 - own_f / f32::from(u16::try_from(SHARE_CAP).unwrap_or(1))).max(0.0)
}

fn jacobian(origin: &ShareCell, hit: &ShareHit, recv: &ShareCell) -> f32 {
    let to_o = hit.point - origin.pos;
    let to_j = hit.point - recv.pos;
    let len_j = to_j.length();
    if to_o.length() <= 0.0 || len_j <= 0.0 {
        return 0.0;
    }
    let dir_o = to_o.normalize();
    let dir_j = to_j.normalize();
    let cos_theta_o = clamp_cos(origin.normal.dot(dir_o));
    let cos_theta_j = clamp_cos(recv.normal.dot(dir_j));
    let cos_phi_o = clamp_cos(hit.normal.dot(-dir_o));
    let cos_phi_j = clamp_cos(hit.normal.dot(-dir_j));
    let t2 = hit.t.max(1.0e-3) * hit.t.max(1.0e-3);
    let d2 = len_j * len_j;
    (cos_theta_j / cos_theta_o) * ((cos_phi_j / d2) / (cos_phi_o / t2))
}

fn clamp_cos(v: f32) -> f32 {
    if v <= COS_FLOOR {
        COS_FLOOR
    } else {
        v
    }
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
        let capped = firefly(hit.radiance * w, other.firefly);
        add_shared(&mut dst, capped, 1.0, faults);
    }
    Leak {
        source: mean_luma(&src),
        other: mean_luma(&dst),
    }
}

fn firefly(rgb: Vec3, cap: f32) -> Vec3 {
    let luma = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
    if luma > cap && luma > 0.0 {
        rgb * (cap / luma)
    } else {
        rgb
    }
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

/// Relative error of the shared estimator against `pi`, for hits at or beyond `t_min`.
#[must_use]
pub fn plane_rel_error() -> f32 {
    let n = 64_u16;
    let origin = cell(Vec3::ZERO, Vec3::Y);
    let recv = cell(Vec3::new(0.2, 0.0, 0.0), Vec3::Y);
    let mut shared_w = 0.0_f32;
    let mut shared_l = 0.0_f32;
    let mut own_l = 0.0_f32;
    let count = f32::from(n);
    for i in 0..n {
        let dir = cosine_dir(i, n);
        let t = 3.0 / dir.y.max(0.05);
        let hit = ShareHit {
            point: origin.pos + dir * t,
            normal: -dir,
            radiance: Vec3::new(1.0, 1.0, 1.0),
            t,
        };
        own_l += 1.0;
        if hit.t < T_MIN {
            continue;
        }
        let faults = Faults::default();
        if let Ok(w) = share_weight(&origin, &hit, &recv, &faults, 1) {
            shared_w += w;
            shared_l += w;
        }
    }
    let pi = std::f32::consts::PI;
    let own = pi * own_l / count;
    let shared = if shared_w > 0.0 {
        pi * shared_l / shared_w
    } else {
        0.0
    };
    let own_err = (own - pi).abs() / pi;
    let shared_err = (shared - pi).abs() / pi;
    own_err.max(shared_err)
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
}
