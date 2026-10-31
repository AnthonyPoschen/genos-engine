//! Light-tree proposal for `ReSTIR` DI.
//!
//! The tree is a binary BVH over point lamps. Each node stores an axis-aligned
//! box, the luminance of the radiant intensity under it, and the radius of the
//! box's bounding sphere. A candidate draws one leaf by walking from the root.
//! At each internal node one uniform number picks a child with probability
//! proportional to a conservative importance.
//!
//! The importance follows Conty Estevez and Kulla, "Importance Sampling of
//! Many Lights with Adaptive Tree Splitting", HPG 2018, and the cluster bound
//! in Estevez and Kulla (the same authors' light-bounds notes): flux times a
//! horizon-clipped cosine over the squared distance to the cluster sphere.
//! Point lamps are isotropic, so there is no emission cone. The receiver normal
//! supplies the cosine bound.
//!
//! Departures from that paper:
//! - One leaf per candidate. The paper's adaptive split emits several samples
//!   from one node. This walk does not.
//! - Flux is the Rec. 709 luminance of radiant intensity, not integrated power.
//! - Distance is to the closest point of the node box (1 mm floor inside it).
//!   The bounding sphere is only the cosine cone. A shading point inside that
//!   sphere uses no cosine.
//! - Square roots are four Newton steps from a shared bit guess, so the CPU
//!   walk and the lavapipe walk consume the same bits.

use genos_math::Vec3;

use crate::reservoir::LAMP_EMPTY;
use crate::rng::Rng;
use crate::shade::{luminance, Lamp, Receiver};

/// Words per node in the GPU buffer. Layout is in [`LightTree::pack_words`].
pub const NODE_WORDS: usize = 12;

/// Distance floor (metres) when the shading point sits inside a cluster sphere.
const MIN_DIST: f32 = 1.0e-3;

/// Steps of `next_f32` (16 bits). The GPU copy is `TREE_STEPS`.
const STEPS: f32 = 65536.0;

/// Left share of a split in `next_f32` steps, rounded. 0 or `STEPS` when a
/// side has no importance. The GPU copy is `tree_split_steps`.
#[must_use]
pub fn split_steps(left_i: f32, right_i: f32, sum: f32) -> f32 {
    if right_i <= 0.0 {
        return STEPS;
    }
    if left_i <= 0.0 {
        return 0.0;
    }
    (left_i / sum * STEPS + 0.5).floor().clamp(1.0, STEPS - 1.0)
}

/// Fault injected into the node importance. The walk still covers every lamp
/// with positive power, so the RIS weight stays unbiased. The draws change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TreeFault {
    /// Conservative bound, including inverse-square distance.
    #[default]
    None,
    /// Drop the inverse-square term. Near and far clusters of equal flux tie.
    IgnoreDistance,
}

impl TreeFault {
    /// CLI name. `none` is the default and is what a dump omits as a fault.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::IgnoreDistance => "ignore-distance",
        }
    }

    /// Parses the CLI and the dump.
    ///
    /// # Errors
    ///
    /// Returns a string when `name` is not a known fault.
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "none" => Ok(Self::None),
            "ignore-distance" => Ok(Self::IgnoreDistance),
            other => Err(format!("unknown tree fault {other}")),
        }
    }
}

/// One BVH node. A leaf names its lamp. An internal node names its children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub min: Vec3,
    pub max: Vec3,
    pub power: f32,
    pub radius: f32,
    pub left: u32,
    pub right: u32,
    pub lamp: u32,
}

impl Node {
    fn blank() -> Self {
        Self {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            power: 0.0,
            radius: 0.0,
            left: LAMP_EMPTY,
            right: LAMP_EMPTY,
            lamp: LAMP_EMPTY,
        }
    }

    fn leaf(position: Vec3, power: f32, lamp: u32) -> Self {
        Self {
            min: position,
            max: position,
            power,
            radius: 0.0,
            left: LAMP_EMPTY,
            right: LAMP_EMPTY,
            lamp,
        }
    }

    fn is_leaf(self) -> bool {
        self.lamp != LAMP_EMPTY
    }
}

/// BVH over the scene lamps. Root is node 0. Built once per lamp set.
#[derive(Clone, Debug, PartialEq)]
pub struct LightTree {
    nodes: Vec<Node>,
}

struct Prim {
    lamp: u32,
    position: Vec3,
    power: f32,
}

impl LightTree {
    /// Median split on the longest axis of the lamp positions. Lamp index breaks ties.
    #[must_use]
    pub fn build(lamps: &[Lamp]) -> Self {
        let mut prims = Vec::with_capacity(lamps.len());
        for (index, lamp) in lamps.iter().enumerate() {
            let Some(lamp_id) = u32::try_from(index).ok() else {
                break;
            };
            let power = luminance(lamp.intensity).max(0.0);
            prims.push(Prim {
                lamp: lamp_id,
                position: lamp.position,
                power,
            });
        }
        let mut nodes = Vec::new();
        if !prims.is_empty() {
            let _ = build_range(&mut prims, &mut nodes);
        }
        Self { nodes }
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// std430-friendly words. Twelve words per node:
    /// min.xyz, power, max.xyz, radius, left, right, lamp, pad.
    #[must_use]
    pub fn pack_words(&self) -> Vec<u32> {
        let mut words = Vec::with_capacity(self.nodes.len().saturating_mul(NODE_WORDS));
        for node in &self.nodes {
            push_f32(&mut words, node.min.x);
            push_f32(&mut words, node.min.y);
            push_f32(&mut words, node.min.z);
            push_f32(&mut words, node.power);
            push_f32(&mut words, node.max.x);
            push_f32(&mut words, node.max.y);
            push_f32(&mut words, node.max.z);
            push_f32(&mut words, node.radius);
            words.push(node.left);
            words.push(node.right);
            words.push(node.lamp);
            words.push(0);
        }
        words
    }

    /// One leaf and its proposal probability. Consumes one `f32` per internal node.
    /// A root leaf consumes none. An empty tree consumes none.
    #[must_use]
    pub fn sample(
        &self,
        receiver: &Receiver,
        fault: TreeFault,
        rng: &mut Rng,
    ) -> Option<(usize, f32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut node_index = 0u32;
        let mut pdf = 1.0f32;
        for _ in 0..64 {
            let Some(node) = self.node_at(node_index) else {
                let _ = rng.next_f32();
                return None;
            };
            if node.is_leaf() {
                return usize::try_from(node.lamp).ok().map(|lamp| (lamp, pdf));
            }
            let (left, right) = (node.left, node.right);
            if self.node_at(left).is_none() || self.node_at(right).is_none() {
                let _ = rng.next_f32();
                return None;
            }
            let left_i = self.importance(left, receiver, fault);
            let right_i = self.importance(right, receiver, fault);
            let sum = left_i + right_i;
            if sum <= 0.0 {
                let _ = rng.next_f32();
                return None;
            }
            // next_f32 has 65536 steps. Quantize the left share to the same
            // steps and charge the pdf that share, so the pdf is the real
            // probability: a branch below one step was picked 1/65536 of the
            // time while its pdf said far less, and phat/pdf blew up.
            let left_q = split_steps(left_i, right_i, sum);
            let draw = rng.next_f32() * STEPS;
            if draw < left_q {
                pdf *= left_q / STEPS;
                node_index = left;
            } else {
                pdf *= (STEPS - left_q) / STEPS;
                node_index = right;
            }
        }
        None
    }

    /// Conservative importance of one node at `receiver`.
    #[must_use]
    pub fn importance(&self, index: u32, receiver: &Receiver, fault: TreeFault) -> f32 {
        let Some(node) = self.node_at(index) else {
            return 0.0;
        };
        node_importance(node, receiver, fault)
    }

    fn node_at(&self, index: u32) -> Option<Node> {
        let slot = usize::try_from(index).ok()?;
        self.nodes.get(slot).copied()
    }
}

/// Same arithmetic as `restir_tree.glsl`. Public so a test can name the bound.
#[allow(
    clippy::manual_midpoint,
    reason = "the lavapipe walk uses 0.5 * (min + max), and midpoint is a different rounding"
)]
#[allow(
    clippy::similar_names,
    reason = "the three box gaps are the x y z components of one vector"
)]
#[must_use]
pub fn node_importance(node: Node, receiver: &Receiver, fault: TreeFault) -> f32 {
    if node.power <= 0.0 {
        return 0.0;
    }
    let center_x = 0.5 * (node.min.x + node.max.x);
    let center_y = 0.5 * (node.min.y + node.max.y);
    let center_z = 0.5 * (node.min.z + node.max.z);
    let dx = center_x - receiver.position.x;
    let dy = center_y - receiver.position.y;
    let dz = center_z - receiver.position.z;
    let dist2 = 0.0 + dx * dx + dy * dy + dz * dz;
    let dist = sqrt_f32(dist2);
    // Closest point on the box. Lamps lie in the box, so this is a lower bound
    // on distance. The sphere is only the cosine cone: a fat sphere around the
    // box would mark a receiver under the box as inside and send 1/d² to the floor.
    let gap_x = axis_gap(receiver.position.x, node.min.x, node.max.x);
    let gap_y = axis_gap(receiver.position.y, node.min.y, node.max.y);
    let gap_z = axis_gap(receiver.position.z, node.min.z, node.max.z);
    let mut box_dist2 = 0.0 + gap_x * gap_x + gap_y * gap_y + gap_z * gap_z;
    let floor2 = MIN_DIST * MIN_DIST;
    if box_dist2 < floor2 {
        box_dist2 = floor2;
    }
    let geom = match fault {
        TreeFault::IgnoreDistance => 1.0,
        TreeFault::None => 1.0 / box_dist2,
    };
    let radius2 = node.radius * node.radius;
    if dist2 <= radius2 {
        return node.power * geom;
    }
    let inv = 1.0 / dist;
    let cos_alpha = 0.0
        + receiver.normal.x * (dx * inv)
        + receiver.normal.y * (dy * inv)
        + receiver.normal.z * (dz * inv);
    let mut sin_theta = node.radius * inv;
    if sin_theta > 1.0 {
        sin_theta = 1.0;
    }
    let cos_theta = sqrt_f32(1.0 - sin_theta * sin_theta);
    let sin_alpha = sqrt_f32(1.0 - cos_alpha * cos_alpha);
    let cos_bound = cos_alpha * cos_theta + sin_alpha * sin_theta;
    if cos_bound <= 0.0 {
        return 0.0;
    }
    node.power * cos_bound * geom
}

/// Four Newton steps. The GPU copy is `tree_sqrt` in `restir_tree.glsl`.
#[allow(
    clippy::manual_midpoint,
    reason = "each Newton step is 0.5 * (y + x/y), matching the shader"
)]
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "NaN takes the shader path !(value > 0)"
)]
#[must_use]
pub fn sqrt_f32(value: f32) -> f32 {
    if !(value > 0.0) {
        return 0.0;
    }
    let bits = value.to_bits();
    let guess = (bits >> 1).wrapping_add(127_u32 << 22);
    let mut y = f32::from_bits(guess);
    y = 0.5 * (y + value / y);
    y = 0.5 * (y + value / y);
    y = 0.5 * (y + value / y);
    y = 0.5 * (y + value / y);
    if y.is_finite() {
        y
    } else {
        0.0
    }
}

fn push_f32(words: &mut Vec<u32>, value: f32) {
    words.push(value.to_bits());
}

fn build_range(prims: &mut [Prim], nodes: &mut Vec<Node>) -> u32 {
    let index = u32::try_from(nodes.len()).unwrap_or(u32::MAX);
    nodes.push(Node::blank());
    if prims.len() <= 1 {
        if let Some(prim) = prims.first() {
            let slot = usize::try_from(index).unwrap_or(0);
            if let Some(place) = nodes.get_mut(slot) {
                *place = Node::leaf(prim.position, prim.power, prim.lamp);
            }
        }
        return index;
    }
    let mid = split(prims);
    let left = build_range(&mut prims[..mid], nodes);
    let right = build_range(&mut prims[mid..], nodes);
    let built = join(nodes, left, right);
    let slot = usize::try_from(index).unwrap_or(0);
    if let Some(place) = nodes.get_mut(slot) {
        *place = built;
    }
    index
}

#[allow(
    clippy::manual_midpoint,
    reason = "the stored centre is 0.5 * (min + max), matching the shader load"
)]
fn join(nodes: &[Node], left: u32, right: u32) -> Node {
    let left_node = node_ref(nodes, left);
    let right_node = node_ref(nodes, right);
    let min = Vec3::new(
        min2(left_node.min.x, right_node.min.x),
        min2(left_node.min.y, right_node.min.y),
        min2(left_node.min.z, right_node.min.z),
    );
    let max = Vec3::new(
        max2(left_node.max.x, right_node.max.x),
        max2(left_node.max.y, right_node.max.y),
        max2(left_node.max.z, right_node.max.z),
    );
    let center_x = 0.5 * (min.x + max.x);
    let center_y = 0.5 * (min.y + max.y);
    let center_z = 0.5 * (min.z + max.z);
    let ex = max.x - center_x;
    let ey = max.y - center_y;
    let ez = max.z - center_z;
    let radius = sqrt_f32(0.0 + ex * ex + ey * ey + ez * ez);
    Node {
        min,
        max,
        power: left_node.power + right_node.power,
        radius,
        left,
        right,
        lamp: LAMP_EMPTY,
    }
}

fn node_ref(nodes: &[Node], index: u32) -> Node {
    usize::try_from(index)
        .ok()
        .and_then(|slot| nodes.get(slot).copied())
        .unwrap_or_else(Node::blank)
}

fn split(prims: &mut [Prim]) -> usize {
    let Some(first) = prims.first() else {
        return 0;
    };
    let mut lo = first.position;
    let mut hi = first.position;
    for prim in prims.iter().skip(1) {
        lo.x = min2(lo.x, prim.position.x);
        lo.y = min2(lo.y, prim.position.y);
        lo.z = min2(lo.z, prim.position.z);
        hi.x = max2(hi.x, prim.position.x);
        hi.y = max2(hi.y, prim.position.y);
        hi.z = max2(hi.z, prim.position.z);
    }
    let ex = hi.x - lo.x;
    let ey = hi.y - lo.y;
    let ez = hi.z - lo.z;
    let axis = if ex >= ey && ex >= ez {
        0
    } else if ey >= ez {
        1
    } else {
        2
    };
    prims.sort_by(|left, right| {
        let order = axis_of(left, axis).total_cmp(&axis_of(right, axis));
        if order == std::cmp::Ordering::Equal {
            left.lamp.cmp(&right.lamp)
        } else {
            order
        }
    });
    let mid = prims.len() / 2;
    mid.clamp(1, prims.len().saturating_sub(1))
}

fn axis_of(prim: &Prim, axis: u8) -> f32 {
    match axis {
        0 => prim.position.x,
        1 => prim.position.y,
        _ => prim.position.z,
    }
}

fn axis_gap(point: f32, min: f32, max: f32) -> f32 {
    if point < min {
        min - point
    } else if point > max {
        point - max
    } else {
        0.0
    }
}

fn min2(left: f32, right: f32) -> f32 {
    if left < right {
        left
    } else {
        right
    }
}

fn max2(left: f32, right: f32) -> f32 {
    if left > right {
        left
    } else {
        right
    }
}

#[cfg(test)]
mod tests {
    use super::{node_importance, LightTree, Node, TreeFault};
    use crate::shade::{Lamp, Receiver};
    use genos_math::Vec3;

    fn lamp(position: Vec3, power: f32) -> Lamp {
        Lamp {
            position,
            intensity: Vec3::new(power, power, power),
        }
    }

    fn receiver() -> Receiver {
        Receiver {
            position: Vec3::ZERO,
            normal: Vec3::Y,
            albedo: Vec3::new(1.0, 1.0, 1.0),
        }
    }

    #[test]
    fn a_pair_of_lamps_is_one_internal_node_over_two_leaves() {
        let tree = LightTree::build(&[
            lamp(Vec3::new(-1.0, 2.0, 0.0), 1.0),
            lamp(Vec3::new(1.0, 2.0, 0.0), 3.0),
        ]);
        assert_eq!(tree.node_count(), 3);
        let root = tree.nodes()[0];
        assert_eq!(root.lamp, crate::reservoir::LAMP_EMPTY);
        assert!(root.power > 0.0);
        let packed = tree.pack_words();
        assert_eq!(packed.len(), 3 * super::NODE_WORDS);
    }

    #[test]
    fn near_equal_flux_outranks_far_and_ignore_distance_flattens_that() {
        let near = Node::leaf(Vec3::new(0.0, 1.0, 0.0), 1.0, 0);
        let far = Node::leaf(Vec3::new(0.0, 8.0, 0.0), 1.0, 1);
        let shading = receiver();
        let near_i = node_importance(near, &shading, TreeFault::None);
        let far_i = node_importance(far, &shading, TreeFault::None);
        assert!(near_i > far_i * 4.0, "near {near_i} far {far_i}");
        let near_flat = node_importance(near, &shading, TreeFault::IgnoreDistance);
        let far_flat = node_importance(far, &shading, TreeFault::IgnoreDistance);
        let ratio = near_i / far_i;
        let flat_ratio = near_flat / far_flat;
        assert!(
            flat_ratio * 2.0 < ratio,
            "distance ratio {ratio} fault ratio {flat_ratio}"
        );
    }

    #[test]
    fn a_lamp_behind_the_receiver_has_zero_importance() {
        let below = Node::leaf(Vec3::new(0.0, -2.0, 0.0), 5.0, 0);
        let importance = node_importance(below, &receiver(), TreeFault::None);
        assert_eq!(importance, 0.0);
    }

    #[test]
    fn two_builds_pack_the_same_words() {
        let lamps = [
            lamp(Vec3::new(0.2, 1.0, -0.4), 0.5),
            lamp(Vec3::new(-1.0, 3.0, 0.2), 8.0),
            lamp(Vec3::new(2.0, 0.4, 1.0), 0.05),
            lamp(Vec3::new(0.0, 2.0, 2.0), 1.5),
        ];
        let left = LightTree::build(&lamps).pack_words();
        let right = LightTree::build(&lamps).pack_words();
        assert_eq!(left, right);
    }
}

#[cfg(test)]
mod bias {
    use std::fmt::Write;

    use super::TreeFault;
    use crate::harness::{evaluate, EvalSpec};
    use crate::metrics::GateLimits;
    use crate::reservoir::Weighting;
    use crate::restir::{Config, History, Proposal, Session, DEFAULT_M_CLAMP};
    use crate::shade::Scene;

    fn tree_config(temporal: bool, fault: TreeFault) -> Config {
        tree_candidates(8, temporal, fault)
    }

    fn tree_candidates(candidates: u32, temporal: bool, fault: TreeFault) -> Config {
        Config {
            candidates,
            spatial_neighbors: 0,
            spatial_radius: 2,
            temporal,
            weighting: Weighting::Balance,
            proposal: Proposal::Tree,
            history: History::Temporal,
            m_clamp: DEFAULT_M_CLAMP,
            tree_fault: fault,
        }
    }

    fn flat_config_temporal(proposal: Proposal) -> Config {
        let mut config = flat_config(proposal);
        config.temporal = true;
        config
    }

    fn flat_config(proposal: Proposal) -> Config {
        Config {
            candidates: 8,
            spatial_neighbors: 0,
            spatial_radius: 2,
            temporal: false,
            weighting: Weighting::Balance,
            proposal,
            history: History::Temporal,
            m_clamp: DEFAULT_M_CLAMP,
            tree_fault: TreeFault::None,
        }
    }

    fn scenes() -> Vec<(&'static str, Scene)> {
        vec![
            ("gate", Scene::gate()),
            ("shadow", Scene::shadow()),
            ("open", Scene::open_grid()),
            ("row", Scene::open_row()),
            ("domain", Scene::domain_pair()),
            ("variance", Scene::variance_point()),
            ("many", Scene::many_lights()),
        ]
    }

    #[test]
    fn many_lights_has_at_least_256_lamps() {
        assert!(Scene::many_lights().lamps.len() >= 256);
    }

    #[test]
    fn tree_mean_stays_within_the_plan_bias_tolerance() -> Result<(), String> {
        let mut limits = GateLimits::reference();
        limits.grid_worst = 10.0;
        limits.frames_to_ref = 8;
        let seeds: Vec<u64> = (1..=128).collect();
        for (name, scene) in scenes() {
            let mut line = format!("bias {name}");
            for (label, config) in [
                ("tree", tree_candidates(8, true, TreeFault::None)),
                ("target", flat_config_temporal(Proposal::Target)),
                ("uniform", flat_config_temporal(Proposal::Uniform)),
            ] {
                let spec = EvalSpec {
                    scene: scene.clone(),
                    config,
                    seeds: seeds.clone(),
                    frames: 8,
                    limits,
                };
                let (report, _) = evaluate(&spec)?;
                if label == "tree" {
                    assert!(
                        report.ref_err_near <= 0.05,
                        "{name} ref_err_near {} above 0.05",
                        report.ref_err_near
                    );
                }
                let _ = write!(line, " {label} {:.6}", report.ref_err_near);
            }
            println!("{line}");
        }
        Ok(())
    }

    #[test]
    fn tree_variance_is_below_the_uniform_proposal_on_many_lights() -> Result<(), String> {
        let mut limits = GateLimits::reference();
        limits.ref_err_near = 10.0;
        limits.grid_worst = 10.0;
        let seeds: Vec<u64> = (1..=48).collect();
        let mut many_tree = 0.0;
        let mut many_uniform = 0.0;
        let mut many_fault = 0.0;
        for (name, scene) in scenes() {
            let mut line = format!("var {name}");
            for (label, config) in [
                ("tree", tree_config(false, TreeFault::None)),
                ("target", flat_config(Proposal::Target)),
                ("uniform", flat_config(Proposal::Uniform)),
                ("fault", tree_config(false, TreeFault::IgnoreDistance)),
            ] {
                let spec = EvalSpec {
                    scene: scene.clone(),
                    config,
                    seeds: seeds.clone(),
                    frames: 1,
                    limits,
                };
                let (report, _) = evaluate(&spec)?;
                let _ = write!(line, " {label} {:.6}", report.variance_early);
                if name == "many" && label == "tree" {
                    many_tree = report.variance_early;
                }
                if name == "many" && label == "uniform" {
                    many_uniform = report.variance_early;
                }
                if name == "many" && label == "fault" {
                    many_fault = report.variance_early;
                }
            }
            println!("{line}");
        }
        let tree = many_tree;
        let uniform = many_uniform;
        let fault = many_fault;
        assert!(
            tree < uniform,
            "tree variance {tree} is not below uniform {uniform}"
        );
        assert!(
            fault > tree,
            "ignore-distance variance {fault} did not rise above the tree {tree}"
        );
        Ok(())
    }

    #[test]
    fn ignore_distance_fails_the_reservoir_diff_and_stays_unbiased() -> Result<(), String> {
        let scene = Scene::many_lights();
        let good = shaded(&scene, TreeFault::None, 1)?;
        let bad = shaded(&scene, TreeFault::IgnoreDistance, 1)?;
        let mismatches = good
            .iter()
            .zip(&bad)
            .filter(|(left, right)| left.lamp != right.lamp)
            .count();
        assert!(mismatches > 0, "ignore-distance matched the tree");
        let mut limits = GateLimits::reference();
        limits.grid_worst = 10.0;
        let spec = EvalSpec {
            scene,
            config: tree_config(true, TreeFault::IgnoreDistance),
            seeds: (1..=96).collect(),
            frames: 8,
            limits,
        };
        let (report, _) = evaluate(&spec)?;
        assert!(
            report.ref_err_near <= 0.05,
            "fault ref_err_near {}",
            report.ref_err_near
        );
        println!(
            "must-fail mismatches {mismatches} fault_bias {:.6}",
            report.ref_err_near
        );
        Ok(())
    }

    fn shaded(
        scene: &Scene,
        fault: TreeFault,
        seed: u64,
    ) -> Result<Vec<crate::reservoir::Reservoir>, String> {
        let mut session = Session::new(scene.clone(), tree_config(false, fault), seed)?;
        let _ = session.step();
        Ok(session.shaded().to_vec())
    }
}
