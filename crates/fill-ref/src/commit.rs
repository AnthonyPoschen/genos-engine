//! When the shown mean updates. The measured rule is the default.

use genos_math::Vec3;

use crate::deposit::Deposit;

/// What the age runs established. See the F1 experiment note.
/// The 1M shown mean is younger than flag off during the lamp move, so the
/// reference publishes the deposit in the step it finishes.
pub const MEASURED_COMMIT: CommitRule = CommitRule::Immediate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitRule {
    /// Publish the deposit in the step it finishes.
    Immediate,
    /// Hold the shown value until `quarters` deposits, then replace it.
    /// A lamp note publishes the running sum in that step.
    HoldQuarters { quarters: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub color: Vec3,
    pub own: u32,
    pub commit_frame: u32,
    held: u32,
    partial: Deposit,
}

impl Shown {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            color: Vec3::ZERO,
            own: 0,
            commit_frame: 0,
            held: 0,
            partial: Deposit {
                rgb: [0, 0, 0],
                weight: 0,
                own: 0,
                shared: 0,
            },
        }
    }
}

impl Default for Shown {
    fn default() -> Self {
        Self::new()
    }
}

/// The only writer of `shown`.
pub fn resolve(shown: &mut Shown, deposit: &Deposit, frame: u32, rule: CommitRule, lamp_note: bool) {
    shown.partial = merge(&shown.partial, deposit);
    shown.held = shown.held.saturating_add(1);
    let publish = match rule {
        CommitRule::Immediate => true,
        CommitRule::HoldQuarters { quarters } => lamp_note || shown.held >= quarters.max(1),
    };
    if !publish {
        return;
    }
    shown.color = crate::deposit::mean_rgb(&shown.partial);
    shown.own = shown.partial.own;
    shown.commit_frame = frame;
    shown.held = 0;
    shown.partial = Deposit::default();
}

fn merge(a: &Deposit, b: &Deposit) -> Deposit {
    Deposit {
        rgb: [
            a.rgb[0].saturating_add(b.rgb[0]),
            a.rgb[1].saturating_add(b.rgb[1]),
            a.rgb[2].saturating_add(b.rgb[2]),
        ],
        weight: a.weight.saturating_add(b.weight),
        own: a.own.saturating_add(b.own),
        shared: a.shared.saturating_add(b.shared),
    }
}

#[must_use]
pub fn resolve_matches(rule: CommitRule) -> bool {
    match rule {
        CommitRule::Immediate => immediate_publishes(),
        CommitRule::HoldQuarters { quarters } => hold_waits(quarters),
    }
}

fn one() -> Deposit {
    let mut dep = Deposit::default();
    crate::deposit::add_own(&mut dep, Vec3::new(1.0, 1.0, 1.0));
    dep
}

fn immediate_publishes() -> bool {
    let mut shown = Shown::new();
    resolve(&mut shown, &one(), 4, CommitRule::Immediate, false);
    shown.commit_frame == 4 && shown.color.x > 0.0
}

fn hold_waits(quarters: u32) -> bool {
    let mut shown = Shown::new();
    let rule = CommitRule::HoldQuarters { quarters };
    for frame in 1..quarters.max(1) {
        resolve(&mut shown, &one(), frame, rule, false);
        if shown.commit_frame != 0 {
            return false;
        }
    }
    resolve(&mut shown, &one(), quarters.max(1), rule, false);
    if shown.commit_frame != quarters.max(1) {
        return false;
    }
    let mut noted = Shown::new();
    resolve(&mut noted, &one(), 2, rule, true);
    noted.commit_frame == 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_publishes_the_step_and_four_quarters_wait_for_the_fourth() {
        assert!(immediate_publishes());
        assert!(hold_waits(4));
    }

    #[test]
    fn a_lamp_note_publishes_a_held_quarter() {
        let mut shown = Shown::new();
        let rule = CommitRule::HoldQuarters { quarters: 4 };
        resolve(&mut shown, &one(), 3, rule, true);
        assert_eq!(shown.commit_frame, 3);
        assert!(shown.color.x > 0.0);
    }
}
