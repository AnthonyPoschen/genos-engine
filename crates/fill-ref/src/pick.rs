//! `need(p) = prox(p) * max(under, stale, disagree)`.

/// `1 / max(d, 0.5 m)`. Near errors win ties.
#[must_use]
pub fn prox(distance_m: f32) -> f32 {
    1.0 / distance_m.max(0.5)
}

/// `max(0, 1 - own_count / fill_target)`. A converged cell is zero.
#[must_use]
pub fn under(own_count: u32, target: u32) -> f32 {
    if target == 0 {
        return 0.0;
    }
    let own = f32::from(u16::try_from(own_count.min(u32::from(u16::MAX))).unwrap_or(u16::MAX));
    let cap = f32::from(u16::try_from(target.min(u32::from(u16::MAX))).unwrap_or(u16::MAX));
    (1.0 - own / cap).max(0.0)
}

/// The pixel weight. Each term is already in `0..=1` except `prox`.
#[must_use]
pub fn need(distance_m: f32, own_count: u32, target: u32, stale: f32, disagree: f32) -> f32 {
    prox(distance_m) * under(own_count, target).max(stale).max(disagree)
}

/// A far tile got more picks than a converged near tile.
#[must_use]
pub fn far_is_fed(far_picks: u32, near_picks: u32) -> bool {
    far_picks > near_picks
}

/// One tile's pixel draw. `m` picks, without replacement, proportional to need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelPick {
    pub pixels: Vec<usize>,
}

impl PixelPick {
    #[must_use]
    pub fn weighted(needs: &[f32], m: u32, seed: u32) -> Self {
        Self {
            pixels: draw(needs, m, seed, false),
        }
    }

    /// The uniform-pick fault: need is ignored, so a far pixel is not preferred.
    #[must_use]
    pub fn uniform(needs: &[f32], m: u32, seed: u32) -> Self {
        Self {
            pixels: draw(needs, m, seed, true),
        }
    }
}

fn draw(needs: &[f32], m: u32, seed: u32, uniform: bool) -> Vec<usize> {
    let mut keyed: Vec<(u32, usize)> = Vec::new();
    for (index, need) in needs.iter().copied().enumerate() {
        if !need.is_finite() || need <= 0.0 {
            continue;
        }
        let weight = if uniform { 1.0 } else { need };
        let at = u32::try_from(index).unwrap_or(0);
        keyed.push((rank(seed, at, weight), index));
    }
    keyed.sort_by_key(|row| row.0);
    let take = usize_from(m).min(keyed.len());
    keyed.iter().take(take).map(|row| row.1).collect()
}

fn rank(seed: u32, index: u32, weight: f32) -> u32 {
    // Efraimidis-Spirakis key, stored as a u32 so the sort needs no float order.
    // A larger key is a smaller rank (we sort ascending and take the front).
    let unit = unit_hash(seed, index);
    let key = unit.powf(1.0 / weight.max(1.0e-6));
    let scaled = key * 1_000_000.0;
    whole_u32(scaled).wrapping_neg()
}

fn unit_hash(seed: u32, index: u32) -> f32 {
    let mixed = mix(seed.wrapping_add(index).wrapping_add(1));
    let low = u16::try_from(mixed & 0xffff).unwrap_or(1).max(1);
    f32::from(low) / 65536.0
}

pub(crate) fn mix(mut x: u32) -> u32 {
    x = x.wrapping_mul(0x9E37_79B1);
    x ^= x >> 16;
    x
}

fn whole_u32(v: f32) -> u32 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    if v >= 65535.0 {
        return 65535;
    }
    let bits = v.to_bits();
    let exp = i32::from(u8::try_from((bits >> 23) & 0xff).unwrap_or(0)) - 127;
    if exp < 0 {
        return 0;
    }
    let frac = (bits & 0x7f_ffff) | 0x80_0000;
    let shift = 23 - exp;
    if shift <= 0 {
        return 65535;
    }
    frac >> u32::try_from(shift).unwrap_or(23)
}

fn usize_from(v: u32) -> usize {
    usize::try_from(v).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prox_floors_at_half_a_metre_and_under_hits_zero_at_the_target() {
        let near = prox(0.1);
        let far = prox(40.0);
        assert!(near > far);
        assert!((prox(0.1) - 2.0).abs() < 1.0e-5);
        assert!(under(512, 512) <= 0.0);
        assert!(under(0, 512) > 0.9);
        let cold_far = need(40.0, 0, 512, 0.0, 0.0);
        let done_near = need(2.0, 512, 512, 0.0, 0.0);
        assert!(cold_far > done_near);
    }

    #[test]
    fn a_zero_need_pixel_is_not_drawn_and_the_only_need_is() {
        let needs = [0.0, 0.0, 4.0, 0.0];
        let pick = PixelPick::weighted(&needs, 1, 7);
        assert_eq!(pick.pixels, vec![2]);
    }

    #[test]
    fn the_same_seed_draws_the_same_pixels() {
        let needs = [0.2, 0.8, 0.1, 0.4];
        let a = PixelPick::weighted(&needs, 2, 3);
        let b = PixelPick::weighted(&needs, 2, 3);
        assert_eq!(a, b);
        assert_eq!(a.pixels.len(), 2);
    }
}
