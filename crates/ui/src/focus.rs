//! Which panel receives the wheel, the keys, and the pointer.
//!
//! A game sets one [`InputPolicy`] per panel and can replace it on a later frame.
//! Input the panel does not take is reported on [`Consumed`] so the game can
//! keep it.

use std::collections::BTreeMap;

use genos_scene::FocusPolicy;

/// Wheel and keys are separate. The debug menu hovers for the wheel and
/// click-focuses for keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputPolicy {
    pub wheel: FocusPolicy,
    pub keys: FocusPolicy,
}

impl Default for InputPolicy {
    fn default() -> Self {
        Self {
            wheel: FocusPolicy::Hover,
            keys: FocusPolicy::ClickToFocus,
        }
    }
}

/// The class of input a policy gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Pointer,
    Wheel,
    Keys,
}

/// Events this frame that the UI took. A false field falls through to the game.
/// Each flag is independent, so a frame can take the wheel and still leave the keys.
#[allow(
    clippy::struct_excessive_bools,
    reason = "pointer, wheel, keys, and blur are independent fall-through bits"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Consumed {
    pub pointer: bool,
    pub wheel: bool,
    pub keys: bool,
    /// Escape or grave was used to leave click-to-focus.
    pub blur: bool,
}

/// Remembers each panel's policy and which panel holds click-to-focus.
#[derive(Clone, Debug, Default)]
pub struct FocusManager {
    policies: BTreeMap<u32, InputPolicy>,
    focused: Option<u32>,
}

impl FocusManager {
    pub fn set_policy(&mut self, panel: u32, policy: InputPolicy) {
        self.policies.insert(panel, policy);
    }

    pub fn policy(&self, panel: u32) -> Option<InputPolicy> {
        self.policies.get(&panel).copied()
    }

    /// The panel that click-to-focus has armed. `None` when keys are not held that way.
    pub fn focused(&self) -> Option<u32> {
        self.focused
    }

    /// Arm or clear click-to-focus. `None` releases every panel.
    pub fn set_focus(&mut self, panel: Option<u32>) {
        self.focused = panel;
    }

    /// Whether `panel` receives `kind` this frame.
    ///
    /// A panel with no policy receives everything, which is the older path.
    /// `over` is true when the pointer is on that panel.
    pub fn allows(&self, panel: u32, kind: InputKind, over: bool) -> bool {
        let Some(policy) = self.policy(panel) else {
            return true;
        };
        let rule = match kind {
            InputKind::Wheel => policy.wheel,
            InputKind::Keys | InputKind::Pointer => policy.keys,
        };
        match rule {
            FocusPolicy::Passive => false,
            FocusPolicy::Modal => true,
            FocusPolicy::Hover => over,
            FocusPolicy::ClickToFocus => match kind {
                InputKind::Pointer => true,
                InputKind::Keys | InputKind::Wheel => self.focused == Some(panel),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unregistered_panel_allows_every_kind() {
        let manager = FocusManager::default();
        assert!(manager.allows(1, InputKind::Pointer, false));
        assert!(manager.allows(1, InputKind::Wheel, false));
        assert!(manager.allows(1, InputKind::Keys, false));
        assert_eq!(manager.focused(), None);
        assert_eq!(manager.policy(1), None);
    }

    #[test]
    fn each_policy_gates_the_kind_it_owns() {
        let mut manager = FocusManager::default();
        manager.set_policy(7, InputPolicy::default());
        assert_eq!(
            manager.policy(7),
            Some(InputPolicy {
                wheel: FocusPolicy::Hover,
                keys: FocusPolicy::ClickToFocus,
            })
        );
        assert!(manager.allows(7, InputKind::Wheel, true));
        assert!(!manager.allows(7, InputKind::Wheel, false));
        assert!(!manager.allows(7, InputKind::Keys, true));
        manager.set_focus(Some(7));
        assert_eq!(manager.focused(), Some(7));
        assert!(manager.allows(7, InputKind::Keys, false));
        assert!(manager.allows(7, InputKind::Pointer, false));

        manager.set_policy(
            7,
            InputPolicy {
                wheel: FocusPolicy::Passive,
                keys: FocusPolicy::Passive,
            },
        );
        assert!(!manager.allows(7, InputKind::Wheel, true));
        assert!(!manager.allows(7, InputKind::Keys, true));
        assert!(!manager.allows(7, InputKind::Pointer, true));

        manager.set_policy(
            7,
            InputPolicy {
                wheel: FocusPolicy::Modal,
                keys: FocusPolicy::Modal,
            },
        );
        assert!(manager.allows(7, InputKind::Keys, false));
        assert!(manager.allows(7, InputKind::Wheel, false));

        manager.set_policy(
            7,
            InputPolicy {
                wheel: FocusPolicy::Hover,
                keys: FocusPolicy::Hover,
            },
        );
        assert!(manager.allows(7, InputKind::Keys, true));
        assert!(!manager.allows(7, InputKind::Keys, false));
        manager.set_focus(None);
        assert_eq!(manager.focused(), None);
    }
}
