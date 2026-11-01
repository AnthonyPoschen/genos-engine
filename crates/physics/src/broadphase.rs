//! Two AABB trees: static bodies in one, dynamic and debris in the other.
//!
//! A batch insert builds a median tree along X. A single insert walks the
//! existing tree. `refit` replaces either tree with a tight median build.
//! Pair order is `(min id, max id)`.

use crate::contact::{self, BOUNDS_MARGIN};
use crate::{Body, Presence};

const NONE: u32 = u32::MAX;
const LAYER_CAP: usize = 32;

/// Object-layer pair table. Layer 0 is static, 1 is dynamic, 2 is debris.
#[derive(Clone, Copy, Debug)]
pub struct LayerPairs {
    allow: [u32; LAYER_CAP],
}

impl LayerPairs {
    /// Statics miss statics. Debris misses debris. Every other pair is kept.
    pub const fn standard() -> Self {
        let mut allow = [u32::MAX; LAYER_CAP];
        allow[0] &= !1;
        allow[2] &= !(1 << 2);
        Self { allow }
    }

    #[must_use]
    pub fn allows(self, a: u16, b: u16) -> bool {
        let (Some(row), Some(col)) = (layer_index(a), layer_index(b)) else {
            return false;
        };
        let Some(bit) = 1_u32.checked_shl(u32::try_from(col).unwrap_or(u32::MAX)) else {
            return false;
        };
        self.allow[row] & bit != 0
    }

    pub fn set(&mut self, a: u16, b: u16, accept: bool) {
        let (Some(row), Some(col)) = (layer_index(a), layer_index(b)) else {
            return;
        };
        let (Some(bit_col), Some(bit_row)) = (
            1_u32.checked_shl(u32::try_from(col).unwrap_or(0)),
            1_u32.checked_shl(u32::try_from(row).unwrap_or(0)),
        ) else {
            return;
        };
        if accept {
            self.allow[row] |= bit_col;
            self.allow[col] |= bit_row;
        } else {
            self.allow[row] &= !bit_col;
            self.allow[col] &= !bit_row;
        }
    }
}

fn layer_index(layer: u16) -> Option<usize> {
    let index = usize::from(layer);
    (index < LAYER_CAP).then_some(index)
}

#[derive(Clone, Copy, Debug)]
struct Node {
    min: [f32; 3],
    max: [f32; 3],
    child1: u32,
    child2: u32,
    parent: u32,
    body: u32,
    height: u16,
}

impl Node {
    fn leaf(body: u32, min: [f32; 3], max: [f32; 3]) -> Self {
        Self {
            min,
            max,
            child1: NONE,
            child2: NONE,
            parent: NONE,
            body,
            height: 0,
        }
    }

    fn is_leaf(self) -> bool {
        self.body != NONE
    }
}

#[derive(Clone, Debug)]
struct Tree {
    nodes: Vec<Node>,
    root: u32,
    leaf_of: Vec<u32>,
}

impl Tree {
    const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: NONE,
            leaf_of: Vec::new(),
        }
    }

    fn node_count(&self) -> u32 {
        u32::try_from(self.nodes.len()).unwrap_or(u32::MAX)
    }

    fn clear(&mut self) {
        self.nodes.clear();
        self.root = NONE;
        self.leaf_of.fill(NONE);
    }

    fn ensure(&mut self, bodies: usize) {
        if self.leaf_of.len() < bodies {
            self.leaf_of.resize(bodies, NONE);
        }
    }

    fn contains(&self, index: usize) -> bool {
        self.leaf_of.get(index).copied().unwrap_or(NONE) != NONE
    }

    fn leaf_bounds(&self, body: u32) -> Option<([f32; 3], [f32; 3])> {
        let leaf = self.leaf_of.get(usize_of(body)).copied()?;
        if leaf == NONE {
            return None;
        }
        let node = self.nodes.get(usize_of(leaf))?;
        Some((node.min, node.max))
    }

    fn rebuild(&mut self, leaves: &mut [Leaf]) {
        self.nodes.clear();
        self.root = NONE;
        if leaves.is_empty() {
            return;
        }
        leaves.sort_by(|left, right| {
            left.min[0]
                .total_cmp(&right.min[0])
                .then(left.body.cmp(&right.body))
        });
        self.root = self.build_range(leaves);
    }

    fn build_range(&mut self, leaves: &[Leaf]) -> u32 {
        if leaves.len() == 1 {
            let leaf = leaves[0];
            return self.push(Node::leaf(leaf.body, leaf.min, leaf.max));
        }
        let mid = leaves.len() / 2;
        let left = self.build_range(&leaves[..mid]);
        let right = self.build_range(&leaves[mid..]);
        self.join(left, right)
    }

    fn insert_leaf(&mut self, leaf: Leaf) {
        let node = self.push(Node::leaf(leaf.body, leaf.min, leaf.max));
        if self.root == NONE {
            self.root = node;
            return;
        }
        let sibling = self.pick_sibling(leaf.min, leaf.max);
        let old_parent = self.nodes[usize_of(sibling)].parent;
        let parent = self.join(sibling, node);
        self.nodes[usize_of(parent)].parent = old_parent;
        if old_parent == NONE {
            self.root = parent;
            return;
        }
        let replaced = &mut self.nodes[usize_of(old_parent)];
        if replaced.child1 == sibling {
            replaced.child1 = parent;
        } else if replaced.child2 == sibling {
            replaced.child2 = parent;
        }
        let mut cursor = old_parent;
        while cursor != NONE {
            let child1 = self.nodes[usize_of(cursor)].child1;
            let child2 = self.nodes[usize_of(cursor)].child2;
            let (min_box, max_box) = union_nodes(&self.nodes, child1, child2);
            let height = self.nodes[usize_of(child1)]
                .height
                .max(self.nodes[usize_of(child2)].height)
                .saturating_add(1);
            let node = &mut self.nodes[usize_of(cursor)];
            node.min = min_box;
            node.max = max_box;
            node.height = height;
            cursor = node.parent;
        }
    }

    fn pick_sibling(&self, min: [f32; 3], max: [f32; 3]) -> u32 {
        let mut index = self.root;
        loop {
            let node = self.nodes[usize_of(index)];
            if node.is_leaf() {
                return index;
            }
            let left = node.child1;
            let right = node.child2;
            let cost_left = area(union(
                self.nodes[usize_of(left)].min,
                self.nodes[usize_of(left)].max,
                min,
                max,
            ));
            let cost_right = area(union(
                self.nodes[usize_of(right)].min,
                self.nodes[usize_of(right)].max,
                min,
                max,
            ));
            index = match cost_left.total_cmp(&cost_right) {
                std::cmp::Ordering::Less => left,
                std::cmp::Ordering::Greater => right,
                std::cmp::Ordering::Equal => left.min(right),
            };
        }
    }

    fn expand(&mut self, body: u32, min: [f32; 3], max: [f32; 3]) -> bool {
        let Some(leaf) = self.leaf_of.get(usize_of(body)).copied() else {
            return false;
        };
        if leaf == NONE {
            return false;
        }
        let node = &mut self.nodes[usize_of(leaf)];
        node.min = vmin(node.min, min);
        node.max = vmax(node.max, max);
        let mut parent = node.parent;
        while parent != NONE {
            let child1 = self.nodes[usize_of(parent)].child1;
            let child2 = self.nodes[usize_of(parent)].child2;
            let (min_box, max_box) = union_nodes(&self.nodes, child1, child2);
            let node = &mut self.nodes[usize_of(parent)];
            node.min = min_box;
            node.max = max_box;
            parent = node.parent;
        }
        true
    }

    fn query(&self, min: [f32; 3], max: [f32; 3], stack: &mut Vec<u32>, out: &mut Vec<u32>) {
        out.clear();
        if self.root == NONE {
            return;
        }
        stack.clear();
        stack.push(self.root);
        while let Some(index) = stack.pop() {
            let node = self.nodes[usize_of(index)];
            if !overlaps(node.min, node.max, min, max) {
                continue;
            }
            if node.is_leaf() {
                out.push(node.body);
            } else {
                stack.push(node.child1);
                stack.push(node.child2);
            }
        }
    }

    fn push(&mut self, node: Node) -> u32 {
        let index = u32::try_from(self.nodes.len()).unwrap_or(NONE);
        self.nodes.push(node);
        if node.is_leaf() {
            self.note_leaf(node.body, index);
        }
        index
    }

    fn note_leaf(&mut self, body: u32, node: u32) {
        let index = usize_of(body);
        if self.leaf_of.len() <= index {
            self.leaf_of.resize(index + 1, NONE);
        }
        self.leaf_of[index] = node;
    }

    fn join(&mut self, child1: u32, child2: u32) -> u32 {
        let (min, max) = union_nodes(&self.nodes, child1, child2);
        let height = self.nodes[usize_of(child1)]
            .height
            .max(self.nodes[usize_of(child2)].height)
            .saturating_add(1);
        let parent = self.push(Node {
            min,
            max,
            child1,
            child2,
            parent: NONE,
            body: NONE,
            height,
        });
        self.nodes[usize_of(child1)].parent = parent;
        self.nodes[usize_of(child2)].parent = parent;
        parent
    }
}

#[derive(Clone, Copy, Debug)]
struct Leaf {
    body: u32,
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Broadphase {
    static_tree: Tree,
    dynamic_tree: Tree,
    leaves: Vec<Leaf>,
    stack: Vec<u32>,
    found: Vec<u32>,
    pub displaced: Vec<u32>,
    pub(crate) static_dirty: bool,
    static_settled: bool,
    pub static_rebuilds: u32,
    dynamics: Vec<u32>,
    planes: Vec<u32>,
    sources: Vec<u32>,
}

impl Broadphase {
    pub const fn new() -> Self {
        Self {
            static_tree: Tree::new(),
            dynamic_tree: Tree::new(),
            leaves: Vec::new(),
            stack: Vec::new(),
            found: Vec::new(),
            displaced: Vec::new(),
            static_dirty: false,
            static_settled: false,
            static_rebuilds: 0,
            dynamics: Vec::new(),
            planes: Vec::new(),
            sources: Vec::new(),
        }
    }

    pub fn dynamic_count(&self) -> usize {
        self.dynamics.len()
    }

    pub fn dynamic_at(&self, slot: usize) -> u32 {
        self.dynamics.get(slot).copied().unwrap_or(u32::MAX)
    }

    pub fn dynamic_ids(&self) -> &[u32] {
        &self.dynamics
    }

    fn note(&mut self, bodies: &[Body], index: usize) {
        let Some(body) = bodies.get(index) else {
            return;
        };
        let Ok(id) = u32::try_from(index) else {
            return;
        };
        if body.presence != Presence::Live {
            return;
        }
        if matches!(body.shape, crate::Shape::Plane { .. }) {
            self.planes.push(id);
        } else if body.broad_phase_layer != 0 {
            self.dynamics.push(id);
        }
    }

    /// Rebuild a dirty static tree. The dynamic tree is rebuilt only when some
    /// dynamic body is awake or a body was moved. A fully asleep island keeps
    /// the previous tight tree and is not queried.
    pub fn prepare(&mut self, bodies: &[Body]) {
        if self.static_dirty {
            self.static_rebuilds = self.static_rebuilds.saturating_add(1);
            self.rebuild_kind(bodies, true);
            self.static_dirty = false;
            self.static_settled = true;
        } else if !self.static_settled && self.static_tree.root == NONE {
            let bounded = bodies
                .iter()
                .enumerate()
                .any(|(index, body)| body.broad_phase_layer == 0 && leaf_of(body, index).is_some());
            if bounded {
                self.static_rebuilds = self.static_rebuilds.saturating_add(1);
                self.rebuild_kind(bodies, true);
            }
            self.static_settled = true;
        }
        let awake = self.dynamics.iter().any(|&id| {
            bodies.get(usize_of(id)).is_some_and(|body| {
                body.presence == Presence::Live && body.broad_phase_layer != 0 && !body.sleeping
            })
        }) || !self.displaced.is_empty();
        if !awake {
            return;
        }
        self.leaves.clear();
        self.dynamic_tree.ensure(bodies.len());
        self.dynamic_tree.clear();
        self.dynamic_tree.ensure(bodies.len());
        for &id in &self.dynamics {
            let index = usize_of(id);
            let Some(body) = bodies.get(index) else {
                continue;
            };
            if let Some(leaf) = leaf_of(body, index) {
                self.leaves.push(leaf);
            }
        }
        let leaves = &mut self.leaves;
        self.dynamic_tree.rebuild(leaves);
    }

    pub fn node_count(&self) -> u32 {
        self.static_tree
            .node_count()
            .saturating_add(self.dynamic_tree.node_count())
    }

    pub fn in_static(&self, index: usize) -> bool {
        self.static_tree.contains(index)
    }

    pub fn in_dynamic(&self, index: usize) -> bool {
        self.dynamic_tree.contains(index)
    }

    pub fn insert_one(&mut self, bodies: &[Body], index: usize) {
        let Some(body) = bodies.get(index) else {
            return;
        };
        if body.presence != Presence::Live {
            return;
        }
        self.note(bodies, index);
        let Some(leaf) = leaf_of(body, index) else {
            return;
        };
        let tree = if body.broad_phase_layer == 0 {
            &mut self.static_tree
        } else {
            &mut self.dynamic_tree
        };
        tree.ensure(bodies.len());
        tree.insert_leaf(leaf);
    }

    pub fn insert_batch(&mut self, bodies: &[Body]) {
        self.rebuild(bodies);
    }

    /// Move one body between the static tree, the dynamic tree, and the plane list.
    pub fn retarget(&mut self, bodies: &[Body], index: usize) {
        let Ok(id) = u32::try_from(index) else {
            return;
        };
        self.dynamics.retain(|stored| *stored != id);
        self.planes.retain(|stored| *stored != id);
        self.static_dirty = true;
        self.static_settled = false;
        self.note(bodies, index);
    }

    pub fn forget(&mut self, index: usize) {
        let Ok(id) = u32::try_from(index) else {
            return;
        };
        self.dynamics.retain(|stored| *stored != id);
        self.planes.retain(|stored| *stored != id);
        self.static_dirty = true;
    }

    pub fn rebuild(&mut self, bodies: &[Body]) {
        self.dynamics.clear();
        self.planes.clear();
        for index in 0..bodies.len() {
            self.note(bodies, index);
        }
        self.static_tree.ensure(bodies.len());
        self.dynamic_tree.ensure(bodies.len());
        self.static_tree.clear();
        self.dynamic_tree.clear();
        self.static_tree.ensure(bodies.len());
        self.dynamic_tree.ensure(bodies.len());
        self.leaves.clear();
        for (index, body) in bodies.iter().enumerate() {
            if body.broad_phase_layer == 0 {
                if let Some(leaf) = leaf_of(body, index) {
                    self.leaves.push(leaf);
                }
            }
        }
        self.static_tree.rebuild(&mut self.leaves);
        self.leaves.clear();
        for (index, body) in bodies.iter().enumerate() {
            if body.broad_phase_layer != 0 {
                if let Some(leaf) = leaf_of(body, index) {
                    self.leaves.push(leaf);
                }
            }
        }
        self.dynamic_tree.rebuild(&mut self.leaves);
        self.static_dirty = false;
        self.static_settled = true;
    }

    fn rebuild_kind(&mut self, bodies: &[Body], static_layer: bool) {
        let tree = if static_layer {
            &mut self.static_tree
        } else {
            &mut self.dynamic_tree
        };
        tree.ensure(bodies.len());
        tree.clear();
        tree.ensure(bodies.len());
        self.leaves.clear();
        for (index, body) in bodies.iter().enumerate() {
            let is_static = body.broad_phase_layer == 0;
            if is_static != static_layer {
                continue;
            }
            if let Some(leaf) = leaf_of(body, index) {
                self.leaves.push(leaf);
            }
        }
        let leaves = &mut self.leaves;
        let tree = if static_layer {
            &mut self.static_tree
        } else {
            &mut self.dynamic_tree
        };
        tree.rebuild(leaves);
    }

    pub fn expand(&mut self, bodies: &[Body], index: usize) {
        let Some(body) = bodies.get(index) else {
            return;
        };
        let Some(leaf) = leaf_of(body, index) else {
            return;
        };
        let tree = if body.broad_phase_layer == 0 {
            &mut self.static_tree
        } else {
            &mut self.dynamic_tree
        };
        if !tree.expand(leaf.body, leaf.min, leaf.max) {
            tree.insert_leaf(leaf);
        }
    }

    /// Bounds pairs for this step. Static-static pairs are not emitted.
    pub fn fill(
        &mut self,
        bodies: &[Body],
        layers: &LayerPairs,
        pairs: &mut Vec<(u32, u32)>,
        planes: &mut Vec<u32>,
    ) {
        pairs.clear();
        planes.clear();
        self.leaves.clear();
        planes.extend_from_slice(&self.planes);
        let displaced = std::mem::take(&mut self.displaced);
        self.sources.clear();
        self.sources.extend_from_slice(&self.dynamics);
        for id in &displaced {
            if !self.sources.contains(id) {
                self.sources.push(*id);
            }
        }
        self.displaced = displaced;
        let count = self.sources.len();
        for slot in 0..count {
            let id = self.sources[slot];
            let index = usize_of(id);
            let Some(body) = bodies.get(index) else {
                continue;
            };
            let moved = self.displaced.contains(&id);
            if body.presence != Presence::Live {
                continue;
            }
            if body.broad_phase_layer == 0 && !moved {
                continue;
            }
            if body.sleeping && !moved {
                continue;
            }
            let Some(tight) = leaf_of(body, index) else {
                continue;
            };
            let (min_box, max_box) = if body.broad_phase_layer == 0 {
                self.static_tree
                    .leaf_bounds(tight.body)
                    .unwrap_or((tight.min, tight.max))
            } else {
                self.dynamic_tree
                    .leaf_bounds(tight.body)
                    .unwrap_or((tight.min, tight.max))
            };
            let id = tight.body;
            self.static_tree
                .query(min_box, max_box, &mut self.stack, &mut self.found);
            push_hits(bodies, layers, id, &self.found, pairs);
            self.dynamic_tree
                .query(min_box, max_box, &mut self.stack, &mut self.found);
            push_hits(bodies, layers, id, &self.found, pairs);
        }
        for &plane in planes.iter() {
            for slot in 0..self.sources.len() {
                let id = self.sources[slot];
                if id == plane {
                    continue;
                }
                let Some(body) = bodies.get(usize_of(id)) else {
                    continue;
                };
                if body.presence != Presence::Live
                    || matches!(body.shape, crate::Shape::Plane { .. })
                {
                    continue;
                }
                if accept_pair(bodies, layers, id, plane) {
                    pairs.push((id.min(plane), id.max(plane)));
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
    }
}

fn push_hits(
    bodies: &[Body],
    layers: &LayerPairs,
    id: u32,
    found: &[u32],
    pairs: &mut Vec<(u32, u32)>,
) {
    for &other in found {
        if other == id {
            continue;
        }
        // Each dynamic-dynamic pair is emitted by the lower id only.
        if other < id {
            let Some(body) = bodies.get(usize_of(other)) else {
                continue;
            };
            // The lower dynamic id emits the pair when it queries. A sleeper
            // does not query, so the waker has to emit it.
            if body.broad_phase_layer != 0 && !body.sleeping {
                continue;
            }
        }
        if accept_pair(bodies, layers, id, other) {
            pairs.push((id.min(other), id.max(other)));
        }
    }
}

fn accept_pair(bodies: &[Body], layers: &LayerPairs, a: u32, b: u32) -> bool {
    let (Some(left), Some(right)) = (bodies.get(usize_of(a)), bodies.get(usize_of(b))) else {
        return false;
    };
    if left.presence != Presence::Live || right.presence != Presence::Live {
        return false;
    }
    if left.inverse_mass == 0.0 && right.inverse_mass == 0.0 {
        return false;
    }
    if left.debris && right.debris {
        return false;
    }
    layers.allows(object_layer_of(left), object_layer_of(right))
}

pub fn object_layer_of(body: &Body) -> u16 {
    if body.object_layer != u16::MAX {
        return body.object_layer;
    }
    if body.debris {
        return 2;
    }
    u16::from(body.inverse_mass != 0.0)
}

fn leaf_of(body: &Body, index: usize) -> Option<Leaf> {
    if body.presence != Presence::Live {
        return None;
    }
    let (lo, hi) = contact::bounds(body)?;
    let margin = BOUNDS_MARGIN;
    let min = [lo.x - margin, lo.y - margin, lo.z - margin];
    let max = [hi.x + margin, hi.y + margin, hi.z + margin];
    Some(Leaf {
        body: u32::try_from(index).ok()?,
        min,
        max,
    })
}

fn usize_of(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

fn vmin(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])]
}

fn vmax(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
}

fn union(
    min_a: [f32; 3],
    max_a: [f32; 3],
    min_b: [f32; 3],
    max_b: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    (vmin(min_a, min_b), vmax(max_a, max_b))
}

fn union_nodes(nodes: &[Node], left: u32, right: u32) -> ([f32; 3], [f32; 3]) {
    let a = &nodes[usize_of(left)];
    let b = &nodes[usize_of(right)];
    union(a.min, a.max, b.min, b.max)
}

fn area(bounds: ([f32; 3], [f32; 3])) -> f32 {
    let dx = bounds.1[0] - bounds.0[0];
    let dy = bounds.1[1] - bounds.0[1];
    let dz = bounds.1[2] - bounds.0[2];
    2.0 * (dx * dy + dy * dz + dz * dx)
}

fn overlaps(min_a: [f32; 3], max_a: [f32; 3], min_b: [f32; 3], max_b: [f32; 3]) -> bool {
    min_a[0] <= max_b[0]
        && min_b[0] <= max_a[0]
        && min_a[1] <= max_b[1]
        && min_b[1] <= max_a[1]
        && min_a[2] <= max_b[2]
        && min_b[2] <= max_a[2]
}

pub fn sync_layer(body: &mut Body) {
    body.broad_phase_layer = u8::from(object_layer_of(body) != 0);
}
