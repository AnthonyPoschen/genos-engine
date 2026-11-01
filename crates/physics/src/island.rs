//! Islands are the contact groups that sleep and wake together.
//!
//! A union-find over this step's dynamic pairs assigns the id. The island
//! sleeps only when every dynamic member has stayed under the speed limits
//! for the sleep time. Waking one member wakes the rest of that id.

use genos_math::Vec3;

use crate::{Body, Presence};

#[derive(Clone, Debug)]
pub struct Islands {
    parent: Vec<u32>,
    hot: Vec<u8>,
    mark: Vec<u8>,
    pairs: Vec<(u32, u32)>,
    ids: Vec<u32>,
}

impl Islands {
    pub const fn new() -> Self {
        Self {
            parent: Vec::new(),
            hot: Vec::new(),
            mark: Vec::new(),
            pairs: Vec::new(),
            ids: Vec::new(),
        }
    }

    /// Dynamic body ids for this step. Later passes do not walk static bodies.
    pub fn bind(&mut self, ids: &[u32]) {
        self.ids.clear();
        self.ids.extend_from_slice(ids);
    }

    pub fn store_pairs(&mut self, pairs: &[(u32, u32)]) {
        self.pairs.clear();
        self.pairs.extend_from_slice(pairs);
    }

    pub fn count(&mut self, bodies: &[Body]) -> u32 {
        let count = bodies.len();
        self.mark.resize(count, 0);
        self.mark.fill(0);
        let mut islands = 0_u32;
        for &id in &self.ids {
            let Some(body) = bodies.get(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            if self.mark[slot] == 0 {
                self.mark[slot] = 1;
                islands = islands.saturating_add(1);
            }
        }
        islands
    }

    /// Wake every dynamic body that shares an island with an awake one.
    pub fn complete_wake(&mut self, bodies: &mut [Body]) {
        let count = bodies.len();
        self.mark.resize(count, 0);
        self.mark.fill(0);
        for &id in &self.ids {
            let Some(body) = bodies.get(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) || body.sleeping {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            self.mark[slot] = 1;
        }
        for &id in &self.ids {
            let Some(body) = bodies.get_mut(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) || !body.sleeping {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            if self.mark[slot] == 1 {
                body.sleeping = false;
                body.sleep_time = 0.0;
            }
        }
    }

    /// A moving body, or one displaced this step, wakes the island it touches.
    pub fn wake_from_pairs(
        &mut self,
        bodies: &mut [Body],
        pairs: &[(u32, u32)],
        displaced: &[u32],
    ) {
        let count = bodies.len();
        self.mark.resize(count, 0);
        self.mark.fill(0);
        for &(left, right) in pairs {
            let (Some(a), Some(b)) = (bodies.get(index_of(left)), bodies.get(index_of(right)))
            else {
                continue;
            };
            let wake_left = a.sleeping && hitter(b, right, displaced);
            let wake_right = b.sleeping && hitter(a, left, displaced);
            if wake_left {
                if let Some(slot) = slot(a.island, count) {
                    self.mark[slot] = 1;
                }
            }
            if wake_right {
                if let Some(slot) = slot(b.island, count) {
                    self.mark[slot] = 1;
                }
            }
        }
        for &id in &self.ids {
            let Some(body) = bodies.get_mut(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            if self.mark[slot] == 1 && body.sleeping {
                body.sleeping = false;
                body.sleep_time = 0.0;
            }
        }
    }

    pub fn assign_and_sleep(
        &mut self,
        bodies: &mut [Body],
        dt: f32,
        linear: f32,
        angular: f32,
        sleep_time: f32,
    ) {
        let count = bodies.len();
        self.parent.resize(count, 0);
        for (index, parent) in self.parent.iter_mut().enumerate() {
            *parent = u32::try_from(index).unwrap_or(0);
        }
        let pairs = std::mem::take(&mut self.pairs);
        for &(left, right) in &pairs {
            let (Some(a), Some(b)) = (bodies.get(index_of(left)), bodies.get(index_of(right)))
            else {
                continue;
            };
            if dynamic_live(a) && dynamic_live(b) && !a.sleeping && !b.sleeping {
                self.union(left, right);
            }
        }
        self.pairs = pairs;
        let id_count = self.ids.len();
        for slot in 0..id_count {
            let id = self.ids[slot];
            let live = bodies
                .get(index_of(id))
                .is_some_and(|body| dynamic_live(body) && !body.sleeping);
            if !live {
                continue;
            }
            let root = self.find(id);
            if let Some(body) = bodies.get_mut(index_of(id)) {
                body.island = root;
            }
        }
        self.hot.resize(count, 0);
        self.hot.fill(0);
        for &id in &self.ids {
            let Some(body) = bodies.get_mut(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) || body.sleeping {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            let calm = body.velocity.length() < linear
                && body.angular_velocity.length() < angular
                && !body.motor
                && body.spring.is_none();
            if calm {
                body.sleep_time += dt;
            } else {
                body.sleep_time = 0.0;
                self.hot[slot] = 1;
            }
        }
        for &id in &self.ids {
            let Some(body) = bodies.get_mut(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) || body.sleeping {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            if self.hot[slot] == 1 {
                body.sleep_time = 0.0;
            }
        }
        self.mark.resize(count, 0);
        self.mark.fill(1);
        for &id in &self.ids {
            let Some(body) = bodies.get(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) || body.sleeping {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            if self.hot[slot] == 1 || body.sleep_time < sleep_time {
                self.mark[slot] = 0;
            }
        }
        for &id in &self.ids {
            let Some(body) = bodies.get_mut(index_of(id)) else {
                continue;
            };
            if !dynamic_live(body) || body.sleeping {
                continue;
            }
            let Some(slot) = slot(body.island, count) else {
                continue;
            };
            if self.mark[slot] == 1 {
                body.sleeping = true;
                body.velocity = Vec3::ZERO;
                body.angular_velocity = Vec3::ZERO;
            }
        }
    }

    fn find(&mut self, mut index: u32) -> u32 {
        while self.parent.get(index_of(index)).copied().unwrap_or(index) != index {
            let parent = self.parent[index_of(index)];
            let grand = self.parent.get(index_of(parent)).copied().unwrap_or(parent);
            self.parent[index_of(index)] = grand;
            index = parent;
        }
        index
    }

    fn union(&mut self, a: u32, b: u32) {
        let root_a = self.find(a);
        let root_b = self.find(b);
        if root_a == root_b {
            return;
        }
        // Lower index stays the root so the id is stable.
        let (root, child) = if root_a < root_b {
            (root_a, root_b)
        } else {
            (root_b, root_a)
        };
        self.parent[index_of(child)] = root;
    }
}

fn dynamic_live(body: &Body) -> bool {
    body.presence == Presence::Live && body.inverse_mass > 0.0
}

fn hitter(body: &Body, index: u32, displaced: &[u32]) -> bool {
    if displaced.contains(&index) {
        return true;
    }
    !body.sleeping
        && body.inverse_mass > 0.0
        && (body.velocity.length() > 0.25 || body.angular_velocity.length() > 0.4)
}

fn index_of(index: u32) -> usize {
    usize::try_from(index).unwrap_or(0)
}

fn slot(island: u32, count: usize) -> Option<usize> {
    let index = usize::try_from(island).ok()?;
    (index < count).then_some(index)
}
