//! Sparse occluder cells in a box centred on the camera.
//!
//! The cell stays 2 m. The box is snapped to those cells and its half-extent is the
//! camera far plane. Only cells a shape overlaps are stored. Those cells sit in 16 m
//! bricks. One word per brick says where that brick is, so a ray jumps empty bricks
//! with one load and walks 2 m cells only inside a brick that holds a shape.
//! `scene_rays.glsl` reads the same words.

use crate::pack::{GpuOcc, SceneGrid, OCC_CELL};

const HEAD: usize = 8;
/// Fine cells on one edge of a brick. The brick is 16 m.
const COARSE: i32 = 8;
/// One slot per fine cell in a brick. Zero means the cell is empty.
const FINE_SLOTS: usize = 512;

/// Words of the occluder directory, the box origin, and the cell counts on x, y, z.
///
/// Header: fine counts x y z, step cap, coarse counts x y, brick count, coarse count z.
/// Then one word per 16 m brick (0 when the brick holds nothing, otherwise the brick
/// word). A brick is 512 fine slots, a record count, records (local, list, count),
/// then the lists. An occupied fine slot is the word where its list count is stored.
/// An empty fine slot in that brick has the high bit set and the low bits hold the
/// chebyshev distance in cells to the nearest occupied cell, so a ray jumps that gap.
pub fn occ_directory(occs: &[GpuOcc], eye: [f32; 3], far: f32) -> ([f32; 3], [u32; 3], Vec<u32>) {
    let (origin, dims) = camera_box(eye, far);
    let mut bricks: std::collections::BTreeMap<
        (i32, i32, i32),
        std::collections::BTreeMap<u32, Vec<u32>>,
    > = std::collections::BTreeMap::new();
    for (index, occ) in occs.iter().enumerate() {
        let (lo, hi) = occ_bounds(occ);
        let Some((x0, x1)) = cell_span(lo[0], hi[0], origin[0], dims[0]) else {
            continue;
        };
        let Some((y0, y1)) = cell_span(lo[1], hi[1], origin[1], dims[1]) else {
            continue;
        };
        let Some((z0, z1)) = cell_span(lo[2], hi[2], origin[2], dims[2]) else {
            continue;
        };
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let coarse = (x >> 3, y >> 3, z >> 3);
                    bricks
                        .entry(coarse)
                        .or_default()
                        .entry(local_index(x, y, z))
                        .or_default()
                        .push(index as u32);
                }
            }
        }
    }
    let ncx = (dims[0] + COARSE as u32 - 1) / COARSE as u32;
    let ncy = (dims[1] + COARSE as u32 - 1) / COARSE as u32;
    let ncz = (dims[2] + COARSE as u32 - 1) / COARSE as u32;
    let safety = ncx + ncy + ncz + 48;
    let mut words = vec![0u32; HEAD + (ncx * ncy * ncz) as usize];
    words[0] = dims[0];
    words[1] = dims[1];
    words[2] = dims[2];
    words[3] = safety.max(4);
    words[4] = ncx;
    words[5] = ncy;
    words[6] = bricks.len() as u32;
    words[7] = ncz;
    for ((cx, cy, cz), cells) in &bricks {
        let brick = words.len() as u32;
        let slots_at = words.len();
        words.resize(slots_at + FINE_SLOTS + 1 + cells.len() * 3, 0);
        words[slots_at + FINE_SLOTS] = pack_span(cells);
        let records = slots_at + FINE_SLOTS + 1;
        for (i, (local, list)) in cells.iter().enumerate() {
            let at = records + i * 3;
            words[at] = *local;
            words[at + 2] = list.len() as u32;
        }
        for (i, (local, list)) in cells.iter().enumerate() {
            let list_at = words.len() as u32;
            words[slots_at + *local as usize] = list_at;
            words[records + i * 3 + 1] = list_at + 1;
            words.push(list.len() as u32);
            words.extend(list);
        }
        write_empty_jumps(&mut words, slots_at, cells.keys().copied());
        write_shape_runs(&mut words, slots_at, &cells);
        let index = ((*cz as u32 * ncy + *cy as u32) * ncx + *cx as u32) as usize;
        words[HEAD + index] = brick;
    }
    (origin, dims, words)
}

/// Half-extent `far`, centred on `eye` snapped down to a cell corner.
pub fn camera_box(eye: [f32; 3], far: f32) -> ([f32; 3], [u32; 3]) {
    let far = far.max(OCC_CELL);
    let n = ((2.0 * far) / OCC_CELL).round().max(1.0) as u32;
    let snapped = eye.map(|v| (v / OCC_CELL).floor() * OCC_CELL);
    let origin = snapped.map(|v| v - n as f32 * OCC_CELL * 0.5);
    (origin, [n, n, n])
}

fn cell_span(lo: f32, hi: f32, origin: f32, n: u32) -> Option<(i32, i32)> {
    if n == 0 || hi <= lo {
        return None;
    }
    let i0 = ((lo - origin) / OCC_CELL).floor() as i32;
    let t = (hi - origin) / OCC_CELL;
    let mut i1 = t.floor() as i32;
    if t <= i1 as f32 + 1.0e-4 {
        i1 -= 1;
    }
    if i1 < 0 || i0 >= n as i32 {
        return None;
    }
    Some((i0.max(0), i1.min(n as i32 - 1)))
}

fn occ_bounds(occ: &GpuOcc) -> ([f32; 3], [f32; 3]) {
    let reach = occ.reach();
    let y0 = occ.center[1] - occ.height * 0.5;
    let y1 = occ.center[1] + occ.height * 0.5;
    (
        [occ.center[0] - reach[0], y0, occ.center[2] - reach[1]],
        [occ.center[0] + reach[0], y1, occ.center[2] + reach[1]],
    )
}

struct Head {
    nx: i32,
    ny: i32,
    nz: i32,
    safety: u32,
    ncx: i32,
    ncy: i32,
    ncz: i32,
    count: u32,
    origin: [f32; 3],
    cell: f32,
}

fn head(grid: &SceneGrid) -> Option<Head> {
    if grid.words.len() < HEAD || grid.occ[2] <= 0.0 {
        return None;
    }
    let w = |i: usize| grid.words[i];
    Some(Head {
        nx: w(0) as i32,
        ny: w(1) as i32,
        nz: w(2) as i32,
        safety: w(3).max(1),
        ncx: w(4) as i32,
        ncy: w(5) as i32,
        ncz: w(7) as i32,
        count: w(6),
        origin: [grid.occ[0], grid.occ[3], grid.occ[1]],
        cell: grid.occ[2],
    })
}

/// Count in the low 10 bits, then the fine-cell box of the shapes: min x y z, max x y z,
/// three bits each. A ray that misses this box misses every shape in the brick.
fn pack_span(cells: &std::collections::BTreeMap<u32, Vec<u32>>) -> u32 {
    let mut lo = [7u32; 3];
    let mut hi = [0u32; 3];
    for &local in cells.keys() {
        let xyz = [local & 7, local >> 6, (local >> 3) & 7];
        for i in 0..3 {
            lo[i] = lo[i].min(xyz[i]);
            hi[i] = hi[i].max(xyz[i]);
        }
    }
    (cells.len() as u32 & 1023)
        | (lo[0] << 10)
        | (lo[1] << 13)
        | (lo[2] << 16)
        | (hi[0] << 19)
        | (hi[1] << 22)
        | (hi[2] << 25)
}

fn span_box(word: u32) -> ([i32; 3], [i32; 3]) {
    let axis = |shift: u32| ((word >> shift) & 7) as i32;
    (
        [axis(10), axis(13), axis(16)],
        [axis(19), axis(22), axis(25)],
    )
}

fn local_index(x: i32, y: i32, z: i32) -> u32 {
    let (fx, fy, fz) = ((x & 7) as u32, (y & 7) as u32, (z & 7) as u32);
    fx + 8 * fz + 64 * fy
}

/// Empty cells of one brick store how far a ray can jump. The high bit marks the
/// word; the low bits are the chebyshev distance in cells to an occupied cell.
const JUMP_BIT: u32 = 0x8000_0000;

fn write_empty_jumps(words: &mut [u32], slots_at: usize, occupied: impl Iterator<Item = u32>) {
    let mut dist = [7u8; FINE_SLOTS];
    let mut queue = [0u16; FINE_SLOTS];
    let mut head_q = 0usize;
    let mut tail_q = 0usize;
    for local in occupied {
        let i = local as usize;
        if i >= FINE_SLOTS {
            continue;
        }
        dist[i] = 0;
        queue[tail_q] = i as u16;
        tail_q += 1;
    }
    while head_q < tail_q {
        let cur = queue[head_q] as usize;
        head_q += 1;
        let d = dist[cur];
        if d >= 7 {
            continue;
        }
        let x = (cur & 7) as i32;
        let z = ((cur >> 3) & 7) as i32;
        let y = (cur >> 6) as i32;
        for dy in -1i32..=1 {
            for dz in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let nx = x + dx;
                    let ny = y + dy;
                    let nz = z + dz;
                    if !(0..=7).contains(&nx) || !(0..=7).contains(&ny) || !(0..=7).contains(&nz) {
                        continue;
                    }
                    let ni = nx as usize + 8 * nz as usize + 64 * ny as usize;
                    if dist[ni] > d + 1 {
                        dist[ni] = d + 1;
                        queue[tail_q] = ni as u16;
                        tail_q += 1;
                    }
                }
            }
        }
    }
    for (i, cell_dist) in dist.iter().enumerate() {
        if *cell_dist == 0 {
            continue;
        }
        words[slots_at + i] = JUMP_BIT | u32::from(*cell_dist);
    }
}

/// Bits 8.. of a cell's count word: the fine-cell box a miss can cross. Every cell
/// in that box is empty or repeats this cell's shapes, so one test covers it.
fn write_shape_runs(
    words: &mut [u32],
    slots_at: usize,
    cells: &std::collections::BTreeMap<u32, Vec<u32>>,
) {
    for (&local, mine) in cells {
        let slot = words[slots_at + local as usize] as usize;
        if slot >= words.len() {
            continue;
        }
        let (x0, y0, z0, x1, y1, z1) = safe_box(cells, mine, local);
        let packed = (words[slot] & 255)
            | (x0 << 8)
            | (y0 << 11)
            | (z0 << 14)
            | (x1 << 17)
            | (y1 << 20)
            | (z1 << 23);
        words[slot] = packed;
    }
}

fn safe_box(
    cells: &std::collections::BTreeMap<u32, Vec<u32>>,
    mine: &[u32],
    local: u32,
) -> (u32, u32, u32, u32, u32, u32) {
    let mut x0 = (local & 7) as i32;
    let mut y0 = (local >> 6) as i32;
    let mut z0 = ((local >> 3) & 7) as i32;
    let (mut x1, mut y1, mut z1) = (x0, y0, z0);
    let safe = |x: i32, y: i32, z: i32| {
        if !(0..=7).contains(&x) || !(0..=7).contains(&y) || !(0..=7).contains(&z) {
            return false;
        }
        match cells.get(&local_index(x, y, z)) {
            None => true,
            Some(theirs) => theirs.iter().all(|shape| mine.contains(shape)),
        }
    };
    loop {
        let mut grew = false;
        if (y0..=y1).all(|y| (z0..=z1).all(|z| safe(x0 - 1, y, z))) {
            x0 -= 1;
            grew = true;
        }
        if (y0..=y1).all(|y| (z0..=z1).all(|z| safe(x1 + 1, y, z))) {
            x1 += 1;
            grew = true;
        }
        if (x0..=x1).all(|x| (z0..=z1).all(|z| safe(x, y0 - 1, z))) {
            y0 -= 1;
            grew = true;
        }
        if (x0..=x1).all(|x| (z0..=z1).all(|z| safe(x, y1 + 1, z))) {
            y1 += 1;
            grew = true;
        }
        if (x0..=x1).all(|x| (y0..=y1).all(|y| safe(x, y, z0 - 1))) {
            z0 -= 1;
            grew = true;
        }
        if (x0..=x1).all(|x| (y0..=y1).all(|y| safe(x, y, z1 + 1))) {
            z1 += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    (
        x0 as u32, y0 as u32, z0 as u32, x1 as u32, y1 as u32, z1 as u32,
    )
}

fn word(grid: &SceneGrid, at: u32) -> u32 {
    grid.words.get(at as usize).copied().unwrap_or(0)
}

fn brick_at(grid: &SceneGrid, head: &Head, coarse: [i32; 3]) -> Option<u32> {
    if coarse[0] < 0
        || coarse[1] < 0
        || coarse[2] < 0
        || coarse[0] >= head.ncx
        || coarse[1] >= head.ncy
        || coarse[2] >= head.ncz
    {
        return None;
    }
    let index = ((coarse[2] as u32 * head.ncy as u32 + coarse[1] as u32) * head.ncx as u32
        + coarse[0] as u32) as u32;
    let brick = word(grid, HEAD as u32 + index);
    (brick != 0).then_some(brick)
}

fn fine_slot(grid: &SceneGrid, brick: u32, cell: [i32; 3]) -> u32 {
    word(grid, brick + local_index(cell[0], cell[1], cell[2]))
}

fn fine_list(grid: &SceneGrid, brick: u32, cell: [i32; 3]) -> Option<(u32, u32)> {
    let at = fine_slot(grid, brick, cell);
    if at == 0 || at & JUMP_BIT != 0 {
        return None;
    }
    let n = word(grid, at) & 255;
    (n != 0).then_some((at + 1, n))
}

/// Exit time of the inclusive fine-cell box `lo..=hi` inside the brick at `base`.
fn box_exit(
    head: &Head,
    origin: [f32; 3],
    inv: &[f32; 3],
    step: &[i32; 3],
    base: [i32; 3],
    lo: [i32; 3],
    hi: [i32; 3],
    t: f32,
) -> Option<f32> {
    let mut best = f32::MAX;
    for i in 0..3 {
        if step[i] == 0 {
            continue;
        }
        let face = if step[i] > 0 {
            base[i] + hi[i] + 1
        } else {
            base[i] + lo[i]
        };
        let plane = head.origin[i] + face as f32 * head.cell;
        let hit = (plane - origin[i]) * inv[i];
        if hit > t + 1.0e-5 {
            best = best.min(hit);
        }
    }
    (best < f32::MAX * 0.5).then_some(best)
}

fn safe_word(raw: u32) -> ([i32; 3], [i32; 3]) {
    let part = |shift: u32| ((raw >> shift) & 7) as i32;
    (
        [part(8), part(11), part(14)],
        [part(17), part(20), part(23)],
    )
}

/// Cells a ray may cross before the next occupied cell. One when the cell is occupied
/// or the slot has no jump.
fn fine_jump(grid: &SceneGrid, brick: u32, cell: [i32; 3]) -> u32 {
    let at = fine_slot(grid, brick, cell);
    if at & JUMP_BIT != 0 {
        (at & 15).max(1)
    } else {
        1
    }
}

fn in_box(head: &Head, cell: [i32; 3]) -> bool {
    cell[0] >= 0
        && cell[1] >= 0
        && cell[2] >= 0
        && cell[0] < head.nx
        && cell[1] < head.ny
        && cell[2] < head.nz
}

fn coarse_of(cell: [i32; 3]) -> [i32; 3] {
    [cell[0] >> 3, cell[1] >> 3, cell[2] >> 3]
}

/// Nearest occluder the ray meets inside the camera box. `steps` counts bricks crossed.
#[derive(Clone, Copy, Debug)]
pub struct GridHit {
    pub t: f32,
    pub occ: u32,
    pub steps: u32,
}

/// Walk `grid`'s stored cells. The same word layout is what `occ_walk` reads.
pub fn walk_grid(
    grid: &SceneGrid,
    occs: &[GpuOcc],
    origin: [f32; 3],
    dir: [f32; 3],
    t0: f32,
    t1: f32,
) -> Option<GridHit> {
    let head = head(grid)?;
    let dir = normalize(dir)?;
    let box_hi =
        [0, 1, 2].map(|i| head.origin[i] + [head.nx, head.ny, head.nz][i] as f32 * head.cell);
    let (mut t_enter, t_leave) = clip_box(origin, dir, head.origin, box_hi, t0, t1)?;
    if head.count == 0 {
        return None;
    }
    t_enter = t_enter.max(t0);
    if t_enter >= t1.min(t_leave) {
        return None;
    }
    let mut t = t_enter;
    let mut cell = cell_at(&head, origin, dir, t);
    let mut best_t = t1.min(t_leave);
    let mut best: Option<u32> = None;
    let mut steps = 0u32;
    let step = [0, 1, 2].map(|i| {
        if dir[i] > 1.0e-8 {
            1
        } else if dir[i] < -1.0e-8 {
            -1
        } else {
            0
        }
    });
    let inv = [0, 1, 2].map(|i| if step[i] != 0 { 1.0 / dir[i] } else { 0.0 });
    let t_delta = [0, 1, 2].map(|i| {
        if step[i] != 0 {
            head.cell * inv[i].abs()
        } else {
            f32::MAX
        }
    });
    let mut t_next = boundary_t(&head, origin, &inv, &step, cell, t, &t_delta);
    while steps < head.safety && t < best_t - 1.0e-4 {
        if !in_box(&head, cell) {
            break;
        }
        steps += 1;
        if let Some(brick) = brick_at(grid, &head, coarse_of(cell)) {
            let (lo, hi) = span_box(word(grid, brick + FINE_SLOTS as u32));
            let local = [cell[0] & 7, cell[1] & 7, cell[2] & 7];
            let in_span = (0..3).all(|i| local[i] >= lo[i] && local[i] <= hi[i]);
            if !in_span {
                match span_enter(&head, origin, &inv, &step, cell, t, best_t, lo, hi) {
                    None => {
                        let Some(jump) = coarse_exit(&head, origin, &inv, &step, cell, t) else {
                            break;
                        };
                        if jump >= best_t - 1.0e-4 {
                            break;
                        }
                        t = jump;
                        cell = cell_at(&head, origin, dir, t);
                        t_next = boundary_t(&head, origin, &inv, &step, cell, t, &t_delta);
                        continue;
                    }
                    Some(enter) if enter > t + 1.0e-4 => {
                        t = enter;
                        cell = cell_at(&head, origin, dir, t);
                        t_next = boundary_t(&head, origin, &inv, &step, cell, t, &t_delta);
                        continue;
                    }
                    Some(_) => {}
                }
            }
            let mut covered = None;
            let jump = if let Some((list, n)) = fine_list(grid, brick, cell) {
                for k in 0..n {
                    let index = word(grid, list + k);
                    let Some(occ) = occs.get(index as usize) else {
                        continue;
                    };
                    if let Some(hit) = occ_hit(occ, origin, dir, t0) {
                        if hit < best_t {
                            best_t = hit;
                            best = Some(index);
                        }
                    }
                }
                let raw = word(grid, fine_slot(grid, brick, cell));
                let (lo, hi) = safe_word(raw);
                covered = box_exit(
                    &head,
                    origin,
                    &inv,
                    &step,
                    coarse_of(cell).map(|c| c << 3),
                    lo,
                    hi,
                    t,
                );
                1
            } else {
                fine_jump(grid, brick, cell)
            };
            if let Some(leave) = covered {
                if best.is_some() && best_t <= leave {
                    break;
                }
                if leave >= best_t - 1.0e-4 {
                    break;
                }
                t = leave;
                cell = cell_at(&head, origin, dir, t);
                t_next = boundary_t(&head, origin, &inv, &step, cell, t, &t_delta);
            } else if jump <= 1 {
                let leave = t_next.into_iter().fold(f32::MAX, f32::min);
                if best.is_some() && best_t <= leave {
                    break;
                }
                if !step_cell(&mut cell, &mut t_next, &t_delta, &step, &mut t, best_t) {
                    break;
                }
            } else {
                let Some(leave) = span_exit(&head, origin, &inv, &step, cell, t, jump) else {
                    break;
                };
                if leave >= best_t - 1.0e-4 {
                    break;
                }
                t = leave;
                cell = cell_at(&head, origin, dir, t);
                t_next = boundary_t(&head, origin, &inv, &step, cell, t, &t_delta);
            }
        } else {
            let Some(jump) = coarse_exit(&head, origin, &inv, &step, cell, t) else {
                break;
            };
            if jump >= best_t - 1.0e-4 {
                break;
            }
            t = jump;
            cell = cell_at(&head, origin, dir, t);
            t_next = boundary_t(&head, origin, &inv, &step, cell, t, &t_delta);
        }
    }
    best.map(|occ| GridHit {
        t: best_t,
        occ,
        steps,
    })
}

fn boundary_t(
    head: &Head,
    origin: [f32; 3],
    inv: &[f32; 3],
    step: &[i32; 3],
    cell: [i32; 3],
    t: f32,
    t_delta: &[f32; 3],
) -> [f32; 3] {
    [0, 1, 2].map(|i| {
        if step[i] == 0 {
            return f32::MAX;
        }
        let face = cell[i] + i32::from(step[i] > 0);
        let plane = head.origin[i] + face as f32 * head.cell;
        let mut hit = (plane - origin[i]) * inv[i];
        if hit <= t + 1.0e-6 {
            hit += t_delta[i];
        }
        hit
    })
}

fn step_cell(
    cell: &mut [i32; 3],
    t_next: &mut [f32; 3],
    t_delta: &[f32; 3],
    step: &[i32; 3],
    t: &mut f32,
    limit: f32,
) -> bool {
    let mut axis = 0usize;
    if t_next[1] < t_next[axis] {
        axis = 1;
    }
    if t_next[2] < t_next[axis] {
        axis = 2;
    }
    if t_next[axis] >= limit - 1.0e-4 || step[axis] == 0 {
        return false;
    }
    *t = t_next[axis];
    cell[axis] += step[axis];
    t_next[axis] += t_delta[axis];
    true
}

/// Ray time where it leaves the empty cube of radius `jump - 1` around `cell`,
/// clipped to that cell's 16 m brick. `jump` is the chebyshev distance to an
/// occupied cell, so the cube it leaves holds no shape.
fn span_exit(
    head: &Head,
    origin: [f32; 3],
    inv: &[f32; 3],
    step: &[i32; 3],
    cell: [i32; 3],
    t: f32,
    jump: u32,
) -> Option<f32> {
    let span = (jump as i32 - 1).max(0);
    let base = coarse_of(cell).map(|c| c << 3);
    let mut best = f32::MAX;
    for i in 0..3 {
        if step[i] == 0 {
            continue;
        }
        let lo = (cell[i] - span).max(base[i]);
        let hi = (cell[i] + span).min(base[i] + COARSE - 1) + 1;
        let face = if step[i] > 0 { hi } else { lo };
        let plane = head.origin[i] + face as f32 * head.cell;
        let hit = (plane - origin[i]) * inv[i];
        if hit > t + 1.0e-5 {
            best = best.min(hit);
        }
    }
    (best < f32::MAX * 0.5 && best > t + 1.0e-5).then_some(best)
}

/// Entry time of the brick's shape box, or none when the ray misses it. `lo` and `hi`
/// are inclusive fine-cell coordinates inside the brick.
fn span_enter(
    head: &Head,
    origin: [f32; 3],
    inv: &[f32; 3],
    step: &[i32; 3],
    cell: [i32; 3],
    t: f32,
    limit: f32,
    lo: [i32; 3],
    hi: [i32; 3],
) -> Option<f32> {
    let base = coarse_of(cell).map(|c| c << 3);
    let mut tin = t;
    let mut tout = limit;
    for i in 0..3 {
        let a = head.origin[i] + (base[i] + lo[i]) as f32 * head.cell;
        let b = head.origin[i] + (base[i] + hi[i] + 1) as f32 * head.cell;
        if step[i] == 0 {
            if origin[i] < a || origin[i] > b {
                return None;
            }
            continue;
        }
        let ta = (a - origin[i]) * inv[i];
        let tb = (b - origin[i]) * inv[i];
        tin = tin.max(ta.min(tb));
        tout = tout.min(ta.max(tb));
    }
    (tin < tout - 1.0e-4).then_some(tin)
}

fn coarse_exit(
    head: &Head,
    origin: [f32; 3],
    inv: &[f32; 3],
    step: &[i32; 3],
    cell: [i32; 3],
    t: f32,
) -> Option<f32> {
    let base = coarse_of(cell).map(|c| c << 3);
    let mut best = f32::MAX;
    for i in 0..3 {
        if step[i] == 0 {
            continue;
        }
        let face = if step[i] > 0 {
            base[i] + COARSE
        } else {
            base[i]
        };
        let plane = head.origin[i] + face as f32 * head.cell;
        let hit = (plane - origin[i]) * inv[i];
        if hit > t + 1.0e-5 {
            best = best.min(hit);
        }
    }
    (best < f32::MAX * 0.5).then_some(best)
}

fn cell_at(head: &Head, origin: [f32; 3], dir: [f32; 3], t: f32) -> [i32; 3] {
    let nudge = 1.0e-3;
    [0, 1, 2].map(|i| {
        let p = origin[i] + dir[i] * (t + nudge);
        ((p - head.origin[i]) / head.cell).floor() as i32
    })
}

fn clip_box(
    origin: [f32; 3],
    dir: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
    t0: f32,
    t1: f32,
) -> Option<(f32, f32)> {
    let mut enter = t0;
    let mut leave = t1;
    for i in 0..3 {
        if dir[i].abs() < 1.0e-8 {
            if origin[i] < lo[i] || origin[i] > hi[i] {
                return None;
            }
            continue;
        }
        let mut a = (lo[i] - origin[i]) / dir[i];
        let mut b = (hi[i] - origin[i]) / dir[i];
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        enter = enter.max(a);
        leave = leave.min(b);
        if enter > leave {
            return None;
        }
    }
    (enter < leave).then_some((enter, leave))
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = v.iter().map(|c| c * c).sum::<f32>().sqrt();
    (len > 1.0e-8).then(|| v.map(|c| c / len))
}

/// Entry distance of `occ`, using the same slab and cylinder tests as `occ_ray`.
fn occ_hit(occ: &GpuOcc, origin: [f32; 3], dir: [f32; 3], t0: f32) -> Option<f32> {
    let (t_in, t_out, _) = occ_span_hit(occ, origin, dir)?;
    (t_out > t0).then_some(t_in.max(t0))
}

fn occ_span_hit(occ: &GpuOcc, origin: [f32; 3], dir: [f32; 3]) -> Option<(f32, f32, [f32; 3])> {
    let y0 = occ.center[1] - occ.height * 0.5;
    let y1 = occ.center[1] + occ.height * 0.5;
    if occ.shape > 0.5 {
        return ray_cylinder(
            origin,
            dir,
            [occ.center[0], occ.center[2]],
            occ.radius,
            y0,
            y1,
        );
    }
    let (s, c) = occ.yaw.sin_cos();
    if s.abs() > 1.0e-6 {
        let local_o = occ_local(c, s, [origin[0] - occ.center[0], origin[2] - occ.center[2]]);
        let local_d = occ_local(c, s, [dir[0], dir[2]]);
        let (t_in, t_out, n) = ray_box(
            [local_o[0], origin[1], local_o[1]],
            [local_d[0], dir[1], local_d[1]],
            [-occ.half_x, y0, -occ.half_z],
            [occ.half_x, y1, occ.half_z],
        )?;
        let nw = occ_world(c, s, [n[0], n[2]]);
        return Some((t_in, t_out, [nw[0], n[1], nw[1]]));
    }
    ray_box(
        origin,
        dir,
        [occ.center[0] - occ.half_x, y0, occ.center[2] - occ.half_z],
        [occ.center[0] + occ.half_x, y1, occ.center[2] + occ.half_z],
    )
}

fn occ_local(c: f32, s: f32, d: [f32; 2]) -> [f32; 2] {
    [c * d[0] - s * d[1], s * d[0] + c * d[1]]
}

fn occ_world(c: f32, s: f32, d: [f32; 2]) -> [f32; 2] {
    [c * d[0] + s * d[1], -s * d[0] + c * d[1]]
}

fn ray_box(
    origin: [f32; 3],
    dir: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
) -> Option<(f32, f32, [f32; 3])> {
    let mut t0 = f32::MIN / 4.0;
    let mut t1 = f32::MAX / 4.0;
    let mut normal = [0.0, 1.0, 0.0];
    for i in 0..3 {
        if dir[i].abs() < 1.0e-8 {
            if origin[i] < lo[i] || origin[i] > hi[i] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[i];
        let mut a = (lo[i] - origin[i]) * inv;
        let mut b = (hi[i] - origin[i]) * inv;
        let mut n = -1.0;
        if a > b {
            std::mem::swap(&mut a, &mut b);
            n = 1.0;
        }
        if a > t0 {
            t0 = a;
            normal = [0.0; 3];
            normal[i] = n * dir[i].signum();
        }
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    (t1 > t0 && t1 > 0.0).then_some((t0, t1, normal))
}

fn ray_cylinder(
    origin: [f32; 3],
    dir: [f32; 3],
    center: [f32; 2],
    radius: f32,
    y0: f32,
    y1: f32,
) -> Option<(f32, f32, [f32; 3])> {
    let o = [origin[0] - center[0], origin[2] - center[1]];
    let d = [dir[0], dir[2]];
    let a = d[0] * d[0] + d[1] * d[1];
    let b = 2.0 * (o[0] * d[0] + o[1] * d[1]);
    let c = o[0] * o[0] + o[1] * o[1] - radius * radius;
    let (side_in, side_out) = if a > 1.0e-8 {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return None;
        }
        let root = disc.sqrt();
        ((-b - root) / (2.0 * a), (-b + root) / (2.0 * a))
    } else if c > 0.0 {
        return None;
    } else {
        (-1.0e20, 1.0e20)
    };
    let (cap_in, cap_out) = if dir[1].abs() > 1.0e-8 {
        let t_a = (y0 - origin[1]) / dir[1];
        let t_b = (y1 - origin[1]) / dir[1];
        (t_a.min(t_b), t_a.max(t_b))
    } else if origin[1] < y0 || origin[1] > y1 {
        return None;
    } else {
        (-1.0e20, 1.0e20)
    };
    let t_in = side_in.max(cap_in);
    let t_out = side_out.min(cap_out);
    if t_out < t_in || t_out <= 0.0 {
        return None;
    }
    let normal = if side_in >= cap_in {
        let at = [
            (o[0] + d[0] * t_in) / radius.max(1.0e-4),
            (o[1] + d[1] * t_in) / radius.max(1.0e-4),
        ];
        [at[0], 0.0, at[1]]
    } else {
        [0.0, if dir[1] < 0.0 { 1.0 } else { -1.0 }, 0.0]
    };
    Some((t_in, t_out, normal))
}

/// Brute-force nearest hit, the oracle for a walk through shapes inside the box.
pub fn analytic_hit(
    occs: &[GpuOcc],
    origin: [f32; 3],
    dir: [f32; 3],
    t0: f32,
    t1: f32,
) -> Option<(f32, u32)> {
    let dir = normalize(dir)?;
    let mut best: Option<(f32, u32)> = None;
    for (index, occ) in occs.iter().enumerate() {
        if let Some(t) = occ_hit(occ, origin, dir, t0) {
            if t < t1 && best.is_none_or(|(b, _)| t < b) {
                best = Some((t, index as u32));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::build_grid;

    fn box_occ(center: [f32; 3], half: [f32; 3]) -> GpuOcc {
        GpuOcc {
            center,
            shape: 0.0,
            half_x: half[0],
            height: half[1] * 2.0,
            half_z: half[2],
            radius: 0.0,
            albedo: [1.0; 3],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
            yaw: 0.0,
        }
    }

    #[test]
    fn the_cell_stays_two_metres_when_the_far_plane_changes() {
        let eye = [3.2, 1.7, -4.0];
        let occ = box_occ([0.0, 1.0, 0.0], [0.5, 1.0, 0.5]);
        let wide = build_grid(&[], &[occ], eye, 200.0);
        let near = build_grid(&[], &[occ], eye, 40.0);
        let back = build_grid(&[], &[occ], eye, 200.0);
        assert!((wide.occ[2] - 2.0).abs() < 1.0e-6);
        assert!((near.occ[2] - 2.0).abs() < 1.0e-6);
        let span = |grid: &SceneGrid| grid.dims[0] as f32 * grid.occ[2];
        assert!((span(&wide) - 400.0).abs() < 1.0e-3);
        assert!((span(&near) - 80.0).abs() < 1.0e-3);
        assert_eq!(wide.dims[0], back.dims[0]);
        assert_eq!(wide.words.len(), back.words.len());
        // Empty air is not a dense array of the camera box.
        let dense = wide.dims[0] as usize * wide.words[1] as usize * wide.dims[1] as usize;
        assert!(
            wide.words.len() * 4 < dense,
            "{} words for {dense} cells",
            wide.words.len()
        );
    }

    #[test]
    fn a_ray_jumps_empty_space_and_hits_the_analytic_box() {
        let eye = [0.0, 1.7, 0.0];
        let lone = box_occ([0.0, 1.0, 60.0], [0.5, 1.0, 0.5]);
        let grid = build_grid(&[], &[lone], eye, 200.0);
        let from_near =
            walk_grid(&grid, &[lone], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0.0, 200.0).unwrap();
        assert!(
            from_near.steps < 8,
            "empty span took {} steps",
            from_near.steps
        );
        assert!((from_near.t - 59.5).abs() < 0.05, "{}", from_near.t);
        let from_far = walk_grid(
            &grid,
            &[lone],
            [0.0, 1.0, 80.0],
            [0.0, 0.0, -1.0],
            0.0,
            200.0,
        )
        .unwrap();
        assert!((from_far.t - 19.5).abs() < 0.05, "{}", from_far.t);
        let oracle = analytic_hit(&[lone], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0.0, 200.0).unwrap();
        assert!((from_near.t - oracle.0).abs() < 1.0e-3);
        // The side wall shares its 16 m bricks with the air the ray travels. The
        // jump still has to stop on the end wall, in a handful of steps.
        let side = box_occ([0.0, 1.0, 24.0], [0.1, 1.0, 24.0]);
        let end = box_occ([6.0, 1.0, 48.0], [8.0, 1.0, 0.1]);
        let pair = [side, end];
        let beside = build_grid(&[], &pair, [6.0, 1.7, 24.0], 200.0);
        let along = walk_grid(&beside, &pair, [0.3, 1.0, 2.0], [0.0, 0.0, 1.0], 0.0, 80.0)
            .expect("end wall");
        let along_oracle =
            analytic_hit(&pair, [0.3, 1.0, 2.0], [0.0, 0.0, 1.0], 0.0, 80.0).unwrap();
        assert_eq!(along.occ, along_oracle.1);
        assert!((along.t - along_oracle.0).abs() < 1.0e-3, "{}", along.t);
        assert!(
            along.steps < 8,
            "air beside a wall took {} steps",
            along.steps
        );
    }

    #[test]
    fn a_room_ray_matches_the_analytic_test_and_drops_shapes_outside_the_box() {
        let eye = [0.0, 1.7, 4.0];
        let wall = box_occ([0.0, 1.5, -5.0], [4.0, 1.5, 0.1]);
        let side = box_occ([-4.0, 1.5, 0.0], [0.1, 1.5, 4.0]);
        let outside = box_occ([0.0, 1.0, 500.0], [1.0, 1.0, 1.0]);
        let occs = [wall, side, outside];
        let grid = build_grid(&[], &occs, eye, 200.0);
        let origin = [0.0, 1.2, 2.0];
        let dir = [0.0, 0.0, -1.0];
        let hit = walk_grid(&grid, &occs, origin, dir, 0.05, 200.0).unwrap();
        let oracle = analytic_hit(&[wall, side], origin, dir, 0.05, 200.0).unwrap();
        assert_eq!(hit.occ, oracle.1);
        assert!((hit.t - oracle.0).abs() < 1.0e-3);
        assert!(walk_grid(&grid, &occs, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0.0, 1.0e4).is_none());
        // A ray that leaves the camera box misses, as a pixel stops at the far plane.
        assert!(walk_grid(&grid, &occs, eye, [0.0, 0.0, 1.0], 0.05, 1.0e4)
            .filter(|hit| hit.occ == 2)
            .is_none());
    }

    #[test]
    fn crossing_one_cell_scrolls_the_slab_and_keeps_world_positions() {
        let back = box_occ([-199.0, 1.0, 0.0], [0.4, 1.0, 0.4]);
        let front = box_occ([201.0, 1.0, 0.0], [0.4, 1.0, 0.4]);
        let mid = box_occ([1.0, 1.0, 1.0], [0.4, 1.0, 0.4]);
        let occs = [back, front, mid];
        let here = build_grid(&[], &occs, [0.0, 1.7, 0.0], 200.0);
        let there = build_grid(&[], &occs, [2.1, 1.7, 0.0], 200.0);
        assert_eq!(occs[0].center[0], -199.0);
        assert_eq!(occs[2].center, [1.0, 1.0, 1.0]);
        let eye = [0.0, 1.0, 3.0];
        let see = |grid: &SceneGrid, at: [f32; 3]| {
            let dir = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]];
            walk_grid(grid, &occs, eye, dir, 0.0, 400.0).map(|hit| hit.occ)
        };
        assert_eq!(see(&here, back.center), Some(0));
        assert_eq!(see(&there, back.center), None);
        assert_eq!(see(&here, front.center), None);
        assert_eq!(see(&there, front.center), Some(1));
        assert_eq!(see(&here, mid.center), Some(2));
        assert_eq!(see(&there, mid.center), Some(2));
    }
}
