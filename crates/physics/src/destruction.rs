//! Hot-swap and `apply_damage` for a cooked fracture asset.
//!
//! There is no fixed-joint solver. A chunk that can still walk bonds to `WORLD`
//! stays inside the intact static body. A remaining bond between two dynamic
//! chunks does not pull them together. Culled debris stay in `bodies` so later
//! indices do not move. A contact impulse on the intact body is applied to the
//! largest non-world bond. That is the seam, not a per-contact manifold.
//! Chunk-versus-chunk contacts do not call `apply_damage`.

use genos_fracture::{Asset, HULL};
use genos_math::Vec3;

use crate::{Body, Presence, World};

const DEBRIS_VOLUME: f32 = 0.02;
const DEBRIS_SLEEP_SECONDS: f32 = 0.1;
const DEBRIS_SLEEP_SPEED: f32 = 0.4;

#[derive(Clone, Debug)]
pub struct Breakable {
    pub asset: Asset,
    intact: Option<usize>,
    chunk_bodies: Vec<Option<usize>>,
    sleep: Vec<f32>,
    pub debris_budget: u32,
    pub debris_volume: f32,
    pub allow_slice: bool,
}

/// Counts the metrics harness prints for a broken asset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DestructionReport {
    pub bonds_broken: u32,
    pub debris_alive: u32,
    pub floor_penetration_m: f32,
}

/// Insert the asset as one body. Chunks are not bodies until a bond breaks.
pub fn spawn_breakable(world: &mut World, asset: Asset, debris_budget: u32) -> usize {
    let supported = asset.chunks.iter().any(|chunk| chunk.support);
    let (center, half) = union_bounds(&asset);
    let mass = if supported { 0.0 } else { 1.0 };
    let body = Body::cuboid(center, mass, half);
    let intact = world.insert(body);
    let count = asset.chunks.len();
    world.breakables.push(Breakable {
        asset,
        intact: Some(intact),
        chunk_bodies: vec![None; count],
        sleep: vec![0.0; count],
        debris_budget,
        debris_volume: DEBRIS_VOLUME,
        allow_slice: false,
    });
    world.breakables.len() - 1
}

/// Subtract `amount` from the bond. Health at or below 0 swaps the unsupported side.
pub fn apply_damage(world: &mut World, breakable: usize, bond: usize, amount: f32) -> bool {
    if !(amount > 0.0) {
        return false;
    }
    let Some(item) = world.breakables.get_mut(breakable) else {
        return false;
    };
    let Some(link) = item.asset.bonds.get_mut(bond) else {
        return false;
    };
    if link.health <= 0.0 {
        return false;
    }
    link.health -= amount;
    if link.health > 0.0 {
        return false;
    }
    link.health = 0.0;
    activate_unsupported(world, breakable);
    true
}

#[must_use]
pub fn destruction_report(world: &World) -> DestructionReport {
    let mut bonds_broken = 0_u32;
    let mut debris_alive = 0_u32;
    let mut floor = 0.0_f32;
    for item in &world.breakables {
        for bond in &item.asset.bonds {
            if bond.chunk_b != HULL && bond.health <= 0.0 {
                bonds_broken = bonds_broken.saturating_add(1);
            }
        }
        for (index, body_index) in item.chunk_bodies.iter().enumerate() {
            let Some(body_index) = body_index else {
                continue;
            };
            let Some(chunk) = item.asset.chunks.get(index) else {
                continue;
            };
            let Some(body) = world.bodies.get(*body_index) else {
                continue;
            };
            if chunk.volume < item.debris_volume && body.presence != Presence::Culled {
                debris_alive = debris_alive.saturating_add(1);
            }
            if body.presence == Presence::Culled {
                continue;
            }
            let sunk = -chunk.half.y - body.position.y;
            if sunk > floor {
                floor = sunk;
            }
        }
    }
    DestructionReport {
        bonds_broken,
        debris_alive,
        floor_penetration_m: floor.max(0.0),
    }
}

pub(crate) fn after_step(world: &mut World, dt: f32) {
    damage_from_contacts(world);
    sleep_debris(world, dt);
    cull_debris(world);
}

fn damage_from_contacts(world: &mut World) {
    let mut hits = Vec::new();
    for (index, item) in world.breakables.iter().enumerate() {
        let Some(intact) = item.intact else {
            continue;
        };
        let Some(body) = world.bodies.get(intact) else {
            continue;
        };
        if body.presence != Presence::Live || body.normal_impulse <= 0.0 {
            continue;
        }
        if let Some(bond) = largest_seam(&item.asset) {
            hits.push((index, bond, body.normal_impulse));
        }
    }
    for (index, bond, impulse) in hits {
        let _ = apply_damage(world, index, bond, impulse);
    }
}

fn largest_seam(asset: &Asset) -> Option<usize> {
    let mut best = None;
    let mut best_area = 0.0_f32;
    for (index, bond) in asset.bonds.iter().enumerate() {
        if bond.chunk_b == HULL || bond.health <= 0.0 {
            continue;
        }
        if best.is_none() || bond.area > best_area {
            best = Some(index);
            best_area = bond.area;
        }
    }
    best
}

fn activate_unsupported(world: &mut World, breakable: usize) {
    let (intact, spawn) = {
        let Some(item) = world.breakables.get(breakable) else {
            return;
        };
        let supported = support_mask(&item.asset);
        let spawn = chunks_to_spawn(item, &supported);
        (item.intact, spawn)
    };
    let (linear, angular) = intact
        .and_then(|index| world.bodies.get(index))
        .map_or((Vec3::ZERO, Vec3::ZERO), |body| {
            (body.velocity, body.angular_velocity)
        });
    for index in spawn {
        spawn_chunk(world, breakable, index, linear, angular);
    }
    refit_intact(world, breakable);
}

fn chunks_to_spawn(item: &Breakable, supported: &[bool]) -> Vec<usize> {
    let mut spawn = Vec::new();
    for (index, chunk) in item.asset.chunks.iter().enumerate() {
        if supported.get(index).copied().unwrap_or(true) {
            continue;
        }
        if chunk.child_count > 0 || item.chunk_bodies.get(index).is_some_and(Option::is_some) {
            continue;
        }
        if chunk.parent != HULL {
            let parent = usize::try_from(chunk.parent).unwrap_or(0);
            if supported.get(parent).copied().unwrap_or(false) {
                continue;
            }
        }
        spawn.push(index);
    }
    spawn
}

fn support_mask(asset: &Asset) -> Vec<bool> {
    let count = asset.chunks.len();
    let mut supported = vec![false; count];
    let mut pending = Vec::new();
    for bond in &asset.bonds {
        if bond.health <= 0.0 || bond.chunk_b != HULL {
            continue;
        }
        let Some(index) = chunk_index(bond.chunk_a, count) else {
            continue;
        };
        if !supported[index] {
            supported[index] = true;
            pending.push(index);
        }
    }
    while let Some(index) = pending.pop() {
        for bond in &asset.bonds {
            if bond.health <= 0.0 || bond.chunk_b == HULL {
                continue;
            }
            let Some(a) = chunk_index(bond.chunk_a, count) else {
                continue;
            };
            let Some(b) = chunk_index(bond.chunk_b, count) else {
                continue;
            };
            if a == index && !supported[b] {
                supported[b] = true;
                pending.push(b);
            } else if b == index && !supported[a] {
                supported[a] = true;
                pending.push(a);
            }
        }
    }
    supported
}

fn chunk_index(index: u32, count: usize) -> Option<usize> {
    let index = usize::try_from(index).ok()?;
    (index < count).then_some(index)
}

fn spawn_chunk(world: &mut World, breakable: usize, index: usize, linear: Vec3, angular: Vec3) {
    let Some(chunk) = world
        .breakables
        .get(breakable)
        .and_then(|item| item.asset.chunks.get(index).cloned())
    else {
        return;
    };
    let debris_volume = world
        .breakables
        .get(breakable)
        .map_or(DEBRIS_VOLUME, |item| item.debris_volume);
    let mass = (chunk.volume * 1_000.0).max(0.01);
    let mut body = Body::cuboid(chunk.center, mass, chunk.half);
    body.velocity = linear;
    body.angular_velocity = angular;
    body.debris = chunk.volume < debris_volume;
    let body_index = world.insert(body);
    if let Some(slot) = world
        .breakables
        .get_mut(breakable)
        .and_then(|item| item.chunk_bodies.get_mut(index))
    {
        *slot = Some(body_index);
    }
}

fn refit_intact(world: &mut World, breakable: usize) {
    let Some(intact) = world.breakables.get(breakable).and_then(|item| item.intact) else {
        return;
    };
    let bounds = world.breakables.get(breakable).and_then(solid_bounds);
    let Some((min, max)) = bounds else {
        if let Some(body) = world.bodies.get_mut(intact) {
            body.presence = Presence::Culled;
            body.velocity = Vec3::ZERO;
            body.angular_velocity = Vec3::ZERO;
            body.inverse_mass = 0.0;
        }
        if let Some(item) = world.breakables.get_mut(breakable) {
            item.intact = None;
        }
        return;
    };
    if let Some(body) = world.bodies.get_mut(intact) {
        body.position = (min + max) * 0.5;
        body.shape = crate::Shape::Box {
            half_extents: (max - min) * 0.5,
        };
        body.inverse_mass = 0.0;
        body.velocity = Vec3::ZERO;
        body.angular_velocity = Vec3::ZERO;
    }
}

fn solid_bounds(item: &Breakable) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
    let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
    let mut any = false;
    for (index, chunk) in item.asset.chunks.iter().enumerate() {
        if !represents_solid(item, index) {
            continue;
        }
        any = true;
        let low = chunk.center - chunk.half;
        let high = chunk.center + chunk.half;
        min = Vec3::new(min.x.min(low.x), min.y.min(low.y), min.z.min(low.z));
        max = Vec3::new(max.x.max(high.x), max.y.max(high.y), max.z.max(high.z));
    }
    any.then_some((min, max))
}

fn represents_solid(item: &Breakable, index: usize) -> bool {
    if item.chunk_bodies.get(index).is_some_and(Option::is_some) {
        return false;
    }
    let Some(chunk) = item.asset.chunks.get(index) else {
        return false;
    };
    if chunk.child_count > 0 {
        let first = usize::try_from(chunk.first_child).unwrap_or(0);
        let count = usize::try_from(chunk.child_count).unwrap_or(0);
        let children_spawned = (0..count).any(|offset| {
            item.chunk_bodies
                .get(first.saturating_add(offset))
                .is_some_and(Option::is_some)
        });
        return !children_spawned;
    }
    chunk.parent == HULL
}

fn union_bounds(asset: &Asset) -> (Vec3, Vec3) {
    let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
    let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
    let mut any = false;
    for chunk in &asset.chunks {
        if chunk.parent != HULL {
            continue;
        }
        any = true;
        let low = chunk.center - chunk.half;
        let high = chunk.center + chunk.half;
        min = Vec3::new(min.x.min(low.x), min.y.min(low.y), min.z.min(low.z));
        max = Vec3::new(max.x.max(high.x), max.y.max(high.y), max.z.max(high.z));
    }
    if !any {
        return (Vec3::ZERO, Vec3::new(0.1, 0.1, 0.1));
    }
    ((min + max) * 0.5, (max - min) * 0.5)
}

fn sleep_debris(world: &mut World, dt: f32) {
    let item_count = world.breakables.len();
    for item_index in 0..item_count {
        let chunk_count = world
            .breakables
            .get(item_index)
            .map_or(0, |item| item.chunk_bodies.len());
        for index in 0..chunk_count {
            let Some(body_index) = debris_body(world, item_index, index) else {
                continue;
            };
            let speed = {
                let Some(body) = world.bodies.get(body_index) else {
                    continue;
                };
                if body.presence == Presence::Culled {
                    continue;
                }
                body.velocity.length()
            };
            let slept = if speed < DEBRIS_SLEEP_SPEED {
                world
                    .breakables
                    .get(item_index)
                    .and_then(|item| item.sleep.get(index).copied())
                    .unwrap_or(0.0)
                    + dt
            } else {
                0.0
            };
            if let Some(slot) = world
                .breakables
                .get_mut(item_index)
                .and_then(|item| item.sleep.get_mut(index))
            {
                *slot = slept;
            }
            if slept >= DEBRIS_SLEEP_SECONDS {
                if let Some(body) = world.bodies.get_mut(body_index) {
                    body.presence = Presence::Asleep;
                    body.velocity = Vec3::ZERO;
                    body.angular_velocity = Vec3::ZERO;
                }
            }
        }
    }
}

fn debris_body(world: &World, item_index: usize, index: usize) -> Option<usize> {
    let item = world.breakables.get(item_index)?;
    let body_index = item.chunk_bodies.get(index).copied().flatten()?;
    let chunk = item.asset.chunks.get(index)?;
    if chunk.volume < item.debris_volume {
        Some(body_index)
    } else {
        None
    }
}

fn cull_debris(world: &mut World) {
    let item_count = world.breakables.len();
    for item_index in 0..item_count {
        let mut asleep = Vec::new();
        let mut alive = 0_u32;
        let budget = world
            .breakables
            .get(item_index)
            .map_or(0, |item| item.debris_budget);
        let chunk_count = world
            .breakables
            .get(item_index)
            .map_or(0, |item| item.chunk_bodies.len());
        for index in 0..chunk_count {
            let Some(body_index) = debris_body(world, item_index, index) else {
                continue;
            };
            let Some(body) = world.bodies.get(body_index) else {
                continue;
            };
            if body.presence == Presence::Culled {
                continue;
            }
            alive = alive.saturating_add(1);
            if body.presence == Presence::Asleep {
                let volume = world
                    .breakables
                    .get(item_index)
                    .and_then(|item| item.asset.chunks.get(index))
                    .map_or(0.0, |chunk| chunk.volume);
                asleep.push((index, volume));
            }
        }
        if alive <= budget {
            continue;
        }
        asleep.sort_by(|left, right| left.1.total_cmp(&right.1));
        let mut extra = alive - budget;
        for (index, _) in asleep {
            if extra == 0 {
                break;
            }
            tombstone(world, item_index, index);
            extra -= 1;
        }
    }
}

fn tombstone(world: &mut World, breakable: usize, chunk: usize) {
    let Some(body_index) = world
        .breakables
        .get(breakable)
        .and_then(|item| item.chunk_bodies.get(chunk).copied().flatten())
    else {
        return;
    };
    if let Some(body) = world.bodies.get_mut(body_index) {
        body.presence = Presence::Culled;
        body.velocity = Vec3::ZERO;
        body.angular_velocity = Vec3::ZERO;
        body.inverse_mass = 0.0;
    }
}

/// `step` does not slice. The flag stays false unless a later game call sets it.
#[must_use]
pub fn allows_slice(world: &World, breakable: usize) -> bool {
    world
        .breakables
        .get(breakable)
        .is_some_and(|item| item.allow_slice)
}

/// The intact body, when it still stands in for unspawned chunks.
#[must_use]
pub fn intact_body(world: &World, breakable: usize) -> Option<usize> {
    world.breakables.get(breakable).and_then(|item| item.intact)
}

/// Spawned body for one chunk.
#[must_use]
pub fn chunk_body(world: &World, breakable: usize, chunk: usize) -> Option<usize> {
    world
        .breakables
        .get(breakable)
        .and_then(|item| item.chunk_bodies.get(chunk).copied().flatten())
}

#[cfg(test)]
mod tests {
    use genos_fracture::cook;
    use genos_math::Vec3;

    use super::{allows_slice, chunk_body, destruction_report, intact_body, spawn_breakable};
    use crate::{apply_damage, step, Body, CookRequest, Presence, World, GRAVITY};

    const DT: f32 = 1.0 / 60.0;

    fn cook_overhang(children: u8) -> crate::Asset {
        cook(&CookRequest {
            min: Vec3::new(-0.2, 1.0, -0.15),
            max: Vec3::new(0.8, 1.4, 0.15),
            coarse: vec![Vec3::new(-0.05, 1.2, 0.0), Vec3::new(0.5, 1.2, 0.0)],
            support: vec![true, false],
            children_per_chunk: children,
            health_per_area: 10.0,
        })
        .unwrap_or_else(|_| crate::Asset {
            health_per_area: 0.0,
            chunks: Vec::new(),
            bonds: Vec::new(),
        })
    }

    fn seam(asset: &crate::Asset) -> usize {
        asset.bond_between(0, 1).unwrap_or(usize::MAX)
    }

    fn child_range(asset: &crate::Asset, parent: usize) -> (usize, usize) {
        let Some(chunk) = asset.chunks.get(parent) else {
            return (0, 0);
        };
        let first = usize::try_from(chunk.first_child).unwrap_or(0);
        let count = usize::try_from(chunk.child_count).unwrap_or(0);
        (first, count)
    }

    #[test]
    fn an_undamaged_breakable_stays_one_body() {
        let asset = cook_overhang(4);
        assert!(asset.chunks.len() > 2, "cook produced no chunks");
        let mut world = World::new(GRAVITY);
        world.insert(Body::plane(Vec3::Y, 0.0));
        let id = spawn_breakable(&mut world, asset, 8);
        for _ in 0..90 {
            step(&mut world, DT);
        }
        let report = destruction_report(&world);
        assert_eq!(report.bonds_broken, 0);
        assert_eq!(report.debris_alive, 0);
        assert!(!allows_slice(&world, id));
        assert!(intact_body(&world, id).is_some());
        let intact = intact_body(&world, id).unwrap_or(0);
        let body = world.bodies.get(intact).copied().unwrap_or_else(rest_body);
        assert_eq!(body.inverse_mass, 0.0);
        assert!(chunk_body(&world, id, 0).is_none());
    }

    #[test]
    fn apply_damage_spawns_only_the_unsupported_children() {
        let asset = cook_overhang(4);
        let bond = seam(&asset);
        let (free_first, free_count) = child_range(&asset, 1);
        let (held_first, held_count) = child_range(&asset, 0);
        assert_eq!(free_count, 4, "free branch children");
        assert!(bond < asset.bonds.len(), "missing seam bond");
        let mut world = World::new(Vec3::ZERO);
        let id = spawn_breakable(&mut world, asset, 8);
        let intact = intact_body(&world, id).unwrap_or(0);
        if let Some(body) = world.bodies.get_mut(intact) {
            body.velocity = Vec3::new(1.0, 2.0, 3.0);
            body.angular_velocity = Vec3::new(0.1, 0.2, 0.3);
        }
        assert!(apply_damage(&mut world, id, bond, 1.0e6));
        let report = destruction_report(&world);
        assert!(report.bonds_broken >= 1);
        for offset in 0..free_count {
            let chunk = free_first + offset;
            let body_index = chunk_body(&world, id, chunk).unwrap_or(usize::MAX);
            let body = world
                .bodies
                .get(body_index)
                .copied()
                .unwrap_or_else(rest_body);
            assert!(
                chunk_body(&world, id, chunk).is_some(),
                "free child {chunk}"
            );
            assert_eq!(body.velocity, Vec3::new(1.0, 2.0, 3.0));
            assert_eq!(body.angular_velocity, Vec3::new(0.1, 0.2, 0.3));
            let volume = world
                .breakables
                .get(id)
                .and_then(|item| item.asset.chunks.get(chunk))
                .map_or(1.0, |item| item.volume);
            assert_eq!(body.debris, volume < 0.02);
        }
        for offset in 0..held_count {
            assert!(
                chunk_body(&world, id, held_first + offset).is_none(),
                "supported child spawned"
            );
        }
        let intact = intact_body(&world, id).unwrap_or(0);
        let body = world.bodies.get(intact).copied().unwrap_or_else(rest_body);
        assert_eq!(body.inverse_mass, 0.0);
    }

    #[test]
    fn debris_from_the_overhang_rest_on_the_floor() {
        let first = drop_overhang();
        let second = drop_overhang();
        assert_eq!(first.0.bodies, second.0.bodies);
        let report = first.1;
        assert!(report.bonds_broken >= 1, "bonds {}", report.bonds_broken);
        assert!(report.debris_alive >= 1, "debris {}", report.debris_alive);
        assert!(report.debris_alive <= 8, "debris {}", report.debris_alive);
        assert!(
            report.floor_penetration_m <= 0.0,
            "floor {}",
            report.floor_penetration_m
        );
        let mut fell = false;
        for body in &first.0.bodies {
            if body.debris && body.presence != Presence::Culled && body.position.y < 0.5 {
                fell = true;
            }
        }
        assert!(fell, "debris stayed in the air");
    }

    fn drop_overhang() -> (World, super::DestructionReport) {
        let asset = cook_overhang(4);
        let bond = seam(&asset);
        let mut world = World::new(GRAVITY);
        world.insert(Body::plane(Vec3::Y, 0.0));
        let id = spawn_breakable(&mut world, asset, 8);
        for index in 0..90 {
            if index == 30 {
                let _ = apply_damage(&mut world, id, bond, 1.0e6);
            }
            step(&mut world, DT);
        }
        let report = destruction_report(&world);
        (world, report)
    }

    #[test]
    fn debris_sleeps_and_the_budget_keeps_the_largest() {
        let asset = cook_overhang(12);
        let bond = seam(&asset);
        let (free_first, free_count) = child_range(&asset, 1);
        assert!(free_count >= 12, "free children {free_count}");
        let mut world = World::new(Vec3::ZERO);
        let gameplay = world.insert(Body::cuboid(
            Vec3::new(0.0, 3.0, 2.0),
            1.0,
            Vec3::new(0.2, 0.2, 0.2),
        ));
        if let Some(body) = world.bodies.get_mut(gameplay) {
            body.velocity = Vec3::new(0.05, 0.0, 0.0);
        }
        let id = spawn_breakable(&mut world, asset, 8);
        assert!(apply_damage(&mut world, id, bond, 1.0e6));
        let mut ranked = Vec::new();
        for offset in 0..free_count {
            let chunk = free_first + offset;
            let Some(body_index) = chunk_body(&world, id, chunk) else {
                continue;
            };
            let volume = world
                .breakables
                .get(id)
                .and_then(|item| item.asset.chunks.get(chunk))
                .map_or(0.0, |item| item.volume);
            ranked.push((body_index, volume));
            if let Some(body) = world.bodies.get_mut(body_index) {
                body.position.y += 4.0;
                body.velocity = Vec3::new(0.0, 0.05, 0.0);
            }
        }
        assert!(ranked.len() >= 12, "spawned {}", ranked.len());
        ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
        let awake = ranked.first().map_or(0, |pair| pair.0);
        if let Some(body) = world.bodies.get_mut(awake) {
            body.velocity = Vec3::new(0.0, 10.0, 0.0);
        }
        for _ in 0..6 {
            step(&mut world, DT);
        }
        let report = destruction_report(&world);
        assert_eq!(report.debris_alive, 8, "budget");
        let awake_body = world.bodies.get(awake).copied().unwrap_or_else(rest_body);
        assert_eq!(awake_body.presence, Presence::Live);
        let gameplay_body = world
            .bodies
            .get(gameplay)
            .copied()
            .unwrap_or_else(rest_body);
        assert_eq!(gameplay_body.presence, Presence::Live);
        let mut culled = 0_u32;
        for (body_index, _) in &ranked {
            let body = world
                .bodies
                .get(*body_index)
                .copied()
                .unwrap_or_else(rest_body);
            if body.presence == Presence::Culled {
                culled = culled.saturating_add(1);
            }
        }
        assert_eq!(culled, 4);
    }

    #[test]
    fn a_sphere_impact_breaks_the_seam() {
        let mut asset = cook_overhang(4);
        for bond in &mut asset.bonds {
            if bond.chunk_b != genos_fracture::HULL {
                bond.health = 1.0e-4;
            }
        }
        let (free_first, free_count) = child_range(&asset, 1);
        let mut world = World::new(GRAVITY);
        world.insert(Body::plane(Vec3::Y, 0.0));
        let id = spawn_breakable(&mut world, asset, 8);
        let mut sphere = Body::sphere(Vec3::new(2.0, 1.2, 0.0), 5.0, 0.2);
        sphere.velocity = Vec3::new(-40.0, 0.0, 0.0);
        world.insert(sphere);
        let mut broke = false;
        for _ in 0..20 {
            step(&mut world, DT);
            if destruction_report(&world).bonds_broken >= 1 {
                broke = true;
                break;
            }
        }
        assert!(broke, "sphere left the seam intact");
        let mut spawned = 0_u32;
        for offset in 0..free_count {
            if chunk_body(&world, id, free_first + offset).is_some() {
                spawned = spawned.saturating_add(1);
            }
        }
        assert!(spawned >= 1, "free children stayed inside the intact body");
    }

    fn rest_body() -> Body {
        Body::sphere(Vec3::ZERO, 1.0, 0.1)
    }
}
