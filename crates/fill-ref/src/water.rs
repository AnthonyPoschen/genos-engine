//! Water-filling over tiles. 64 log buckets. Higher buckets are filled first.

use crate::pick::{mix, need};

pub const BUCKETS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct TileNeed {
    pub id: u32,
    pub need: f32,
}

/// Log bucket. Zero need stays in bucket 0 and is not filled.
#[must_use]
pub fn bucket(need: f32) -> usize {
    if !need.is_finite() || need <= 0.0 {
        return 0;
    }
    let t = ((need.log2() + 16.0) / 18.0).clamp(0.0, 1.0);
    let scaled = t * 62.0;
    let idx = whole_small(scaled).saturating_add(1);
    usize::try_from(idx.min(63)).unwrap_or(BUCKETS - 1)
}

/// Picks per tile. The sum is at most `ceiling`, and each tile is at most `max_per`.
#[must_use]
pub fn assign(tiles: &[TileNeed], ceiling: u32, max_per: u32) -> Vec<u32> {
    let mut picks = vec![0_u32; tiles.len()];
    if ceiling == 0 || max_per == 0 {
        return picks;
    }
    let mut groups: Vec<Vec<usize>> = (0..BUCKETS).map(|_| Vec::new()).collect();
    for (index, tile) in tiles.iter().enumerate() {
        groups[bucket(tile.need)].push(index);
    }
    let mut remaining = ceiling;
    for level in (1..BUCKETS).rev() {
        if remaining == 0 {
            break;
        }
        let group = &groups[level];
        if group.is_empty() {
            continue;
        }
        let count = u32::try_from(group.len()).unwrap_or(u32::MAX);
        let room = remaining / count.max(1);
        let base = room.min(max_per);
        let used_base = base.saturating_mul(count.min(remaining));
        let leftover = remaining.saturating_sub(used_base);
        let extra_cap = max_per.saturating_sub(base);
        let extra = leftover.min(extra_cap.saturating_mul(count));
        admit(group, tiles, base, extra, max_per, &mut picks, &mut remaining);
    }
    picks
}

fn admit(
    group: &[usize],
    tiles: &[TileNeed],
    base: u32,
    extra: u32,
    max_per: u32,
    picks: &mut [u32],
    remaining: &mut u32,
) {
    let mut order: Vec<usize> = group.to_vec();
    order.sort_by_key(|&index| (mix(tiles[index].id), tiles[index].id));
    let mut left = extra;
    for index in order {
        if *remaining == 0 {
            break;
        }
        let mut m = base;
        if left > 0 && m < max_per {
            m = m.saturating_add(1);
            left -= 1;
        }
        m = m.min(max_per).min(*remaining);
        picks[index] = m;
        *remaining -= m;
    }
}

fn whole_small(v: f32) -> u32 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    if v >= 62.0 {
        return 62;
    }
    let bits = v.to_bits();
    let exp = i32::from(u8::try_from((bits >> 23) & 0xff).unwrap_or(0)) - 127;
    if exp < 0 {
        return 0;
    }
    let frac = (bits & 0x7f_ffff) | 0x80_0000;
    let shift = 23 - exp;
    if shift <= 0 {
        return 62;
    }
    frac >> u32::try_from(shift).unwrap_or(23)
}

pub struct Budget {
    pub total: u32,
    pub far: u32,
    pub within_ceiling: bool,
}

/// One far cold tile and several near converged tiles, under a 1M-style ceiling.
#[must_use]
pub fn hall_budget() -> Budget {
    let tiles = hall_tiles();
    let ceiling = 8;
    let picks = assign(&tiles, ceiling, 4);
    let total = picks.iter().copied().fold(0, u32::saturating_add);
    Budget {
        total,
        far: picks.first().copied().unwrap_or(0),
        within_ceiling: total <= ceiling && picks.iter().all(|p| *p <= 4),
    }
}

#[must_use]
pub fn hall_tiles() -> Vec<TileNeed> {
    // The far tile is short of samples. The near tiles still have a little
    // need, so a uniform pick spends on them too.
    let mut tiles = vec![TileNeed {
        id: 1,
        need: need(40.0, 0, 512, 0.0, 0.0),
    }];
    for id in 2..9 {
        tiles.push(TileNeed {
            id,
            need: need(2.0, 500, 512, 0.0, 0.0),
        });
    }
    tiles
}

/// Uniform pick: one each, in id order, until the ceiling. Need is ignored.
#[must_use]
pub fn uniform_assign(tiles: &[TileNeed], ceiling: u32) -> Vec<u32> {
    let mut picks = vec![0_u32; tiles.len()];
    let mut left = ceiling;
    let mut order: Vec<usize> = (0..tiles.len()).collect();
    order.sort_by_key(|&index| tiles[index].id);
    for index in order {
        if left == 0 {
            break;
        }
        if !tiles[index].need.is_finite() || tiles[index].need <= 0.0 {
            continue;
        }
        picks[index] = 1;
        left -= 1;
    }
    picks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pick::far_is_fed;

    #[test]
    fn the_sum_stays_inside_the_ceiling_and_a_higher_bucket_is_not_crowded_out() {
        let tiles = [
            TileNeed { id: 1, need: 1.0 },
            TileNeed { id: 2, need: 0.25 },
            TileNeed { id: 3, need: 0.01 },
            TileNeed { id: 4, need: 0.0 },
        ];
        let picks = assign(&tiles, 3, 4);
        let total = picks.iter().copied().fold(0, u32::saturating_add);
        assert!(total <= 3);
        assert_eq!(picks[3], 0);
        assert!(picks[0] >= picks[1]);
        assert!(picks[1] >= picks[2]);
        assert!(picks[0] > picks[2]);
    }

    #[test]
    fn a_crossing_bucket_does_not_take_from_the_bucket_above_it() {
        let high = bucket(1.0);
        let low = bucket(0.05);
        assert!(high > low);
        let mut tiles = Vec::new();
        for id in 0..6 {
            tiles.push(TileNeed { id, need: 1.0 });
        }
        for id in 6..20 {
            tiles.push(TileNeed { id, need: 0.05 });
        }
        let picks = assign(&tiles, 10, 4);
        let high_sum: u32 = picks[..6].iter().copied().fold(0, u32::saturating_add);
        let low_sum: u32 = picks[6..].iter().copied().fold(0, u32::saturating_add);
        assert_eq!(high_sum, 10);
        assert_eq!(low_sum, 0);
    }

    #[test]
    fn uniform_pick_starves_the_far_tile() {
        let tiles = hall_tiles();
        let weighted = assign(&tiles, 8, 4);
        let uniform = uniform_assign(&tiles, 8);
        assert!(far_is_fed(weighted[0], weighted[1]));
        assert!(!far_is_fed(uniform[0], uniform[1]));
        assert!(weighted[0] > uniform[0]);
    }
}
